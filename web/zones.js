// Per-course zone presentation catalog: one entry per selectable course
// (spec: .obvious/specs/wave8-tracks.md, "HUD integration").
//
// Each entry mirrors its course's zone table from crates/world/src/courses.rs
// — the source of truth for the spans; the shell re-states only the
// presentation-relevant slice (name + per-lap distance span, in meters). The
// loader feeds the track-select control's chosen entry to
// buildZoneMarkers/highlightZone, so the lap bar follows the selected course.
// If a course's zone table ever changes there, this table must be updated to
// match — the build/test gates cannot catch a drift between the two.
export const COURSES = [
  {
    id: 'coast-run',
    name: 'Coast Run',
    lapLength: 2000,
    zones: [
      { name: 'coast', start: 0, end: 520 },
      { name: 'town', start: 520, end: 1080 },
      { name: 'forest', start: 1080, end: 1600 },
      { name: 'ridge', start: 1600, end: 2000 },
    ],
  },
  {
    id: 'island-hop',
    name: 'Island Hop',
    lapLength: 1600,
    zones: [
      { name: 'lagoon', start: 0, end: 480 },
      { name: 'village', start: 480, end: 920 },
      { name: 'jungle', start: 920, end: 1280 },
      { name: 'summit', start: 1280, end: 1600 },
    ],
  },
];

// Builds the DOM for the lap bar's zone ticks: one tick at each zone
// boundary (the first zone starts at 0, the bar's own left edge) and one
// label per zone. Returns a container to append into the lap-bar element.
export function buildZoneMarkers(zones, lapLength) {
  const markers = document.createElement('div');
  markers.id = 'zone-markers';
  for (const zone of zones) {
    if (zone.start > 0) {
      const tick = document.createElement('span');
      tick.className = 'zone-tick';
      tick.dataset.zone = zone.name;
      tick.style.left = `${((zone.start / lapLength) * 100).toFixed(2)}%`;
      markers.appendChild(tick);
    }
    const label = document.createElement('span');
    label.className = 'zone-label';
    label.dataset.zone = zone.name;
    label.style.left = `${(((zone.start + zone.end) / 2 / lapLength) * 100).toFixed(2)}%`;
    label.textContent = zone.name;
    markers.appendChild(label);
  }
  return markers;
}

// Marks the one zone label whose span contains `distance` as current, and
// unmarks the rest. Pure DOM update; safe to call every frame.
export function highlightZone(markers, zones, distance) {
  const current = zones.find((z) => distance >= z.start && distance < z.end);
  const name = current ? current.name : null;
  if (markers.dataset.current === name) return;
  markers.dataset.current = name;
  for (const el of markers.querySelectorAll('.zone-label')) {
    el.classList.toggle('current', el.dataset.zone === name);
  }
}
