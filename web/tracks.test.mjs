// Wave 8 failing acceptance tests for the web shell (spec:
// .obvious/specs/wave8-tracks.md, "Verification").
//
// RED STATE: both tests fail on wave-7-world at assertion — the shell has no
// per-course zone catalog and no track-select control. Each passes,
// unmodified, once the wave-8 implementation lands. Run with:
//
//     node --test web/tracks.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

// zones.js exists on the base (the demo course's presentation slice); the
// test asserts the wave-8 per-course catalog export on top of it. Importing
// an existing module keeps the failure at assertion, not at import.
import * as zones from './zones.js';

// The spec's expected course table, mirrored from crates/world's catalog
// tests: every course the shell must present, with its lap length and zone
// sequence.
const SPEC_COURSES = [
  {
    id: 'coast-run',
    lapLength: 2000,
    zoneNames: ['coast', 'town', 'forest', 'ridge'],
  },
  {
    id: 'island-hop',
    lapLength: 1600,
    zoneNames: ['lagoon', 'village', 'jungle', 'summit'],
  },
];

test('wave8: the shell exposes a per-course zone catalog', () => {
  assert.ok(
    Array.isArray(zones.COURSES),
    'web/zones.js must export COURSES — one entry per course (id, lapLength, zones) — ' +
      'so the lap-bar markers follow the selected course instead of the hardcoded DEMO_ZONES',
  );
  for (const expected of SPEC_COURSES) {
    const course = zones.COURSES.find((c) => c.id === expected.id);
    assert.ok(course, `COURSES must contain '${expected.id}'`);
    assert.equal(
      course.lapLength,
      expected.lapLength,
      `'${expected.id}' must carry the spec'd lap length`,
    );
    assert.deepEqual(
      course.zones.map((z) => z.name),
      expected.zoneNames,
      `'${expected.id}' must carry the spec'd zone sequence`,
    );
  }
});

test('wave8: the shell offers a track-select control', () => {
  const html = readFileSync(new URL('./index.html', import.meta.url), 'utf8');
  const loader = readFileSync(new URL('./loader.js', import.meta.url), 'utf8');
  // The boot overlay carries the control; the loader reads its value and
  // boots the selected course (spec: "Track-select flow").
  assert.match(
    html,
    /id="track-select"/,
    'web/index.html must carry a track-select control on the boot overlay',
  );
  assert.match(
    loader,
    /track-select/,
    'web/loader.js must read the selected course and boot it',
  );
});
