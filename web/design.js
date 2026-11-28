// The page half of the design system.
//
// Two ideas carried over from frame-driven anime production:
//
// 1. One palette. The wasm owns the tokens (render/src/style.rs) and hands
//    them over as CSS custom properties, so the HUD, the canvases drawn here
//    and the 3D scene always agree. `applyStyle` puts them on the page.
//
// 2. One direction table. The simulation is the master clock; it raises cues
//    (crash, lap, overtake…) and this table says what the page does about
//    each: a toast, a sound, an edge flash. Picture-side responses (impact
//    frames, sparks, shake) are already driven from the same cues inside the
//    wasm. Nothing else in the page decides what a cue looks or sounds like.

import { CUE, HUD, WEAPONS, formatTime, ordinal } from './hud.js';

// Parses `--name: value;` pairs as produced by `style_css`.
export function parseStyle(css) {
  const out = {};
  for (const part of String(css).split(';')) {
    const i = part.indexOf(':');
    if (i < 0) continue;
    const k = part.slice(0, i).trim();
    const v = part.slice(i + 1).trim();
    if (k.startsWith('--') && v) out[k] = v;
  }
  return out;
}

// Sets the tokens on `el` (the document root by default) and returns them.
export function applyStyle(css, el = globalThis.document?.documentElement) {
  const vars = parseStyle(css);
  if (el) for (const [k, v] of Object.entries(vars)) el.style.setProperty(k, v);
  return vars;
}

// Colours for canvas drawing, read from the token map.
export function canvasPalette(vars) {
  const v = (k, d) => vars[k] || d;
  return {
    ink: v('--ink', '#14121a'),
    paper: v('--paper', '#f4f1ea'),
    hot: v('--hot', '#ff4d2e'),
    gold: v('--gold', '#ffc23d'),
    boost: v('--boost', '#48c8ff'),
    good: v('--good', '#6fe39b'),
    accent: v('--accent', '#ffb347'),
  };
}

// True when the player asked the OS for reduced motion: no impact frames
// and no edge flashes.
export function prefersReducedMotion() {
  return !!globalThis.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
}

const place = (h) => `${h[HUD.POSITION]}${ordinal(h[HUD.POSITION])}`;

// What the page does for each cue. Each entry takes the cue `{code, a, b}`
// and the HUD array and returns any of:
//   toast: [text, kind]      kind '' | 'best' | 'bad'
//   later: [ms, text, kind]  a second toast after a beat
//   flash: 0..1              edge flash strength (skipped for reduced motion)
//   sound: [hz, secs, wave, gain]
//   finished: true           the race is over for the player
export const DIRECTION = {
  [CUE.WALL]: (c) => ({
    flash: Math.min(0.8, 0.2 + c.a / 40),
    sound: [70, 0.12, 'sawtooth', Math.min(0.5, 0.15 + c.a / 60)],
  }),
  [CUE.CONTACT]: () => ({ sound: [120, 0.06, 'square', 0.2] }),
  // Hits and crashes get an impact frame in the 3D picture; the page only
  // adds the sound and the word.
  // a = 1 for a kick.
  [CUE.HIT_LANDED]: (c) =>
    c.a > 0
      ? { toast: ['Kick!', 'best'], sound: [140, 0.12, 'triangle', 0.5] }
      : { toast: ['Hit!', 'best'], sound: [180, 0.08, 'triangle', 0.4] },
  // Your guard soaked a strike.
  [CUE.GUARDED]: () => ({ toast: ['Blocked', ''], sound: [260, 0.05, 'square', 0.2] }),
  // You raised your guard just in time: the attacker is stunned.
  [CUE.PARRIED]: () => ({ toast: ['Parry!', 'best'], flash: 0.25, sound: [880, 0.09, 'triangle', 0.35] }),
  // Your strike was parried and you are wobbling.
  [CUE.WAS_PARRIED]: () => ({ toast: ['Parried', 'bad'], sound: [110, 0.14, 'sawtooth', 0.3] }),
  [CUE.KNOCKOFF]: () => ({ toast: ['Knock-off · full boost', 'best'], sound: [520, 0.16, 'triangle', 0.4] }),
  // a = 1 for a perfect launch.
  [CUE.LAUNCH]: (c) =>
    c.a > 0
      ? { toast: ['Perfect start', 'best'], sound: [740, 0.1, 'sine', 0.25] }
      : { toast: ['Wheelspin', 'bad'] },
  [CUE.GOT_HIT]: () => ({ sound: [95, 0.1, 'square', 0.35] }),
  // You went into a car: the loudest thing in the game.
  [CUE.TRAFFIC_CRASH]: () => ({
    toast: ['Wrecked', 'bad'],
    flash: 0.9,
    sound: [48, 0.45, 'sawtooth', 0.6],
  }),
  // a = 1 when the car was coming the other way (worth more boost).
  [CUE.NEAR_MISS]: (c) =>
    c.a > 0
      ? { toast: ['Near miss · oncoming', 'best'], sound: [990, 0.06, 'sine', 0.22] }
      : { toast: ['Near miss', 'best'], sound: [820, 0.05, 'sine', 0.16] },
  // a = weapon code (1 club, 2 chain).
  [CUE.ARMED]: (c) => ({ toast: [`${WEAPONS[c.a] || 'Weapon'} grabbed`, 'best'], sound: [420, 0.12, 'triangle', 0.35] }),
  [CUE.DISARMED]: () => ({ toast: ['Weapon gone', 'bad'], sound: [150, 0.1, 'square', 0.25] }),
  // a = rival's bike index; the loader adds `name` (cues carry indices).
  [CUE.HUNTED]: (c) => ({ toast: [`${c.name || 'A rival'} is coming for you`, 'bad'], sound: [300, 0.18, 'sawtooth', 0.2] }),
  [CUE.CRASH]: () => ({ sound: [55, 0.3, 'sawtooth', 0.5] }),
  [CUE.LAP]: (c, h) => {
    const best = h[HUD.BEST_LAP];
    const isBest = best >= 0 && Math.abs(best - c.b) < 0.0005;
    const d = { toast: [`Lap ${c.a}  ${formatTime(c.b)}${isBest ? '  · best' : ''}`, isBest ? 'best' : ''] };
    if (c.a === h[HUD.LAPS] - 1) d.later = [600, 'Final lap', 'bad'];
    return d;
  },
  [CUE.FINISHED]: () => ({ finished: true }),
  [CUE.OVERTAKE]: (c, h) =>
    c.a > 0 ? { toast: [place(h), 'best'], sound: [660, 0.07, 'sine', 0.18] } : { toast: [place(h), 'bad'] },
};

// The direction for one cue, or an empty object.
export function direct(cue, hud) {
  const f = DIRECTION[cue.code];
  return f ? f(cue, hud) : {};
}
