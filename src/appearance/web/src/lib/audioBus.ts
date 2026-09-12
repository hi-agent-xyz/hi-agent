// AudioBus — one AudioContext + AnalyserNode that the Presence reads each frame.
//
// The microphone is the only source, and it is the only one this graph may ever
// have: the agent's own voice was attached here once, and rendering it through
// Web Audio is what hid it from the platform's echo canceller and let the agent
// hear itself back through the mic (see `voicePlayer.ts`). So the dot-matrix
// reflects the room, not the agent — restoring the agent's half has to come from
// outside this graph.
//
// `read()` returns a level + log-spaced frequency bands, the same mapping the
// chosen `demos/dot-matrix.html` reference uses.

export interface Reading {
  /** RMS amplitude, 0..1. */
  level: number;
  /** Log-spaced frequency-band magnitudes, 0..1, length `bandCount`. */
  bands: Float32Array;
}

const NB = 56;
const MIN_HZ = 80;
const MAX_HZ = 6500;
const FFT = 1024;
const GAIN = 1.6;

type AudioCtor = typeof AudioContext;

export class AudioBus {
  readonly ctx: AudioContext;
  private analyser: AnalyserNode;
  // Explicit <ArrayBuffer> so the AnalyserNode getters accept these (TS 5.7+
  // made the typed arrays generic over their backing buffer).
  private freqBytes: Uint8Array<ArrayBuffer>;
  private timeBytes: Uint8Array<ArrayBuffer>;
  private bands = new Float32Array(NB);

  constructor() {
    const Ctor: AudioCtor =
      window.AudioContext ?? (window as unknown as { webkitAudioContext: AudioCtor }).webkitAudioContext;
    this.ctx = new Ctor();
    this.analyser = this.ctx.createAnalyser();
    this.analyser.fftSize = FFT;
    this.analyser.smoothingTimeConstant = 0.55;
    this.freqBytes = new Uint8Array(this.analyser.frequencyBinCount);
    this.timeBytes = new Uint8Array(this.analyser.fftSize);
  }

  /**
   * Bring the context back to `running`. WebKit (so the macOS WKWebView, and
   * Safari) parks a backgrounded context in a non-standard `"interrupted"` state
   * that `AudioContextState` doesn't name and that needs the same resume as
   * `"suspended"` — miss it and the graph renders nothing while every visible
   * sign says the mic is on. Rejects if the browser still wants a gesture.
   */
  async resume(): Promise<void> {
    const state: string = this.ctx.state;
    if (state === "suspended" || state === "interrupted") await this.ctx.resume();
  }

  /** Whether the graph is actually rendering (anything else produces no audio). */
  get running(): boolean {
    return this.ctx.state === "running";
  }

  /** Connect a mic source node into the analyser (never the speakers). */
  attachMic(node: AudioNode): void {
    node.connect(this.analyser);
  }

  get bandCount(): number {
    return NB;
  }

  /** Sample the analyser once. Visual smoothing is the caller's concern. */
  read(): Reading {
    this.analyser.getByteFrequencyData(this.freqBytes);
    this.analyser.getByteTimeDomainData(this.timeBytes);

    let sum = 0;
    for (let i = 0; i < this.timeBytes.length; i++) {
      const v = (this.timeBytes[i]! - 128) / 128;
      sum += v * v;
    }
    const level = Math.min(1, Math.sqrt(sum / this.timeBytes.length) * GAIN * 2.4);

    const binHz = this.ctx.sampleRate / FFT;
    for (let b = 0; b < NB; b++) {
      const fLo = MIN_HZ * Math.pow(MAX_HZ / MIN_HZ, b / NB);
      const fHi = MIN_HZ * Math.pow(MAX_HZ / MIN_HZ, (b + 1) / NB);
      const lo = Math.max(0, Math.floor(fLo / binHz));
      const hi = Math.max(lo, Math.min(this.freqBytes.length - 1, Math.ceil(fHi / binHz)));
      let m = 0;
      for (let k = lo; k <= hi; k++) m += this.freqBytes[k]!;
      let val = m / (hi - lo + 1) / 255;
      val *= 1 + (b / NB) * 1.4; // lift the consonant-range highs
      this.bands[b] = Math.min(1, val * GAIN);
    }
    return { level, bands: this.bands };
  }

  close(): void {
    try {
      this.analyser.disconnect();
    } catch {
      /* ignore */
    }
    void this.ctx.close();
  }
}
