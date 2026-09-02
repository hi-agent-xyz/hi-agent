# Mechanism calls — reaching the app's hands

## Goal

Let the core use eyes and hands it does not have — on whichever machine the person is
actually at — without the core learning a platform API, and without inventing a second
protocol beside the one an app already speaks.

[`foundation.md`](foundation.md) already splits every OS capability: the raw touch is
**mechanism** and belongs to whoever holds the OS session, the cross-platform judgment is
**policy** and belongs to the core. [`topology.md`](topology.md#app) already assigns the
mechanisms to the app. What has never existed is the call that crosses between them.

## Decisions

| Decision | Reasoning |
|---|---|
| **The app answers calls as well as making them** | This is the whole object. Every other seam has the app asking and the core answering; a capability the core needs is the one case where the core must ask. Nothing else about the wire changes |
| **The app always dials, even though the core now originates requests** | A core that had to dial an app could not reach one behind NAT, and a relayed attachment routes *into* a core, never out of one. Dialing is what keeps loopback, direct and relayed three carriers of one protocol instead of three protocols |
| **Perception stays pulled** | [`surfaces.md`](surfaces.md#why-a-ref-and-not-the-bytes): a photo arriving does not mean the agent looked. Screen pixels are the same — the core asks for a frame when a turn warrants one. A continuous encode is a *cast*, which is different work with a different consumer |
| **Audio is the only thing that streams, and it already does** | `WS /api/in/audio/stream` exists and the browser mic uses it today. A shell streaming PCM is that endpoint with a different client, not a new design |
| **Pixels ride binary frames** | A PNG is bytes. Base64 inside a JSON frame costs a third again and a copy, on the one payload big enough for either to matter |
| **What an app can do is what it declared when it dialed** | `available()` stops being a compile-time `cfg` and becomes a fact about who is attached — and one that changes while running, when a laptop sleeps |

## Why this is smaller than it looked

`CLAUDE.md` calls this the biggest design object in the refactor and describes it as a
bidirectional streaming protocol, "part of it streaming — frames, audio are continuous".
Two thirds of that does not survive contact with what is already built.

**Audio needs nothing.** `WS /api/in/audio/stream` publishes into `ingest_pcm_stream`, and
the in-process macOS capture path already feeds the same sink — its own comment says "the
same as the browser mic". Moving mic capture to the shell changes which client opens the
socket and nothing else. `WS /api/in/vision/stream` is the same story for the camera.

**Frames are not continuous.** Perception is pulled, by the rule in `surfaces.md` and by
the capability's own stance — a still frame is the irreducible primitive, and a smooth live
stream lands with cast-to-view, a separate piece of work with a `<video>` on the other end.

**What is left is initiative, and only initiative.** Today the core reaches a mechanism by
calling a `cfg`-gated Rust function in its own address space. Across a process boundary
that call needs a wire, and the system has no wire on which the core is the one asking.
That is the design object: not a streaming protocol, one inversion on an existing wire.

## What crosses

Read off the capabilities that must move, not from a guess at what a shell might want.

**Core → app — calls, each with a reply:**

| Call | Reply | Replaces |
|---|---|---|
| `screen.windows(app?)` | window refs | `screencast::list_windows` |
| `screen.grab(window?)` | PNG, binary frame | `screencast::grab_window_png` / `grab_screen_png` |
| `screen.size()` | points | `input::main_display_point_size` |
| `input.perform(action)` | ok / error | `input::perform` |
| `ax.inspect()` | elements | `accessibility::inspect` |
| `desktop.context()` | focused app + window | `desktop_context::capture` |

**Core → app — state, no reply:** the tray's `flash`, `set_listening`, `set_text`,
`open_chat`. These are pushes, not questions; nothing waits on them and a dropped one is
survivable.

**App → core — events, no reply:** the attention gesture's key edges. The *tap* is
mechanism and moves to the shell; the machine that tells a double-tap from a press-and-hold
is policy and stays in the core, so the edges have to arrive as events. This is why the
connection carries both directions rather than the app simply POSTing: an edge is not one
of the five inbound channels and has no business inventing a sixth.

**App → core — perception, over the endpoints that already exist and are unchanged:** mic
PCM on `WS /api/in/audio/stream`, camera on `WS /api/in/vision/stream`, a handed screenshot
on `POST /api/in/file`.

## The connection

One long-lived WebSocket, dialed by the app, held open: `WS /api/mechanisms`. Loopback-gated
when local, and carrying the app's credential like any other request when it is not — the
same gate the config API uses, not a mechanism of its own.

Text frames carry calls, replies and events as JSON. Binary frames carry payloads, correlated
to a call by its id. The app may have several calls outstanding; replies are matched by id
and never by order, because a screen grab and an AX read have nothing to do with each other's
timing.

**The connection is the liveness signal**, exactly as it is for the tunnel: an app with no
live connection has no hands, and that is a state to degrade into rather than an error to
raise. There is no heartbeat.

This is the tunnel's trick mirrored. The tunnel is dialed by the core and carries requests
*inward*; this is dialed by the app and carries requests *outward*. Both exist for the same
reason — the side that can always dial is not the side that needs to ask.

## Capability is a fact about who is attached

`screencast::available()` is `cfg(target_os = "macos")` today: a constant, decided when the
binary was built. After the split it means *an app is attached and it declared `screen`* —
runtime, and mutable while running.

The consequence worth stating: **an unavailable capability is normal, not broken.** A core
in Docker has no hands and never will; a laptop closes its lid mid-turn. `surfaces.md`
already rules this: every channel degrades rather than fails, and what the person must act
on goes verbatim into the channel they are actually on. A mechanism the core cannot reach is
answered the same way — the agent says it cannot see the screen right now, which is true and
useful, rather than a capability erroring into a turn.

## What this does not change

The biometric and ML layer stays in the core and is untouched — `buffalo_l`, `CAM++`,
clustering, the reflex recognizer, every model call. Camera and mic bytes may still arrive
from the web face rather than a native shell, which is exactly why that layer is
cross-platform and stays put.

`hi_say` and `hi_show` are unaffected. Speech and showing are outbound tool calls the core
resolves; nothing about them needs the app's hands.

## Open

1. **Which app has the hands when several are attached.** A desktop and a phone may both
   declare `screen`, and *"look at my screen"* then names no machine. The conversation's
   current locus is the obvious tiebreak and `desktop.context()` is evidence for it, but
   picking wrong means quietly showing the agent the wrong machine — a consequential fork,
   and unresolved here on purpose.

2. **Whether tray state belongs on this connection at all.** `set_listening` and `set_text`
   are push-shaped: the app wants the current value, including the value from before it
   attached. That is a subscription, and modelling it as a fire-and-forget call gives an app
   that reconnects a stale tray until the next change.

3. **Whether the reflex path survives the process boundary.** `host.md` holds that the
   reflex path never reaches a model because stopping when someone starts talking cannot
   wait a generation. Today the key tap is in-process. Moving it to the shell puts an IPC
   hop inside a sub-second budget that already has a person's patience at the end of it.
   **Measure this before committing to it** — if barge-in misses its budget, the tap is the
   one mechanism that has to stay wherever the reflex lives.

## See also

[`foundation.md`](foundation.md) for the mechanism/policy split ·
[`topology.md`](topology.md#app) for what an app owns and the two wires this adds a
direction to · [`surfaces.md`](surfaces.md) for pulled perception and degradation ·
[`../platforms/apple-macos.md`](../platforms/apple-macos.md) for the migration this
unblocks.
