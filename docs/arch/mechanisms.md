# Mechanism calls — reaching the app's hands

## Goal

Let the core reach an OS touch it does not hold — on whichever machine the person is
actually at — without the core learning a platform API, and without inventing a second
protocol beside the one an app already speaks.

The split is already decided, in [`../../CLAUDE.md`](../../CLAUDE.md) § *Mechanism vs
policy*: the raw touch is **mechanism** and belongs to whoever holds the OS session, the
cross-platform judgment is **policy** and belongs to the core.
[`topology.md`](topology.md#app) already assigns the mechanisms to the app. What has never
existed is the call that crosses between them.

**This document used to be about eyes and hands, and it is not any more.** Screen capture,
input synthesis, the accessibility tree and the frontmost-app read were the whole reason to
want core-originated calls; all four are deleted, and § *Computer use does not cross this
seam* is why. What is left needing initiative is small and dull — raising a window, a line
of tray text, a handed screenshot — and the seam is kept for it and for Phase 2, not for
perception. Smaller still than it reads: two things once counted here turned out not to
need it at all. Whether the ear is open is *state several surfaces read*, so it is
`GET /api/listening` (Open 2). And the key edges only cross when the tap is the shell's,
which is a question rather than a given on an OS that gates the tap behind nothing
(Open 4).

## Decisions

| Decision | Reasoning |
|---|---|
| **The app answers calls as well as making them** | This is the whole object. Every other seam has the app asking and the core answering; a capability the core needs is the one case where the core must ask. Nothing else about the wire changes |
| **The app always dials, even though the core now originates requests** | A core that had to dial an app could not reach one behind NAT, and a relayed attachment routes *into* a core, never out of one. Dialing is what keeps loopback, direct and relayed three carriers of one protocol instead of three protocols |
| **Perception stays pulled** | [`surfaces.md`](surfaces.md#why-a-ref-and-not-the-bytes): a photo arriving does not mean the agent looked. Screen pixels are the same — the core asks for a frame when a turn warrants one. A continuous encode is a *cast*, which is different work with a different consumer |
| **Audio is the only thing that streams, and it already does** | `WS /api/in/audio/stream` exists and the browser mic uses it today. A shell streaming PCM is that endpoint with a different client, not a new design |
| **Pixels ride binary frames** | A PNG is bytes. Base64 inside a JSON frame costs a third again and a copy, on the one payload big enough for either to matter |
| **Computer use does not cross this seam — it does not exist in the core at all** | The rows this table was built for are gone. Driving a machine is a note over that machine's own tools, like `browser` and `phone`; the test that settled it is that a mechanism here must be rewritten for X11, Wayland, Windows and Android, while the judgment that reads a screen and picks a target is the same everywhere. Full reasoning below |
| **The mic moves to the shell only where a per-app grant makes it move** | `CLAUDE.md` § *Permission model* tips mic capture out of the core to keep it free of TCC. That reasoning is macOS's, and it is the whole of the reasoning — cpal is one portable dependency with no per-OS branch in it, the same shape as ffmpeg and ONNX Runtime, which the core links without anyone calling it impure. Where the OS has no per-app microphone grant to hold, nothing tips, and the core opens the mic itself |
| **What an app can do is what it declared when it dialed** | `available()` is a fact about who is attached, not a compile-time `cfg` — and one that changes while running, when a laptop sleeps |
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

**Core → app — calls, each with a reply: none today.**

This table used to list six — `screen.windows`, `screen.grab`, `screen.size`,
`input.perform`, `ax.inspect`, `desktop.context`. Every one of them named a capability that
has since been deleted from the core, so none of them has an originator and none is
specified here any more. The call machinery stays (`POST /api/mechanisms/call` exercises it),
because the direction is what Phase 2 needs and because the next mechanism that wants asking
should not have to re-derive the wire. What it must not become is a general perceive/act
surface re-entering by the back door: see § *Computer use does not cross this seam*.

**Core → app — state, no reply:** the tray's `flash` and `set_text`, and `open_chat`.
These are pushes, not questions; nothing waits on them and a dropped one is survivable.

`set_listening` was on that list and is **not a push at all** — see Open 2, now answered.
Whether the ear is open is a fact several things draw (a menu bar, a notification area, a
phone, a browser face), so it is state on the core that anyone may read:
`GET /api/listening`. The general form is worth keeping: **a fact more than one surface
renders does not belong in a call addressed to one of them.** A push that only ever had one
possible recipient — raising *this* app's window — is the shape that stays here.

**App → core — events, no reply:** the attention gesture's key edges, *where the tap is
the shell's*. The machine that tells a double-tap from a press-and-hold is policy and stays
in the core either way, so when the tap is across the wire the edges have to arrive as
events. This is why the connection carries both directions rather than the app simply
POSTing: an edge is not one of the five inbound channels and has no business inventing a
sixth.

Whether the tap is the shell's on every platform is Open 4 — on an OS that gates the tap
behind a per-app grant it plainly is, and on one that gates nothing the question is open and
the code currently answers it the other way.

**App → core — perception, over the endpoints that already exist and are unchanged:** mic
PCM on `WS /api/in/audio/stream`, camera on `WS /api/in/vision/stream`, a handed screenshot
on `POST /api/in/file`.

### "Come and see this" is not a mechanism call

Worth stating outright, because the two are easy to confuse and the confusion invents a
problem that does not exist.

**Handing the agent your screen is app-initiated.** The person fires it — a Shortcut on the
phone, the ⌘⌘ gesture on the desktop — and the app that ran the gesture is, by construction,
the app holding the screen in question. It arrives as a file with a note, on the door that
already exists, and the core is told rather than asked. Nothing about it needs this
connection, and it works today.

**There is no longer an "other thing" to confuse it with.** `screen.grab` used to be the
agent driving a machine — the look→act loop that captures pixels, decides, and synthesizes a
click — and that is now a note, not a call. So every screenshot the core sees is one a person
handed it, and the targeting question this subsection existed to settle ("whose screen?")
answers itself: a handed screen names its own device by arriving from it.

The macOS side of the gesture is the one screen grab left in the tree, and it is deliberately
*not* a capability: [`crate::body::gesture`] shells out to `screencapture` directly. One
caller, one platform, no vendor to select, nothing to delegate — the gesture happens on the
machine whose key was tapped.

## Computer use does not cross this seam

The core had four OS-backed capabilities for driving a desktop — `screencast`, `input`,
`accessibility`, `desktop_context` — plus the taught quick-action **reflex** rung that was
their only consumer. All of it is deleted. Not moved to the shell, not deferred: deleted.

**The test is whether the code grows with the platform count.** An accessibility tree is AX
on macOS, UI Automation on Windows, AT-SPI on Linux; input synthesis is CGEvent, SendInput,
uinput/libei; capture is `screencapture`, DXGI, a portal. Each is a separate implementation
with a separate permission model, and re-homing them in the shell only changes *which*
per-platform file has to be written — it does not make them fewer. What does not multiply is
the part that matters: reading a screen and deciding where to click is one piece of judgment,
identical everywhere, and a model already has it.

So the general path is the one `browser` and `phone` take — **a note over the tools the target
machine already has** (`driving-a-desktop.md`, and `tools.md` § on why a tool is a note). The
note tells the agent what to find out and what goes wrong; it does not hand it a command,
because there is no one command to hand.

**Why the reflex rung went with them.** A reflex runs *because* no model is in its loop, so it
is the one path that cannot read a note — that was the whole argument for keeping the
capabilities. It does not survive the second fact: nothing could ever teach a reflex.
`hi_record_reflex` was advertised to no role, so the store was permanently empty, the
recognizer permanently abstained, and `fire` never ran once. Keeping four per-platform
mechanisms alive for a consumer that could not be fed is the shape this repo calls
described-and-absent.

**What replaces it is expected to be mostly outside this repo:** a learned script or a tool
the agent writes for the machine in front of it and leaves beside the note
(`equipping-a-tool.md`), not a rung rebuilt in Rust. If a grooved model-less move turns out to
need core support after all, that is a new design with a live consumer behind it — and this
seam is still here to carry it.

## The connection

One long-lived WebSocket, dialed by the app, held open: `WS /api/mechanisms`. Loopback-gated
when local, and carrying the app's credential like any other request when it is not — the
same gate the config API uses, not a mechanism of its own.

Text frames carry calls, replies and events as JSON. Binary frames carry payloads, correlated
to a call by its id. The app may have several calls outstanding; replies are matched by id
and never by order, because two calls have nothing to do with each other's timing.

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

The obvious worry about putting a process boundary under a capability is latency. The
numbers below were taken when this seam was still expected to carry perception and
actuation — the sharpest case there was, since a model-less fast path spends from a budget
with a person's patience at the end of it. That case is gone with the capabilities, so the
measurement now proves something easier than it was taken to prove. It is kept because it
**closes the question for whatever crosses next**: nobody should re-open "is a process hop
too slow for this" without these numbers in hand.

**Measured on an M4, loopback, warm connection, payload round-tripped:**

| Frame | Median | p99 |
|---|---|---|
| 64 B — a small call, e.g. a tray push | 0.012 ms | 0.053 ms |
| 4 KB — a structured reply | 0.013 ms | 0.018 ms |
| 2 MB — a payload the size of a screen grab | 0.288 ms | 0.580 ms |

A dozen round trips is **under a fifth of a millisecond of transport**. Any real mechanism is
three to four orders of magnitude more expensive — a window capture is tens of milliseconds
before anything is sent anywhere. The boundary is noise against its own payload.

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
clustering, every model call. Camera and mic bytes may still arrive
from the web face rather than a native shell, which is exactly why that layer is
cross-platform and stays put.

`hi_say` and `hi_show` are unaffected. Speech and showing are outbound tool calls the core
resolves; nothing about them needs the app's hands.

## Open

1. **What a core-initiated call defaults to when it names no app and two could serve it.**
   Naming one is a parameter, not a policy — the core knows its attachments. The only real
   question is the default, and it is small: two desktops attached at once is the case, a
   phone cannot serve these mechanisms at all, and one desktop leaves nothing to decide.
   (This entry previously claimed *"look at my screen"* was ambiguous. It is not — see
   *"Come and see this" is not a mechanism call* above.)

2. ~~**Whether tray state belongs on this connection at all.**~~ **Answered: no.** The
   objection was right and it generalises — an app wants the current value, including the
   value from before it attached, and a fire-and-forget call leaves a reconnecting app with
   a stale tray until the next change. So the ear is state on the core, read at
   `GET /api/listening`, and every surface that draws it reads the same thing. What stays on
   this connection is the pushes with exactly one possible recipient: `flash`, `set_text`,
   and raising a window.

3. **Where the attention gesture's discrimination lives.** The key tap is mechanism and the
   double-tap-versus-hold machine is policy, which puts the two on opposite sides of the
   wire — and the *hold* is a duration measured between two edges. Timing it in the core
   means timing it from arrival stamps rather than from the events themselves. Latency is
   not the problem (it is microseconds, above); a shell that is briefly busy and delivers
   two edges late but adjacent is, because that reads as a double-tap.

4. **Whether a mechanism with no grant behind it still belongs in the shell.** The rule in
   `foundation.md` puts every OS touch in the shell, and its stated reason is that the shell
   is "the one holding the session + grants". A global key tap on macOS needs Input
   Monitoring, so the reason bites. On Windows a low-level keyboard hook needs nothing: any
   process in the interactive session may install one, and the engine is in that session.

   Two answers, and neither is obviously wrong. **Shell anyway**, because the engine is meant
   to link no platform GUI API and `SetWindowsHookExW` is one, because a rule that holds
   everywhere is worth more than a rule with a carve-out, and because the shell already has a
   message pump while the engine has to start a thread for one. **Engine**, because the rule's
   own reason does not apply, because the round trip adds a hop to the one path where two
   edges arriving adjacent get misread as a double-tap (Open 3), and because a tap in the
   engine is testable on a machine with no shell.

   **The code answers "engine" today, and that is a named loan, not a settled answer**:
   `src/foundation/vendors/windows_hotkey.rs` is Win32 inside the engine because no Windows
   shell dials this connection yet, and moving it is a file to delete and a `hello` line to
   extend. The microphone beside it is *not* a loan — see the Decisions row above, which
   turns on a per-app grant existing rather than on who dialed.

## See also

[`foundation.md`](foundation.md) for the mechanism/policy split ·
[`topology.md`](topology.md#app) for what an app owns and the two wires this adds a
direction to · [`surfaces.md`](surfaces.md) for pulled perception and degradation ·
[`../platforms/apple-macos.md`](../platforms/apple-macos.md) for the migration this
unblocks.
