// Acceptance tests for Wave 8.3 event cues (spec:
// .obvious/specs/wave8-sound.md). Both FAIL on the spec branch and PASS once
// implemented in web/audio.js.
//
//   T4  the recovery cue fires exactly once per crash recovery — not per frame
//   T5  the zone-transition cue fires exactly once per crossing — not per frame
//
// Spec-locked probe surface: RaceAudio.diagnostics() exposes `cues`, an
// array of { name, at } — every cue fired since unlock(), in order. Cue
// names: 'crash', 'hit', 'countdown', 'green', 'recovery', 'zone'.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { makeAudio, frame, runFrames, cueLog } from './helpers/test-util.mjs';

test('T4: recovery cue fires exactly once per recovery', async () => {
  const { audio, ctx } = await makeAudio();

  // Ride normally, crash, hold the crashed state for 130 frames (~2.2 s —
  // past the sim's 2.0 s CRASH_DURATION_SECS), then recover and hold 60
  // frames. The recovery cue is the moment the crashed bit clears.
  runFrames(audio, ctx, 30, () => frame({ speedKmh: 100, rpm: 9000, throttle: 'accelerate' }));

  runFrames(audio, ctx, 130, () => frame({ speedKmh: 20, rpm: 3000, throttle: 'coast', state: 1, stamina: 0.6 }));

  const before = cueLog(audio).filter((c) => c.name === 'recovery').length;
  const crashCues = cueLog(audio).filter((c) => c.name === 'crash').length;
  assert.equal(crashCues, 1, 'the crash cue must fire exactly once at the rising edge (web/audio.js)');

  runFrames(audio, ctx, 60, () => frame({ speedKmh: 30, rpm: 4000, throttle: 'accelerate' }));

  const recoveries = cueLog(audio).filter((c) => c.name === 'recovery').length - before;
  assert.equal(
    recoveries, 1,
    `exactly one recovery cue expected when the crashed bit clears, got ${recoveries} — cues must fire once per event, never per frame (web/audio.js)`,
  );
});

test('T5: zone-transition cue fires exactly once per crossing', async () => {
  const { audio, ctx } = await makeAudio();

  // Cross from coast into town, then hold town for 90 frames. The cue marks
  // the crossing, not the frames spent in the zone.
  runFrames(audio, ctx, 30, () => frame({ speedKmh: 100, rpm: 9000, throttle: 'accelerate', zone: 'coast' }));

  runFrames(audio, ctx, 90, () => frame({ speedKmh: 100, rpm: 9000, throttle: 'accelerate', zone: 'town' }));

  const zoneCues = cueLog(audio).filter((c) => c.name === 'zone').length;
  assert.equal(
    zoneCues, 1,
    `exactly one zone cue expected for one zone crossing, got ${zoneCues} — cues must fire once per event, never per frame (web/audio.js)`,
  );
});
