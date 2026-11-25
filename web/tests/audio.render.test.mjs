// Acceptance tests for Wave 8.3 mix budgets (spec:
// .obvious/specs/wave8-sound.md). Both FAIL on the spec branch and PASS once
// implemented in web/audio.js.
//
//   T6  no sustained amplitude above the loudness budget at any speed
//   T7  no harsh-band resurgence at top speed (1–3 kHz RMS ceiling)
//
// Both are measured, not inspected: the harness renders the engine chain
// offline (44.1 kHz sample-by-sample through the real node graph —
// helpers/test-context.mjs implements the OfflineAudioContext-shaped probe)
// and computes time-domain RMS and FFT band RMS. Budgets were calibrated
// against today's single-sawtooth chain at b0b83d1 (max RMS 0.0333 at
// 180 km/h; 1–3 kHz band RMS 0.001026) and set BELOW those values on
// purpose: the layered graph adds a whine layer that lives partly in the
// 1–3 kHz band, so the budgets force the mix to rebalance rather than sum.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { makeAudio, renderSteady, rms, bandRms } from './helpers/test-util.mjs';

// Spec-locked loudness budget: rendered output RMS ≤ 0.030 at every speed.
const LOUDNESS_BUDGET = 0.030;
// Spec-locked harsh-band budget: 1–3 kHz band RMS ≤ 0.0009 at top speed.
const HARSH_BAND_BUDGET = 0.0009;

test('T6: rendered amplitude stays within the loudness budget at every speed', async () => {
  let worst = { speedKmh: null, level: 0 };
  for (const speedKmh of [0, 45, 90, 135, 180]) {
    const { audio, ctx } = await makeAudio();
    const { samples } = renderSteady(audio, ctx, { speedKmh, rpm: 10000, throttle: 'accelerate' });
    const level = rms(samples);
    if (level > worst.level) worst = { speedKmh, level };
  }
  assert.ok(
    worst.level <= LOUDNESS_BUDGET,
    `engine mix exceeds the loudness budget: ${worst.level.toFixed(4)} at ${worst.speedKmh} km/h > ${LOUDNESS_BUDGET} sustained RMS (web/audio.js)`,
  );
});

test('T7: the harsh band stays under budget at top speed', async () => {
  const { audio, ctx } = await makeAudio();
  const { samples, sampleRate } = renderSteady(audio, ctx, {
    speedKmh: 180, rpm: 12000, throttle: 'accelerate',
  });
  const harsh = bandRms(samples, sampleRate, 1000, 3000);
  assert.ok(
    harsh <= HARSH_BAND_BUDGET,
    `1–3 kHz band RMS ${harsh.toFixed(6)} exceeds the harshness budget ${HARSH_BAND_BUDGET} at 180 km/h — the whine layer must be mixed below the harshness ceiling (web/audio.js)`,
  );
});
