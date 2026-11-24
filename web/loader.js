// Loader: picks the wasm bundle matching the browser's GPU backend, boots it,
// and drives the frame loop.
//
// Backend selection mirrors render::QualityTier::select — the loader probes
// for a WebGPU adapter, then loads exactly one of the two builds produced by
// web/build.sh. A failed WebGPU boot falls back to the WebGL2 build once;
// failing both surfaces a clean error on the loading screen.
//
// Canvas sizing (playtest E6): the CSS keeps the canvas full-window; the
// backing store is window size x devicePixelRatio, with the DPR capped at 2
// for the 2015-iGPU fill budget, refreshed on resize. The renderer is built
// for the boot size; wgpu 25 reconfigures a canvas surface whose element
// size has changed when `get_current_texture` runs, so live resizes work
// without rebuilding the renderer.
//
// Aspect ratio: the runtime composes every frame in a fixed 800x450 pixel
// space (runtime::VIEWPORT_WIDTH/HEIGHT) and the ortho camera stretches that
// space over whatever surface size it gets — the render path tolerates any
// size and aspect, but a non-16:9 window stretches the scene non-uniformly.
// That fixed viewport is load-bearing in runtime's frame builder (another
// worker's crate), so it is documented here rather than changed.

const overlay = document.getElementById('overlay');
const status = document.getElementById('status');
const hud = document.getElementById('hud');
const speedValue = document.getElementById('speed-value');
const hudPos = document.getElementById('hud-pos');
const hudLap = document.getElementById('hud-lap');
const lapFill = document.getElementById('lap-fill');
const riderStateEl = document.getElementById('rider-state');
const countdownEl = document.getElementById('countdown');
const finishEl = document.getElementById('finish');
const finishText = document.getElementById('finish-text');
const restartButton = document.getElementById('restart');
const canvas = document.getElementById('game');

// Dirty-check helper (playtest E2): DOM text is only written when the
// formatted string actually changed, so the 60 Hz loop stops churning
// layout for identical frames.
const lastText = new Map();
function setText(el, text) {
  if (lastText.get(el) !== text) {
    lastText.set(el, text);
    el.textContent = text;
  }
}

function setStatus(text) {
  setText(status, text);
}

function showError(text) {
  overlay.classList.add('error');
  setStatus(text);
}

function hideOverlay() {
  overlay.remove();
}

// Adapter-present probe: a `navigator.gpu` existence check alone lies on
// browsers where WebGPU is exposed but no adapter can be created.
async function detectWebGpu() {
  if (!('gpu' in navigator)) return false;
  try {
    return (await navigator.gpu.requestAdapter()) !== null;
  } catch {
    return false;
  }
}

// Same facts the tier ladder consumes. `deviceMemory` is the Device Memory
// API in GiB and is missing on Firefox/Safari; `hardwareConcurrency` is the
// broader signal. Either floor trips low-end.
function detectLowEndDevice() {
  const cores = navigator.hardwareConcurrency ?? 8;
  const memoryGiB = navigator.deviceMemory ?? 8;
  return cores <= 4 || memoryGiB <= 4;
}

// Backing store from window size x DPR, capped (playtest E6). Called before
// boot — the renderer adopts whatever size is set — and on every resize
// while running.
const MAX_DPR = 2;
function resizeBackingStore() {
  const dpr = Math.min(window.devicePixelRatio || 1, MAX_DPR);
  const width = Math.max(1, Math.round(canvas.clientWidth * dpr));
  const height = Math.max(1, Math.round(canvas.clientHeight * dpr));
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
  }
}
resizeBackingStore();
addEventListener('resize', resizeBackingStore);

async function importBundle(bundle) {
  const glue = await import(`./dist/${bundle}/racing_web.js`);
  await glue.default();
  return glue;
}

async function boot() {
  const lowEnd = detectLowEndDevice();
  const hasWebGpu = await detectWebGpu();

  let glue;
  if (hasWebGpu) {
    setStatus('Loading WebGPU build…');
    glue = await importBundle('pkg-webgpu');
  } else if (window.WebGL2RenderingContext) {
    setStatus('Loading WebGL2 build…');
    glue = await importBundle('pkg-webgl2');
  } else {
    showError(
      'This game needs WebGPU or WebGL2, and this browser offers neither. ' +
        'Try a current version of Chrome, Edge, or Firefox.',
    );
    return;
  }

  setStatus('Booting…');
  let game;
  try {
    game = await glue.BrowserGame.boot(canvas, lowEnd);
  } catch (bootError) {
    if (!hasWebGpu) throw bootError;
    // The WebGPU adapter probed fine but bring-up failed (context loss,
    // driver blocklist). The WebGL2 build renders on the same browser.
    console.warn('WebGPU boot failed, falling back to WebGL2:', bootError);
    setStatus('Falling back to WebGL2 build…');
    glue = await importBundle('pkg-webgl2');
    game = await glue.BrowserGame.boot(canvas, lowEnd);
  }

  console.info(`racing: booted the ${glue.backend_name()} build`);
  hideOverlay();
  hud.hidden = false;

  const keys = { accelerate: false, brake: false, steerLeft: false, steerRight: false };
  const map = {
    ArrowUp: 'accelerate', KeyW: 'accelerate',
    ArrowDown: 'brake', KeyS: 'brake', Space: 'brake',
    ArrowLeft: 'steerLeft', KeyA: 'steerLeft',
    ArrowRight: 'steerRight', KeyD: 'steerRight',
  };
  addEventListener('keydown', (e) => {
    const key = map[e.code];
    if (key) { keys[key] = true; e.preventDefault(); }
  });
  addEventListener('keyup', (e) => {
    const key = map[e.code];
    if (key) { keys[key] = false; e.preventDefault(); }
  });

  restartButton.addEventListener('click', () => {
    game.restart();
    finishEl.hidden = true;
  });

  const ORDINALS = ['1st', '2nd', '3rd', '4th', '5th', '6th'];
  const ordinal = (pos) => ORDINALS[pos - 1] ?? `${pos}th`;

  // Rider-state badge (playtest E5): the wasm's bitmask maps to one visible
  // badge, most severe first. The start grid counts as a state too — that is
  // the moment the playtest flagged as an "inconsistency" (0 km/h, last
  // place, rivals drawn right beside the player): the values are correct,
  // the red lights explain them.
  const STATE_LABELS = [
    [1, 'CRASHED', 'crashed'],
    [2, 'OFF ROAD', 'offroad'],
    [4, 'BRAKING', 'braking'],
  ];
  function updateStateBadge(state, countingDown) {
    const found = STATE_LABELS.find(([bit]) => (state & bit) !== 0);
    const [text, className] = found ?? (countingDown ? ['ON GRID', 'grid'] : [null, null]);
    if (text === null) {
      riderStateEl.hidden = true;
      return;
    }
    riderStateEl.hidden = false;
    setText(riderStateEl, text);
    const nextClass = `rider-state ${className}`.trim();
    if (riderStateEl.className !== nextClass) riderStateEl.className = nextClass;
  }

  const frame = () => {
    game.advance_frame(keys.accelerate, keys.brake, keys.steerLeft, keys.steerRight);

    // HUD: speed, position, lap of N with progress toward the lap length
    // (playtest E1/E2/E3).
    setText(speedValue, String(game.speed_kmh()));
    setText(hudPos, `${ordinal(game.race_position())} of ${game.bike_count()}`);
    const lap = Math.min(game.lap_index() + 1, game.total_laps());
    const lapLength = Math.round(game.lap_length());
    setText(
      hudLap,
      `lap ${lap} of ${game.total_laps()} · ` +
        `${Math.floor(game.player_distance()).toLocaleString('en-US')} / ` +
        `${lapLength.toLocaleString('en-US')} m`,
    );
    const progressPct = `${(Math.min(game.lap_progress(), 1) * 100).toFixed(1)}%`;
    if (lapFill.style.width !== progressPct) lapFill.style.width = progressPct;

    updateStateBadge(game.rider_state(), game.is_counting_down());

    // Green-light countdown.
    if (game.is_counting_down()) {
      countdownEl.hidden = false;
      setText(countdownEl, String(game.countdown_secs()));
    } else {
      countdownEl.hidden = true;
    }

    // Finish: show the result, race keeps rendering behind it.
    if (game.is_finished()) {
      setText(
        finishText,
        `You finished ${ordinal(game.race_position())} of ${game.bike_count()}.`,
      );
      finishEl.hidden = false;
    }

    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
}

boot().catch((error) => {
  console.error(error);
  showError(`Boot failed: ${error}`);
});
