// Loader: picks the wasm bundle matching the browser's GPU backend, boots it,
// and drives the frame loop.
//
// Backend selection mirrors render::QualityTier::select — the loader probes
// for a WebGPU adapter, then loads exactly one of the two builds produced by
// web/build.sh. A failed WebGPU boot falls back to the WebGL2 build once;
// failing both surfaces a clean error on the loading screen.

const overlay = document.getElementById('overlay');
const status = document.getElementById('status');
const hud = document.getElementById('hud');
const canvas = document.getElementById('game');

function setStatus(text) {
  status.textContent = text;
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
    game = await glue.RacingWeb.boot(canvas, lowEnd);
  } catch (bootError) {
    if (!hasWebGpu) throw bootError;
    // The WebGPU adapter probed fine but bring-up failed (context loss,
    // driver blocklist). The WebGL2 build renders on the same browser.
    console.warn('WebGPU boot failed, falling back to WebGL2:', bootError);
    setStatus('Falling back to WebGL2 build…');
    glue = await importBundle('pkg-webgl2');
    game = await glue.RacingWeb.boot(canvas, lowEnd);
  }

  console.info(`racing: booted the ${glue.RacingWeb.backend_name()} build`);
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

  const frame = () => {
    game.advance_frame(keys.accelerate, keys.brake, keys.steerLeft, keys.steerRight);
    hud.textContent =
      `lap ${game.lap_index()}\n` +
      `${Math.floor(game.player_distance())} m`;
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
}

boot().catch((error) => {
  console.error(error);
  showError(`Boot failed: ${error}`);
});
