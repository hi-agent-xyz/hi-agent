/**
 * Where the agent's voice leaves the page: as the remote track of a WebRTC
 * connection the page makes to itself, so the page's own echo canceller has
 * it as a reference.
 *
 * `getUserMedia`'s `echoCancellation` subtracts from the mic only the audio the
 * canceller is given as "what the far end said", and Chromium's own canceller is
 * given exactly one thing: audio received on an `RTCPeerConnection`. An `<audio>`
 * element, a MediaSource and a Web Audio graph are all invisible to it
 * (https://crbug.com/687574, which Chrome does not consider a bug). This page has
 * played speech both of the last two ways, and the agent heard itself say its own
 * sentences back as messages from the person. Other engines, and some platforms'
 * own cancellers, cover more of a page's output — but nothing a page can detect
 * says which one it is running under, and audio received over WebRTC is the one
 * path they all treat as the far end. That is why voice products put it there.
 *
 * So sources connect to {@link input}, which feeds a MediaStream that goes out
 * one local peer connection and in the other, and the received track is what an
 * `<audio>` element plays. Both peers live in this page, so the "network" is a
 * host candidate talking to itself.
 *
 * # Until the loopback is up, speech goes straight to the speakers
 *
 * **Speech must never go silent because this has not connected.** The input is
 * also wired to the context's destination through a gain that is open until the
 * loopback connects and closes the moment it does — audible either way, and
 * cancelled only once it can be.
 *
 * **A loopback that failed is tried again when the mic starts ({@link retry}).**
 * Measured in headless Chrome on 2026-09-14: a page with no microphone permission
 * is given only obfuscated `.local` host candidates, and the two peers sat in
 * `checking` until the timeout; the same page with the permission granted got real
 * addresses and connected at once. Echo only exists while a mic is capturing, and
 * capturing is exactly what grants the permission — so that is the moment a second
 * attempt is both needed and able to work.
 */

/**
 * How long one attempt gets before it is abandoned. A same-page connection with
 * usable candidates completes well inside a second; this only has to outlast a
 * slow ICE gather on a phone.
 */
const CONNECT_TIMEOUT_MS = 5000;

export class VoiceRoute {
  /** Connect whatever should be heard as the agent's voice here. */
  readonly input: GainNode;
  /** The straight path to the speakers: open until the loopback carries speech. */
  private readonly direct: GainNode;
  private readonly sink: MediaStreamAudioDestinationNode | null = null;
  private readonly out: HTMLAudioElement;
  private peers: RTCPeerConnection[] = [];
  /** Bumped per attempt, so a superseded attempt's late events change nothing. */
  private attempt = 0;
  private connecting = false;
  private connected = false;

  constructor(ctx: AudioContext) {
    this.input = ctx.createGain();
    this.direct = ctx.createGain();
    this.input.connect(this.direct);
    this.direct.connect(ctx.destination);
    this.out = new Audio();
    this.out.autoplay = true;
    if (typeof RTCPeerConnection !== "undefined" && typeof ctx.createMediaStreamDestination === "function") {
      this.sink = ctx.createMediaStreamDestination();
      this.input.connect(this.sink);
      this.connect();
    }
  }

  /**
   * Start the output element. Called whenever a turn starts playing, because
   * that is the moment the page is allowed to — the same autoplay rules the
   * turn's own element is under.
   */
  play(): void {
    if (!this.connected) return;
    void this.out.play().catch(() => {
      /* autoplay race; the next turn retries */
    });
  }

  /** Try the loopback again if it is not up and no attempt is running. */
  retry(): void {
    if (this.sink && !this.connected && !this.connecting) this.connect();
  }

  private connect(): void {
    const stream = this.sink?.stream;
    if (!stream) return;
    for (const pc of this.peers) pc.close();
    const attempt = ++this.attempt;
    const current = () => attempt === this.attempt;
    const from = new RTCPeerConnection();
    const to = new RTCPeerConnection();
    this.peers = [from, to];
    this.connecting = true;
    let received: MediaStream | null = null;

    from.onicecandidate = (e) => {
      if (e.candidate) void to.addIceCandidate(e.candidate).catch(() => {});
    };
    to.onicecandidate = (e) => {
      if (e.candidate) void from.addIceCandidate(e.candidate).catch(() => {});
    };

    const giveUp = () => {
      if (!current()) return;
      clearTimeout(timer);
      for (const pc of this.peers) pc.close();
      this.peers = [];
      this.connecting = false;
      this.connected = false;
      this.out.srcObject = null;
      this.direct.gain.value = 1;
    };
    const timer = setTimeout(giveUp, CONNECT_TIMEOUT_MS);
    // The switch waits for both the track and the connection, in whichever order
    // they arrive. ICE state is read alongside the connection state because an
    // engine that reports only one of them must not be mistaken for a failure.
    // A failure after the switch hands speech back to the speakers the same way.
    const settle = () => {
      if (!current()) return;
      const state = to.connectionState;
      const ice = to.iceConnectionState;
      if (state === "failed" || ice === "failed" || state === "closed") {
        giveUp();
      } else if (!this.connected && received && (state === "connected" || ice === "connected" || ice === "completed")) {
        clearTimeout(timer);
        this.connecting = false;
        this.connected = true;
        this.out.srcObject = received;
        this.direct.gain.value = 0;
        this.play();
      }
    };
    to.ontrack = (e) => {
      received = e.streams[0] ?? new MediaStream([e.track]);
      settle();
    };
    to.onconnectionstatechange = settle;
    to.oniceconnectionstatechange = settle;

    for (const track of stream.getAudioTracks()) from.addTrack(track, stream);
    void (async () => {
      const offer = await from.createOffer();
      await from.setLocalDescription(offer);
      await to.setRemoteDescription(offer);
      const answer = await to.createAnswer();
      await to.setLocalDescription(answer);
      await from.setRemoteDescription(answer);
    })().catch(giveUp);
  }
}
