import { useCallback, useEffect, useRef, useState } from "react";
import {
  fetchOlderMessages,
  subscribeOutText,
  type Conversation,
  type Message,
} from "../channels/out/text";
import { subscribeAudioTurns } from "../channels/out/audio";
import { subscribeActivity, type AgentActivity } from "../channels/out/activity";
import { postInText } from "../channels/in/text";
import { AudioBus } from "../lib/audioBus";
import { ActivityMeter } from "../lib/activityMeter";
import { AudioStreamer } from "../lib/audioStreamer";
import { VideoStreamer } from "../lib/videoStreamer";
import { PresenceStiller } from "../lib/presenceStiller";
import { VoicePlayer } from "../lib/voicePlayer";
import { onNativeLifecycle } from "../lib/nativeBridge";
import {
  projectActivityState,
  type PresenceState,
} from "../ui/Presence";

// How many older messages one scrollback request asks for. A page of chat, not a
// page of log: the next request starts from whatever came back.
const SCROLLBACK_PAGE = 50;

// Mic liveness (see the watchdog below). Frames come every 100 ms while the
// graph renders, so three seconds without one is thirty missed in a row — a
// stopped audio thread, not a hiccup. Checked twice within that window.
const MIC_STALL_MS = 3000;
const MIC_CHECK_MS = 1500;

/**
 * What our own window is doing. Local to this page now — it used to be reported
 * to the backend, which derived a belief about the person from it.
 *
 * `closed` survives the removal because the native shell still reports it through
 * the lifecycle bridge, and it means something here: shut is the one state where
 * we should not hold channels open at all.
 */
export type WindowState = "active" | "background" | "closed";

// ---- Channel preferences (persisted client-side) -------------------------
// The user's chosen on/off state for each channel, remembered across visits in
// localStorage. These are *intents*: a saved "audio on" is reapplied on the
// next visit (the mic is re-acquired after the wake gesture), and survives a
// failed acquisition so it retries rather than silently sticking off.
interface ChannelPrefs {
  audioInput: boolean;
  videoInput: boolean;
  audioOutput: boolean;
}

const PREFS_KEY = "hi.channels.v1";
const DEFAULT_PREFS: ChannelPrefs = {
  audioInput: true,
  videoInput: false,
  audioOutput: true,
};

// True only when the browser can confirm a device permission is already granted,
// so a saved-on channel can be restored *silently* — no prompt, no gesture. A
// "prompt"/"denied" state, or a browser that can't answer the query (older
// Safari / Firefox), both read as "can't restore silently": the channel stays
// off and a click re-requests it.
async function permissionGranted(name: "microphone" | "camera"): Promise<boolean> {
  const perms = navigator.permissions;
  if (!perms?.query) return false;
  try {
    const status = await perms.query({ name: name as PermissionName });
    return status.state === "granted";
  } catch {
    return false;
  }
}

function loadPrefs(): ChannelPrefs {
  try {
    const raw = localStorage.getItem(PREFS_KEY);
    if (!raw) return { ...DEFAULT_PREFS };
    const p = JSON.parse(raw) as Partial<ChannelPrefs>;
    return {
      audioInput: typeof p.audioInput === "boolean" ? p.audioInput : DEFAULT_PREFS.audioInput,
      videoInput: typeof p.videoInput === "boolean" ? p.videoInput : DEFAULT_PREFS.videoInput,
      audioOutput: typeof p.audioOutput === "boolean" ? p.audioOutput : DEFAULT_PREFS.audioOutput,
    };
  } catch {
    return { ...DEFAULT_PREFS };
  }
}

export interface AgentSession {
  state: PresenceState;
  reactive: boolean;
  bus: AudioBus | null;
  /** Live cognition cadence (streamed-chunk pulses) the field reacts to. */
  activity: ActivityMeter;
  /** The conversation, oldest first. Append-only; nothing here is ever rewritten. */
  messages: Message[];
  /** The live recognition partial, shown pending at the tail. Not a message. */
  interim?: string | undefined;
  /**
   * Prepend a page of older messages. Resolves to how many were added, so a
   * caller can stop asking when it reaches the beginning.
   */
  loadOlder: () => Promise<number>;
  /** Whether the session's output graph is up (auto-started on mount). */
  woken: boolean;
  /** Whether the mic (audio input) channel is currently live. */
  audioInput: boolean;
  /** Surfaced if turning the audio channel on failed (denied / no device). */
  audioError: string | null;
  /** Whether the camera (vision input) channel is currently live. */
  videoInput: boolean;
  /** Surfaced if turning the vision channel on failed (denied / no device). */
  videoError: string | null;
  /** The live camera stream while vision is on (for a self-view), else null. */
  visionStream: MediaStream | null;
  /** Whether the agent's voice (audio output) channel is on. */
  audioOutput: boolean;
  /** Whether the text channel is on — the conversation and the line that adds to
   * it, which are one surface and go up and away together. */
  text: boolean;
  /** Flip the audio-input channel on/off independently of the others. */
  toggleAudio: () => void;
  /** Flip the vision-input channel on/off independently of the others. */
  toggleVideo: () => void;
  /** Flip the agent's voice (audio output) on/off; text output is unaffected. */
  toggleAudioOutput: () => void;
  /** Show the conversation, or put it away. */
  setTextChannel: (on: boolean) => void;
  /** Rejects if the line did not reach the server, so a caller can say so. */
  sendText: (text: string) => Promise<void>;
}

/**
 * The coordinator — deliberately a *dumb face*. After the wake gesture it owns
 * the input channels (mic → /api/in/audio/stream, continuous PCM; camera →
 * /api/in/vision, a frame every couple seconds) and subscribes to every channel
 * on both boundaries, rendering whatever arrives: /api/out/audio plays on
 * arrival, while /api/out/text supplies whole snapshots of the backend-owned
 * current exchange. Typed and recognized human text are folded into that same
 * state server-side, so every attached client renders the same appearance
 * whether or not it owns the input device.
 *
 * Crucially it does NOT decide turns. Turn-taking — when the agent speaks, which
 * drafts to suppress — lives in the mind (the reaction), which commits after the
 * inbound signal stream goes quiet.
 */
export function useAgentSession(): AgentSession {

  // Saved channel intents. Held in a ref (read synchronously by startSession /
  // the toggles) and written through on every explicit user change.
  const prefsRef = useRef<ChannelPrefs>(loadPrefs());
  const persistPrefs = useCallback(() => {
    try {
      localStorage.setItem(PREFS_KEY, JSON.stringify(prefsRef.current));
    } catch {
      /* storage unavailable (private mode / quota) — prefs just won't persist */
    }
  }, []);

  const [woken, setWoken] = useState(false);
  const [bus, setBus] = useState<AudioBus | null>(null);
  const [conversation, setConversation] = useState<Conversation>({ messages: [] });

  const [audioInput, setAudioInput] = useState(false);
  const [audioError, setAudioError] = useState<string | null>(null);
  const [videoInput, setVideoInput] = useState(false);
  const [videoError, setVideoError] = useState<string | null>(null);
  // The live camera stream while vision is on, so the UI can render a self-view
  // (the host shows it; null when the camera is off). Held in state alongside
  // the upload-only `visionRef` so a render is triggered when it appears/clears.
  const [visionStream, setVisionStream] = useState<MediaStream | null>(null);
  const [audioOutput, setAudioOutput] = useState(prefsRef.current.audioOutput);
  // The text channel starts on, and is not remembered across visits: it is the
  // default face of the application, and putting it away is something a person
  // does to *this* screen for a minute — not a setting that should still be in
  // force tomorrow, least of all when a stray press behind the popover is one of
  // the ways to do it.
  const [textOn, setTextOn] = useState(true);
  const [backendActivity, setBackendActivity] = useState<AgentActivity | null>(null);
  const [ttsPlaying, setTtsPlaying] = useState(false);
  // Is anyone actually looking at this window right now?
  //
  // This is the client half of the presence model: the backend derives `reach`
  // from which out-channels are *open* (`presence.rs`), so an out-channel that
  // stays subscribed behind another window reports a person who isn't there.
  // Text state is not consumed by that connection, but the false presence claim
  // would still make the host speak aloud into an unattended room.
  //
  // So attendance is a first-class client fact, and holding an out-channel open is
  // the claim "someone is reading this". Three states, all of them about the page's
  // own window and never a probe of anything else:
  //   • `active` — up and being looked at.
  //   • `background` — open but not read: tab hidden, app hidden (⌘H), miniaturized,
  //     or fully covered by another window. That last one no web API reports — an
  //     occluded WKWebView keeps `visibilityState === "visible"` — so it arrives as
  //     a native beat (`windowDidChangeOcclusionState:` in `macos_window.rs`).
  //   • `closed` — shut, via the native `closed` beat. The WKWebView is reused
  //     across close/open, so the React tree never unmounts and this is the only
  //     signal that it happened.
  //
  // The face treats `background` and `closed` identically — both drop the
  // out-channels, because neither is being read. They are reported separately
  // because *presence* reads them differently: closing is a decision and means away
  // at once, backgrounding is ambient and lets the ordinary decay do its work.
  // Nothing but this client can tell them apart, which is why it is asked.
  const [windowState, setWindowState] = useState<WindowState>(() =>
    document.visibilityState === "visible" ? "active" : "background",
  );
  const attended = windowState === "active";

  const busRef = useRef<AudioBus | null>(null);
  const micRef = useRef<AudioStreamer | null>(null);
  const micStreamRef = useRef<MediaStream | null>(null);
  // The capture's source node, held only so releasing the mic can unwire it. A
  // node stays in the render graph as long as something is connected to it, and
  // the analyser it feeds is pulled whether or not anything downstream listens —
  // so a source left connected is a silent node rendered forever.
  const micNodeRef = useRef<MediaStreamAudioSourceNode | null>(null);
  // Reentrancy guard for enableAudio: set synchronously before its first await,
  // so two near-simultaneous calls (e.g. StrictMode's double-invoked effect)
  // can't both open a /api/in/audio/stream socket — a second socket would
  // transcribe + dispatch every utterance a second time, duplicating it.
  const micStartingRef = useRef(false);
  // Bumped by disableAudio/unmount to cancel an in-flight enableAudio: a start
  // that finishes acquiring devices after a teardown tears its own socket down
  // instead of leaking it.
  const micGenRef = useRef(0);
  const voiceRef = useRef<VoicePlayer | null>(null);
  const visionRef = useRef<VideoStreamer | null>(null);
  const presenceRef = useRef<PresenceStiller | null>(null);
  const visionStreamRef = useRef<MediaStream | null>(null);
  // Live cognition cadence: bumped per streamed chunk, decays between them, so
  // the Presence pulses with the agent's real output rate (not a canned loop).
  const activityRef = useRef(new ActivityMeter());
  // Last window state pushed to the backend, for burst coalescing, and the setter
  // itself so the native lifecycle handler below can reuse it (it lives in another
  // effect and must not duplicate the reporting rule).
  const enterWindowRef = useRef<((next: WindowState) => void) | null>(null);

  // Backend activity is the authoritative source for Typing and Working.
  // Losing the stream returns the face to Starting until a fresh snapshot arrives.
  useEffect(
    () =>
      subscribeActivity(setBackendActivity, (live) => {
        if (!live) setBackendActivity(null);
      }),
    [],
  );

  // ---- GET /out/text current-state stream (after wake) -------------------
  // Held open only while the window is attended, which keeps reach honest. The
  // first snapshot on every connection is the backend's current appearance, so
  // foregrounding, refreshing, or opening another window converges immediately
  // without a per-window reading position.
  useEffect(() => {
    if (!woken || !attended) return;
    const ctrl = new AbortController();
    let cancelled = false;
    let previousInterim: string | undefined;

    void (async () => {
      while (!cancelled) {
        try {
          for await (const frame of subscribeOutText({ signal: ctrl.signal })) {
            if (cancelled) break;
            switch (frame.kind) {
              // A whole window: the opening frame, or a resync after this
              // subscriber fell behind. Replace rather than merge — the backend
              // is the authority on what the conversation is.
              case "reset":
                previousInterim = frame.conversation.interim;
                setConversation(frame.conversation);
                break;
              // One message, complete. The agent's own messages pulse the
              // activity meter; the person's don't, since the field reacts to
              // the agent thinking, not to typing.
              case "append": {
                const { message } = frame;
                if (message.role === "agent") {
                  activityRef.current.bump(Math.min(1, message.text.length / 40));
                }
                setConversation((prev) => ({
                  ...prev,
                  // Guard against a duplicate id: a resync can race an append,
                  // and a message appearing twice in a chat is very visible.
                  messages: prev.messages.some((m) => m.id === message.id)
                    ? prev.messages
                    : [...prev.messages, message],
                  interim: undefined,
                }));
                break;
              }
              // Speech being recognized. A fresh partial is the barge-in trigger:
              // it stops playback hundreds of ms before the sentence settles.
              case "interim":
                if (frame.text && frame.text !== previousInterim) {
                  const voice = voiceRef.current;
                  if (voice?.isPlaying()) voice.stop();
                }
                previousInterim = frame.text;
                setConversation((prev) => ({ ...prev, interim: frame.text }));
                break;
            }
          }
        } catch {
          if (cancelled || ctrl.signal.aborted) break;
          await new Promise((r) => setTimeout(r, 1500));
        }
      }
    })();

    return () => {
      cancelled = true;
      ctrl.abort();
    };
  }, [woken, attended]);

  // ---- Window state --------------------------------------------------------
  // What our own window is doing, kept locally. It used to be *reported* to the
  // backend as well, on `POST /api/in/attention`, where it fed a belief about how
  // present the person was. That belief is gone (`docs/arch/host.md#attachment`)
  // and so is the route.
  //
  // What it still does is the part that was always sound: while the window is not
  // attended we drop the out-channels, which is what keeps "is a speaker attached"
  // an honest answer rather than a subscription left open behind an editor.
  useEffect(() => {
    const enter = (next: WindowState) => {
      setWindowState(next);
    };
    enterWindowRef.current = enter;
    // Mount is an arrival: the page being up at all is someone opening it.
    const fromVisibility = (): WindowState =>
      document.visibilityState === "visible" ? "active" : "background";
    enter(fromVisibility());
    const onVisible = () => enter(fromVisibility());
    // Focus without a visibility change — clicking back from another app while the
    // window stayed on screen. Still an arrival; not a state change.
    const onFocus = () => {
      if (document.visibilityState === "visible") enter("active");
    };
    document.addEventListener("visibilitychange", onVisible);
    window.addEventListener("focus", onFocus);
    return () => {
      document.removeEventListener("visibilitychange", onVisible);
      window.removeEventListener("focus", onFocus);
    };
  }, []);

  // ---- GET /out/audio subscription loop (TTS playback) -------------------
  // Pure render: each response is one turn's continuous audio. Stream its body
  // straight into the player as it arrives — no clip queue. The mind only puts
  // speech on the wire once it has committed to it, so there's nothing to gate
  // here.
  //
  // Subscribed only while the window is attended *and* the user has voice output
  // on. Holding this open is the claim "there is a speaker in the room", and it is
  // the one thing the host's presence gate reads before synthesizing
  // (`sequencer.rs`, `open_tts`). Muting used to be a local flag on the player
  // while the subscription stayed up, so the gate saw a speaker, TTS was
  // synthesized and billed, `say` answered "spoken aloud" — and it was streamed
  // into a muted sink. A voice nobody can hear is spent, which is the single
  // failure this gate exists to prevent, so the mute has to reach the wire.
  useEffect(() => {
    if (!woken || !attended || !audioOutput) return;
    const ctrl = new AbortController();
    let cancelled = false;
    void (async () => {
      while (!cancelled) {
        try {
          for await (const turn of subscribeAudioTurns({ signal: ctrl.signal })) {
            if (cancelled) break;
            const voice = voiceRef.current;
            if (!voice) continue;
            const token = voice.beginTurn(turn.mime);
            const reader = turn.body.getReader();
            try {
              while (!cancelled) {
                const { value, done } = await reader.read();
                if (done) break;
                if (value) voice.pushChunk(token, value);
              }
            } finally {
              voice.endTurn(token);
              reader.releaseLock();
            }
          }
        } catch {
          if (cancelled || ctrl.signal.aborted) break;
          await new Promise((r) => setTimeout(r, 1500));
        }
      }
    })();
    return () => {
      cancelled = true;
      ctrl.abort();
      // Cut anything mid-flight: the subscription is going away because nobody can
      // hear it, so the tail of the current utterance must not keep playing.
      voiceRef.current?.stop();
    };
  }, [woken, attended, audioOutput]);

  // ---- audio-input channel: acquire/release the mic (and vision) ---------
  // Independent of the session itself — text and audio are coequal input
  // channels, each freely toggled on or off. Enabling needs the session's
  // AudioBus to already exist (built in startSession).
  const enableAudio = useCallback(async () => {
    const audioBus = busRef.current;
    // Web Audio itself never came up (`startSession`'s catch — no AudioContext
    // constructor at all). Say so rather than returning into the void: a tap
    // that changes nothing on screen is indistinguishable from a broken button,
    // and this guard silently swallowing every tap is exactly how a dead mic
    // looked from the outside.
    if (!audioBus) {
      setAudioError("audio is unavailable here");
      return;
    }
    // Already live, or a start is already in flight. The micStartingRef check
    // closes the async gap below: micRef is only set after two awaits, so
    // without it a concurrent second call would slip past and open a duplicate
    // socket.
    if (micRef.current || micStartingRef.current) return;
    micStartingRef.current = true;
    const gen = ++micGenRef.current;
    // True once a teardown (disableAudio/unmount) has superseded this start.
    const superseded = () => micGenRef.current !== gen;
    try {
      // The graph has to be running before it can capture anything. The context
      // may have been parked since the last time we looked (autoplay policy at
      // startup, or WebKit interrupting it while the window was in the
      // background), and a mic wired into a parked context renders nothing while
      // reporting itself on. Not awaited: this call is the one that runs inside
      // the tap, which is the gesture iOS wants, but WebKit's answer to a resume
      // it won't honour is a promise that never settles — and waiting on that
      // would strand the acquisition here, before `getUserMedia` is ever
      // reached. A resume that doesn't take is not a mic failure; the watchdog
      // below keeps trying.
      void audioBus.resume().catch(() => {});
      const stream = await navigator.mediaDevices.getUserMedia({
        // echoCancellation MUST stay on: with the mic and speaker both open, the
        // agent's own TTS loops back into the mic and gets re-transcribed. (We
        // tried disabling AEC+NS to shed the ~1-core CPU in the "Graphics and
        // Media" process — it neither helped CPU nor was worth the loopback, so
        // it's back on. The media-process CPU burn is AEC-independent; hunt it
        // elsewhere — the AudioWorklet / WebAudio render path is the prime suspect.)
        audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true },
      });
      if (superseded()) {
        stream.getTracks().forEach((t) => t.stop());
        return;
      }
      const micNode = audioBus.ctx.createMediaStreamSource(stream);
      audioBus.attachMic(micNode);

      // Passthrough: stream every mic frame to the backend; the upstream STT
      // segments and transcribes. No client-side VAD. The socket is upload-only;
      // recognized text is folded into the shared /out/text appearance state,
      // so this window renders the same transcript as every other one.
      const streamer = await AudioStreamer.create(audioBus.ctx, micNode);
      if (superseded()) {
        // Disabled while we were acquiring — don't leave the socket open, and
        // don't leave the source node wired into a graph nobody will unwire it
        // from (the teardown that superseded us has already run).
        streamer.stop();
        micNode.disconnect();
        stream.getTracks().forEach((t) => t.stop());
        return;
      }
      micStreamRef.current = stream;
      micNodeRef.current = micNode;
      micRef.current = streamer;
      setAudioError(null);
      setAudioInput(true);
    } catch (err) {
      const msg = (err instanceof Error ? err.message : String(err)).toLowerCase();
      setAudioError(
        msg.includes("denied") || msg.includes("permission") || msg.includes("notallowed")
          ? "microphone permission needed"
          : "couldn't reach the microphone",
      );
      setAudioInput(false);
    } finally {
      // Leave the flag untouched if a newer start/teardown already owns it.
      if (!superseded()) micStartingRef.current = false;
    }
  }, []);

  // Let go of everything the capture owns: the upload socket and its worklet, the
  // source node's edges into the bus, and the device itself (which is what turns
  // the OS mic indicator off). All of it is built fresh by the next `enableAudio`.
  // The one thing deliberately *not* released is the AudioContext — playback and
  // the Presence analyser hang off it too, so it outlives every mic toggle.
  const releaseMic = useCallback(() => {
    // Cancel any enableAudio still acquiring devices, and clear the in-flight
    // flag so a later enable can start.
    micGenRef.current++;
    micStartingRef.current = false;
    micRef.current?.stop();
    micRef.current = null;
    micNodeRef.current?.disconnect();
    micNodeRef.current = null;
    micStreamRef.current?.getTracks().forEach((t) => t.stop());
    micStreamRef.current = null;
  }, []);

  const disableAudio = useCallback(() => {
    releaseMic();
    setAudioInput(false);
  }, [releaseMic]);

  const toggleAudio = useCallback(() => {
    const next = !audioInput;
    prefsRef.current.audioInput = next;
    persistPrefs();
    if (next) void enableAudio();
    else disableAudio();
  }, [audioInput, disableAudio, enableAudio, persistPrefs]);

  // Drop the capture and take it again, without touching the channel's on/off
  // state: the mic is still meant to be on, its capture just died under us.
  const recycleAudio = useCallback(async () => {
    releaseMic();
    await enableAudio();
  }, [enableAudio, releaseMic]);

  // ---- mic liveness watchdog ----------------------------------------------
  // The mic can go deaf while every visible sign says it is on. The context gets
  // parked (WebKit does this to a backgrounded window and calls it "interrupted")
  // or the captured track ends under us on a device change; either way the
  // worklet renders nothing, the upload socket stays open with no frames on it,
  // and the upstream STT ends its session after 8 s without a packet. Nothing on
  // either side of that wire notices — the socket is up, the toggle says on, and
  // the only cure was restarting the app.
  //
  // So watch the one fact that means deafness whatever the cause: no PCM frame
  // handed up by the audio thread, which runs at a 100 ms cadence when the graph
  // is live. A parked context is the common cause and the cheap fix, so try that
  // first and let the next tick judge it; if the context is running and frames
  // still aren't coming, the capture itself is dead and only a fresh device will
  // do. (getUserMedia doesn't re-prompt — the permission is already granted.)
  useEffect(() => {
    if (!audioInput) return;
    let busy = false;
    const check = async () => {
      const streamer = micRef.current;
      const bus = busRef.current;
      if (busy || !streamer || !bus) return;
      if (streamer.msSinceLastFrame() < MIC_STALL_MS) return;
      busy = true;
      try {
        if (!bus.running) {
          // Fire and let the next tick judge it — awaiting a resume WebKit has
          // decided not to honour never returns, and `busy` would stay set,
          // which kills the watchdog for the rest of the session.
          void bus.resume().catch(() => {});
          return;
        }
        console.debug("[mic] audio thread went quiet — re-acquiring the device");
        await recycleAudio();
      } finally {
        busy = false;
      }
    };
    const timer = setInterval(() => void check(), MIC_CHECK_MS);
    return () => clearInterval(timer);
  }, [audioInput, recycleAudio]);

  // ---- vision-input channel: acquire/release the camera ------------------
  // A continuous channel like the mic, but fully independent — usable with or
  // without audio, and toggled on its own. The camera streams continuously as
  // WebM (MediaRecorder → WS); the backend decides how much to look. No
  // client-side sampling.
  const enableVision = useCallback(async () => {
    if (visionRef.current) return; // already live
    try {
      // `ideal` at 4K asks for the camera's best; the browser clamps down to the
      // device's true native max rather than failing when 4K isn't available. But
      // `ideal` only steers size, not orientation: the returned mode follows the
      // camera's native sensor, so a portrait-native camera (a phone front cam,
      // iPhone Continuity Camera) hands back a vertical frame. Bias the request
      // toward the viewport's own orientation — landscape on a desktop screen,
      // portrait on an upright phone — so the feed reads the way the device does.
      const portrait =
        typeof window !== "undefined" &&
        window.matchMedia?.("(orientation: portrait)").matches;
      const long = { ideal: 3840 };
      const short = { ideal: 2160 };
      // The width/height `ideal` only steers size; an `aspectRatio` hint is what
      // tips the browser off a portrait-native high-res mode toward a landscape
      // one (when the camera exposes both). Still a hint, not a guarantee.
      const aspectRatio = { ideal: portrait ? 9 / 16 : 16 / 9 };
      const videoStream = await navigator.mediaDevices.getUserMedia({
        video: portrait
          ? { width: short, height: long, aspectRatio }
          : { width: long, height: short, aspectRatio },
      });
      const got = videoStream.getVideoTracks()[0]?.getSettings();
      console.debug("[vision] captured", got?.width, "x", got?.height, got);
      visionStreamRef.current = videoStream;
      visionRef.current = await VideoStreamer.create(videoStream, {});
      // Start the presence lane on the same stream — a cheap low-res still feed for
      // real-time local face recognition, beside the full-fidelity video upload.
      presenceRef.current = new PresenceStiller(videoStream, {});
      setVisionStream(videoStream);
      setVideoError(null);
      setVideoInput(true);
    } catch (err) {
      // Stop a half-acquired stream so a denied/failed start leaves no camera on.
      visionStreamRef.current?.getTracks().forEach((t) => t.stop());
      visionStreamRef.current = null;
      setVisionStream(null);
      const msg = (err instanceof Error ? err.message : String(err)).toLowerCase();
      setVideoError(
        msg.includes("denied") || msg.includes("permission") || msg.includes("notallowed")
          ? "camera permission needed"
          : "couldn't reach the camera",
      );
      setVideoInput(false);
    }
  }, []);

  const disableVision = useCallback(() => {
    visionRef.current?.stop();
    visionRef.current = null;
    presenceRef.current?.stop();
    presenceRef.current = null;
    visionStreamRef.current?.getTracks().forEach((t) => t.stop());
    visionStreamRef.current = null;
    setVisionStream(null);
    setVideoInput(false);
  }, []);

  const toggleVideo = useCallback(() => {
    const next = !videoInput;
    prefsRef.current.videoInput = next;
    persistPrefs();
    if (next) void enableVision();
    else disableVision();
  }, [videoInput, disableVision, enableVision, persistPrefs]);

  // ---- voice output channel: mute/unmute the agent's TTS -----------------
  // Independent of everything else — silencing the voice leaves the agent's
  // words flowing as text on /out/text.
  //
  // `audioOutput` gates the /out/audio subscription (above), so turning it off
  // drops the channel and the host stops synthesizing rather than synthesizing
  // into silence. `setMuted` still fires for the instant cut: the effect teardown
  // is a render away, and a spoken tail must stop on the click, not after it.
  const toggleAudioOutput = useCallback(() => {
    setAudioOutput((on) => {
      const next = !on;
      prefsRef.current.audioOutput = next;
      persistPrefs();
      voiceRef.current?.setMuted(!next);
      return next;
    });
  }, [persistPrefs]);

  // ---- text channel: show the conversation, or put it away ---------------
  const setTextChannel = useCallback((on: boolean) => setTextOn(on), []);

  // Restore the input channels the user last had on — but *honestly*: a saved-on
  // mic/camera is re-acquired only when its permission is already granted, so the
  // restore is silent (no surprise prompt). A channel that can't be restored
  // silently stays off; its control shows off and a click re-requests the device.
  // Shared by the initial startup restore and the foreground restore below, and
  // driven off `prefsRef` so a mid-session toggle is reflected next time.
  const restoreInputChannels = useCallback(async () => {
    const prefs = prefsRef.current;
    if (prefs.audioInput && (await permissionGranted("microphone"))) void enableAudio();
    if (prefs.videoInput && (await permissionGranted("camera"))) void enableVision();
  }, [enableAudio, enableVision]);

  // ---- start the session: build the output graph, restore channels -------
  // Runs once on mount — no wake gate, no dedicated gesture. Building the
  // AudioBus is always allowed; the context may start suspended (autoplay
  // policy), so we resume on the first incidental interaction, which unlocks TTS
  // without a tap. Input channels are then restored *honestly*: a saved-on
  // mic/camera is re-acquired only when its permission is already granted (a
  // silent restore). If it can't be restored silently the channel stays off —
  // the control shows it off, and a click re-requests the device (that click is
  // the gesture/permission prompt the browser wants).
  const startSession = useCallback(() => {
    if (woken) return;
    void (async () => {
      try {
        const audioBus = new AudioBus();
        // Kicked, never waited on. iOS WebKit refuses to start an AudioContext
        // outside a user gesture — `AudioContext::constructCommon` takes the
        // restriction from the page's "requires a user gesture for audio
        // playback", which an iOS WKWebView sets by default and a macOS one
        // does not, and which is why only the phone showed this. It refuses by
        // rejecting or by never settling the promise at all.
        // Awaiting it here made that refusal fatal to everything below: `busRef`
        // stayed null, and every later mic tap hit `enableAudio`'s `!audioBus`
        // guard and returned — a button that did nothing, said nothing, and
        // never prompted, on a face whose camera worked (vision touches no
        // AudioContext). A parked context is a normal state, not a failure; the
        // gesture listener below and `enableAudio`'s own resume un-park it.
        void audioBus.resume().catch(() => {});
        // Unconditional: a context is `suspended` the instant it is built and
        // the resume above has not settled yet, so there is nothing to test.
        // The listener is a self-removing one-shot and resuming a running
        // context is a no-op, so arming it when it wasn't needed costs nothing.
        const events = ["pointerdown", "keydown", "touchstart"];
        // Capture, so this is a sensor for *any* interaction rather than a key
        // handler: keys typed into host chrome stop at the document and never
        // reach the window (`lib/keyboard.ts`), and the first thing a person
        // does is often type a message.
        const resumeOnGesture = () => {
          void audioBus.resume().catch(() => {});
          for (const ev of events) window.removeEventListener(ev, resumeOnGesture, true);
        };
        for (const ev of events) window.addEventListener(ev, resumeOnGesture, true);
        const voice = new VoicePlayer(
          audioBus,
          () => setTtsPlaying(true),
          () => setTtsPlaying(false),
        );
        busRef.current = audioBus;
        voiceRef.current = voice;
        const prefs = prefsRef.current;
        voice.setMuted(!prefs.audioOutput);
        setBus(audioBus);
        setWoken(true);
        // Restore the mic/camera the user last had on, silently (see helper above).
        void restoreInputChannels();
      } catch (err) {
        // The output graph couldn't be built (no Web Audio, etc.). The text
        // channel still works, so mark the session up and leave audio off.
        console.debug("[session] audio graph unavailable", err);
        setWoken(true);
      }
    })();
  }, [woken, restoreInputChannels]);

  // Auto-start on mount — the session builds itself and restores channels per
  // the honest policy above. The ref guard keeps StrictMode's double-invoke (and
  // any re-render of startSession) from starting a second graph.
  const startedRef = useRef(false);
  useEffect(() => {
    if (startedRef.current) return;
    startedRef.current = true;
    startSession();
  }, [startSession]);

  // Native desktop lifecycle: pause on background (window closed, or fully covered
  // by another app's window), restore on foreground. The macOS WKWebView is reused
  // across close/open, so the React tree never unmounts and the unmount cleanup
  // below never runs — without this a closed window would keep the mic/camera live
  // and keep the agent talking to an empty room. Handling per channel:
  //   • input (mic/camera): released on background so the OS indicators actually go
  //     off; re-acquired on foreground per the honest, permission-gated startup
  //     restore. Preferences untouched — a hand-muted mic stays muted across cycles.
  //   • output (voice *and* text): `attended` goes false, which drops both
  //     out-channel subscriptions. That is the whole point rather than an
  //     optimization: an open out-channel is how the backend concludes someone is
  //     there, so leaving them up behind a closed window is what let the agent
  //     speak into it and count the words delivered.
  // Text state itself stays backend-owned while this window is away. Foreground
  // opens one fresh subscription and receives the current state immediately;
  // nothing advances or queues on behalf of this window while it is hidden.
  useEffect(() => {
    return onNativeLifecycle((phase) => {
      if (phase === "background" || phase === "closed") {
        // Same handling either way — neither is being read — but reported apart,
        // because closing is a decision and presence reads it as away at once.
        enterWindowRef.current?.(phase);
        disableAudio();
        disableVision();
        voiceRef.current?.setMuted(true);
      } else {
        enterWindowRef.current?.("active");
        void restoreInputChannels();
        voiceRef.current?.setMuted(!prefsRef.current.audioOutput);
      }
    });
  }, [disableAudio, disableVision, restoreInputChannels]);

  // cleanup on unmount
  useEffect(() => {
    return () => {
      // Cancel an in-flight enableAudio so a start that resolves post-unmount
      // tears its own socket down instead of leaking it.
      micGenRef.current++;
      micStartingRef.current = false;
      micRef.current?.stop();
      visionRef.current?.stop();
      presenceRef.current?.stop();
      voiceRef.current?.stop();
      micStreamRef.current?.getTracks().forEach((t) => t.stop());
      visionStreamRef.current?.getTracks().forEach((t) => t.stop());
      busRef.current?.close();
    };
  }, []);

  // ---- keyboard fallback send --------------------------------------------
  // **A failed send is reported, not swallowed.** This used to end in
  // `.catch(() => {})` while the composer cleared the box the moment Enter was
  // pressed — so a rejected POST took the words with it and looked exactly like a
  // successful one. Nothing here shows the failure itself; it hands the rejection
  // back so the caller that still holds the draft can.
  const sendText = useCallback(
    async (text: string): Promise<void> => {
      const trimmed = text.trim();
      if (!trimmed) return;
      // The server appends the accepted line to the conversation and it arrives
      // back on the stream, so this window keeps no private optimistic copy.
      await postInText({ body: trimmed });
    },
    [],
  );

  // ---- scrollback ----------------------------------------------------------
  // Ask for the page before the oldest message we hold. In flight at most once:
  // the scroller can fire this on every scroll event near the top, and two
  // overlapping requests would prepend the same page twice.
  const loadingOlderRef = useRef(false);
  const loadOlder = useCallback(async (): Promise<number> => {
    if (loadingOlderRef.current) return 0;
    const oldest = conversation.messages[0];
    if (!oldest) return 0;
    loadingOlderRef.current = true;
    try {
      const older = await fetchOlderMessages(oldest.id, { limit: SCROLLBACK_PAGE });
      if (older.length === 0) return 0;
      setConversation((prev) => {
        const known = new Set(prev.messages.map((m) => m.id));
        const fresh = older.filter((m) => !known.has(m.id));
        if (fresh.length === 0) return prev;
        return { ...prev, messages: [...fresh, ...prev.messages] };
      });
      return older.length;
    } catch {
      // Reaching the start of the conversation and failing to reach the server
      // look the same from here, and neither is worth surfacing: the scroller
      // simply stops growing, and a later scroll tries again.
      return 0;
    } finally {
      loadingOlderRef.current = false;
    }
  }, [conversation.messages]);

  const state: PresenceState = projectActivityState({
    ready: woken && backendActivity?.reaction_ready === true,
    listening: conversation.interim !== undefined,
    speaking: ttsPlaying,
    reactionBusy: backendActivity?.reaction_busy === true,
    delegatedBusy: (backendActivity?.delegated_busy_count ?? 0) > 0,
  });

  // Dots track the agent's voice while it plays.
  const reactive = state === "speaking" && ttsPlaying;

  return {
    state,
    reactive,
    bus,
    activity: activityRef.current,
    messages: conversation.messages,
    interim: conversation.interim,
    loadOlder,
    woken,
    audioInput,
    audioError,
    videoInput,
    videoError,
    visionStream,
    audioOutput,
    text: textOn,
    toggleAudio,
    toggleVideo,
    toggleAudioOutput,
    setTextChannel,
    sendText,
  };
}
