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
| **The hop costs nothing worth designing around** | Measured, below. The mechanism is thousands of times more expensive than the call that asks for it, so the boundary is never the thing to optimize |

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

## The hop is not the cost

The obvious worry about putting a process boundary under a capability is latency, and the
sharpest version of it is the bottom of the tempo ladder: the reflex rung runs *because* a
generation is too slow, so anything added to its path is spent from a budget that has a
person's patience at the end of it.

**Measured on an M4, loopback, warm connection, payload round-tripped:**

| Frame | Median | p99 |
|---|---|---|
| 64 B — a call like `input.perform` | 0.012 ms | 0.053 ms |
| 4 KB — an accessibility tree | 0.013 ms | 0.018 ms |
| 2 MB — a screen grab | 0.288 ms | 0.580 ms |

A reflex is *recognize the field, click it, type the value* — one `ax.inspect` and a handful
of `input.perform`s, so roughly a dozen round trips, or **under a fifth of a millisecond of
transport**. The mechanisms themselves are three to four orders of magnitude more expensive:
a window capture is tens of milliseconds before anything is sent anywhere. The boundary is
noise against its own payload.

Two things that measurement does *not* say, because they are the ways it could still go
wrong in practice:

- **It bounds the transport, not the implementation.** It excludes frame masking, JSON
  encoding, and the mechanism's own work. Those are all either negligible or already paid
  today. It is also Python on both ends, so a Rust or Swift peer lands at or below these
  numbers — the figures are pessimistic, which is the direction to be wrong in.
- **`TCP_NODELAY` is load-bearing.** These numbers are with Nagle off. Left on, a small
  frame waiting for an ACK is the classic path from twelve microseconds to forty
  milliseconds — the one implementation detail on this seam that can turn a non-issue into
  the exact problem this section rules out.

**And barge-in was never the case at risk.** Stopping when someone starts talking is decided
at the mouth — [the floor](host.md#the-floor) — from inbound voice activity, and both of its
ends already cross a socket today: the mic arrives on
`WS /api/in/audio/stream`, and the voice leaves on `GET /api/out/audio` to whatever is
playing it — the browser, already, on every desktop. Moving capture into a native shell adds
no hop that path does not already take.

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

3. **Where the attention gesture's discrimination lives.** The key tap is mechanism and the
   double-tap-versus-hold machine is policy, which puts the two on opposite sides of the
   wire — and the *hold* is a duration measured between two edges. Timing it in the core
   means timing it from arrival stamps rather than from the events themselves. Latency is
   not the problem (it is microseconds, above); a shell that is briefly busy and delivers
   two edges late but adjacent is, because that reads as a double-tap.

## See also

[`foundation.md`](foundation.md) for the mechanism/policy split ·
[`topology.md`](topology.md#app) for what an app owns and the two wires this adds a
direction to · [`surfaces.md`](surfaces.md) for pulled perception and degradation ·
[`../platforms/apple-macos.md`](../platforms/apple-macos.md) for the migration this
unblocks.
