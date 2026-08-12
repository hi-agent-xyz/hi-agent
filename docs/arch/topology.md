# Topology — core, app, community

**Status:** proposed August 11, 2026. Defines the split of the system into three roles, how
a person is addressed, and how an app proves it may reach one. Nothing here changes what the
agent *is*; it says where the parts of it run and how they find each other.

## Goal

Let a person be reached from wherever you are, without the person becoming a service.

Everything below follows from taking that literally. A person has one mind and several ways
you can be with them — in the room, on the phone. So the fan-out is **many surfaces onto one
person**, never many people behind one surface, and never one person split across two bodies.

## Decisions

| Decision | Reasoning |
|---|---|
| Three roles, not two | The client and the agent are separate processes with an API between them. Collapse them and "reach a core that runs elsewhere" cannot be said at all |
| The core is headless and location-independent | It already compiles to exactly this shape on Linux/Docker. Making location a parameter costs one field; assuming locality costs a rewrite |
| An app renders a core; it never *is* one | Identity lives in one place. An app that could be a person would need memory, a handle, and a life |
| **An address is a base URL** | `http://localhost:12358` and `https://hi-agent.xyz/ana` are the same kind of thing. Local, relayed and directly-public stop being modes and become values |
| **The core checks auth; the community never does** | In two of the three shapes the community is not in the path at all. Auth at the community would not replace core-side auth, only add to it — two mechanisms to keep in agreement |
| The community is infrastructure, never a principal | It has no name, cannot be addressed, and signs nothing. The moment it needs a key to speak *as* someone, the model has broken |
| Registration is independent of the account | A BYOK install pays us nothing and must still get a name and be reachable. Sharing a key between the registry and billing would quietly make the address a paid feature |

## The three roles

| | What it is | Owns | Never |
|---|---|---|---|
| **core** | the person | agent identity, handle, memory, cognition, all state, **who may reach it** | renders anything; knows other cores exist |
| **app** | a surface onto a core | the roster, the face, the OS session | holds person-identity; decides authorization |
| **community** | always-on shared infrastructure | handle namespace, routing, push tokens | holds person-keys; signs as a person; holds a roster; checks access |

---

## Core

**The core is hi-agent.** Everything the architecture already describes — the tempo ladder,
the one conversation, `data/` as the whole agent — is the core. This doc adds nothing to it
except a name, an address, and the fact that it has no opinion about where it runs.

**Headless by construction.** No GUI, no window server, no OS-session dependency. It runs on
a laptop, on a server, in Docker, and on a phone the day a platform can host one. It is
already this on every platform but macOS.

**Scope:**

- Identity: its handle, and the credentials it accepts.
- Everything in `data/`, all cognition, all capability *policy*.
- One conversation, rendered by any number of windows.

**Interface — unchanged.** The core's API is the HTTP surface it already serves: the
channels (`/api/in/*`, `/api/out/*`), views, tasks, settings. Adding remote reach adds no
endpoint. Three things are new, and all are inward-facing:

- a **client** that dials the community and holds the connection open;
- a **second listener** that serves the same router over that connection;
- an **auth layer** on off-box requests.

**Trust is structural.** Requests arriving on the tunnel listener are off-box; requests on
loopback are not. That distinction is a property of *which acceptor received the request*,
so it cannot be forged by a header the sender controls — and it is why there is no IP
allowlist anywhere in this design. An allowlist would be inert in the relayed shape anyway,
where every request shares the community's source address.

The core never learns that an app talks to other cores. Attachment reaches it only as
channels opening and closing, which it already models as presence.

## App

**The app renders a core.** It owns the process and everything needing the OS session, and
it holds a roster of the cores it may attach to — one of which, on a platform that can host,
may be a core it runs itself.

**A roster entry is `(base URL, credential, label)`.** Adding a core means acquiring a
credential for it. Two consequences fall out rather than needing rules: rosters do not sync
between apps, and revocation is per-core.

**Scope:**

- The roster, and which entry is attached.
- The face, and the platform's capability *mechanisms* (camera frames, mic bytes, screen
  pixels where the OS allows) — feeding the core's policy, per
  [`foundation.md`](foundation.md).
- Supervision of local cores. A remote core cannot be supervised, only observed.

**The face never knows where the core is.** The web face talks only to the app's local
proxy; the app routes that to the attached core — loopback or tunnel — and attaches the
credential upstream. Three things follow, and the third is the point:

- the webview never holds a credential;
- switching who you are with is the app repointing its proxy, with no face involvement;
- **desktop and mobile run identical face code**, which is what "no architectural
  difference between them" has to mean concretely.

On iOS the same is done with a `WKURLSchemeHandler` rather than a local port. Credentials
live in the OS keychain, never a plist or `localStorage`.

**Host and client are capabilities of an app instance, never properties of a platform.** An
app asks "can I host a core here?" and today a mobile app answers no. When that answer
changes, nothing structural does.

## Community

Always-on shared infrastructure — the things a single core cannot provide for itself. Four
services, independent, deliberately not sharing keys:

| Service | Does | State |
|---|---|---|
| **registry** | handle ↔ core; claim, rename, lease | the namespace |
| **relay** | routes an inbound request by handle into that core's live connection | the routing table |
| **broker** | provider role: LLM credentials and energy (`foundation/broker.rs` today) | accounts, billing |
| **post** | push to a surface, on a core's instruction; later, mail for a sleeping core | push tokens |

**The registry does not depend on the broker.** Separate tables, no foreign key, and
registration paths that never touch billing code. An optional link between a core and an
account is allowed; a required one is the bug this rule exists to prevent.

**The community never issues, checks, or holds access.** It has no ACL, no notion of which
app may reach which core, and no roster. It routes by handle and forwards bytes.

**It holds no person-keys and signs nothing as a person.** It has a TLS identity so a core
can tell the real community from an impostor — a transport identity at a different layer,
and the only one it gets.

---

## Addressing

**A person's address is a base URL.**

| Shape | Address | In the path |
|---|---|---|
| local | `http://localhost:12358` | nobody |
| directly public | `https://agent.example.com` | nobody |
| relayed | `https://hi-agent.xyz/ana` | the community |

The community addresses cores by **subpath**: one certificate, no wildcard issuance, no
per-handle DNS.

Two requirements come with that choice, and neither is optional:

**Reserved paths.** A handle cannot collide with a community route — `/`, `/healthz`,
`/api/*`, `/downloads/*` today, and whatever is added later. Because a handle cannot be
reclaimed once held, the reserved list must be deliberately over-broad from day one
(`admin`, `settings`, `login`, `help`, `about`, `assets`, `static`, `docs`, `status`, `up`,
`auth`, …). This is the username-versus-route trap, and it is only cheap before launch.

**The core is told its public base URL** at registration. Stripping the prefix at the
community is not enough: the core emits absolute paths for `/assets/*`, `/generated/*` and
the import map that `index()` injects, and under a subpath those must render as
`/ana/assets/*` or views will not resolve. `HI_AGENT_BASE_URL` is the precedent.

**One origin is shared, and that is a real property rather than a caveat.** Every relayed
core lives on `hi-agent.xyz`, so they share storage and a cookie jar; `Path=/ana` decides
what is *sent* where and is not a boundary. It matters more here than in a typical app
because a core serves agent-generated code, and the usual mitigation is unavailable: views
deliberately resolve bare imports through the page's import map to the **host's shared React
instance**, so they cannot be moved to a sandboxed origin without redesigning that.

The scope this is safe in is the scope the app already draws. **Through an app the browser
holds nothing** — it talks to the app's local proxy, the app holds the credentials, and no
core's session is ever in a page. The shared jar exists only for a browser pointed straight
at `hi-agent.xyz/ana`, which is the owner visiting their own agent.

Two smaller consequences: `__Host-` cookies require `Path=/` and are therefore incompatible
with per-core path scoping (take the scoping), and a LAN address like
`http://192.168.1.5:12358` is not a secure context, so it gets no microphone or camera —
`localhost` does.

---

## Auth

**One mechanism, at the core, identical in every shape.**

A long-lived **credential**, exchanged once for a short session:

```
POST /api/session      Authorization: Bearer <credential>
  → 200, Set-Cookie: hi_surface=<session>; HttpOnly; Secure; SameSite=Lax; Path=/…
```

Two presentations of one credential, because a header alone cannot carry a browser:
`EventSource` cannot set headers, browser `WebSocket` cannot set headers, and neither can
plain navigation — and the core serves all three. Apps and `curl` use the bearer header;
anything browser-shaped rides the cookie, which SSE, WebSocket and navigation all send
automatically.

Exchanging once rather than signing every request also keeps the long-lived secret off the
wire, and makes `POST /api/session` the single seam where a stronger proof can be swapped in
later without touching anything downstream.

**Access is shareable, and sharing it shares everything.** A credential says *this surface
may reach me* and never *and who is holding it* — so handing someone a pairing code is a
thing a person may simply do, and what they get is the whole conversation, the whole memory,
the whole ledger. That is what lending someone your laptop means, and it is the person's
call rather than a capability the architecture withholds. There is no guest, no per-person
scoping and no permission tier; who is *speaking* is a question the agent answers the way it
answers it in a room — by voice, by face, by asking.

**Storage.** `(id, label, hash, created_at, last_seen_at, revoked_at)`. The label is what
makes a device list readable and revocation meaningful. Compare in constant time, and
rate-limit failures.

**Hash with SHA-256, not argon2id.** A slow KDF exists to frustrate guessing of low-entropy
*passwords*; a 32-byte random credential is not guessable, so argon2 buys nothing and costs
latency on every attach. The broker's argon2id use is correct because those are human
passwords — this is a different thing, and the reason belongs in a comment or someone will
"fix" it.

### What is gated

| | |
|---|---|
| loopback | ungated — `make dev`, curl journey testing, the popover and MCP workers are unaffected |
| off-box (relayed or directly public) | gated |
| `/up/{token}`, `/api/up/{token}` | own one-time token, stays open |
| `/healthz`, `POST /api/session`, pairing | open by definition |

Off-box HTML navigation without a session serves a small "enter your pairing code" page
rather than a bare 401 — which is also how browser-direct onboarding starts.

### CSRF

A cookie introduces what a bearer header does not: another site can make the browser issue
authenticated cross-origin requests. `SameSite=Lax` blocks the form-POST class;
state-changing endpoints additionally require a JSON content type or a custom header, both
of which force a preflight a simple cross-site request cannot satisfy. Small, but it will
not happen unless it is written down.

### The credential is an opaque random token

Not a keypair. A keypair would buy one thing — a credential a reader in the middle cannot
walk away with — and there is a reader in exactly one shape: relayed, where the community
terminates TLS. That is a community we already trust to route our traffic honestly, so
paying for it in every attach, every verify and a second credential type is paying for a
guarantee we are not otherwise relying on.

Stated plainly rather than hedged: **in the relayed shape the community is trusted not to
replay what it forwards.** The storage table would hold a key type if that ever stopped
being acceptable, but nothing is designed around the possibility.

---

## The two wires

Because an address is a base URL and the community never checks access, an app talks only to
its cores. There is no app-to-community wire.

### app ↔ core — the existing API

The core's HTTP surface, unchanged, over whichever transport the address implies. Loopback,
direct and relayed are three carriers of one protocol, never three protocols.

**The transcript already covers reconnection**, which is why remote attachment needs no
history mechanism of its own: a window that reconnects receives a `reset` frame with the
current conversation and scrolls back through `/api/messages`. See
[`text-transcript.md`](text-transcript.md) — *"nothing was missed: the messages are still
there."* No cursor, no per-device state, nothing for a phone to keep.

Push tokens travel this way too: the app hands its token to the **core**, which passes it to
post when it wants to notify. The app never registers anything with the community.

### core ↔ community — the tunnel

One outbound connection, dialed by the core, held open. It carries two kinds of traffic:

| | Carries | How |
|---|---|---|
| **control** | register, claim, renew, push instructions, disconnect reason | ordinary HTTPS requests to the community, dialed per call |
| **routed** | inbound requests for this handle, one stream each | the held connection |

**Control is not tunnel traffic.** The core can always dial out — that is the premise of the
whole shape — so control is a REST call like any other. Putting it inside the tunnel would
mean writing, versioning and debugging a second request/response protocol whose only
advantage is sharing a socket.

**The tunnel is a stream multiplexer over one WebSocket.** Multiplexed with per-stream flow
control — a stalled audio stream must not freeze text — and each routed stream carries plain
HTTP/1.1, so a WebSocket upgrade passes through as an ordinary `Upgrade` and the core hands
the stream to the same router it already serves. Concretely: yamux over WSS, one implementation
each side. The alternative was reversed HTTP/2, which is a better fit for everything *except*
the `Upgrade` — carrying WebSocket over it needs extended CONNECT, and audio and vision capture
from a remote surface are exactly the traffic that would ride it.

**The connection is the liveness signal.** It renews the handle lease as a side effect of
being reachable, so there is no separate heartbeat to drift out of agreement with reality.

Dialing out is what makes this work behind NAT with no configuration: anywhere the core can
already reach the community, it can be reached back.

---

## Identity

| | Where | Means |
|---|---|---|
| **handle** | the registry | the address |
| **credential** | issued by a core, held by an app | this app may reach that core |

**Naming is three layers**, and collapsing them is the classic mistake:

| | Mutable | Unique | For |
|---|---|---|---|
| core id | never | yes | references, audit. Never reused |
| handle | renameable | yes | the address |
| display name | freely | no | what it calls itself |

**A handle is a lease, not a deed.** It is held while the core checks in and released after
prolonged dormancy — generous, months not days, and a claimed-but-never-seen handle expires
far sooner than a live one gone quiet. That is what lets registration be free without
inviting bulk squatting: a hoarded handle has to be kept alive by a running core.

**One body per person.** Two machines running one handle would be one identity with two
memories, two ledgers and two presences. A second machine is either a second person or a
migration — never a second body.

---

## Attachment

| State | The app holds | The core sees |
|---|---|---|
| **attached** | text + view channels, rendered | a window is open |
| **ambient** | the connection only | nobody is there |
| **detached** | nothing | nobody is there |

**The core gains no new states**, and nothing is lost by being away: the conversation is an
append-only list, so a message said to nobody waits in it (see
[`text-transcript.md`](text-transcript.md)). Attachment answers one question — is a speaker
attached, so a spoken span is worth synthesizing.

Windows are cheap: the conversation is one backend-owned list rendered by any number of
them, so a Mac window, a popover and a phone are three subscribers to one list. Nothing
forks, no session multiplies.

---

## Workflows

**First run, app hosting a core.** The app finds no local core, creates a `data/` directory
and starts one, then attaches over loopback — ungated, so nothing is needed yet. Claiming a
handle and dialing the community are separate, optional steps: an offline core with no
handle works, unreachable.

**Claiming a handle.** The core asks the registry, which records the handle and starts the
lease. Rename is the same call; the id underneath never changes.

**Pairing a second app — three paths, and all three are needed.**

- **By QR.** The core displays its base URL and a short-lived one-time token — the mechanism
  `/api/handoff` and `/api/qr` already use for phone upload. The new app posts it and
  receives a credential.
- **By an app that already has access.** Your Mac authorizes your phone. This is the normal
  path for a core with no screen — one in Docker on a server — and it is `authorized_keys`
  again.
- **By first-boot credential**, printed once to the core's log. Bootstrap only, for when no
  app has access yet.

**Attaching remotely.** The app opens a request to the base URL. The community routes it
into that core's live connection as a stream; the core serves it from the tunnel listener —
off-box, therefore gated — checks the credential, and answers. To the face this is
indistinguishable from loopback.

**The core is asleep.** No live connection for the handle, so the community answers with a
plain "asleep" page and `Retry-After` for HTML, and a JSON error for `/api/*`. Nothing is
queued: mail for a sleeping core is deliberately later work.

**Waking a surface.** The core instructs post to notify a surface; the app raises a
notification; opening it attaches. This is the only way to reach someone whose app holds no
channel, which is the normal state of a phone.

**Revoking a surface.** Remove its credential at the core. No community involvement — losing
a phone does not require the community to be reachable, or trusted, to fix.

**Revoking while the core is asleep.** The one case the above cannot serve. The community
may **refuse to route** for a surface reported lost. That is a routing decision, not an
authorization one — the credential is still only ever checked by the core — and it is
superseded by a real revocation the moment the core is reachable.

---

## Invariants

Each is testable, and each has a real failure behind it.

1. **The community is never a principal.** It has no name, cannot be addressed, holds no
   person-credential, and nothing it serves is authored by it — it routes bytes and
   forwards them unchanged.
2. **The community may know you; it must never require billing you.** The registry and the
   broker share no key.
3. **The core is the sole authority on who may reach it.** The community never issues,
   checks or holds access. *Stated plainly: in the relayed shape it is trusted not to
   replay what it forwards, because a bearer token is a bearer token.*
4. **The core never learns of other cores.** The roster is app state and stays there.
5. **One body per person.** One handle, one live core.
6. **Off-box trust is structural** — decided by which listener accepted the request, never
   by a header and never by an address.
7. **Host and client are capabilities of an app instance**, never properties of a platform.

---

## Not built

Two things, and each is a thing the design has a shape for and no code behind:

- **Mail for a sleeping core.** The community can hold it; the core has nowhere to put it
  yet. Until then an inbound request for a sleeping core is answered "asleep" and nothing
  is queued.
- **A core on iOS.** Blocked by the wire being a spawned binary, not by effort. Everything
  above already treats hosting as a capability of an app instance rather than a property of
  a platform, so the day that changes, nothing structural does.

## See also

[`arch.md`](arch.md) for the layers inside a core ·
[`surfaces.md`](surfaces.md) for how the world reaches it once a wire exists ·
[`text-transcript.md`](text-transcript.md) for why reconnection needs nothing ·
[`foundation.md`](foundation.md) for the mechanism/policy split an app inherits
