// Zone presentation config for the demo course's lap-progress bar.
//
// DUPLICATED from crates/world/src/geometry.rs — ZoneLayout::FOR_TRACK, which
// is the source of truth for the demo course's fixed zone table. The demo
// course is fixed, so the shell re-states only the presentation-relevant
// slice (name + per-lap distance span, in meters of the 2 000 m lap). If the
// course's zone table ever changes there, this table must be updated to
// match — the build/test gates cannot catch a drift between the two.
export const DEMO_ZONES = [
  { name: 'coast', start: 0, end: 520 },
  { name: 'town', start: 520, end: 1080 },
  { name: 'forest', start: 1080, end: 1600 },
  { name: 'ridge', start: 1600, end: 2000 },
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
