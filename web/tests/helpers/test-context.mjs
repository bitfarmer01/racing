// Test double for the Web Audio API subset web/audio.js uses: oscillators,
// gain/biquad filters, buffer sources, an analyser passthrough and param
// automation (setValueAtTime / setTargetAtTime + oscillator->param LFOs).
//
// Unlike a plain stub it can also RENDER: startRendering() walks the graph
// sample by sample at 44.1 kHz (RBJ biquads, integrated oscillator phase,
// first-order setTargetAtTime smoothing), so amplitude/band tests measure the
// actual audio math the engine chain performs, not just graph topology.
//
// Test-only surface: startedNodes (every oscillator/buffer source started),
// so cue tests can cross-check the cue log against the nodes actually fired.

const TWO_PI = Math.PI * 2;

export class TestAudioParam {
  constructor(value) {
    this._initial = value;
    this._events = []; // { t, type: 'set'|'target', value, tc }
    this._modulators = []; // nodes whose output sums onto this param
    this._v0memo = new Map();
  }
  setValueAtTime(value, t) {
    this._events.push({ t, type: 'set', value });
  }
  setTargetAtTime(target, startTime, tc) {
    this._events.push({ t: startTime, type: 'target', value: target, tc });
  }
  exponentialRampToValueAtTime(value, t) {
    this._events.push({ t, type: 'exp', value });
  }
  // Latest declared target/base (what RaceAudio.diagnostics() reads back).
  get value() {
    const events = this._events;
    if (events.length === 0) return this._initial;
    const last = events[events.length - 1];
    return last.type === 'set' ? last.value : last.value;
  }
  set value(v) {
    this._events.push({ t: -Infinity, type: 'set', value: v });
    this._events.sort((a, b) => a.t - b.t);
    this._v0memo.clear();
  }
  // Automation value at time t (modulators excluded; the renderer adds them).
  automationAt(t, depth = 0) {
    let base = this._initial;
    let idx = -1;
    for (let i = 0; i < this._events.length; i++) {
      if (this._events[i].t <= t) {
        base = this._events[i].type === 'set' ? this._events[i].value : this._events[i].value;
        idx = i;
      } else break;
    }
    if (idx >= 0 && this._events[idx].type === 'target') {
      const e = this._events[idx];
      if (!this._v0memo.has(idx) && depth < 64) {
        this._v0memo.set(idx, this.automationAt(e.t, depth + 1));
      }
      const v0 = this._v0memo.get(idx) ?? base;
      return e.value + (v0 - e.value) * Math.exp(-(t - e.t) / e.tc);
    }
    if (idx >= 0 && this._events[idx].type === 'exp') {
      // Geometric ramp from the previous event's value to e.value at e.t.
      const e = this._events[idx];
      if (t >= e.t) return e.value;
      const prev = this.automationAt(idx > 0 ? Math.min(e.t - 1e-6, t) : t, depth + 1);
      // progress measured from the previous event's time
      const prevT = idx > 0 ? this._events[idx - 1].t : t;
      const span = Math.max(e.t - prevT, 1e-6);
      const progress = Math.min(1, Math.max(0, (t - prevT) / span));
      const v0 = prev > 0 ? prev : e.value;
      return v0 * Math.pow(e.value / v0, progress);
    }
    return base;
  }
}

function waveSample(type, phase) {
  const p = phase - Math.floor(phase); // [0, 1)
  switch (type) {
    case 'sine': return Math.sin(TWO_PI * p);
    case 'sawtooth': return 2 * p - 1;
    case 'square': return p < 0.5 ? 1 : -1;
    case 'triangle': return 4 * Math.abs(p - 0.5) - 1;
    default: return Math.sin(TWO_PI * p);
  }
}

export class TestAudioContext {
  constructor() {
    this.sampleRate = 44100;
    this.state = 'running';
    this.currentTime = 0;
    this.destination = { _inputs: [] };
    this.startedNodes = [];
  }
  resume() { this.state = 'running'; }

  createOscillator() {
    const node = {
      kind: 'oscillator',
      type: 'sine',
      frequency: new TestAudioParam(440),
      detune: new TestAudioParam(0),
      _started: false,
      _phase: 0,
      connect(dest) { return connectTo(this, dest); },
      start: (t) => { node._started = true; node._startTime = t; this.startedNodes.push(node); },
      stop() {},
    };
    return node;
  }
  createGain() {
    const node = { kind: 'gain', gain: new TestAudioParam(1), _inputs: [], connect(d) { return connectTo(this, d); } };
    return node;
  }
  createBiquadFilter() {
    const node = {
      kind: 'biquad', type: 'lowpass',
      frequency: new TestAudioParam(350), Q: new TestAudioParam(1), gain: new TestAudioParam(0),
      _inputs: [], _state: [0, 0, 0, 0], _coeffs: null, _coeffTime: -1,
      connect(d) { return connectTo(this, d); },
    };
    return node;
  }
  createAnalyser() {
    const node = {
      kind: 'analyser', fftSize: 2048, frequencyBinCount: 1024,
      _inputs: [], connect(d) { return connectTo(this, d); },
      getFloatFrequencyData() {},
    };
    return node;
  }
  createBuffer(channels, length, sampleRate) {
    const data = new Float32Array(length);
    return { channels, length, sampleRate, data, getChannelData: (ch) => data };
  }
  createBufferSource() {
    const node = {
      kind: 'buffer', buffer: null, _inputs: [],
      connect(d) { return connectTo(this, d); },
      start: (t) => { node._started = true; node._startTime = t; this.startedNodes.push(node); },
      stop() {},
    };
    return node;
  }

  // Render [this.currentTime, this.currentTime + seconds) through the
  // destination sink, one sample at a time, honouring param automation and
  // LFO modulators. Returns the mono mix as Float32Array.
  startRendering(seconds) {
    const sr = this.sampleRate;
    const n = Math.floor(seconds * sr);
    const out = new Float32Array(n);
    const t0 = this.currentTime;
    const memo = new Map();
    for (let i = 0; i < n; i++) {
      const t = t0 + i / sr;
      out[i] = renderNode(this.destination, t, 1 / sr, memo, 0);
    }
    this._lastRender = out;
    return out;
  }
}

function connectTo(source, dest) {
  if (dest instanceof TestAudioParam) {
    dest._modulators.push(source);
  } else {
    dest._inputs.push(source);
  }
  return dest;
}

// Recursive per-sample evaluation of the DAG. Memoized per render via a
// (node -> last index/sample) map so shared subgraphs render once.
function renderNode(node, t, dt, memo, depth) {
  if (depth > 64) return 0;
  const m = memo.get(node);
  if (m && m.index === t) return m.sample;

  let sample = 0;
  switch (node.kind ?? 'destination') {
    case 'destination':
      for (const input of node._inputs) sample += renderNode(input, t, dt, memo, depth + 1);
      break;
    case 'gain': {
      let inSum = 0;
      for (const input of node._inputs) inSum += renderNode(input, t, dt, memo, depth + 1);
      sample = inSum * paramSample(node.gain, t, dt, memo, depth);
      break;
    }
    case 'biquad': {
      let inSum = 0;
      for (const input of node._inputs) inSum += renderNode(input, t, dt, memo, depth + 1);
      sample = biquadSample(node, inSum, t, dt, memo, depth);
      break;
    }
    case 'analyser':
      for (const input of node._inputs) sample += renderNode(input, t, dt, memo, depth + 1);
      break;
    case 'oscillator': {
      const f = paramSample(node.frequency, t, dt, memo, depth);
      const detune = paramSample(node.detune, t, dt, memo, depth);
      node._phase += f * Math.pow(2, detune / 1200) * dt;
      sample = waveSample(node.type, node._phase);
      break;
    }
    case 'buffer': {
      if (node._started && node.buffer) {
        const i = Math.floor((t - node._startTime) * node.buffer.sampleRate);
        sample = i >= 0 && i < node.buffer.length ? node.buffer.data[i] : 0;
      }
      break;
    }
  }
  memo.set(node, { index: t, sample });
  return sample;
}

// A param's per-sample value: automation plus connected modulator nodes
// (LFOs routed through depth gains onto AudioParams).
function paramSample(param, t, dt, memo, depth) {
  let v = param.automationAt(t);
  for (const mod of param._modulators) v += renderNode(mod, t, dt, memo, depth + 1);
  return v;
}

// RBJ biquads, coefficients recomputed only when the cutoff/gain moved.
function biquadSample(node, x, t, dt, memo, depth) {
  const fc = paramSample(node.frequency, t, dt, memo, depth);
  const Q = node.Q.automationAt(t);
  const gDb = node.gain.automationAt(t);
  const key = `${node.type}|${fc.toFixed(4)}|${Q.toFixed(4)}|${gDb.toFixed(4)}`;
  if (node._coeffKey !== key) {
    const sr = 44100;
    const w0 = TWO_PI * Math.min(fc, sr / 2 - 1) / sr;
    const cw = Math.cos(w0), sw = Math.sin(w0);
    let b0, b1, b2, a0, a1, a2;
    if (node.type === 'lowpass') {
      const alpha = sw / (2 * Q);
      b0 = (1 - cw) / 2; b1 = 1 - cw; b2 = (1 - cw) / 2;
      a0 = 1 + alpha; a1 = -2 * cw; a2 = 1 - alpha;
    } else if (node.type === 'highshelf') {
      const A = Math.pow(10, gDb / 40);
      const tc = 2 * Math.tan(w0 / 2);
      const sq = 2 * Math.sqrt(A) * Math.sqrt(A + 1) * tc / 2; // simplification of RBJ shelf
      const twoA = 2 * A;
      b0 = A * ((A + 1) + (A - 1) * cw + sq);
      b1 = -2 * A * ((A - 1) + (A + 1) * cw);
      b2 = A * ((A + 1) + (A - 1) * cw - sq);
      a0 = (A + 1) - (A - 1) * cw + sq;
      a1 = 2 * ((A - 1) - (A + 1) * cw);
      a2 = (A + 1) - (A - 1) * cw - sq;
      void twoA;
    } else { // lowshelf
      const A = Math.pow(10, gDb / 40);
      const tc = 2 * Math.tan(w0 / 2);
      const sq = 2 * Math.sqrt(A) * Math.sqrt(A + 1) * tc / 2;
      b0 = A * ((A + 1) - (A - 1) * cw + sq);
      b1 = 2 * A * ((A - 1) - (A + 1) * cw);
      b2 = A * ((A + 1) - (A - 1) * cw - sq);
      a0 = (A + 1) + (A - 1) * cw + sq;
      a1 = -2 * ((A - 1) + (A + 1) * cw);
      a2 = (A + 1) + (A - 1) * cw - sq;
    }
    node._coeffs = [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0];
    node._coeffKey = key;
  }
  const [b0, b1, b2, a1, a2] = node._coeffs;
  const [x1, x2, y1, y2] = node._state;
  const y = b0 * x + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
  node._state = [x, x1, y, y1];
  return y;
}
