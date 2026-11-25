// Web Audio for the shell, derived entirely from state the loader already
// sees: the engine tone tracks the same km/h figure the HUD renders, cues
// hang off the rider-state bitmask / stamina bar / countdown overlay, and the
// context is created-or-resumed inside the CLICK TO RIDE engagement (browser
// autoplay policy refuses audio before a user gesture). No wasm API changes.
//
// Extracted as a class so a headless probe can construct it, call update()
// directly, and assert the engine frequency tracks speed without a real
// AudioContext.

export class RaceAudio {
  constructor() {
    this.ctx = null;
    this.master = null;
    this.engineOsc = null;
    this.engineGain = null;
    this.muted = false;
    this.prevState = 0;
    this.prevStamina = 1;
    this.prevCountdown = null;
  }

  // Called from the first user-gesture engagement (click / keydown on the
  // CLICK TO RIDE overlay path). Creates the context on the gesture and
  // resumes it if the browser suspended it.
  unlock() {
    if (!this.ctx) {
      const Ctx = window.AudioContext || window.webkitAudioContext;
      if (!Ctx) return; // very old browsers: the game stays silent, not broken
      this.ctx = new Ctx();
      this.master = this.ctx.createGain();
      this.master.gain.value = this.muted ? 0 : 0.5;
      this.master.connect(this.ctx.destination);
      this.startEngine();
    }
    if (this.ctx.state === 'suspended') this.ctx.resume();
  }

  startEngine() {
    // Engine: a sawtooth through a lowpass reads as a buzzy motor at low
    // gain; frequency is driven per-frame from the HUD's km/h figure.
    this.engineOsc = this.ctx.createOscillator();
    this.engineOsc.type = 'sawtooth';
    this.engineOsc.frequency.value = 40;
    this.engineGain = this.ctx.createGain();
    this.engineGain.gain.value = 0;
    const filter = this.ctx.createBiquadFilter();
    filter.type = 'lowpass';
    filter.frequency.value = 900;
    this.engineOsc.connect(this.engineGain).connect(filter).connect(this.master);
    this.engineOsc.start();
  }

  // Engine frequency from the HUD's speed: 0 km/h sits at a 40 Hz idle,
  // ~180 km/h tops out near 190 Hz. Returned so probes can assert tracking.
  static engineFreq(speedKmh) {
    return 40 + Math.min(speedKmh, 180) * (150 / 180);
  }

  // One short pitched blip; used for countdown and combat cues.
  blip(freq, duration, type = 'square', gain = 0.35) {
    if (!this.ctx) return;
    const osc = this.ctx.createOscillator();
    const g = this.ctx.createGain();
    const t = this.ctx.currentTime;
    osc.type = type;
    osc.frequency.setValueAtTime(freq, t);
    g.gain.setValueAtTime(gain, t);
    g.gain.exponentialRampToValueAtTime(0.001, t + duration);
    osc.connect(g).connect(this.master);
    osc.start(t);
    osc.stop(t + duration);
  }

  // Per-frame update, fed exactly what the loader already reads for the HUD:
  // speedKmh (speed-value), state (rider_state bitmask), stamina
  // (stamina-fill), countingDown/countdownSecs (countdown overlay), paused,
  // finished. Cue edges:
  //   - bit 1 (crashed) rising  -> crash noise burst
  //   - stamina drop > 0.08 in one frame -> hit thud (a landed punch or kick)
  //   - countdown second change -> red-light beep, green light chirp
  update({ speedKmh, state, stamina, countingDown, countdownSecs, paused, finished }) {
    if (!this.ctx) return;
    const t = this.ctx.currentTime;

    // Engine tone follows the HUD speed; silent while paused/finished so the
    // tone stops when the sim does.
    const freq = RaceAudio.engineFreq(speedKmh);
    this.engineOsc.frequency.setTargetAtTime(freq, t, 0.05);
    const running = !paused && !finished && this.ctx.state === 'running';
    const engineGain = running ? 0.04 + Math.min(speedKmh, 180) / 180 * 0.06 : 0;
    this.engineGain.gain.setTargetAtTime(engineGain, t, 0.08);

    // Crash cue: crashed bit rising edge.
    if ((state & 1) !== 0 && (this.prevState & 1) === 0) {
      this.crash();
    }
    this.prevState = state;

    // Hit cue: a sudden stamina drop is what a landed combat hit looks like
    // in the bar the shell already renders.
    if (this.prevStamina - stamina > 0.08) {
      this.blip(110, 0.12, 'triangle', 0.5);
    }
    this.prevStamina = stamina;

    // Countdown cue: beep each red-light second, higher chirp on the green.
    if (countingDown) {
      if (this.prevCountdown !== null && countdownSecs !== this.prevCountdown) {
        this.blip(440, 0.15);
      }
      this.prevCountdown = countdownSecs;
    } else if (this.prevCountdown !== null) {
      this.blip(880, 0.3); // green light
      this.prevCountdown = null;
    }
  }

  crash() {
    if (!this.ctx) return;
    // Filtered noise burst reads as a skid-and-impact far better than a tone.
    const len = Math.floor(this.ctx.sampleRate * 0.5);
    const buffer = this.ctx.createBuffer(1, len, this.ctx.sampleRate);
    const data = buffer.getChannelData(0);
    for (let i = 0; i < len; i++) {
      data[i] = (Math.random() * 2 - 1) * (1 - i / len);
    }
    const src = this.ctx.createBufferSource();
    src.buffer = buffer;
    const g = this.ctx.createGain();
    g.gain.value = 0.6;
    const filter = this.ctx.createBiquadFilter();
    filter.type = 'lowpass';
    filter.frequency.value = 700;
    src.connect(filter).connect(g).connect(this.master);
    src.start();
  }

  setMuted(muted) {
    this.muted = muted;
    if (this.master) {
      this.master.gain.setTargetAtTime(muted ? 0 : 0.5, this.ctx.currentTime, 0.02);
    }
  }

  // Probe/diagnostic surface: the live engine frequency and context state.
  diagnostics() {
    return {
      state: this.ctx ? this.ctx.state : 'none',
      engineFreq: this.engineOsc ? this.engineOsc.frequency.value : null,
      muted: this.muted,
    };
  }
}
