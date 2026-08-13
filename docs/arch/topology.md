# Topology — core, app, community

**Status:** proposed August 11, 2026; **most of it built, deployed and live-verified against
the real community by August 13** —
see [Status](#status) at the end for what is real, what is written but unwitnessed, and what
has no code. Defines the split of the system into three roles, how a person is addressed, and
how an app proves it may reach one. Nothing here changes what the agent *is*; it says where
the parts of it run and how they find each other.

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
| **A handle is owned by an account, permanently** | An address is only worth handing out if it survives a new laptop and a quiet month. Permanence needs an owner that outlives any one machine, and the only such thing is an account. A lease would put the burden on the person to keep proving they still want their own name |
| Claiming may require an account; it must never require a **paid** one | A BYOK install pays us nothing and must still get a name. "Free" is the load-bearing word — sign-up is a cost the person pays once, billing is a cost that would make the address a product |

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
| **registry** | handle ↔ account; claim, rename | the namespace |
| **relay** | routes an inbound request by handle into that core's live connection | the routing table |
| **broker** | provider role: LLM credentials and energy (`foundation/broker.rs` today) | accounts, billing |
| **post** | push to a surface, on a core's instruction; later, mail for a sleeping core | push tokens |

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

### Nothing behind the gate is `public`

A gated `200` was served *because* a credential checked out, so labelling it `public` invites
any shared cache to keep it and hand it to the next caller — who was never asked for one. The
gate is then intact and bypassed at the same time, and the core never sees the request that
walked around it.

This is not a hypothetical about some future CDN. **Relayed, there is always one**: the
community terminates TLS, and hi-agent.xyz sits behind an edge besides. Measured against the
real one, an authorized fetch of `/<handle>/assets/*` turned a later *unauthenticated* fetch
of the same path from the core's `401` into a `200` served from the edge.

So every cacheable response the gate protects is **`private`** — the browser cache, which is
all it was ever for, without the shared one. Content-addressed names are why a module may be
cached *forever*; they were never a reason to cache it *shared*, and the two properties read
so alike that this is worth stating rather than assuming.

**The fix belongs at the core, not the relay.** The community forwards bytes unchanged and
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

**The connection is the liveness signal**, and the only one: a handle with no live connection
is asleep, not lost. There is no heartbeat and nothing to renew.

Dialing out is what makes this work behind NAT with no configuration: anywhere the core can
already reach the community, it can be reached back.

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
   replay what it forwards, because a bearer token is a bearer token.*
4. **The core never learns of other cores.** The roster is app state and stays there.
5. **One body per person.** One handle, one live core.
6. **Off-box trust is structural** — decided by which listener accepted the request, never
   by a header and never by an address.
7. **Host and client are capabilities of an app instance**, never properties of a platform.

---

## Status

The rest of this document is the goal state and is written in the present tense throughout,
as design here always is. This section is the exception: it says what is actually true of the
code, as of **August 13, 2026**.

### Real, and watched working

Verified against running processes — a real core, a real community, real sockets — not by
reading the code.

| | What was seen |
|---|---|
| **local** | unchanged; always worked |
| **directly public** | a second, gated acceptor. Loopback `200` and off-box `401` on the same route, the pairing page on an HTML navigation, `/healthz` open |
| **relayed** | `hi-agent.xyz/ana` reaching a core that only ever dialed out — text in, the conversation streamed back, a WebSocket audio stream through the tunnel, and the asleep page when the core stops |
| **the gate** | one credential in two presentations, the CSRF rule, all three pairing paths, the device list, and revocation taking a working credential to `401` |
| **the app** | a roster and a local proxy; two cores at once, switching between them, the face never holding a credential |
| **a name** | claimed by a signed-in account, permanent, refused to a second account and to an anonymous one, and surviving a wiped data directory |
| **the subpath** | the page at `/ana/` emitting `/ana/assets/*` and an import map to match, all of it serving through the tunnel |
| **`_builtin/reach`** | the one screen for all of the above — name, address, devices, add, revoke — rendered in a real browser |
| **deployed** | the community running on the real box, behind the real CDN: a name claimed against the production registry, a tunnel dialled from a Mac to `wss://hi-agent.xyz`, and the whole conversation driven from a third machine that had only the address |

**What the edge changed, and nothing on a laptop could have shown.** Relayed, the community
is not the only thing in the path — hi-agent.xyz terminates TLS at a CDN. Three things were
open until it was deployed, and all three are now measured rather than assumed: the `Upgrade`
passes (yamux came up first try), SSE is **not** buffered (the `reset` frame on connect, then
each `append` as it happened), and a gated response is no longer shared-cacheable — which it
was, and which is the one real defect the deploy found. See
[Nothing behind the gate is `public`](#nothing-behind-the-gate-is-public).

### Written, not witnessed

Each of these is built and green, and nothing has watched it do its job.

- **The relayed page has never been opened in a browser.** The bytes are right; React
  mounting, view import-map resolution and SSE reconnect under a prefix are inferred.
- **A genuinely new machine keeping its name** is covered by a test, not by a live run — the
  device id is machine-derived, so two data directories on one Mac share an account.
- **The Docker shape's gate.** A published port is off-box, so an existing deployment is
  gated from first run; that path has been reasoned about, not exercised.

### No code at all

- **post** — the push service, and with it "waking a surface". The app hands its token to the
  core and the core instructs post: none of that exists.
- **Refusing to route for a surface reported lost** — the one revocation case a sleeping core
  cannot serve.
- **Mail for a sleeping core**, and therefore core-to-core addressing. Nothing is queued; an
  inbound request is answered "asleep".
- **The roster on a screen.** Switching between cores is an API call with no surface, and the
  surface belongs to the app rather than to any core.
- **Credentials in the OS keychain.** An app keeps them in its config store today.
- **A core on iOS.** Blocked by the wire being a spawned binary, not by effort. Everything
  above already treats hosting as a capability of an app instance rather than a property of a
  platform, so the day that changes, nothing structural does.

## See also

[`arch.md`](arch.md) for the layers inside a core ·
[`surfaces.md`](surfaces.md) for how the world reaches it once a wire exists ·
[`text-transcript.md`](text-transcript.md) for why reconnection needs nothing ·
[`foundation.md`](foundation.md) for the mechanism/policy split an app inherits
