// Shared plumbing for the audio acceptance tests: builds an unlocked
// RaceAudio over the renderable TestAudioContext, advances sim frames the
// way web/loader.js does (one update() per 1/60 s), and measures rendered
// buffers (RMS + band RMS via a small radix-2 FFT).

import { TestAudioContext } from './test-context.mjs';

// A RaceAudio wired to a fresh TestAudioContext. unlock() rides the fake
// window exactly like the CLICK TO RIDE gesture does in the shell.
export async function makeAudio() {
  const { RaceAudio } = await import('../../audio.js');
  const ctx = new TestAudioContext();
  globalThis.window = {
    AudioContext: class {
      constructor() { return ctx; }
    },
  };
  const audio = new RaceAudio();
  audio.unlock();
  return { audio, ctx };
}

// The full sim-state frame the spec locks as audio's input contract. update()
// ignores unknown fields today, so feeding rpm/throttle/zone now is harmless
// pre-implementation and load-bearing once Wave 8.3 lands.
export function frame({ speedKmh = 0, rpm = null, throttle = 'coast', zone = 'coast', ...rest } = {}) {
  return {
    speedKmh,
    rpm,
    throttle,
    zone,
    state: 0,
    stamina: 1,
    countingDown: false,
    countdownSecs: 0,
    paused: false,
    finished: false,
    ...rest,
  };
}

// Drive n 60 Hz frames, advancing the context clock like the real loop does.
export function runFrames(audio, ctx, n, makeFrame = () => frame()) {
  for (let i = 0; i < n; i++) {
    audio.update(makeFrame(i));
    ctx.currentTime += 1 / 60;
  }
}

// Cue log from the spec-locked probe surface ([] pre-implementation).
export function cueLog(audio) {
  return audio.diagnostics().cues ?? [];
}

export function rms(samples) {
  let sum = 0;
  for (let i = 0; i < samples.length; i++) sum += samples[i] * samples[i];
  return Math.sqrt(sum / samples.length);
}

function fft(re, im) {
  const n = re.length;
  for (let i = 1, j = 0; i < n; i++) {
    let bit = n >> 1;
    for (; j & bit; bit >>= 1) j ^= bit;
    j ^= bit;
    if (i < j) {
      [re[i], re[j]] = [re[j], re[i]];
      [im[i], im[j]] = [im[j], im[i]];
    }
  }
  for (let len = 2; len <= n; len <<= 1) {
    const ang = (-2 * Math.PI) / len;
    const wr = Math.cos(ang), wi = Math.sin(ang);
    for (let i = 0; i < n; i += len) {
      let cr = 1, ci = 0;
      for (let k = 0; k < len / 2; k++) {
        const ur = re[i + k], ui = im[i + k];
        const vr = re[i + k + len / 2] * cr - im[i + k + len / 2] * ci;
        const vi = re[i + k + len / 2] * ci + im[i + k + len / 2] * cr;
        re[i + k] = ur + vr; im[i + k] = ui + vi;
        re[i + k + len / 2] = ur - vr; im[i + k + len / 2] = ui - vi;
        const ncr = cr * wr - ci * wi;
        ci = cr * wi + ci * wr; cr = ncr;
      }
    }
  }
}

// Single-sided band RMS of a rendered buffer in [lo, hi) Hz. Calibrated
// against a known 1 kHz unit sine in the harness's own dev check.
export function bandRms(samples, sampleRate, lo, hi) {
  const n = 1 << Math.floor(Math.log2(samples.length));
  const re = new Float64Array(n), im = new Float64Array(n);
  for (let i = 0; i < n; i++) re[i] = samples[i];
  fft(re, im);
  const binHz = sampleRate / n;
  let power = 0;
  for (let k = 1; k < n / 2; k++) {
    const f = k * binHz;
    if (f >= lo && f < hi) power += re[k] * re[k] + im[k] * im[k];
  }
  // sqrt(2) folds the negative-frequency half back in; / n^2 normalizes the DFT.
  return (Math.sqrt(power) / n) * Math.SQRT2;
}

// Steady-state render: warm the engine chain with warmupFrames of updates so
// every setTargetAtTime has settled, then render renderSeconds through the
// graph. Returns { samples, sampleRate }.
export function renderSteady(audio, ctx, state, { warmupFrames = 120, renderSeconds = 0.5 } = {}) {
  runFrames(audio, ctx, warmupFrames, () => frame(state));
  return { samples: ctx.startRendering(renderSeconds), sampleRate: ctx.sampleRate };
}
