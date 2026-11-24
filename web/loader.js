// Loader: picks the wasm bundle matching the browser's GPU backend, boots it,
// and drives the frame loop.
import { badgeForState } from './badge.js';
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
const focusOverlay = document.getElementById('focus-overlay');
const hudKeys = document.getElementById('hud-keys');
const pausedEl = document.getElementById('paused');
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
// Backing-store cap (playtest finding): a 3840x1886 backing store with 4x
// MSAA measured ~116 MB of GPU memory. The internal resolution is capped at
// 1920 wide and CSS scales the canvas up — above that cap, extra pixels buy
// memory cost, not detail, since the scene composes in a fixed 800x450 space.
const MAX_BACKING_WIDTH = 1920;
function resizeBackingStore() {
  const dpr = Math.min(window.devicePixelRatio || 1, MAX_DPR);
  let width = Math.max(1, Math.round(canvas.clientWidth * dpr));
  let height = Math.max(1, Math.round(canvas.clientHeight * dpr));
  if (width > MAX_BACKING_WIDTH) {
    height = Math.round((height * MAX_BACKING_WIDTH) / width);
    width = MAX_BACKING_WIDTH;
  }
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
  hudKeys.hidden = false;

  const keys = { accelerate: false, brake: false, steerLeft: false, steerRight: false };
  const map = {
    ArrowUp: 'accelerate', KeyW: 'accelerate',
    ArrowDown: 'brake', KeyS: 'brake', Space: 'brake',
    ArrowLeft: 'steerLeft', KeyA: 'steerLeft',
    ArrowRight: 'steerRight', KeyD: 'steerRight',
  };

  // Keys are captured on BOTH window and document: inside an iframe, which
  // of the two receives an event depends on how the embedding page targets
  // the frame, and covering only one is how the frame can sit at 0 km/h with
  // keys landing in the parent page. A bubbling event reaches both listeners
  // once per target; setting the flag twice is idempotent.
  // preventDefault stays limited to mapped game keys (space scrolling, arrow
  // scrolling); unmapped keys and browser shortcuts pass through untouched,
  // and held-key repeats just re-set the same flag.
  const onKeyDown = (e) => {
    const key = map[e.code];
    if (key) { keys[key] = true; e.preventDefault(); }
  };
  const onKeyUp = (e) => {
    const key = map[e.code];
    if (key) { keys[key] = false; e.preventDefault(); }
  };
  addEventListener('keydown', onKeyDown);
  addEventListener('keyup', onKeyUp);
  document.addEventListener('keydown', onKeyDown);
  document.addEventListener('keyup', onKeyUp);

  // Focus capture: in an iframe artifact, keyboard focus often stays with
  // the parent page, so the sim reads no input and the race appears frozen.
  // Attempt focus on boot, then surface CLICK TO RIDE until the frame has
  // been engaged (a focus event, a click, or a keydown — whichever comes
  // first). Engagement is sticky: once focused, the prompt never returns.
  let engaged = false;
  const refreshFocusUI = () => {
    if (document.hasFocus()) engaged = true;
    if (focusOverlay.hidden !== engaged) focusOverlay.hidden = engaged;
  };
  addEventListener('focus', refreshFocusUI);
  addEventListener('blur', refreshFocusUI);
  addEventListener('pointerdown', refreshFocusUI, true);
  addEventListener('keydown', refreshFocusUI, true);
  document.addEventListener('pointerdown', refreshFocusUI, true);
  document.addEventListener('keydown', refreshFocusUI, true);
  // hasFocus() can flip without a focus/blur event firing in some embedding
  // setups, so poll as a backstop.
  setInterval(refreshFocusUI, 300);
  window.focus(); // best-effort: same-origin embedders may allow it; the overlay covers the rest
  refreshFocusUI();

  restartButton.addEventListener('click', () => {
    game.restart();
    finishEl.hidden = true;
  });

  // Fixed 60 Hz simulation (playtest finding: one advance_frame per rAF made
  // a 120 Hz display play at double speed). advance_frame is one sim tick;
  // elapsed time accumulates and each rAF drains whole ticks, with the
  // per-frame catch-up clamped to MAX_CATCH_UP_TICKS so a slow render path
  // (SwiftShader, weak iGPU) falls behind gradually instead of spiraling.
  const TICK_MS = 1000 / 60;
  const MAX_CATCH_UP_TICKS = 5;
  let lastFrameTime = performance.now();
  let accumulator = 0;

  // Keyboard affordances (playtest): R restarts alongside the button,
  // Escape pauses the sim shell-side — rendering keeps running, but no
  // advance_frame ticks are fed, so the whole race holds still without any
  // wasm-side pause API. Neither key is in the game-key map, so the browser
  // keeps default handling for them.
  // R and Escape are toggle/action semantics, not idempotent flag-sets, so a
  // single event reaching both the window and document listeners must act
  // once: dedupe on event identity (the same Event instance bubbles through
  // both targets).
  let lastRestartEvent = null;
  const onRestartKey = (e) => {
    if (e.code !== 'KeyR' || e.repeat || e === lastRestartEvent) return;
    lastRestartEvent = e;
    game.restart();
    finishEl.hidden = true;
  };
  addEventListener('keydown', onRestartKey);
  document.addEventListener('keydown', onRestartKey);

  let paused = false;
  let lastPauseEvent = null;
  const onPauseKey = (e) => {
    if (e.code !== 'Escape' || e.repeat || e === lastPauseEvent) return;
    lastPauseEvent = e;
    paused = !paused;
    pausedEl.hidden = !paused;
    if (!paused) {
      // Drop the backlog so unpausing doesn't burst a wave of catch-up ticks.
      accumulator = 0;
      lastFrameTime = performance.now();
    }
  };
  addEventListener('keydown', onPauseKey);
  document.addEventListener('keydown', onPauseKey);

  const ORDINALS = ['1st', '2nd', '3rd', '4th', '5th', '6th'];
  const ordinal = (pos) => ORDINALS[pos - 1] ?? `${pos}th`;

  // Rider-state badge (playtest E5): the wasm's bitmask maps to one visible
  // badge, most severe first — see badge.js for the (probe-asserted) mapping.
  // The start grid counts as a state too — that is the moment the playtest
  // flagged as an "inconsistency" (0 km/h, last place, rivals drawn right
  // beside the player): the values are correct, the red lights explain them.
  function updateStateBadge(state, countingDown) {
    const badge = badgeForState(state, countingDown);
    if (badge === null) {
      riderStateEl.hidden = true;
      return;
    }
    riderStateEl.hidden = false;
    setText(riderStateEl, badge.text);
    if (riderStateEl.className !== badge.className) riderStateEl.className = badge.className;
  }

  const frame = () => {
    // Drain whole 60 Hz ticks due since the last frame; the elapsed clamp in
    // the accumulator keeps this loop at MAX_CATCH_UP_TICKS iterations worst
    // case. Rendering still happens every rAF. While paused, no ticks are fed
    // and the clock is kept current so unpausing doesn't burst catch-up ticks.
    const now = performance.now();
    if (paused) {
      lastFrameTime = now;
    } else {
      accumulator += Math.min(now - lastFrameTime, MAX_CATCH_UP_TICKS * TICK_MS);
      lastFrameTime = now;
      while (accumulator >= TICK_MS) {
        game.advance_frame(keys.accelerate, keys.brake, keys.steerLeft, keys.steerRight);
        accumulator -= TICK_MS;
      }
    }

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
