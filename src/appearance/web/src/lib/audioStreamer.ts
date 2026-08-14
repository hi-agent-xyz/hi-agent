// AudioStreamer — continuous mic passthrough over a WebSocket.
//
// The mic stays open and every frame is shipped to the backend as raw 16 kHz
// mono 16-bit PCM. There is NO client-side voice-activity detection: the browser
// streams audio blindly and the upstream STT does all the endpointing. This is
// UPLOAD-ONLY — nothing comes back on the socket. Recognized speech is published
// by the server to the conversation's observe stream (`GET /api/in/audio`), so the
// uploading client reads its own words there like every other client; display
// and barge-in are driven from that stream, not from this socket.
//
// The socket self-heals: an unexpected close (server restart, network blip, or
// the upstream STT session ending) reopens it while the mic is still meant to be
// on, so the session never goes silently deaf. The audio graph is independent of
// the socket, so a reconnect swaps only the WebSocket — the worklet keeps
// producing frames, which backlog briefly and resume on the fresh socket. Only
// `stop()` (mic toggled off / unmount) closes it for good.
//
// What an open socket does NOT tell you is that audio is still being captured:
// if the graph stops rendering (a suspended context, a capture device that went
// away) the frames simply stop while the socket stays up, and the upstream STT
// quietly ends its session 8 s later. `msSinceLastFrame` exposes that fact so
// the session can act on it — see the liveness watchdog in `useAgentSession`.
//
// This replaces the old MicCapture, whose homegrown RMS VAD segmented utterances
// client-side. Moving segmentation to the upstream's ML VAD is both simpler and
// more reliable; the only thing we do here is resample + frame the audio.
//
// The resampling/framing runs on the audio thread in `pcmWorklet.js` (an
// AudioWorklet processor); this class just wires up the node and ships the
// finished frames it posts back over the WebSocket.

// Vite resolves this to the (hashed, statically served) URL of the worklet
// module, which `addModule` fetches and evaluates in the audio thread.
import workletUrl from "./pcmWorklet.js?url";

import { url } from "./base";

// Reconnect backoff: first retry is quick, then doubles to a ceiling so a server
// that's down (e.g. a dev rebuild) isn't hammered. Reset once a socket opens.
const RECONNECT_BASE_MS = 500;
const RECONNECT_MAX_MS = 5000;
// Cap the pre-open backlog so a prolonged outage can't grow it without bound —
// losing audio across an outage is fine; the point is to recover, not buffer.
const MAX_BACKLOG_FRAMES = 64;
// The worklet posts a frame every 100 ms while the graph renders, so a gap this
// long is not a hiccup: the audio thread has stopped producing. Reported by
// `msSinceLastFrame` for the session's liveness watchdog, and used here to hold
// the reconnect back — see `scheduleReconnect`.
const STALE_FRAME_MS = 3000;

/**
 * How long to wait before reopening the upload socket. Consecutive failures
 * double the wait up to the ceiling — but a quiet audio thread pins it to the
 * ceiling straight away: a socket we have nothing to feed is worth nothing, and
 * reopening one only hands the upstream STT another session to time out. The
 * eager first retry is for the case it exists for, a server restart or a blip
 * while audio is still flowing.
 */
export function reconnectDelayMs(retry: number, msSinceLastFrame: number): number {
  if (msSinceLastFrame > STALE_FRAME_MS) return RECONNECT_MAX_MS;
  return Math.min(RECONNECT_BASE_MS * 2 ** retry, RECONNECT_MAX_MS);
}

// Tracks which contexts already have the worklet module so we never call
// addModule twice for the same one (a redundant network round-trip).
const loaded = new WeakSet<BaseAudioContext>();

async function ensureWorklet(ctx: BaseAudioContext): Promise<void> {
  if (loaded.has(ctx)) return;
  // `addModule` is neither fetch nor EventSource, so the subpath goes on here.
  await ctx.audioWorklet.addModule(url(workletUrl));
  loaded.add(ctx);
}

export class AudioStreamer {
  private node: AudioWorkletNode;
  private sink: GainNode;
  private ws!: WebSocket;
  private readonly url: string;

  // Frames captured before the (re)opening socket finished connecting.
  private backlog: ArrayBuffer[] = [];
  private stopped = false;
  // Consecutive failed (re)connects; drives the backoff, reset on open.
  private retry = 0;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  // When the audio thread last handed up a frame. The socket being open says
  // nothing about whether audio is still being captured, and this is the one
  // fact that does.
  private lastFrameAt = performance.now();

  /**
   * Start streaming `source`'s audio to the backend. Async because the worklet
   * module is loaded lazily (`addModule` returns a promise).
   */
  static async create(
    ctx: AudioContext,
    source: AudioNode,
  ): Promise<AudioStreamer> {
    await ensureWorklet(ctx);
    return new AudioStreamer(ctx, source);
  }

  private constructor(ctx: AudioContext, source: AudioNode) {
    const proto = location.protocol === "https:" ? "wss" : "ws";
    // Built by hand from `location`, so the community's subpath goes on by hand.
    this.url = `${proto}://${location.host}${url("/api/in/audio/stream")}`;
    this.open();

    this.node = new AudioWorkletNode(ctx, "pcm-stream", {
      numberOfInputs: 1,
      numberOfOutputs: 1,
      channelCount: 1,
    });
    // The worklet posts back ready-to-send SEND_SAMPLES PCM frames (transferred).
    this.node.port.onmessage = (ev) => {
      this.lastFrameAt = performance.now();
      if (!this.stopped) this.send(ev.data as ArrayBuffer);
    };
    source.connect(this.node);
    // The node only renders (and thus pulls input) while it's wired to the
    // destination; route its silent output through a zeroed gain so nothing is
    // audible.
    this.sink = ctx.createGain();
    this.sink.gain.value = 0;
    this.node.connect(this.sink);
    this.sink.connect(ctx.destination);
  }

  // (Re)open the upload socket. Frames produced before it's OPEN backlog and
  // flush on connect; an unexpected close schedules a reconnect.
  private open(): void {
    const ws = new WebSocket(this.url);
    ws.binaryType = "arraybuffer";
    ws.onopen = () => {
      this.retry = 0;
      for (const buf of this.backlog) ws.send(buf);
      this.backlog = [];
    };
    // Upload-only: the server never sends on this socket (recognized speech rides
    // the observe stream instead), so there is no onmessage handler. A close we
    // didn't ask for means we lost the STT session — reopen so a server restart
    // or network blip self-heals instead of leaving the mic silently deaf.
    ws.onclose = () => {
      if (!this.stopped) this.scheduleReconnect();
    };
    this.ws = ws;
  }

  /** Milliseconds since the audio thread last handed up a PCM frame. */
  msSinceLastFrame(): number {
    return performance.now() - this.lastFrameAt;
  }

  private scheduleReconnect(): void {
    if (this.stopped || this.reconnectTimer !== null) return;
    const delay = reconnectDelayMs(this.retry, this.msSinceLastFrame());
    this.retry++;
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      if (!this.stopped) this.open();
    }, delay);
  }

  private send(buf: ArrayBuffer): void {
    if (this.ws.readyState === WebSocket.OPEN) this.ws.send(buf);
    else if (this.ws.readyState === WebSocket.CONNECTING) {
      this.backlog.push(buf);
      if (this.backlog.length > MAX_BACKLOG_FRAMES) this.backlog.shift();
    }
    // closing/closed → drop (covers the gap between a drop and the reconnect)
  }

  stop(): void {
    this.stopped = true;
    if (this.reconnectTimer !== null) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    this.node.port.onmessage = null;
    try {
      this.node.disconnect();
    } catch {
      /* ignore */
    }
    try {
      this.sink.disconnect();
    } catch {
      /* ignore */
    }
    try {
      this.ws.close();
    } catch {
      /* ignore */
    }
  }
}
