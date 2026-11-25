// Web Audio for the shell, derived entirely from state the loader already
// sees: the engine tone tracks the same km/h figure the HUD renders, cues
// hang off the rider-state bitmask / stamina bar / countdown overlay, and the
// context is created-or-resumed inside the CLICK TO RIDE engagement (browser
// autoplay policy refuses audio before a user gesture). No wasm API changes.
//
// Wave 8.3 layer model (spec: .obvious/specs/wave8-sound.md): the engine is a
// two-layer graph modeled on the Kawasaki Ninja H2/S's supercharged inline-4
// — a sawtooth 'core' (the inline-4 throb) pitched by crankshaft RPM when the
// drivetrain glue provides it (speed-derived fallback until Wave 8.2 lands),
// plus a 'supercharger' whine sine locked at harmonic order 8 above the core,
// its level building with revs and throttle load. The lowpass opens under
// throttle (load-dependent timbre); event cues fire once per event under a
// per-name 150 ms refractory and log to diagnostics().cues; the layered mix
// is held inside the spec's loudness (0.030 RMS) and harshness (0.0009
// 1–3 kHz band RMS) budgets at every speed.
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
    this.whineOsc = null;
    this.whineGain = null;
    this.engineAnalyser = null;
    this.engineSpectrumBuf = null;
    this.pulseLfo = null;
    this.muted = false;
    this.prevCrashed = false;
    this.prevZone = null;
    this.prevStamina = 1;
    this.prevCountdown = null;
    this.layers = [];
    this.cues = [];
    this.lastCueAt = {};
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

    // Supercharger whine: a sine locked at WHINE_ORDER times the core
    // frequency — the centrifugal-blower signature riding on top of the
    // throb. Mixed low into the shared tone stack ahead of the highshelf,
    // so the harsh-band budget caps it.
    this.whineOsc = this.ctx.createOscillator();
    this.whineOsc.type = 'sine';
    this.whineOsc.frequency.value = RaceAudio.WHINE_ORDER * 40;
    this.whineGain = this.ctx.createGain();
    this.whineGain.gain.value = 0;
    this.whineOsc.connect(this.whineGain).connect(this.engineLowpass);
    this.whineOsc.start();
  }

  // Engine frequency from the HUD's speed: 0 km/h sits at a 40 Hz idle,
  // ~180 km/h tops out near 190 Hz. Returned so probes can assert tracking.
  static engineFreq(speedKmh) {
    return 40 + Math.min(speedKmh, 180) * (150 / 180);
  }

  // Supercharger whine sits at a fixed harmonic order above the core —
  // order 8 lands the whine around 1–2 kHz at speed (spec allows 6–10).
  static WHINE_ORDER = 8;
  // Full throttle opens the lowpass by this much at fixed speed
  // (load-dependent timbre; the spec requires ≥150 Hz).
  static THROTTLE_CUTOFF_OPEN = 250;
  // Per-name cue refractory: an event fires once, never per frame.
  static CUE_REFRACTORY_SECS = 0.15;

  // Core pitch from crankshaft RPM: 40 Hz idle → 210 Hz at the 12000
  // redline (spec-locked mapping).
  static coreFreq(rpm) {
    return 40 + 170 * (Math.min(rpm, 12000) / 12000);
  }

  // Lowpass cutoff from the same speed: bright at idle, tightening to ~620 Hz
  // at top speed so the grating upper partials roll off as speed rises.
  static engineCutoff(speedKmh) {
    return 1400 - Math.min(speedKmh, 180) * (780 / 180);
  }

  // Engine level from speed. Retuned for the layered mix: the
  // throttle-opened filter passes more of the core's upper harmonics, and
  // the 1–3 kHz harsh-band budget (0.0009 band RMS at 180 km/h full
  // throttle, measured by the offline harness) is set by the CORE's own
  // partials there — the whine layer is already buried under the lowpass.
  // So the core gives back headroom (about half the old curve at top
  // speed): the budgets, not the old single-layer level, are the locked
  // constraint the layers share.
  static engineBaseGain(speedKmh) {
    return 0.017 + (Math.min(speedKmh, 180) / 180) * 0.015;
  }

  // Supercharger whine level: builds with revs (core frequency stands in
  // for blower speed) and with throttle load — ≥20% louder at full throttle
  // than coast at fixed rpm. Kept small: the whine lands partly in the
  // 1–3 kHz harshness band, so its mix is capped by the 0.0009 band budget.
  static whineLevel(coreFreq, throttle) {
    const load = throttle === 'accelerate' ? 1.6 : throttle === 'brake' ? 0.7 : 1.0;
    return 0.0032 * (coreFreq / 210) * load;
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
  // finished — plus the Wave 8.3 additive inputs rpm (crankshaft RPM from
  // the 8.2 drivetrain glue; null/absent falls back to the speed-derived
  // pitch), throttle ('coast' | 'accelerate' | 'brake', mirroring
  // bike::Control) and zone (HUD zone id). Cue edges — each fires once per
  // event under a 150 ms per-name refractory, logged in diagnostics().cues:
  //   - bit 1 (crashed) rising   -> crash noise burst
  //   - bit 1 (crashed) clearing -> recovery chirp (pick-the-bike-up beat)
  //   - stamina drop > 0.08 in one frame -> hit thud (a landed punch or kick)
  //   - zone id change between frames -> soft two-note marker
  //   - countdown second change -> red-light beep, green light chirp
  update({ speedKmh, state, stamina, countingDown, countdownSecs, paused, finished, rpm, throttle, zone }) {
    if (!this.ctx) return;
    const t = this.ctx.currentTime;
    const load = throttle === 'accelerate' || throttle === 'brake' ? throttle : 'coast';

    // Core pitch follows crankshaft RPM when the sim provides it — gear
    // changes become audible dips — falling back to the speed-derived curve
    // until the 8.2 glue getter lands.
    const coreFreq = rpm == null ? RaceAudio.engineFreq(speedKmh) : RaceAudio.coreFreq(rpm);
    this.engineOsc.frequency.setTargetAtTime(coreFreq, t, 0.05);
    // The filter closes as speed rises and opens under throttle — the
    // load-dependent timbre (full throttle ≥150 Hz brighter at fixed speed,
    // coasting darkens).
    const cutoff =
      RaceAudio.engineCutoff(speedKmh) + (load === 'accelerate' ? RaceAudio.THROTTLE_CUTOFF_OPEN : 0);
    this.engineLowpass.frequency.setTargetAtTime(cutoff, t, 0.12);
    // Firing pulses: roughly one per four revolutions, clamped to a band
    // that reads as pulse rather than a second buzz.
    this.pulseLfo.frequency.setTargetAtTime(
      Math.min(48, Math.max(8, coreFreq / 4)),
      t,
      0.1,
    );
    const running = !paused && !finished && this.ctx.state === 'running';
    const coreGain = running ? RaceAudio.engineBaseGain(speedKmh) : 0;
    this.engineGain.gain.setTargetAtTime(coreGain, t, 0.08);

    // Supercharger whine: order 8 above the core, building with revs and
    // throttle load (a centrifugal blower makes boost with revs under
    // power), silent whenever the engine is silent.
    const whineFreq = coreFreq * RaceAudio.WHINE_ORDER;
    this.whineOsc.frequency.setTargetAtTime(whineFreq, t, 0.05);
    const whineGain = running ? RaceAudio.whineLevel(coreFreq, load) : 0;
    this.whineGain.gain.setTargetAtTime(whineGain, t, 0.08);

    // Probe surface: the layer levels the tests (and any browser-side
    // probe) read back from diagnostics().
    this.layers = [
      { name: 'core', freq: coreFreq, gain: coreGain },
      { name: 'supercharger', freq: whineFreq, gain: whineGain },
    ];

    // Crash cue: crashed bit rising edge. Recovery cue: the same bit
    // clearing after a crash — the rider picking the bike back up.
    const crashed = (state & 1) !== 0;
    if (crashed && !this.prevCrashed) {
      this.fireCue('crash', () => this.crash());
    } else if (!crashed && this.prevCrashed) {
      this.fireCue('recovery', () => this.recovery());
    }
    this.prevCrashed = crashed;

    // Hit cue: a sudden stamina drop is what a landed combat hit looks like
    // in the bar the shell already renders.
    if (this.prevStamina - stamina > 0.08) {
      this.fireCue('hit', () => this.blip(110, 0.12, 'triangle', 0.5));
    }
    this.prevStamina = stamina;

    // Zone cue: crossing into a new zone marks the crossing frame, not the
    // frames spent inside the zone.
    if (typeof zone === 'string') {
      if (this.prevZone === null) {
        this.prevZone = zone; // first frame establishes the starting zone
      } else if (zone !== this.prevZone) {
        this.prevZone = zone;
        this.fireCue('zone', () => this.zoneMarker());
      }
    }

    // Countdown cue: beep each red-light second, higher chirp on the green.
    if (countingDown) {
      if (this.prevCountdown !== null && countdownSecs !== this.prevCountdown) {
        this.fireCue('countdown', () => this.blip(440, 0.15));
      }
      this.prevCountdown = countdownSecs;
    } else if (this.prevCountdown !== null) {
      this.fireCue('green', () => this.blip(880, 0.3)); // green light
      this.prevCountdown = null;
    }
  }

  // Fire a named cue exactly once per event: a per-name refractory guards
  // against double-fires, and every fired cue lands in the ordered
  // diagnostics().cues log (oldest first).
  fireCue(name, play) {
    const now = this.ctx.currentTime;
    if (now - (this.lastCueAt[name] ?? -Infinity) < RaceAudio.CUE_REFRACTORY_SECS) return;
    this.lastCueAt[name] = now;
    this.cues.push({ name, at: now });
    play();
  }

  // Recovery chirp: a short rising tone for the pick-the-bike-up beat.
  recovery() {
    if (!this.ctx) return;
    const osc = this.ctx.createOscillator();
    const g = this.ctx.createGain();
    const t = this.ctx.currentTime;
    osc.type = 'sine';
    osc.frequency.setValueAtTime(330, t);
    osc.frequency.exponentialRampToValueAtTime(880, t + 0.18);
    g.gain.setValueAtTime(0.25, t);
    g.gain.exponentialRampToValueAtTime(0.001, t + 0.2);
    osc.connect(g).connect(this.master);
    osc.start(t);
    osc.stop(t + 0.2);
  }

  // Zone marker: a soft two-note figure — a landable chime, not a horn.
  zoneMarker() {
    if (!this.ctx) return;
    const t = this.ctx.currentTime;
    for (const [freq, offset] of [[523.25, 0], [783.99, 0.1]]) {
      const osc = this.ctx.createOscillator();
      const g = this.ctx.createGain();
      osc.type = 'sine';
      osc.frequency.setValueAtTime(freq, t + offset);
      g.gain.setValueAtTime(0.16, t + offset);
      g.gain.exponentialRampToValueAtTime(0.001, t + offset + 0.12);
      osc.connect(g).connect(this.master);
      osc.start(t + offset);
      osc.stop(t + offset + 0.12);
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
  // context state, mute state — plus the Wave 8.3 additions, the synthesis
  // layers ({ name, freq, gain }) and the ordered cue log ({ name, at }).
  diagnostics() {
    return {
      state: this.ctx ? this.ctx.state : 'none',
      engineFreq: this.engineOsc ? this.engineOsc.frequency.value : null,
      engineCutoff: this.engineLowpass ? this.engineLowpass.frequency.value : null,
      muted: this.muted,
      layers: this.layers.map((layer) => ({ ...layer })),
      cues: this.cues.slice(),
    };
  }
}
