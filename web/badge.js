// Rider-state badge mapping, extracted pure so a headless probe can assert
// the badge text/class for each state value without booting the wasm game.
//
// The wasm exposes a rider-state bitmask (1 crashed, 2 off road, 4 braking);
// one badge is visible at a time, most severe first. Playtest finding: the
// old shell destructured [bit, label, class] as [text, class], so the badge
// displayed the raw bit with the label as its (nonexistent) CSS class.
export const STATE_LABELS = [
  [1, 'CRASHED', 'crashed'],
  [2, 'OFF ROAD', 'offroad'],
  [4, 'BRAKING', 'braking'],
];

// Returns { text, className } for a visible badge, or null when the badge is
// hidden (no state bit set and not counting down).
export function badgeForState(state, countingDown) {
  const found = STATE_LABELS.find(([bit]) => (state & bit) !== 0);
  if (found) return { text: found[1], className: `rider-state ${found[2]}` };
  if (countingDown) return { text: 'ON GRID', className: 'rider-state grid' };
  return null;
}
