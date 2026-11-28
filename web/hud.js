// Pure HUD helpers shared by loader.js and the node tests.

// Layout of Game.hud() (crates/web/src/lib.rs, pack_hud).
export const HUD = Object.freeze({
  POSITION: 0, RACERS: 1, LAP: 2, LAPS: 3, LAP_TIME: 4, LAST_LAP: 5, BEST_LAP: 6,
  RACE_TIME: 7, SPEED: 8, GEAR: 9, RPM: 10, BOOST: 11, BOOSTING: 12, COUNTDOWN: 13,
  WRONG_WAY: 14, CRASHED: 15, FINISHED: 16, BALANCE: 17, PROGRESS: 18, DRAFTING: 19, PAUSED: 20,
  GUARD: 21, STRIKE_READY: 22, WEAPON: 23, WEAPON_USES: 24, KNOCKOFFS: 25, NEAR_MISSES: 26, HITS: 27,
});

// Number of floats in Game.hud() (HUD_LEN in crates/web/src/lib.rs).
export const HUD_LEN = 28;

// HUD.WEAPON codes -> names.
export const WEAPONS = Object.freeze(['', 'Club', 'Chain']);

// Cue codes from Game.frame() (crates/web/src/lib.rs, pack_cues).
export const CUE = Object.freeze({
  BEEP: 1, GO: 2, WALL: 3, CONTACT: 4, HIT_LANDED: 5, GOT_HIT: 6, CRASH: 7, LAP: 8, FINISHED: 9, OVERTAKE: 10,
  GUARDED: 11, PARRIED: 12, WAS_PARRIED: 13, KNOCKOFF: 14, LAUNCH: 15,
  TRAFFIC_CRASH: 16, NEAR_MISS: 17, ARMED: 18, DISARMED: 19, HUNTED: 20, MOOD: 21,
});

// Difficulty levels (crates/game/src/difficulty.rs) for the menu.
export const DIFFICULTIES = Object.freeze([
  { slug: 'easy', name: 'Easy', blurb: 'Rivals wait for you and hit softer' },
  { slug: 'normal', name: 'Normal', blurb: 'A fair fight at full race pace' },
  { slug: 'hard', name: 'Hard', blurb: 'Quicker, braver rivals who block and parry' },
]);

// A stored slug if it names a level, otherwise 'normal'.
export function pickDifficulty(stored) {
  return DIFFICULTIES.some((d) => d.slug === stored) ? stored : 'normal';
}

export const REDLINE_RPM = 12500;
export const MAX_RPM = 14000;

// 83.456 -> "1:23.456"; negative or missing -> "–".
export function formatTime(t) {
  if (t == null || !(t >= 0)) return '–';
  const ms = Math.round(t * 1000);
  const m = Math.floor(ms / 60000);
  const s = Math.floor((ms % 60000) / 1000);
  const r = ms % 1000;
  return `${m}:${String(s).padStart(2, '0')}.${String(r).padStart(3, '0')}`;
}

export function ordinal(n) {
  const v = n % 100;
  if (v >= 11 && v <= 13) return 'th';
  return ['th', 'st', 'nd', 'rd'][n % 10] ?? 'th';
}

// Splits Game.cues triples into objects.
export function decodeCues(arr) {
  const out = [];
  for (let i = 0; i + 2 < arr.length; i += 3) out.push({ code: arr[i], a: arr[i + 1], b: arr[i + 2] });
  return out;
}

// "slug|name|laps|blurb" lines -> [{ slug, name, laps, blurb }].
export function parseTracks(text) {
  return text
    .split('\n')
    .filter(Boolean)
    .map((line) => {
      const [slug, name, laps, ...rest] = line.split('|');
      return { slug, name, laps: Number(laps), blurb: rest.join('|') };
    });
}

// "name|time|done" lines -> [{ name, time, done }].
export function parseStandings(text) {
  return text
    .split('\n')
    .filter(Boolean)
    .map((line) => {
      const [name, time, done] = line.split('|');
      return { name, time: Number(time), done: done === '1' };
    });
}

// Fits x,z points into a square canvas of `size` px with `pad` px margin.
// Returns a mapper (x, z) -> [px, py], keeping the aspect ratio.
export function fitMap(points, size, pad) {
  let minX = Infinity, maxX = -Infinity, minZ = Infinity, maxZ = -Infinity;
  for (let i = 0; i + 1 < points.length; i += 2) {
    minX = Math.min(minX, points[i]); maxX = Math.max(maxX, points[i]);
    minZ = Math.min(minZ, points[i + 1]); maxZ = Math.max(maxZ, points[i + 1]);
  }
  const span = Math.max(maxX - minX, maxZ - minZ, 1);
  const k = (size - pad * 2) / span;
  const ox = pad + (size - pad * 2 - (maxX - minX) * k) / 2;
  const oz = pad + (size - pad * 2 - (maxZ - minZ) * k) / 2;
  return (x, z) => [ox + (x - minX) * k, oz + (z - minZ) * k];
}

// Dynamic resolution: nudges the render scale toward a frame-time target.
// Feed it the measured frame interval (ms); it returns the scale to use.
export class ResolutionGovernor {
  constructor({ min = 0.5, max = 1.0, start = 1.0, budgetMs = 17.6 } = {}) {
    this.min = min;
    this.max = max;
    this.scale = start;
    this.budget = budgetMs;
    this.avg = budgetMs;
    this.calm = 0;
  }

  sample(ms) {
    if (!(ms > 0) || ms > 250) return this.scale; // tab switch, breakpoint
    this.avg += (ms - this.avg) * 0.08;
    if (this.avg > this.budget * 1.12) {
      this.scale = Math.max(this.min, this.scale - 0.05);
      this.avg = this.budget;
      this.calm = 0;
    } else if (this.avg < this.budget) {
      // Only climb back after a sustained calm spell, to avoid oscillation.
      if (++this.calm > 180) {
        this.scale = Math.min(this.max, this.scale + 0.05);
        this.calm = 0;
      }
    } else {
      this.calm = 0;
    }
    return this.scale;
  }
}
