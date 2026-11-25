// Acceptance tests for the Wave 8.3 strategic sound design (spec:
// .obvious/specs/wave8-sound.md). All tests in this file FAIL on the spec
// branch and PASS once the layered engine synthesis is implemented in
// web/audio.js.
//
//   T1  core pitch follows RPM at fixed speed (not speed alone)
//   T2  supercharger whine layer sits at a fixed harmonic order above the
//       core and gets louder under load
//   T3  timbre is load/throttle-dependent (cutoff opens under throttle)
//
// Layer model these tests assert: RaceAudio.diagnostics().layers is an array
// of { name, freq, gain } — 'core' (inline-4 throb) and 'supercharger'
// (whine) at minimum — and update() accepts rpm (crankshaft RPM, from the
// 8.2 drivetrain glue) and throttle ('coast'|'accelerate'|'brake').

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { makeAudio, frame, runFrames, cueLog, renderSteady, rms, bandRms } from './helpers/test-util.mjs';

// Spec-locked pitch mapping: coreFreq(rpm) = 40 + 170 · clamp(rpm, 0, 12000) / 12000.
function expectedCoreFreq(rpm) {
  return 40 + 170 * Math.min(rpm, 12000) / 12000;
}

function layerByName(audio, name) {
  return (audio.diagnostics().layers ?? []).find((l) => l.name === name) ?? null;
}

test('T1: engine core pitch follows RPM at fixed speed', async () => {
  const { audio, ctx } = await makeAudio();

  // Same 120 km/h in both runs; only crankshaft RPM differs (different gear).
  runFrames(audio, ctx, 120, () => frame({ speedKmh: 120, rpm: 6000, throttle: 'accelerate' }));
  const lowRpm = layerByName(audio, 'core');
  assert.ok(lowRpm, 'diagnostics().layers must expose a "core" layer (web/audio.js)');
  assert.ok(
    Math.abs(lowRpm.freq - expectedCoreFreq(6000)) < 4,
    `core pitch at 6000 rpm should be ~${expectedCoreFreq(6000)} Hz, got ${lowRpm.freq} Hz (web/audio.js)`,
  );

  runFrames(audio, ctx, 120, () => frame({ speedKmh: 120, rpm: 11000, throttle: 'accelerate' }));
  const highRpm = layerByName(audio, 'core');
  assert.ok(
    Math.abs(highRpm.freq - expectedCoreFreq(11000)) < 4,
    `core pitch at 11000 rpm should be ~${expectedCoreFreq(11000)} Hz, got ${highRpm.freq} Hz — pitch must follow RPM, not speed alone (web/audio.js)`,
  );

  // And the reverse: RPM constant while speed changes must NOT move the pitch
  // (wheel slip / gear ratio independence of the tone source).
  runFrames(audio, ctx, 120, () => frame({ speedKmh: 150, rpm: 11000, throttle: 'accelerate' }));
  const sameRpm = layerByName(audio, 'core');
  assert.ok(
    Math.abs(sameRpm.freq - highRpm.freq) < 4,
    `pitch must be keyed on RPM when rpm is provided: moved ${Math.abs(sameRpm.freq - highRpm.freq)} Hz at constant 11000 rpm (web/audio.js)`,
  );
});

test('T2: supercharger whine layer tracks the core at fixed harmonic order and rises under load', async () => {
  const { audio, ctx } = await makeAudio();

  runFrames(audio, ctx, 120, () => frame({ speedKmh: 150, rpm: 10000, throttle: 'accelerate' }));
  const core = layerByName(audio, 'core');
  const whine = layerByName(audio, 'supercharger');
  assert.ok(core, 'diagnostics().layers must expose a "core" layer (web/audio.js)');
  assert.ok(whine, 'diagnostics().layers must expose a "supercharger" layer — the H2/S whine is the identity of the bike (web/audio.js)');

  // Centrifugal supercharger: the whine is a fixed harmonic multiple of core
  // speed (spec order 8, allowed 6–10), landing around 1–2 kHz at speed.
  const ratio = whine.freq / core.freq;
  assert.ok(
    ratio >= 6 && ratio <= 10,
    `whine/core frequency ratio must be 6–10 (spec: 8), got ${ratio.toFixed(2)} (web/audio.js)`,
  );

  // Centrifugal boost (and therefore whine presence) builds with load.
  const whineFull = whine.gain;
  runFrames(audio, ctx, 120, () => frame({ speedKmh: 150, rpm: 10000, throttle: 'coast' }));
  const whineCoast = layerByName(audio, 'supercharger').gain;
  assert.ok(
    whineFull > whineCoast * 1.2,
    `whine gain must rise under throttle: full=${whineFull} coast=${whineCoast}, need >20% louder (web/audio.js)`,
  );
});

test('T3: timbre is load/throttle-dependent at fixed speed', async () => {
  const { audio, ctx } = await makeAudio();

  runFrames(audio, ctx, 120, () => frame({ speedKmh: 90, rpm: 8000, throttle: 'accelerate' }));
  const cutoffFull = audio.diagnostics().engineCutoff;
  assert.ok(cutoffFull !== null, 'engineCutoff must be reported by diagnostics() (web/audio.js)');

  runFrames(audio, ctx, 120, () => frame({ speedKmh: 90, rpm: 8000, throttle: 'coast' }));
  const cutoffCoast = audio.diagnostics().engineCutoff;

  assert.ok(
    Math.abs(cutoffFull - cutoffCoast) >= 150,
    `lowpass cutoff must respond to throttle at fixed speed: full=${cutoffFull} Hz coast=${cutoffCoast} Hz, need ≥150 Hz apart (web/audio.js)`,
  );
});
