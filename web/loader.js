// Loader and shell: boots the wasm bundle that matches the browser (WebGPU,
// else WebGL2), then runs menus, HUD, input, audio and the frame loop. The
// game itself (simulation, camera, all 3D drawing) lives in the wasm.

import { RaceAudio } from './audio.js';
import {
  HUD, CUE, REDLINE_RPM, MAX_RPM, formatTime, ordinal, decodeCues, parseTracks, parseStandings, fitMap,
  ResolutionGovernor,
} from './hud.js';

const $ = (id) => document.getElementById(id);
let canvasEl = $('game');

function replaceCanvas() {
  const fresh = canvasEl.cloneNode();
  canvasEl.replaceWith(fresh);
  canvasEl = fresh;
  sizeCanvas();
}
const ui = {
  boot: $('boot'), bootStatus: $('boot-status'), menu: $('menu'), tracks: $('tracks'), start: $('start'),
  hud: $('hud'), pos: $('h-pos'), suf: $('h-suf'), of: $('h-of'), lap: $('h-lap'), time: $('h-time'), best: $('h-best'),
  bal: $('h-bal'), speed: $('h-speed'), gear: $('h-gear'), boost: $('h-boost'), boostLbl: $('h-boostlbl'), draft: $('draft'),
  count: $('count'), banner: $('banner'), toast: $('toast'), hint: $('hint'), flash: $('flash'),
  tach: $('tach'), minimap: $('minimap'),
  pause: $('pause'), pauseSub: $('pause-sub'), resume: $('resume'), restart: $('restart'), quit: $('quit'),
  results: $('results'), rTitle: $('r-title'), rSub: $('r-sub'), rTable: $('r-table'), again: $('again'), toMenu: $('tomenu'),
  mute: $('mute'), fs: $('fs'),
};

const params = new URLSearchParams(location.search);
const DPR_CAP = 1.5;

// ---------------------------------------------------------------- boot ----

async function hasWebGpu() {
  if (!navigator.gpu) return false;
  try {
    return !!(await navigator.gpu.requestAdapter());
  } catch {
    return false;
  }
}

function sizeCanvas() {
  const dpr = Math.min(window.devicePixelRatio || 1, DPR_CAP);
  const w = Math.max(1, Math.round(window.innerWidth * dpr));
  const h = Math.max(1, Math.round(window.innerHeight * dpr));
  if (canvasEl.width !== w || canvasEl.height !== h) {
    canvasEl.width = w;
    canvasEl.height = h;
    return true;
  }
  return false;
}

async function bootBundle(name, slug) {
  const mod = await import(`./${name}/racing_web.js`);
  await mod.default();
  const game = await mod.Game.boot(canvasEl, slug);
  return { mod, game };
}

async function boot() {
  sizeCanvas();
  const forced = params.get('backend');
  const order = forced === 'webgl2' ? ['pkg-webgl2'] : forced === 'webgpu' ? ['pkg-webgpu'] : (await hasWebGpu()) ? ['pkg-webgpu', 'pkg-webgl2'] : ['pkg-webgl2'];
  let lastErr;
  for (const name of order) {
    ui.bootStatus.textContent = `Loading ${name === 'pkg-webgpu' ? 'WebGPU' : 'WebGL2'} renderer…`;
    try {
      // A failed WebGPU boot leaves the canvas bound to that context, so the
      // WebGL2 retry gets a fresh canvas.
      if (lastErr) replaceCanvas();
      return await bootBundle(name, localStorage.getItem('track') || '');
    } catch (e) {
      console.error(e);
      lastErr = e;
    }
  }
  throw lastErr;
}


// --------------------------------------------------------------- input ----

const keys = new Set();
let attackLatch = false;
const KEYMAP = {
  throttle: ['KeyW', 'ArrowUp'],
  brake: ['KeyS', 'ArrowDown'],
  left: ['KeyA', 'ArrowLeft'],
  right: ['KeyD', 'ArrowRight'],
  boost: ['ShiftLeft', 'ShiftRight', 'Space'],
};
const held = (k) => KEYMAP[k].some((c) => keys.has(c));
const GAME_KEYS = new Set([...Object.values(KEYMAP).flat(), 'KeyX', 'KeyC']);

function readInput() {
  let throttle = held('throttle') ? 1 : 0;
  let brake = held('brake') ? 1 : 0;
  let steer = (held('right') ? 1 : 0) - (held('left') ? 1 : 0);
  let analog = false;
  let boost = held('boost');
  let attack = attackLatch;
  attackLatch = false;
  const pad = navigator.getGamepads ? [...navigator.getGamepads()].find((p) => p && p.connected) : null;
  if (pad) {
    const b = (i) => pad.buttons[i] || { pressed: false, value: 0 };
    const sx = pad.axes[0] || 0;
    if (Math.abs(sx) > 0.12) {
      steer = Math.sign(sx) * ((Math.abs(sx) - 0.12) / 0.88);
      analog = true;
    }
    throttle = Math.max(throttle, b(7).value, b(0).pressed ? 1 : 0);
    brake = Math.max(brake, b(6).value);
    boost = boost || b(5).pressed || b(4).pressed;
    attack = attack || padEdge(pad, 2) || padEdge(pad, 1);
    if (padEdge(pad, 3)) cycleCamera();
    if (padEdge(pad, 9)) onEscape();
  }
  return { throttle, brake, steer, analog, boost, attack };
}

const padPrev = new Map();
function padEdge(pad, i) {
  const now = !!(pad.buttons[i] && pad.buttons[i].pressed);
  const was = padPrev.get(i) || false;
  padPrev.set(i, now);
  return now && !was;
}

// ---------------------------------------------------------------- state ----

let game = null;
let tracks = [];
let selected = 0;
let mode = 'menu'; // menu | race | paused | results
let outline = null;
let mapper = null;
let mapBg = null;
let finishedAt = null;
let lastHud = null;
const audio = new RaceAudio();
const governor = new ResolutionGovernor();

function show(el, on) {
  el.classList.toggle('hidden', !on);
}

function setText(el, text) {
  if (el._t !== text) {
    el._t = text;
    el.textContent = text;
  }
}

function renderTrackList() {
  ui.tracks.innerHTML = '';
  tracks.forEach((t, i) => {
    const b = document.createElement('button');
    b.className = 'track' + (i === selected ? ' sel' : '');
    b.innerHTML = `<div class="t"><span></span><small></small></div><div class="b"></div>`;
    b.querySelector('.t span').textContent = t.name;
    b.querySelector('.t small').textContent = `${t.laps} laps`;
    b.querySelector('.b').textContent = t.blurb;
    b.addEventListener('click', () => selectTrack(i));
    b.addEventListener('dblclick', startRace);
    ui.tracks.appendChild(b);
  });
}

function selectTrack(i) {
  selected = (i + tracks.length) % tracks.length;
  localStorage.setItem('track', tracks[selected].slug);
  renderTrackList();
  game.show(tracks[selected].slug);
  loadMap();
}

function loadMap() {
  outline = game.track_outline();
  mapper = fitMap(outline, ui.minimap.width, 26);
  mapBg = null;
}

function enterMenu() {
  mode = 'menu';
  audio.update(audioFrame(null, { paused: true }));
  show(ui.menu, true);
  show(ui.hud, false);
  show(ui.pause, false);
  show(ui.results, false);
  game.set_paused(false);
  game.show(tracks[selected].slug);
  loadMap();
}

function startRace() {
  audio.unlock();
  mode = 'race';
  finishedAt = null;
  show(ui.menu, false);
  show(ui.pause, false);
  show(ui.results, false);
  show(ui.hud, true);
  ui.hint.style.opacity = '1';
  setTimeout(() => (ui.hint.style.opacity = '0'), 9000);
  ui.toast.innerHTML = '';
  game.start_race(tracks[selected].slug);
  loadMap();
  canvasEl.focus();
}

function setPaused(on) {
  if (mode !== 'race' && mode !== 'paused') return;
  mode = on ? 'paused' : 'race';
  game.set_paused(on);
  show(ui.pause, on);
  if (on && lastHud) {
    ui.pauseSub.textContent = `${tracks[selected].name} · lap ${lastHud[HUD.LAP]} of ${lastHud[HUD.LAPS]}`;
  }
}

function onEscape() {
  if (mode === 'race') setPaused(true);
  else if (mode === 'paused') setPaused(false);
  else if (mode === 'results') enterMenu();
}

function cycleCamera() {
  if (mode === 'race') game.cycle_camera();
}

function toast(text, cls = '') {
  const d = document.createElement('div');
  d.textContent = text;
  if (cls) d.className = cls;
  ui.toast.appendChild(d);
  setTimeout(() => d.remove(), 2400);
  while (ui.toast.children.length > 3) ui.toast.firstChild.remove();
}

function flash(strength) {
  ui.flash.style.transition = 'none';
  ui.flash.style.opacity = String(Math.min(1, strength));
  requestAnimationFrame(() => {
    ui.flash.style.transition = 'opacity 0.45s';
    ui.flash.style.opacity = '0';
  });
}

// ------------------------------------------------------------------ HUD ----

function drawTach(rpm, boosting) {
  const c = ui.tach;
  const g = c.getContext('2d');
  const w = c.width, h = c.height;
  g.clearRect(0, 0, w, h);
  const bars = 36;
  const gap = 3;
  const bw = (w - gap * (bars - 1)) / bars;
  const lit = (rpm / MAX_RPM) * bars;
  for (let i = 0; i < bars; i++) {
    const r = (i + 1) / bars * MAX_RPM;
    const hh = h * (0.32 + 0.68 * Math.pow(i / (bars - 1), 1.6));
    const on = i < lit;
    let col = 'rgba(255,255,255,0.12)';
    if (on) col = r > REDLINE_RPM ? '#ff4d2e' : r > REDLINE_RPM * 0.8 ? '#ffc23d' : boosting ? '#48c8ff' : '#f4f1ea';
    g.fillStyle = col;
    g.fillRect(i * (bw + gap), h - hh, bw, hh);
  }
}

function drawMinimap(points) {
  const c = ui.minimap;
  const g = c.getContext('2d');
  if (!mapBg) {
    mapBg = document.createElement('canvas');
    mapBg.width = c.width;
    mapBg.height = c.height;
    const b = mapBg.getContext('2d');
    b.lineJoin = 'round';
    b.beginPath();
    for (let i = 0; i + 1 < outline.length; i += 2) {
      const [x, y] = mapper(outline[i], outline[i + 1]);
      if (i === 0) b.moveTo(x, y);
      else b.lineTo(x, y);
    }
    b.closePath();
    b.strokeStyle = 'rgba(0,0,0,0.5)';
    b.lineWidth = 18;
    b.stroke();
    b.strokeStyle = 'rgba(244,241,234,0.85)';
    b.lineWidth = 8;
    b.stroke();
    const [sx, sy] = mapper(outline[0], outline[1]);
    b.fillStyle = '#fff';
    b.fillRect(sx - 9, sy - 9, 18, 18);
    b.fillStyle = '#000';
    b.fillRect(sx - 9, sy - 9, 9, 9);
    b.fillRect(sx, sy, 9, 9);
  }
  g.clearRect(0, 0, c.width, c.height);
  g.drawImage(mapBg, 0, 0);
  for (let i = points.length - 2; i >= 0; i -= 2) {
    const [x, y] = mapper(points[i], points[i + 1]);
    const me = i === 0;
    g.beginPath();
    g.arc(x, y, me ? 13 : 9, 0, Math.PI * 2);
    g.fillStyle = me ? '#ff4d2e' : 'rgba(30,34,44,0.95)';
    g.fill();
    g.lineWidth = me ? 4 : 3;
    g.strokeStyle = me ? '#fff' : 'rgba(244,241,234,0.8)';
    g.stroke();
  }
}

function updateHud(h) {
  const pos = h[HUD.POSITION];
  setText(ui.pos, String(pos));
  setText(ui.suf, ordinal(pos));
  setText(ui.of, `/ ${h[HUD.RACERS]}`);
  const lap = h[HUD.LAP], laps = h[HUD.LAPS];
  setText(ui.lap, `${lap} / ${laps}`);
  setText(ui.time, formatTime(h[HUD.LAP_TIME]));
  setText(ui.best, formatTime(h[HUD.BEST_LAP]));
  setText(ui.speed, String(Math.round(h[HUD.SPEED])));
  setText(ui.gear, String(h[HUD.GEAR]));
  ui.boost.style.width = `${Math.round(h[HUD.BOOST] * 100)}%`;
  ui.boostLbl.classList.toggle('on', h[HUD.BOOSTING] > 0 || h[HUD.BOOST] > 0.98);
  setText(ui.boostLbl, h[HUD.BOOSTING] > 0 ? 'BOOSTING' : h[HUD.BOOST] > 0.98 ? 'BOOST READY' : 'BOOST');
  ui.draft.classList.toggle('on', h[HUD.DRAFTING] > 0);
  const bal = Math.max(0, Math.min(1, h[HUD.BALANCE] / 100));
  ui.bal.style.width = `${Math.round(bal * 100)}%`;
  ui.bal.classList.toggle('low', bal < 0.35);
  drawTach(h[HUD.RPM], h[HUD.BOOSTING] > 0);

  const cd = h[HUD.COUNTDOWN];
  if (cd > 0) {
    ui.count.classList.remove('go');
    setText(ui.count, String(Math.ceil(cd)));
  } else if (h[HUD.RACE_TIME] < 1.2 && h[HUD.FINISHED] < 0) {
    ui.count.classList.add('go');
    setText(ui.count, 'GO!');
  } else {
    setText(ui.count, '');
  }

  let banner = '';
  let warn = false;
  if (h[HUD.FINISHED] >= 0) banner = 'FINISH';
  else if (h[HUD.WRONG_WAY] > 0) { banner = 'WRONG WAY'; warn = true; }
  else if (h[HUD.CRASHED] > 0) banner = 'GET BACK ON!';
  setText(ui.banner, banner);
  show(ui.banner, banner !== '');
  ui.banner.classList.toggle('warn', warn);
}

function showResults() {
  mode = 'results';
  show(ui.hud, false);
  show(ui.results, true);
  const rows = parseStandings(game.standings());
  const me = rows.findIndex((r) => r.name === 'You');
  ui.rTitle.textContent = me === 0 ? 'Victory!' : `${me + 1}${ordinal(me + 1)} place`;
  const h = lastHud;
  ui.rSub.textContent = `${tracks[selected].name} · best lap ${formatTime(h[HUD.BEST_LAP])}`;
  ui.rTable.innerHTML = '';
  rows.forEach((r, i) => {
    const tr = document.createElement('tr');
    if (r.name === 'You') tr.className = 'me';
    const cells = [String(i + 1), r.name, r.done ? formatTime(r.time) : 'racing…'];
    for (const t of cells) {
      const td = document.createElement('td');
      td.textContent = t;
      tr.appendChild(td);
    }
    ui.rTable.appendChild(tr);
  });
}

// ---------------------------------------------------------------- audio ----

function audioFrame(h, extra = {}) {
  const inp = extra.input || { throttle: 0, brake: 0 };
  return {
    speedKmh: h ? h[HUD.SPEED] : 0,
    state: h && h[HUD.CRASHED] > 0 ? 1 : 0,
    // Rider balance (0..100) drives the audio's hit cue on a sudden drop.
    stamina: h ? h[HUD.BALANCE] / 100 : 1,
    countingDown: !!h && h[HUD.COUNTDOWN] > 0,
    countdownSecs: h ? Math.ceil(h[HUD.COUNTDOWN]) : 0,
    paused: extra.paused ?? false,
    finished: !!h && h[HUD.FINISHED] >= 0,
    rpm: h ? h[HUD.RPM] : 0,
    throttle: inp.brake > 0.1 ? 'brake' : inp.throttle > 0.1 ? 'accelerate' : 'coast',
    zone: h ? `lap${h[HUD.LAP]}` : null,
  };
}

function handleCues(cues, h) {
  for (const c of cues) {
    switch (c.code) {
      case CUE.WALL:
        flash(0.25 + c.a / 40);
        if (audio.ctx) audio.blip(70, 0.12, 'sawtooth', Math.min(0.5, 0.15 + c.a / 60));
        break;
      case CUE.CONTACT:
        if (audio.ctx) audio.blip(120, 0.06, 'square', 0.2);
        break;
      case CUE.HIT_LANDED:
        toast('Hit!', 'best');
        if (audio.ctx) audio.blip(180, 0.08, 'triangle', 0.4);
        break;
      case CUE.GOT_HIT:
        flash(0.5);
        break;
      case CUE.CRASH:
        flash(0.9);
        break;
      case CUE.LAP: {
        const lapDone = c.a;
        const best = h[HUD.BEST_LAP];
        const isBest = best >= 0 && Math.abs(best - c.b) < 0.0005;
        toast(`Lap ${lapDone}  ${formatTime(c.b)}${isBest ? '  · best' : ''}`, isBest ? 'best' : '');
        if (lapDone === h[HUD.LAPS] - 1) setTimeout(() => toast('Final lap', 'bad'), 600);
        break;
      }
      case CUE.FINISHED:
        finishedAt = performance.now();
        break;
      case CUE.OVERTAKE:
        if (c.a > 0) {
          toast(`${h[HUD.POSITION]}${ordinal(h[HUD.POSITION])}`, 'best');
          if (audio.ctx) audio.blip(660, 0.07, 'sine', 0.18);
        } else {
          toast(`${h[HUD.POSITION]}${ordinal(h[HUD.POSITION])}`, 'bad');
        }
        break;
      default:
        break;
    }
  }
}

// ----------------------------------------------------------------- loop ----

let last = performance.now();
let lastScale = 1;

function frame(now) {
  const dtMs = now - last;
  last = now;
  const dt = Math.min(dtMs / 1000, 0.1);

  if (sizeCanvas()) game.resize(canvasEl.width, canvasEl.height);
  const scale = governor.sample(dtMs);
  if (Math.abs(scale - lastScale) > 0.001) {
    lastScale = scale;
    game.set_render_scale(scale);
  }

  const playing = mode === 'race';
  const input = playing ? readInput() : { throttle: 0, brake: 0, steer: 0, analog: false, boost: false, attack: false };
  if (!playing && navigator.getGamepads) pollMenuPad();
  const cues = decodeCues(game.frame(dt, input.throttle, input.brake, input.steer, input.analog, input.boost, input.attack));

  if (mode === 'race' || mode === 'paused' || mode === 'results') {
    const h = game.hud();
    lastHud = h;
    if (mode === 'race') {
      handleCues(cues, h);
      updateHud(h);
      drawMinimap(game.bike_points());
      if (finishedAt && now - finishedAt > 3200) showResults();
    }
    audio.update(audioFrame(h, { paused: mode === 'paused', input }));
  }
  requestAnimationFrame(frame);
}

function pollMenuPad() {
  const pad = [...navigator.getGamepads()].find((p) => p && p.connected);
  if (!pad) return;
  if (mode === 'menu') {
    if (padEdge(pad, 0) || padEdge(pad, 9)) startRace();
    if (padEdge(pad, 12)) selectTrack(selected - 1);
    if (padEdge(pad, 13)) selectTrack(selected + 1);
  } else if (mode === 'paused') {
    if (padEdge(pad, 9) || padEdge(pad, 0)) setPaused(false);
  } else if (mode === 'results') {
    if (padEdge(pad, 0)) startRace();
    if (padEdge(pad, 1)) enterMenu();
  }
}

// --------------------------------------------------------------- events ----

window.addEventListener('keydown', (e) => {
  if (!game) return;
  audio.unlock();
  if (GAME_KEYS.has(e.code)) e.preventDefault();
  if (e.repeat) {
    keys.add(e.code);
    return;
  }
  keys.add(e.code);
  switch (e.code) {
    case 'Escape':
    case 'KeyP':
      onEscape();
      break;
    case 'KeyX':
      attackLatch = true;
      break;
    case 'KeyC':
      cycleCamera();
      break;
    case 'KeyR':
      if (mode === 'race' || mode === 'paused' || mode === 'results') startRace();
      break;
    case 'Enter':
      if (mode === 'menu' || mode === 'results') startRace();
      else if (mode === 'paused') setPaused(false);
      break;
    case 'ArrowUp':
    case 'ArrowDown':
      if (mode === 'menu') selectTrack(selected + (e.code === 'ArrowUp' ? -1 : 1));
      break;
    default:
      break;
  }
});
window.addEventListener('keyup', (e) => keys.delete(e.code));
window.addEventListener('blur', () => {
  keys.clear();
  if (mode === 'race') setPaused(true);
});
document.addEventListener('visibilitychange', () => {
  if (document.hidden && mode === 'race') setPaused(true);
});

ui.start.addEventListener('click', startRace);
ui.resume.addEventListener('click', () => setPaused(false));
ui.restart.addEventListener('click', startRace);
ui.quit.addEventListener('click', enterMenu);
ui.again.addEventListener('click', startRace);
ui.toMenu.addEventListener('click', enterMenu);
ui.mute.addEventListener('click', () => {
  audio.unlock();
  audio.setMuted(!audio.muted);
  localStorage.setItem('muted', audio.muted ? '1' : '');
  ui.mute.textContent = audio.muted ? '🔇' : '🔊';
});
ui.fs.addEventListener('click', () => {
  if (document.fullscreenElement) document.exitFullscreen();
  else document.documentElement.requestFullscreen?.();
});
window.addEventListener('pointerdown', () => audio.unlock(), { once: false });

// ---------------------------------------------------------------- start ----

(async () => {
  try {
    const { mod, game: g } = await boot();
    game = g;
    tracks = parseTracks(mod.track_list());
    selected = Math.max(0, tracks.findIndex((t) => t.slug === game.track()));
    renderTrackList();
    if (localStorage.getItem('muted')) {
      audio.setMuted(true);
      ui.mute.textContent = '🔇';
    }
    loadMap();
    show(ui.boot, false);
    show(ui.menu, true);
    console.info(`racing: ${game.backend()} on ${game.adapter()} (HDR ${game.hdr_format()})`);
    window.__racing = { game, governor, stats: () => game.stats() };
    last = performance.now();
    requestAnimationFrame(frame);
  } catch (e) {
    ui.bootStatus.textContent = `This browser couldn't start the renderer (${e && e.message ? e.message : e}). Try a recent Chrome, Edge, Firefox or Safari.`;
    document.querySelector('#boot .spin').style.display = 'none';
  }
})();
