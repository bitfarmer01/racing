import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  formatTime, ordinal, decodeCues, parseTracks, parseStandings, fitMap, ResolutionGovernor, CUE,
  DIFFICULTIES, pickDifficulty,
} from '../hud.js';

test('formatTime renders minutes, seconds and milliseconds', () => {
  assert.equal(formatTime(83.4567), '1:23.457');
  assert.equal(formatTime(5), '0:05.000');
  assert.equal(formatTime(-1), '–');
  assert.equal(formatTime(null), '–');
});

test('ordinal suffixes', () => {
  assert.deepEqual([1, 2, 3, 4, 11, 12, 13, 21, 22].map(ordinal), ['st', 'nd', 'rd', 'th', 'th', 'th', 'th', 'st', 'nd']);
});

test('cue triples decode in order', () => {
  const cues = decodeCues(new Float32Array([CUE.LAP, 1, 61.5, CUE.OVERTAKE, 1, 0]));
  assert.deepEqual(cues, [{ code: CUE.LAP, a: 1, b: 61.5 }, { code: CUE.OVERTAKE, a: 1, b: 0 }]);
});

test('track and standings lists parse', () => {
  const t = parseTracks('a|Alpha|3|Fast | flowing\nb|Beta|2|Tight');
  assert.deepEqual(t[0], { slug: 'a', name: 'Alpha', laps: 3, blurb: 'Fast | flowing' });
  assert.equal(t.length, 2);
  const s = parseStandings('You|190.5|1\nRex|0|0');
  assert.deepEqual(s, [{ name: 'You', time: 190.5, done: true }, { name: 'Rex', time: 0, done: false }]);
});

test('minimap fit keeps the track inside the padded square', () => {
  const pts = [-500, 0, 500, 0, 0, 250, 0, -250];
  const map = fitMap(pts, 400, 20);
  for (let i = 0; i < pts.length; i += 2) {
    const [x, y] = map(pts[i], pts[i + 1]);
    assert.ok(x >= 20 && x <= 380 && y >= 20 && y <= 380, `${x},${y}`);
  }
  assert.deepEqual(map(-500, 0).map(Math.round), [20, 200]);
});

test('resolution governor drops under load and recovers when calm', () => {
  const g = new ResolutionGovernor({ budgetMs: 17 });
  for (let i = 0; i < 120; i++) g.sample(33);
  assert.ok(g.scale < 0.8, `scale ${g.scale}`);
  const low = g.scale;
  for (let i = 0; i < 2000; i++) g.sample(10);
  assert.ok(g.scale > low && g.scale <= 1.0);
  assert.equal(g.sample(1000), g.scale, 'ignores long stalls');
});

test('difficulty list matches the game and unknown picks fall back to normal', () => {
  assert.deepEqual(DIFFICULTIES.map((d) => d.slug), ['easy', 'normal', 'hard']);
  assert.equal(pickDifficulty('hard'), 'hard');
  assert.equal(pickDifficulty(null), 'normal');
  assert.equal(pickDifficulty('insane'), 'normal');
});
