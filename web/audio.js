// Web Audio for the shell, derived entirely from state the loader already
// sees: the engine tone tracks the same km/h figure the HUD renders, cues
// hang off the rider-state bitmask / stamina bar / countdown overlay, and the
// context is created-or-resumed inside the CLICK TO RIDE engagement (browser
// autoplay policy refuses audio before a user gesture). No wasm API changes.
//
// Engine tone shaping (playtest: high-speed engine read as a grating static
// drone): the old chain was one sawtooth through a FIXED 900 Hz lowpass, so
// at speed the sawtooth's upper harmonics (760/950 Hz at the ~190 Hz top
// speed pitch) sat permanently in the harshest band at constant amplitude.
// Three changes, loudness held comparable:
//   1. Speed-tightening lowpass — cutoff falls from ~1400 Hz at idle to
//      ~620 Hz at top speed, so the bright partials roll off exactly as the
//      speed (and the ear's exposure) rises. A fixed highshelf cut above
//      ~1.1 kHz plus a small lowshelf lift at ~160 Hz keeps the body so the
//      tone softens without simply getting quieter.
//   2. Breathing amplitude — a slow ~0.6 Hz LFO swells the engine level a
//      few percent, so the tone swells and recedes instead of sitting still.
//   3. Firing pulses — a second LFO whose rate tracks the engine pitch
//      (roughly one pulse per four revolutions) chops the amplitude a few
//      percent, modelling combustion pulses; plus a very slow detune drift
//      so the pitch itself never locks into one static buzz.
// The LFOs modulate their own gain nodes in the chain (not the engine gain
// parameter), so a silent engine (paused/finished) stays exactly silent.
//
// Extracted as a class so a headless probe can construct it, call update()
// directly, and assert the engine frequency tracks speed without a real
// AudioContext. An AnalyserNode taps the engine chain so probes can also
// assert the frequency-domain behaviour (engineSpectrum / bandEnergy).

export class RaceAudio {
  constructor() {
    this.ctx = null;
    this.master = null;
    this.engineOsc = null;
    this.engineGain = null;
    this.engineLowpass = null;
    this.engineAnalyser = null;
    this.engineSpectrumBuf = null;
    this.pulseLfo = null;
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
    // Sawtooth core through a speed-tightening filter stack reads as a motor
    // that leans into its powerband; frequency is driven per-frame from the
    // HUD's km/h figure. The analyser taps the shaped tone pre-master so
    // probes (and diagnostics) can read the actual delivered spectrum.
    this.engineOsc = this.ctx.createOscillator();
    this.engineOsc.type = 'sawtooth';
    this.engineOsc.frequency.value = 40;

    this.engineGain = this.ctx.createGain();
    this.engineGain.gain.value = 0;

    // Breathing LFO: slow level swell, ±7% around unity. Runs into its own
    // gain node in the chain so a zeroed engine gain still silences it.
    const breathGain = this.ctx.createGain();
    breathGain.gain.value = 1;
    const breathLfo = this.ctx.createOscillator();
    breathLfo.type = 'sine';
    breathLfo.frequency.value = 0.6;
    const breathDepth = this.ctx.createGain();
    breathDepth.gain.value = 0.07;
    breathLfo.connect(breathDepth).connect(breathGain.gain);
    breathLfo.start();

    // Firing-pulse LFO: rate follows the engine pitch (set per-frame), ±6%
    // chop — the pulse character that stops the tone holding one flat buzz.
    const pulseGain = this.ctx.createGain();
    pulseGain.gain.value = 1;
    this.pulseLfo = this.ctx.createOscillator();
    this.pulseLfo.type = 'sine';
    this.pulseLfo.frequency.value = 10;
    const pulseDepth = this.ctx.createGain();
    pulseDepth.gain.value = 0.06;
    this.pulseLfo.connect(pulseDepth).connect(pulseGain.gain);
    this.pulseLfo.start();

    // Pitch drift: a very slow ±11-cent wobble so the pitch itself breathes.
    const driftLfo = this.ctx.createOscillator();
    driftLfo.type = 'sine';
    driftLfo.frequency.value = 0.37;
    const driftDepth = this.ctx.createGain();
    driftDepth.gain.value = 11;
    driftLfo.connect(driftDepth).connect(this.engineOsc.detune);
    driftLfo.start();

    // Tone shaping: the cutoff tightens with speed (engineCutoff), the
    // highshelf trims the harsh top band, the lowshelf keeps the body so
    // loudness stays comparable to the pre-shaping tone.
    this.engineLowpass = this.ctx.createBiquadFilter();
    this.engineLowpass.type = 'lowpass';
    this.engineLowpass.frequency.value = RaceAudio.engineCutoff(0);
    this.engineLowpass.Q.value = 0.8;
    const highshelf = this.ctx.createBiquadFilter();
    highshelf.type = 'highshelf';
    highshelf.frequency.value = 1100;
    highshelf.gain.value = -7.5;
    const lowshelf = this.ctx.createBiquadFilter();
    lowshelf.type = 'lowshelf';
    lowshelf.frequency.value = 160;
    lowshelf.gain.value = 2;

    this.engineAnalyser = this.ctx.createAnalyser();
    this.engineAnalyser.fftSize = 2048;
    this.engineSpectrumBuf = new Float32Array(this.engineAnalyser.frequencyBinCount);

    this.engineOsc
      .connect(this.engineGain)
      .connect(breathGain)
      .connect(pulseGain)
      .connect(this.engineLowpass)
      .connect(highshelf)
      .connect(lowshelf)
      .connect(this.engineAnalyser)
      .connect(this.master);
    this.engineOsc.start();
  }

  // Engine frequency from the HUD's speed: 0 km/h sits at a 40 Hz idle,
  // ~180 km/h tops out near 190 Hz. Returned so probes can assert tracking.
  static engineFreq(speedKmh) {
    return 40 + Math.min(speedKmh, 180) * (150 / 180);
  }

  // Lowpass cutoff from the same speed: bright at idle, tightening to ~620 Hz
  // at top speed so the grating upper partials roll off as speed rises.
  static engineCutoff(speedKmh) {
    return 1400 - Math.min(speedKmh, 180) * (780 / 180);
  }

  // Engine level from the same speed. Slightly above the old curve (the
  // tightened filter and shelf cut remove energy) so perceived loudness
  // stays comparable instead of the fix just reading as "quieter".
  static engineBaseGain(speedKmh) {
    return 0.046 + (Math.min(speedKmh, 180) / 180) * 0.066;
  }

  // Mean linear magnitude of the engine spectrum in [lo, hi) Hz, from a
  // getFloatFrequencyData snapshot. Pure, so probes can assert band levels.
  static bandEnergy(spectrum, sampleRate, lo, hi) {
    const binHz = sampleRate / (spectrum.length * 2);
    const first = Math.max(0, Math.ceil(lo / binHz));
    const last = Math.min(spectrum.length, Math.ceil(hi / binHz));
    let sum = 0;
    let n = 0;
    for (let i = first; i < last; i++) {
      const mag = Math.pow(10, spectrum[i] / 20);
      sum += mag * mag;
      n++;
    }
    return n === 0 ? 0 : Math.sqrt(sum / n);
  }

  // Current engine spectrum (dB per bin) or null before unlock().
  engineSpectrum() {
    if (!this.engineAnalyser) return null;
    this.engineAnalyser.getFloatFrequencyData(this.engineSpectrumBuf);
    return this.engineSpectrumBuf;
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
    // The filter closes as speed rises — the high-end softening.
    const cutoff = RaceAudio.engineCutoff(speedKmh);
    this.engineLowpass.frequency.setTargetAtTime(cutoff, t, 0.12);
    // Firing pulses: roughly one per four revolutions, clamped to a band
    // that reads as pulse rather than a second buzz.
    this.pulseLfo.frequency.setTargetAtTime(
      Math.min(48, Math.max(8, freq / 4)),
      t,
      0.1,
    );
    const running = !paused && !finished && this.ctx.state === 'running';
    const engineGain = running ? RaceAudio.engineBaseGain(speedKmh) : 0;
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

  // Probe/diagnostic surface: the live engine frequency, filter cutoff,
  // context state and mute state.
  diagnostics() {
    return {
      state: this.ctx ? this.ctx.state : 'none',
      engineFreq: this.engineOsc ? this.engineOsc.frequency.value : null,
      engineCutoff: this.engineLowpass ? this.engineLowpass.frequency.value : null,
      muted: this.muted,
    };
  }
}
