// Race clock, derived entirely from state the shell already sees: the loader
// owns the fixed 60 Hz accumulator, so total race time is the count of ticks
// fed since the green light times 1/60, and lap boundaries are visible as
// changes in the lap counter. Extracted pure so a headless probe can drive
// the clock without booting the wasm game.

const TICK_HZ = 60;

// Formats seconds as m:ss.t (tenths read better under a countdown-accurate
// clock than hundredths that never settle).
export function formatTime(seconds) {
  if (!Number.isFinite(seconds) || seconds < 0) return '—';
  const m = Math.floor(seconds / 60);
  const s = seconds - m * 60;
  return `${m}:${s.toFixed(1).padStart(4, '0')}`;
}

export class RaceTimer {
  constructor() {
    this.ticks = 0; // ticks fed since the green light
    this.totalSeconds = 0;
    this.bestLapSeconds = null;
    this.lastLapSeconds = null;
    this.currentLapStartSeconds = 0;
    this.lastLapIndex = 0;
    this.started = false;
  }

  // Feed one sim tick. `countingDown` and `lapIndex` come from the glue the
  // loader already reads every frame; `finished` closes the final lap, since
  // the sim stops counting laps once the race is over.
  tick({ countingDown, lapIndex, finished }) {
    if (countingDown) return; // red lights: the race clock has not started
    this.started = true;
    this.ticks += 1;
    this.totalSeconds = this.ticks / TICK_HZ;

    // Lap boundary: the counter moved up since the last green-light tick.
    if (lapIndex > this.lastLapIndex) {
      this.closeLap();
      this.currentLapStartSeconds = this.totalSeconds;
      this.lastLapIndex = lapIndex;
    }
    if (finished) this.closeLap();
  }

  closeLap() {
    const lapSeconds = this.totalSeconds - this.currentLapStartSeconds;
    // closeLap can fire on every frame after the finish; only the first
    // close per lap boundary counts.
    if (lapSeconds <= 0) return;
    this.lastLapSeconds = lapSeconds;
    if (this.bestLapSeconds === null || lapSeconds < this.bestLapSeconds) {
      this.bestLapSeconds = lapSeconds;
    }
    this.currentLapStartSeconds = this.totalSeconds;
  }

  reset() {
    this.ticks = 0;
    this.totalSeconds = 0;
    this.bestLapSeconds = null;
    this.lastLapSeconds = null;
    this.currentLapStartSeconds = 0;
    this.lastLapIndex = 0;
    this.started = false;
  }
}
