// Season: prize money from each race, spent in the garage on upgrades that
// the wasm fits to the player's bike (Loadout::from_levels in
// crates/game/src/race.rs). Pure functions plus a small store, so the node
// tests can drive it without a browser.

export const MAX_LEVEL = 3;

// The three upgrades, in the order start_race takes them.
export const UPGRADES = Object.freeze([
  { key: 'engine', name: 'Engine', blurb: 'More pull and top speed' },
  { key: 'handling', name: 'Handling', blurb: 'More grip through the bends' },
  { key: 'armour', name: 'Armour', blurb: 'Hits cost you less balance' },
]);

// Price of the next level (index = current level). Level 3 is the last.
export const COSTS = Object.freeze([900, 1800, 3200]);

// Purse by finishing place (1st first); anyone further back gets the last.
export const PURSE = Object.freeze([1500, 1000, 750, 550, 400, 300, 220, 160]);
export const KNOCKOFF_BONUS = 150;
export const NEAR_MISS_BONUS = 40;
// Harder fields pay more.
export const DIFFICULTY_PAY = Object.freeze({ easy: 0.6, normal: 1, hard: 1.6 });

export function freshSeason() {
  return { cash: 0, engine: 0, handling: 0, armour: 0, races: 0, wins: 0, earned: 0 };
}

const clampLevel = (v) => Math.max(0, Math.min(MAX_LEVEL, Math.floor(Number(v) || 0)));

// Parses a stored season, repairing anything missing or out of range.
export function loadSeason(text) {
  const s = freshSeason();
  let raw = null;
  try {
    raw = JSON.parse(text);
  } catch {
    return s;
  }
  if (!raw || typeof raw !== 'object') return s;
  for (const k of ['cash', 'races', 'wins', 'earned']) {
    const v = Math.floor(Number(raw[k]));
    if (Number.isFinite(v) && v > 0) s[k] = v;
  }
  for (const u of UPGRADES) s[u.key] = clampLevel(raw[u.key]);
  return s;
}

export function saveSeason(s) {
  return JSON.stringify(s);
}

// What one race paid: { lines: [[label, amount]], total }.
// `place` is 1-based; `difficulty` a slug.
export function payout({ place, knockoffs = 0, nearMisses = 0, difficulty = 'normal' }) {
  const k = DIFFICULTY_PAY[difficulty] ?? 1;
  const purse = PURSE[Math.min(Math.max(place, 1), PURSE.length) - 1];
  const lines = [[`${place}${place === 1 ? 'st' : place === 2 ? 'nd' : place === 3 ? 'rd' : 'th'} place`, Math.round(purse * k)]];
  if (knockoffs > 0) lines.push([`${knockoffs} knock-off${knockoffs > 1 ? 's' : ''}`, Math.round(knockoffs * KNOCKOFF_BONUS * k)]);
  if (nearMisses > 0) lines.push([`${nearMisses} near miss${nearMisses > 1 ? 'es' : ''}`, Math.round(nearMisses * NEAR_MISS_BONUS * k)]);
  return { lines, total: lines.reduce((t, [, v]) => t + v, 0) };
}

// Banks a race result; returns the new season (the input is not changed).
export function bankRace(s, result) {
  const p = payout(result);
  return {
    ...s,
    cash: s.cash + p.total,
    earned: s.earned + p.total,
    races: s.races + 1,
    wins: s.wins + (result.place === 1 ? 1 : 0),
  };
}

// Cost of the next level of `key`, or null when it is maxed.
export function nextCost(s, key) {
  const lvl = s[key] ?? 0;
  return lvl >= MAX_LEVEL ? null : COSTS[lvl];
}

// Buys one level of `key` if affordable; returns the new season or null.
export function buy(s, key) {
  const cost = nextCost(s, key);
  if (cost == null || s.cash < cost) return null;
  return { ...s, cash: s.cash - cost, [key]: s[key] + 1 };
}

// $1,234
export function money(n) {
  return '$' + Math.round(n).toLocaleString('en-US');
}
