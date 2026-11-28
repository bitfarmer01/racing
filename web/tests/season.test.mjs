import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  freshSeason, loadSeason, saveSeason, payout, bankRace, nextCost, buy, money, COSTS, MAX_LEVEL, UPGRADES,
} from '../season.js';

test('a broken or missing save starts a fresh season', () => {
  assert.deepEqual(loadSeason(null), freshSeason());
  assert.deepEqual(loadSeason('not json'), freshSeason());
  const s = loadSeason('{"cash": -5, "engine": 9, "handling": "2", "armour": 1.7}');
  assert.equal(s.cash, 0);
  assert.equal(s.engine, MAX_LEVEL);
  assert.equal(s.handling, 2);
  assert.equal(s.armour, 1);
});

test('a season survives a save and load', () => {
  const s = { ...freshSeason(), cash: 1234, engine: 2, wins: 3 };
  assert.deepEqual(loadSeason(saveSeason(s)), s);
});

test('winning pays more than losing, and harder fields pay more', () => {
  const win = payout({ place: 1 }).total;
  const fifth = payout({ place: 5 }).total;
  const last = payout({ place: 20 }).total;
  assert.ok(win > fifth && fifth > last && last > 0);
  assert.ok(payout({ place: 1, difficulty: 'hard' }).total > win);
  assert.ok(payout({ place: 1, difficulty: 'easy' }).total < win);
});

test('knock-offs and near misses are paid and listed', () => {
  const p = payout({ place: 3, knockoffs: 2, nearMisses: 1 });
  assert.equal(p.lines.length, 3);
  assert.match(p.lines[0][0], /3rd place/);
  assert.match(p.lines[1][0], /2 knock-offs/);
  assert.match(p.lines[2][0], /1 near miss$/);
  assert.equal(p.total, p.lines.reduce((t, [, v]) => t + v, 0));
});

test('banking a race adds the purse and counts the win', () => {
  const s = bankRace(freshSeason(), { place: 1, knockoffs: 1, difficulty: 'normal' });
  assert.equal(s.races, 1);
  assert.equal(s.wins, 1);
  assert.equal(s.cash, payout({ place: 1, knockoffs: 1 }).total);
  assert.equal(bankRace(s, { place: 4 }).wins, 1);
});

test('upgrades cost cash, climb one level at a time and stop at the top', () => {
  let s = { ...freshSeason(), cash: COSTS.reduce((a, b) => a + b, 0) };
  assert.equal(buy({ ...s, cash: 10 }, 'engine'), null, 'too poor');
  for (let i = 0; i < MAX_LEVEL; i++) {
    assert.equal(nextCost(s, 'engine'), COSTS[i]);
    s = buy(s, 'engine');
  }
  assert.equal(s.engine, MAX_LEVEL);
  assert.equal(s.cash, 0);
  assert.equal(nextCost(s, 'engine'), null);
  assert.equal(buy({ ...s, cash: 1e6 }, 'engine'), null, 'maxed');
  assert.deepEqual(UPGRADES.map((u) => u.key), ['engine', 'handling', 'armour']);
});

test('money reads like a price', () => {
  assert.equal(money(1234.4), '$1,234');
  assert.equal(money(0), '$0');
});
