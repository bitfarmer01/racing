import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseStyle, applyStyle, canvasPalette, direct, DIRECTION } from '../design.js';
import { CUE, HUD } from '../hud.js';

const CSS = '--ink: #14121a; --paper: #f4f1ea; --hot: #ff4d2e; --accent: #ffb347; --accent-rgb: 255, 179, 71;';

test('parseStyle reads the wasm token string', () => {
  const v = parseStyle(CSS);
  assert.equal(v['--hot'], '#ff4d2e');
  assert.equal(v['--accent-rgb'], '255, 179, 71');
  assert.equal(Object.keys(parseStyle('nonsense; color: red;')).length, 0);
});

test('applyStyle sets each token on the element', () => {
  const set = {};
  const el = { style: { setProperty: (k, v) => (set[k] = v) } };
  applyStyle(CSS, el);
  assert.equal(set['--accent'], '#ffb347');
  assert.equal(set['--ink'], '#14121a');
});

test('canvasPalette falls back to the shared palette', () => {
  const p = canvasPalette({ '--accent': '#ff5fa2' });
  assert.equal(p.accent, '#ff5fa2');
  assert.equal(p.hot, '#ff4d2e');
});

const hud = () => {
  const h = new Array(23).fill(0);
  h[HUD.POSITION] = 3;
  h[HUD.LAPS] = 3;
  h[HUD.BEST_LAP] = 61.25;
  return h;
};

test('every cue the wasm raises has a direction', () => {
  for (const code of Object.values(CUE)) {
    if (code === CUE.BEEP || code === CUE.GO) continue; // countdown is drawn from the HUD
    assert.ok(DIRECTION[code], `cue ${code} has no direction`);
  }
  assert.deepEqual(direct({ code: 999, a: 0, b: 0 }, hud()), {});
});

test('a best lap is called out and the final lap is announced', () => {
  const d = direct({ code: CUE.LAP, a: 2, b: 61.25 }, hud());
  assert.match(d.toast[0], /Lap 2 .*best/);
  assert.equal(d.toast[1], 'best');
  assert.deepEqual(d.later, [600, 'Final lap', 'bad']);
  assert.equal(direct({ code: CUE.LAP, a: 1, b: 70 }, hud()).later, undefined);
});

test('overtakes name the new place, gained or lost', () => {
  assert.deepEqual(direct({ code: CUE.OVERTAKE, a: 1, b: 0 }, hud()).toast, ['3rd', 'best']);
  assert.deepEqual(direct({ code: CUE.OVERTAKE, a: -1, b: 0 }, hud()).toast, ['3rd', 'bad']);
});

test('harder wall hits flash harder, capped', () => {
  const soft = direct({ code: CUE.WALL, a: 4, b: 0 }, hud()).flash;
  const hard = direct({ code: CUE.WALL, a: 60, b: 0 }, hud()).flash;
  assert.ok(hard > soft);
  assert.ok(hard <= 0.8);
  assert.ok(direct({ code: CUE.FINISHED, a: 1, b: 0 }, hud()).finished);
});

test('combat cues get their words', () => {
  assert.deepEqual(direct({ code: CUE.HIT_LANDED, a: 1, b: 0 }, hud()).toast, ['Kick!', 'best']);
  assert.deepEqual(direct({ code: CUE.HIT_LANDED, a: 0, b: 0 }, hud()).toast, ['Hit!', 'best']);
  assert.deepEqual(direct({ code: CUE.PARRIED, a: 0, b: 0 }, hud()).toast, ['Parry!', 'best']);
  assert.deepEqual(direct({ code: CUE.WAS_PARRIED, a: 0, b: 0 }, hud()).toast, ['Parried', 'bad']);
  assert.deepEqual(direct({ code: CUE.GUARDED, a: 0, b: 0 }, hud()).toast, ['Blocked', '']);
  assert.match(direct({ code: CUE.KNOCKOFF, a: 0, b: 0 }, hud()).toast[0], /full boost/);
  assert.deepEqual(direct({ code: CUE.LAUNCH, a: 1, b: 0 }, hud()).toast, ['Perfect start', 'best']);
  assert.deepEqual(direct({ code: CUE.LAUNCH, a: 0, b: 0 }, hud()).toast, ['Wheelspin', 'bad']);
});
