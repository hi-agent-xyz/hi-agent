# Topology — core, app, community

Defines the split of the system into three roles, how a person is addressed, and how an app
proves it may reach one. Nothing here changes what the agent *is*; it says where the parts of
it run and how they find each other.

This is the goal state, in the present tense throughout, as design here always is. It says
nothing about what is built — the worklist tracks that, and a design document that doubles as
a status report goes stale in a way that makes readers distrust the design too.

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
| **A platform with no webview cannot run an app** | The face is a web app everywhere on purpose, and an agent-generated view is a compiled ESM module mounted into that page. A platform that cannot render a page can only be given a second face — hand-written, permanently drifting, and still unable to run a view. tvOS is the case that forced this |
| **An address is a base URL** | `http://localhost:12358` and `https://ana.hi-agent.xyz` are the same kind of thing. Local, relayed and directly-public stop being modes and become values |
| **One origin per core** | A core is at the root of its own origin in every shape, so "reached from outside" stops being a case. A prefix would otherwise have to be remembered by every path the core hands out, and a shared origin puts one person's agent-generated code inside another person's security boundary |
| **The core checks auth; the community never does** | In two of the three shapes the community is not in the path at all. Auth at the community would not replace core-side auth, only add to it — two mechanisms to keep in agreement |
| The community is infrastructure, never a principal | It has no name, cannot be addressed, and signs nothing. The moment it needs a key to speak *as* someone, the model has broken |
| **A handle is owned by an account, permanently** | An address is only worth handing out if it survives a new laptop and a quiet month. Permanence needs an owner that outlives any one machine, and the only such thing is an account. A lease would put the burden on the person to keep proving they still want their own name |
| Claiming may require an account; it must never require a **paid** one | A BYOK install pays us nothing and must still get a name. "Free" is the load-bearing word — sign-up is a cost the person pays once, billing is a cost that would make the address a product |
| **Content does not ride the tunnel** | The relay is an 11 Mbps box, measured. A small machine can carry anyone's conversation and nobody's photographs. Immutable bytes go to a cache the app fetches directly, ahead of being asked for |

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
a laptop, on a server, in Docker, and on a phone the day a platform can host one.

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

**The app holds the long-lived credential; the webview gets a short-lived session, and
loads the core's own address.** The app presents its credential once at
`POST <core>/api/session`, takes the `hi_surface` cookie that comes back, installs it in
the webview's cookie store, and navigates the webview at the core. Three things follow,
and the third is the point:

- the webview never holds the long-lived credential — only a session that expires and can
  be revoked at the core that issued it;
- switching who you are with is loading a different URL, with no face involvement;
- **desktop and mobile run identical face code**, which is what "no architectural
  difference between them" has to mean concretely.

Switching cores now switches *origins*, so a webview never holds two people's sessions at
once and the app needs no ritual to keep them apart — the browser's own boundary does it.
That is the practical shape of [one origin per core](#addressing), and it is why an
attach/detach hook clearing the cookie store is not part of this design.

**Not a local proxy — that was tried and lost.** Until 2026-09-03 this said the face talked
only to a loopback proxy the app ran (`crates/hi-app`), which forwarded to the attached core
and attached the credential upstream. Both shapes keep the credential out of the page, and
the proxy cost a listener, a second port, a roster table and 1,200 lines to do it. What
settled it is that the mobile clients never adopted it: iOS shipped the session exchange in
`CoreClient.swift` and Android in `CoreClient.kt`, both loading the core directly, and the
paragraph claiming one Rust implementation spared "a Swift retelling" was describing a
sharing that never happened — neither project links any Rust. On the desktop the proxy was
attaching a credential to reach a loopback listener that is
[ungated by construction](#auth), so it bought nothing there either. Deleted rather than
kept: the session exchange is the design, on every platform.

**A page must be served from a potentially-trustworthy origin**, which is the constraint
underneath all of this and the reason a `WKURLSchemeHandler` is not an option. A custom
scheme is not a secure context, so a page served through one gets no microphone and no
camera — which is most of why a phone is an interesting surface at all. `https://` for a
remote core and `http://127.0.0.1` for one on this machine both qualify. Credentials live
in the OS keychain, never a plist or `localStorage`.

**Host and client are capabilities of an app instance, never properties of a platform.** An
app asks "can I host a core here?" and a mobile app answers no. When that answer changes,
nothing structural does.

**The one platform fact that does bind is the webview, and no page at all is where an app
stops being possible. tvOS is that case, and an Apple TV client was declined on 2026-09-03
rather than deferred.** tvOS ships no WebKit: there is no `WKWebView`, no browser and no
HTML rendering on Apple TV at any deployment target, so this is not a version to wait for.
The wire itself would carry one — an app presents the bearer header and needs no cookie at
all ([Auth](#auth)), `GET /api/out/text` is the whole conversation, and a client holding
`GET /api/out/audio` is counted as a speaker by
[`Attachments::speaker_attached`](../../src/body/attachments.rs), which a television in a
living room genuinely is. What defeats it is what would sit on top: **a second face,
written by hand, drifting behind the web one for as long as both exist** — and one that
still could not run an agent-generated view, because a view is a compiled ESM module
mounted into a page. The nearest thing available is a photograph: `view_shots.rs` already
renders every show through headless Chromium at 480px for the band's tiles, so a
television could show a picture of what the agent put up and never the thing itself. Voice
in the room and the last few things said are real, and they are not worth a face that
permanently disagrees with the real one. A television is reachable without a client
anyway, by mirroring the phone onto it — untried, and needing no code either way.

## Community

Always-on shared infrastructure — the things a single core cannot provide for itself. Five
services, independent, deliberately not sharing keys:

| Service | Does | State |
|---|---|---|
| **registry** | handle ↔ account; claim, rename | the namespace |
| **relay** | routes an inbound request by the handle in its `Host` into that core's live connection | the routing table |
| **broker** | provider role: LLM credentials and energy | accounts, billing |
| **post** | push to a surface, on a core's instruction; later, mail for a sleeping core | push tokens |
| **cache** | serves immutable bytes a core has mirrored, against a signature that core minted | mirrored objects only, all of them discardable |

**The cache is the one service holding a person's own bytes**, and it holds only a copy: see
[*Content*](#content) for what may be mirrored and why the core remains the truth.

**The registry knows accounts and must never know billing.** A handle is owned by an account,
so the two share an identity — but nothing on the claim path reads a tier, a balance or a
payment, and a free account claims a name exactly as a paid one does. That is the line: the
community may require you to be someone; it may never require you to be a customer.

**The community never issues, checks, or holds access.** It has no ACL, no notion of which
app may reach which core, and no roster. It routes by handle and forwards bytes.

**It holds no person-keys and signs nothing as a person.** It has a TLS identity so a core
can tell the real community from an impostor — a transport identity at a different layer,
and the only one it gets.

---

## Addressing

**A person's address is a base URL.**

| Shape | Address | In between |
|---|---|---|
| local | `http://localhost:12358` | nobody |
| directly public | `https://agent.example.com` | nobody |
| relayed | `https://ana.hi-agent.xyz` | the community |

The community addresses cores by **subdomain**, one per handle, under a wildcard
`*.hi-agent.xyz`.

**A core is at the root of its own origin, in all three shapes.** That is the property this
choice exists for, and everything below is either a consequence of it or a cost of it.
Nothing the core emits carries a prefix, nothing rebases a path at the edge of the page, and
"reached from outside" is not a case any code distinguishes.

### Why not a subpath

This said `hi-agent.xyz/ana` until 2026-09-08. The reasoning then was one certificate, no
wildcard issuance and no per-handle DNS, accepting a shared origin **with a named trigger to
revisit it: a browser visiting another person's core.** Working out how a view could be handed
to someone who is not the owner fired that trigger, and both halves of the original trade then
turned out worse than they read.

**A prefix is a standing bug generator, and that is the larger of the two reasons.** The page
can only rebase what it *initiates* — `fetch` and `EventSource` — so a root-absolute path in
an `<img src>`, a CSS `url()`, an `<a href>` or a `new Audio()` names the community's root
rather than the core's. Three shipped surfaces had exactly that bug (`people-review`'s face
crops and voice clips, the views band's tile pictures, `tasks`' file links), the view-builder
prompt carries a whole section of discipline that exists only to prevent it, and **grep does
not find the misses**: the band's was `src={shot}`, a backend-supplied path in a variable,
indistinguishable from a correct call site. A rule every author must remember, on a failure
mode no search can enumerate, is not a design — it is a recurring outage with documentation.
On a phone this is not even the exceptional case: the mobile clients point their webview
straight at the paired address, so the prefix is the *only* case there and nowhere else.

**A shared origin has no boundary inside it, and a core serves agent-generated code.** Cookie
`Path` decides where a cookie is *sent*, matched against the request's path and never against
the initiating document — so a page under one handle can call another handle's API and the
browser attaches that other person's session. The credential is not stolen (it is `HttpOnly`);
it is *used*, which is worse to reason about and just as total. None of the existing
mitigations reach it: `SameSite` and the preflight requirement both discriminate cross-site
requests, and this is same-origin. Nor can views simply be moved to an origin of their own:
they resolve bare imports through the page's import map to the **host's shared React
instance**, so separating them would mean redesigning that. Handing a view to someone is what
makes this concrete — it puts one person's model-authored code in another person's browser on
one click — but the exposure was never limited to that.

**And the cost that bought the subpath was overstated.** Wildcard DNS is one record, not one
per handle. EdgeOne accepts `*.hi-agent.xyz` as an accelerated hostname, so no name has to be
registered as a handle is claimed. And because the edge terminates TLS while the origin runs
plain HTTP behind it, **the origin needs no wildcard certificate at all** — the one real cost
sits at the edge, where a certificate was already being managed.

### What comes with it

**Reserved handles.** A handle cannot collide with a name the community itself needs — `www`,
the apex, `api`, `app`, `mail`, `admin`, `assets`, `static`, `docs`, `status`, `auth`,
`shared`, and whatever is added later. Because a handle cannot be reclaimed once held, the
reserved list must be deliberately over-broad from day one; this is the username-versus-route
trap, and it is only cheap before launch. Under a wildcard it is sharper than under a subpath:
an unclaimed name no longer 404s at a site that owns the path space — **it resolves, reaches
the relay, and is served by something**. The relay answers a name it does not know the way it
answers a sleeping one, and reserved names are held by the registry rather than by a route.

**Routing is by `Host`.** The relay reads the handle from the request's hostname, not from its
first path segment. The edge forwards the requested hostname unchanged; nothing rewrites a
path on the way in or on the way out.

**Cookies are host-only — nothing ever sets `Domain=`.** A `Domain=hi-agent.xyz` cookie is sent
to every subdomain, which would hand back the shared jar this section exists to remove. This is
the browser's default and therefore easy to "fix" by accident, which is why it is written down
rather than assumed.

Two smaller consequences: `__Host-` cookies are usable again (they require `Path=/`, which
per-core path scoping had made impossible), and a LAN address like `http://192.168.1.5:12358`
is still not a secure context, so it gets no microphone or camera — `localhost` does.

### The transition

Handles are already claimed and devices are already paired against `hi-agent.xyz/<handle>`,
and a roster entry *is* a base URL, so the format cannot simply change under them.

- The community serves **both** forms during the transition, `hi-agent.xyz/ana` answering with
  a `308` to `ana.hi-agent.xyz` — never proxying, because a second way in is a second thing to
  keep in agreement. **`308` and not `301`**: this path carries `POST /api/session`, the first
  call an already-paired app makes, and `301` is the redirect browsers historically rewrite to
  `GET`. That would drop the body and turn an attach into a silent failure, which is the one
  outcome a transition mechanism must not have.
- A redirect crosses an origin, so **the session cookie does not survive it**. An app follows
  the redirect, records the new base URL in its roster entry, and re-presents its long-lived
  credential at `POST /api/session` on the new origin — which is the ordinary attach path, not
  a migration path. Nothing needs re-pairing; the credential was never origin-bound.
- The path form is **dated for deletion, not kept as a compatibility layer** — it goes when the
  last roster entry has moved, and a redirect that outlives that is the thing this repo has
  learned to delete rather than deprecate.

---

## Auth

**One mechanism, at the core, identical in every shape.**

A long-lived **credential**, exchanged once for a session:

```
POST /api/session      Authorization: Bearer <credential>
  → 200, Set-Cookie: hi_surface=<session>; HttpOnly; Secure; SameSite=Lax; Path=/
```

`Path=/` and no `Domain=`: the cookie covers the whole of this core because the origin *is*
this core, and it is host-only so it reaches no other one.

**The session is durable and rolling.** It lives in the same table the credential does, and
survives a restart — because a browser's only durable secret *is* the cookie. An app
re-exchanges what it keeps in the OS keychain and never notices a restart; a page at
`https://ana.hi-agent.xyz/` has nothing to re-present, so a session held in process memory
made every restart a walk to the machine for a fresh code, while the `Set-Cookie` already
sent claimed thirty days. The server was forgetting something it had promised to remember.

Rolling: a use more than a day into a session's life extends it to a fresh thirty, and the
gate re-sends the same cookie. Open the address once a month and it keeps working; leave it
thirty days and it asks again. **The token is not rotated** — rotation breaks concurrent
in-flight requests from one page, and buys a guarantee nothing here relies on, since the row
is what revocation removes either way. A second refresh token is likewise not the shape:
it would add a credential type to do what extending one row does.

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

**Three ways a surface is admitted**, and the third is the only one that works from across
the house:

1. a credential it already holds;
2. a one-time pairing code, read off the core's screen or scanned from its QR;
3. **asking** — the device states the agent's name, the core files a request, and the person
   approves it from the `reach` view. Both screens show the same six digits so the approver
   can tell which device is theirs. That code is **not a secret and authorizes nothing**; the
   secret is handed to the asking device in the response body and spent once for the
   credential.

The third exists because the first two need a keyboard or a camera pointed at the machine,
and a TV has neither. `POST /api/access/request` is therefore **unauthenticated**, and what
bounds it is a cap on how many may be waiting at once (eight; a ninth is refused `429`) and
the ten-minute life they share with pairing codes. The poll secret is bounded by its own 32
bytes: a wrong one counts a failure, but an open path is answered before the throttle is
consulted, exactly as `POST /api/session` is — a bootstrap route that refuses on a spent
budget locks a device out of the only way it has in.

**The agent is never told a request is waiting.** Anything that let an unauthenticated write
reach Cognition would let a stranger make someone else's agent speak. The `reach` view is the
only place a request appears.

**Storage.** `(id, label, hash, created_at, last_seen_at, revoked_at)` for the credential, and
`(hash, credential_id, created_at, expires_at)` for each session standing on it. The label is
what makes a device list readable and revocation meaningful; revoking drops both halves, so a
revoked phone stops working at once rather than when its session lapses. Compare in constant
time, and rate-limit failures.

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
| `/api/access/request` — **singular**, `POST` and `GET` | open: the caller is a device with no way in. Capped, timed, throttled |
| `/api/access/request**s**` — the approver's side | gated. A device that could read this could read every waiting code |
| a shared view, and only the paths its share derives | its own grant, public or keyed — [`sharing.md`](sharing.md) |

Off-box HTML navigation without a session serves a small page with both ways in rather than a
bare 401 — which is also how browser-direct onboarding starts. **Asking leads there**, because
that page is most often reached by a browser whose session lapsed, on a machine that is not the
one the agent runs on; a code would mean walking to the other one.

### CSRF

A cookie introduces what a bearer header does not: another site can make the browser issue
authenticated cross-origin requests. `SameSite=Lax` blocks the form-POST class;
state-changing endpoints additionally require a JSON content type or a custom header, both
of which force a preflight a simple cross-site request cannot satisfy. Small, but it will
not happen unless it is written down.

**These now cover another core, which under a shared origin they could not.** Both defences
discriminate by site, and two cores on one host were neither cross-site nor cross-origin — so
a page under one handle reaching another handle's API was indistinguishable, to every layer
that could have stopped it, from that person's own face. Per-core origins are what make the
CSRF story apply to the case it most needed to.

### Nothing behind the gate is `public`

A gated `200` is served *because* a credential checked out, so labelling it `public` invites
any shared cache to keep it and hand it to the next caller — who was never asked for one. The
gate is then intact and bypassed at the same time, and the core never sees the request that
walked around it.

This is not a hypothetical about some future CDN. **Relayed, there is always one**: the
community terminates TLS, and the origin sits behind an edge besides. An authorized fetch of
`https://ana.hi-agent.xyz/assets/*` populates that cache; the next fetch of the same URL,
carrying no credential at all, is answered out of it, and the core's `401` never runs. A
per-core origin does not help here — the edge keys on the URL, and every visitor to that core
asks for the same one.

So every cacheable response the gate protects is **`private`** — the browser cache, which is
all it was ever for, without the shared one. Content-addressed names are why a module may be
cached *forever*; they were never a reason to cache it *shared*, and the two properties read
so alike that this is worth stating rather than assuming.

**This belongs at the core, not the relay.** The community forwards bytes unchanged and
decides nothing about access — a cache-control rewrite at the relay would be exactly the
second authorization mechanism invariant 3 exists to prevent.

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
its cores. **There is no app-to-community wire** — an app never registers, asks, or
authenticates there.

An app does fetch bytes from the community's [cache](#content), which is not a third wire and
not an exception to that sentence: it carries no protocol, asks nothing, and returns only
immutable bytes some core already decided to put there. The app arrives holding a redirect its
own core issued.

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

**One direction on this wire runs the other way round.** The app owns the platform's
capability mechanisms, so a core that wants one — the tray updated, today — has to *ask the
app*, the only case in the system where the core is the one asking. It stays on this wire and
the app still dials, because a core that had to dial an app could not reach one behind NAT.
The perceive/act mechanisms that used to motivate this direction (a screen frame, a
synthesized click, the accessibility tree) are **deleted from the core**, not waiting to move.
See [`mechanisms.md`](mechanisms.md).

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

**The connection is the liveness signal**, and the only one: a handle with no live connection
is asleep, not lost. There is no heartbeat and nothing to renew.

Dialing out is what makes this work behind NAT with no configuration: anywhere the core can
already reach the community, it can be reached back.

---

## Content

**The tunnel carries everything today, and it cannot.** The community box is a 4 vCPU CVM
whose public egress measures **1.4 MB/s** — about 11 Mbps, flat over 77 seconds, which is a
shaper and not congestion. Every byte any app ever sees passes through it, so a 4 MB photo is
2.9 seconds on its own and a view holding twelve of them is 34 seconds. Parallelism buys
nothing against a bandwidth gate: twelve streams divide the same 48 MB.

So the two wires split again, by what they carry rather than by who is talking:

| | What | Size | Path |
|---|---|---|---|
| **control** | the conversation, SSE, view JSON, every API call | KB | the tunnel |
| **content** | photos, video, audio, attachments | MB to hundreds of MB | the community's cache, fetched directly |

Compiled views, their pictures and the SPA bundle are content by size and still ride the
tunnel — the pictures because they change in place today, the rest because a signed redirect
cannot carry script — see [*What may be mirrored*](#what-may-be-mirrored-the-response-already-says)
and Open 5.

At 11 Mbps the control plane is free; it was never the problem. All of the problem is that
content shares the pipe with it.

**The property that makes this worth building is not the cache — it is that the transfer can
happen before the demand.** A core mirrors a photo while nobody is looking, over an uplink
that is otherwise idle. A cache in front of the tunnel cannot do this: it accelerates the
*second* fetch of an object, and in a life record the first fetch is the common case, because
what a person opens is what just happened.

### The core is the truth; the cache holds a subset of it

The cache may be emptied at any moment with nothing lost. It is not storage and nothing is
ever only there. Eviction is not a failure mode — it returns an object to the state every
object starts in, and the next request puts it back.

**It is a prefix, `cache/`, not a bucket of its own.** It may share a bucket with things that
must never be emptied — the community's release downloads share one today — because nothing
else about them is shared: a core's write key reaches only `cache/<handle>/`, the edge serves
each by its own path rule, and the one hazard is the lifecycle rule, which must be filtered to
`cache/`.

**That lifecycle rule empties it continually, and that is the eviction a core can see**: the
rule's length comes with the core's credentials and the core stops vouching for an object
before the rule reaches it. **Emptying it by hand is the eviction a core cannot see** — the edge
would answer for objects the core still believes are there. So it is done by first withdrawing
the grants: a refused core stops redirecting, and it re-asks for its keys the first time it is
asked for anything after its hour-long write key runs out, on a read as much as on a write. A
new bucket is a new target the same way, and reads as "not mirrored" without anything cleared.

### What may be mirrored: the response already says

Not a list of directories. A directory list is a thing to maintain, it goes stale as the
agent grows new places to write, and the data dir is mostly files that no off-box client ever
fetches — a live SQLite file, append-only frame logs, a drive edited in place. Mirroring those
would upload gigabytes to accelerate nothing, and the mutable ones would be wrong the instant
they were copied.

The property that makes bytes both safe and worth mirroring is that **the bytes at this path
never change**, and every route that serves such bytes already declares it:

| Route | Declares | Mirrored |
|---|---|---|
| `/api/media/{ref}`, a signal's own blob | `private, max-age=31536000, immutable` | yes |
| `/api/media/{ref}`, the keepsake a faded day left | `private, max-age=31536000` | no — the ref named the original first |
| `/api/attachments/*` | `private, max-age=31536000, immutable` | yes — content-addressed by construction, and uploaded at placement ([showing.md](showing.md#serving)) |
| `/views/_shots/*` | `private, max-age=31536000` | no — a picture is healed in place when an older renderer left the wrong shape, re-rendered after pruning, and `ref/` is re-taken on a clock and told apart only by `?v=`. [showing.md](showing.md) makes a view's picture a derivation keyed by what it pictures |
| `/views/_compiled/*` | `private, max-age=31536000, immutable` | no — script |
| `/assets/*` | `private, max-age=31536000, immutable` | pictures and fonts only |
| `/views/*` (source) | `no-store` | no |
| `/api/drive/file/*` | `no-cache` | no — revalidated, never assumed |
| `/api/people/{subject}/{modality}/{stem}` | `private, max-age=86400` | no — a TTL, not immutability |

`/api/media` does not answer `drive/…` refs: a drive file is edited in place, so no header it
could carry there would be both useful and true, and [showing.md](showing.md#what-this-deletes)
takes those refs off the route.

**So the rule is: a response marked `immutable` may be mirrored, and nothing else may.**

**`immutable` vouches for the path, not the URL**, because the path is what the cache is
keyed on. A route that is re-taken in place and told apart by a query string is immutable
as a URL — a browser can keep each stamped version forever — and must still not say it.
The same goes for a route that answers one path with two sets of bytes over its life, which
is what a faded day does to its refs.

**This is a correctness rule, not a privacy one, and it must not be read as a security
measure** — the drive holds plaintext secrets and is excluded, but that is a consequence, not
the reason. Someone who reads this as a defence will eventually notice that a core already
trusts the community with everything it relays, conclude the rule is redundant, and delete it.
What breaks then is cache coherence: an object wrongly marked immutable goes **permanently
stale**. The only check left behind it is the length the core uploaded (below), which catches
a change of size and nothing else, because there is no ETag anywhere in this server to fall
back on.

The asymmetry is what makes it opt-in. Forgetting to mark a new route `immutable` costs
acceleration and nothing else. Marking a mutable one costs correctness, forever. So the one
judgment a person has to make — *do these bytes ever change?* — is the one the annotation
already asks for.

**Consequently `immutable` is now load-bearing in a way it was not.** It used to govern a
browser cache; it now decides what leaves the machine. A test belongs on it: every path
declaring `immutable` must be content-addressed or timestamp-addressed, and every case on
those same routes that is neither must be shown not to declare it.

**A backstop, not a second rule: the core checks the length it uploaded against the length
it would serve, at every redirect.** The route runs before the redirect is decided (see
below), so the whole object's length is in hand for free — the body's for a `200`,
`Content-Range`'s for a `206`. A mismatch is the one failure this design can have, caught:
the path is logged and never mirrored again, because the edge may still hold the old bytes
under that key and a re-upload would not reach them. The cure is still at the route.

**Two narrower conditions ride on top, and both are about being served from another path,
not about the bytes.**

- **The content type cannot resolve anything against its own URL** — pictures, sound, video,
  fonts, PDF. The redirect changes the path a response is read from and the signature lives in
  its query, so a module's `import "./chunk.js"` or a stylesheet's `url(./font.woff2)` would
  resolve beside it at the edge *without* a signature and be refused. Script, style and markup
  stay on the tunnel however immutable they are. The list is an allow-list, so a type nobody
  thought about fails towards "served from here".
- **The path is plain ASCII** (`[A-Za-z0-9._~/-]`), so the path the edge hashes is, byte for
  byte, the path the core signed. Anything else is served from here.

### The key is the request path

The thing cached is an HTTP response, not a file, so the object key is the URL:

    GET  <core>/api/media/file/2026-09-22/14/03-22.jpg
      →  <bucket>/cache/<handle>/api/media/file/2026-09-22/14/03-22.jpg
      ←  served at <core>/cache/<handle>/api/media/file/2026-09-22/14/03-22.jpg

**And the key is also where the edge serves it, on the core's own origin.** The edge already
fronts every `<handle>.hi-agent.xyz`; one rule has it answer `/cache/*` from the bucket and
pass everything else to the relay. The URL path at the edge *is* the object's key, so the rule
rewrites nothing. The handle is in the path as well as the host because the key needs it, a
rule that built keys from hostnames would be a translation to keep in agreement, and it is
what a per-handle signature will check against the host (Open 1).

The redirect target is derived mechanically from the request; neither side holds a translation
table. **The disk layout stops mattering**, which is the point — a core may write wherever it
likes and no mapping has to be kept in agreement with it. Routes that compute a response from
several files come along for free.

### One endpoint, two branches

Nothing in a view changes. The URL a view emits is the core's own path, as it is today, so
there is nothing to rebase and no repeat of the prefix class of bug — `<img src>`, CSS
`url()`, `<a href>` and `new Audio()` all keep working because none of them were ever touched.

    GET /api/media/{ref}          ← unchanged, relayed, carries the cookie
      → cookie invalid            → 401, as today
      → the route answers         → as today; then, if it is immutable and mirrorable:
          → object is in the cache    → drop the body,
                                        302 /cache/<handle>/<path>?auth_key=…,
                                        Cache-Control: private, max-age=TTL
          → object is not             → send the bytes, and enqueue the upload

**Only a relayed request is redirected.** The cache exists to take content off the tunnel;
a request that arrived on a public bind or over the home network has no tunnel to relieve,
and a phone on the same Wi-Fi would be sent from a local link to a remote edge. The tunnel
marks what it routes in, and nothing else is ever redirected.

**The route runs first, and that is what makes the redirect safe.** It costs a `stat` and an
open, and it means the core has checked, at the moment of redirecting, that the object still
exists, still calls itself immutable, and is still the length it uploaded. A file deleted here
stops being redirected to at once; one that faded to a keepsake stops claiming immutability
and is forgotten on its next request.

**The core knows what is in the cache from its own record, not by asking the bucket.** One row
per request path — the bucket and prefix it went under, its length, when — in a file of its
own beside `config.db`, as disposable as the bucket it describes: deleting it loses nothing,
and every object goes back to the second branch. A row stops vouching one signature validity
and a day before the bucket's lifecycle rule would expire the object — a redirect issued at the
last moment can still be followed for one validity, and the day covers however the bucket
rounds its expiry — so a redirect never names one that is gone. The lifecycle's length arrives
with the credentials, so the two cannot disagree. A new bucket
or a renamed handle reads as "not mirrored" without anything being cleared.

**The redirect never leaves the core's origin.** It is root-relative, so there is no second
name to register, certify or keep in agreement, no CORS for a font or a picture a view draws
to a canvas, and nothing above the core — a view, a page, an app — ever learns the cache
exists. Invariant 8 holds with one addition: the community answers one path of each core's
origin, `/cache/`, and only with bytes that core put there. So **`/cache/` is reserved on every
core**: it serves nothing there itself, and no view may be shared under that name — it could
never be opened, because the edge answers first.

**The second branch is a degradation, not an error.** An object that has not been mirrored is
exactly as slow as it is today and no slower, so there is no flag day, no migration script and
no broken image. Every object crosses over independently, and a self-hosted core with no
bucket configured simply takes that branch forever — the same code, correct in both
deployments.

The redirect is browser-cacheable, so its round trip is paid once per object per device rather
than once per render. **One path signs to one URL for hours at a time** — the signing time is
rounded down to a quarter of the signature's validity — because the browser keys the object
itself under the whole signed URL, and a fresh signature per redirect would be a fresh
download of something it already has. The redirect lives half the validity, so one replayed
from the browser cache at the end of its life still names a URL with a quarter left to run.

### Uploading: a queue, with that second branch as its backstop

**The trigger is the core writing the file**, not a request for it. Writing enqueues; a
background worker uploads at low priority, when the uplink is otherwise idle. Those bytes have
to cross the uplink either way, and the only question is whether someone is watching a screen
while they do.

**What is written because someone will look at it is enqueued as it is written** — a file a
person handed over, and an attachment at placement ([showing.md](showing.md#serving)). **What is written because the core perceived it waits
for its first look** — a camera still, a mic clip. Mirroring ahead is a bet that someone will
look, paid in uplink; a thing handed over is looked at, usually from the device that handed it
over, and a perception frame mostly never is.

The uploader fetches each object through the core's own router, as a loopback request, and
puts exactly that response in the bucket — `Content-Type` and `Cache-Control` included. There
is no second way of finding a file on disk to keep in agreement with the first, which is the
key-is-the-request-path decision paying out again.

The lazy branch is the safety net, not the mechanism: it catches an object that predates the
feature, or whose upload failed, or that was asked for while the queue was still behind. It
also means **the backlog is never migrated** — only writes from here on are enqueued, and
everything older crosses over the first time someone looks at it. A one-time upload of the
whole history would be the worst possible first day.

An upload that fails leaves the object unmirrored, which is the second branch, which is
today's behaviour. Nothing needs to be rolled back.

Upload is multipart, for resumability on a home connection. It goes directly from the core to
the bucket and **not through the tunnel**, so the relay's egress is not on this path and
neither is whatever periodically cuts that connection.

### Two credentials, in opposite directions

| | Purpose | Held by | Scope | Lifetime |
|---|---|---|---|---|
| **write** | upload to the bucket | the core | `<handle>/*` | ~1 hour, refreshed |
| **read** | sign the URL the redirect points at | the core | the whole domain | long-lived |

Both arrive in one answer to `POST <community>/api/cache/credential {handle}`, presented with
the account's token for a handle the account owns — the same check the tunnel makes, because
the prefix a core may write is the name it answers to. With them come the bucket, the prefix
(`cache/<handle>/`), the signature's parameter and validity, and the bucket's lifecycle. A core asks only when it has
something to upload, or once its write key has run out and something is asked for — never on
a clock — and keeps nothing of it on disk. **A refusal
withdraws both halves**: the core stops redirecting, not only uploading, so declining to mint
is a revocation in fact and not just in name.

The write credential is minted by the broker against the core's existing account token, scoped
by policy to that handle's prefix. No long-lived storage key is ever on a person's machine, a
compromised core can write only its own prefix, and revocation is the broker declining to mint.
**This does not weaken invariant 3** — that invariant governs who may reach a core, while this
governs who may write to the community's own storage, and the broker already mints scoped
credentials for a core in exactly this shape.

**The read credential is a named loan.** One signing secret per domain means every core holds
a key that can sign a URL for any path, and the key layout is public. That is acceptable while
one person is the only person and unacceptable the moment a second one exists. **What takes it
back: per-handle keys validated by an edge function**, which needs no callback and so costs no
latency. The loan expires on the second user, not on a date.

### Decisions

| Decision | Reasoning |
|---|---|
| **Content leaves the tunnel; control stays in it** | The relay is an 11 Mbps box. A small machine can carry a conversation for anyone; it cannot carry everyone's photographs |
| **Mirror ahead of demand, not on demand** | A cache in front of the tunnel only speeds up the second fetch, and the first fetch is the common case. The uplink cost is identical; only whether someone is waiting differs |
| **`immutable` is the rule, not a directory list** | A list is maintained by hand, goes stale as the agent grows new places to write, and fails towards including something it should not. The annotation already asserts exactly the property a cache needs |
| **The key is the request path, not the disk path** | What is cached is a response. Deriving the key from the URL means no translation table, and a core may write wherever it likes |
| **The view's URL does not change** | The redirect keeps authorization at the core and the signature invisible to everything above it. A view that had to know about a second origin is the prefix bug rebuilt |
| **The cache answers on the core's own origin, at `/cache/`** | The edge already fronts every handle's hostname, so one path rule is the whole of it: no second domain, certificate or DNS name, no CORS, and a root-relative redirect. The key is the path at the edge, so the rule rewrites nothing |
| **The unmirrored branch is today's behaviour** | Migration with no flag day, and a failure shape of "this one is as slow as last week" rather than a broken image |
| **Only a relayed request is redirected** | The tunnel is the bottleneck this exists for. A LAN or public-bind client has none, and would be sent from a local link to a remote edge |
| **The route runs before the redirect is decided** | A `stat` buys a redirect that is never to a deleted, faded or changed object — and the length check that backs up `immutable` |
| **Script and style are never redirected** | The signature is in the query and relative references do not carry it. An allow-list of types that resolve nothing, so a new type fails towards the tunnel |
| **Enqueue at write what will be looked at; wait for the first look for what was perceived** | Uploading ahead is a bet paid in uplink. A handed-over file is looked at; a camera frame mostly is not |

### Open

1. **Per-handle read keys via an edge function** — what repays the loan above. On the core's
   own origin it can also refuse a `/cache/<handle>/` path whose handle is not the host's.
2. **A size floor for mirroring.** Below some size the redirect's round trip costs more than
   the bytes it saves. That number has to be measured, not chosen, and the rule must skip
   *small* objects: a rule that skips large ones would silently exclude exactly the files this
   whole section exists for.
3. **The drive is not mirrored**, because it is mutable in place. What used to make that hurt
   was that generated images and video landed there; they are attachments now
   ([showing.md](showing.md)), which are immutable, mirrored at placement, and no longer in
   this sentence. What is left in the drive is what an agent filed by hand. It *revalidates* rather
   than refetching — `no-cache` plus the file service's `Last-Modified`, so an unchanged
   picture costs a `304` — which closes the part of this that hurt most. What is still open is
   the validator: `Last-Modified` cannot tell two writes inside one second apart, and an
   `ETag` exists nowhere in this server to fall back on.
4. **Deleting must reach the cache.** A core that forgets something has not forgotten it while
   a mirrored copy answers. Half of this is closed by running the route before redirecting: a
   deleted or faded object is never redirected to again, so a copy answers only to a signature
   already issued — at most one validity period — and is stored until the lifecycle expires it.
   What is open is the storage half: whether forgetting should also delete the object and purge
   the edge, or whether the lifecycle bounding it is enough. Immutability makes a stale copy
   *correct*, which is precisely why this one needs saying: nothing else in the design will
   notice.
5. **Whether `/assets/*` needs a signature at all.** It is byte-identical for every core and
   is not anyone's personal data, so it could be one shared public copy — a different question
   from the rest of this section, and the one with the largest effect on the cold path. **The
   signed path cannot carry it at all**, which narrows the question: the bundle is script and
   style whose chunks import one another by relative path, and a relative import from a URL
   signed in its query arrives at the edge unsigned. So it is a public, path-addressed copy
   uploaded once per release, or it stays on the tunnel. Compiled views are held off by the
   type allow-list rather than by that constraint: each is a single module whose imports
   resolve through the page's import map, on the same origin, so nothing but the allow-list
   stops one being mirrored — whether script known to import nothing relatively may pass is
   the open half.

**One decision elsewhere inverts because of this.** `VIEW_PRELOAD_SPECIFIERS` excludes
`motion/react` deliberately, reasoning that preloading 183 kB would "trade a round trip for
bytes on a connection where bytes are the scarcer thing". That is correct at 11 Mbps. Once
`/assets/*` is served at the edge's bandwidth, round trips become the scarcer thing again and
the exclusion should reverse. It is a sound decision whose premise this section removes — not
an oversight, and not to be changed until the premise actually goes.

---

## Identity

| | Where | Means |
|---|---|---|
| **handle** | the registry, owned by an account | the address |
| **credential** | issued by a core, held by an app | this app may reach that core |

**Naming is three layers**, and collapsing them is the classic mistake:

| | Mutable | Unique | For |
|---|---|---|---|
| account id | never | yes | who owns the name. Never reused |
| handle | renameable | yes | the address |
| display name | freely | no | what it calls itself |

**A handle belongs to an account, and it is permanent.** Not a lease: an address that has to
be kept alive is one a person can lose by going quiet, and every link and QR they ever handed
out then points at a stranger. Nothing expires, and nothing has to be renewed.

That is also what makes replacing a machine survivable. A handle bound to a *core* would be
lost with the laptop it was minted on — an ordinary event, and a worse outcome than the
squatting a lease was guarding against. Bound to an account, a new install claims the same
name back by signing in.

**Squatting is bounded by the account instead**, which is where the cost naturally sits: a
small number of handles per account, and an account you have to be able to sign back into.
The account may be free and must be — see the decision above — but it cannot be anonymous,
because permanence you cannot recover is not permanence.

**An account owns names; a machine serves one of them, and records which.** The registry
answers with every name the account holds and cannot know which machine answers to which, so
the core keeps that choice itself — written when it claims a name, and read back at every
start. Renaming is then what the word means: the core dials the new name at once and still
dials it after a restart, while the old name stays claimed and permanent, simply unserved.
Picking off the registry's list instead — the oldest name, the newest, any of them — makes a
rename a thing that undoes itself the next time the machine boots.

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

**Claiming a handle.** The core presents its account and asks; the registry records the name
against that account and it is theirs from then on. Rename is the same call, and the account
underneath never changes. A core with no account has no handle and works fine without one —
it is simply reachable from its own machine only.

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
2. **The community may require you to be someone; never to be a customer.** Claiming a handle
   needs an account, and no path from claiming reaches a tier, a balance or a payment.
3. **The core is the sole authority on who may reach it.** The community never issues,
   checks or holds access. *Stated plainly: in the relayed shape it is trusted not to
   replay what it forwards, because a bearer token is a bearer token.* The cache validating
   a signature a core minted is enforcement, not a decision — it holds no ACL and can grant
   nothing a core did not sign.
4. **The core never learns of other cores.** The roster is app state and stays there.
5. **One body per person.** One handle, one live core.
6. **Off-box trust is structural** — decided by which listener accepted the request, never
   by a header and never by an address.
7. **Host and client are capabilities of an app instance**, never properties of a platform.
8. **A core's origin is its own.** One handle, one origin, root-relative throughout; no core
   emits a prefixed path and no cookie sets `Domain=`. The failure behind it is a browser
   holding two people at once, where the only thing between them is a cookie `Path` — which
   is a delivery rule and was never a boundary. The one path the community answers on it,
   `/cache/`, holds only bytes that core mirrored, and only pictures, sound, video, fonts
   and documents — never anything that runs.

---

## See also

[`arch.md`](arch.md) for the layers inside a core ·
[`sharing.md`](sharing.md) for the one thing a non-owner may be given, and why it is a page ·
[`surfaces.md`](surfaces.md) for how the world reaches it once a wire exists ·
[`text-transcript.md`](text-transcript.md) for why reconnection needs nothing ·
[`foundation.md`](foundation.md) for the mechanism/policy split an app inherits
