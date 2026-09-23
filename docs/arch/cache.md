# Cache — a core's pictures served from the edge, at the core's own paths

A core mirrors its immutable bytes — photos, video, audio, attachments — into a bucket. When
an app reaches that core by its name, the edge in front of `<handle>.hi-agent.xyz` answers
those paths from the bucket instead of sending the bytes down the tunnel. Nothing about the
URL changes, and the core still decides who may read.

This is the goal state, in the present tense, as design here always is. Where the tunnel and
the community sit is [topology.md](topology.md); what an attachment is, [showing.md](showing.md).

## Goal

**The relay is an 11 Mbps box, measured.** Its public egress is 1.4 MB/s, flat over 77
seconds — a shaper, not congestion. A conversation fits through it; a photograph does not. A
4 MB photo is 2.9 seconds on its own, a view of twelve is 34, and parallelism buys nothing
against a bandwidth gate. So content leaves the tunnel, under three constraints:

1. **One URL everywhere.** A picture is `/api/media/…` on `http://127.0.0.1:12358`, on
   `https://agent.example.com`, and on `https://ana.hi-agent.xyz`. No page, view or app
   knows a cache exists, and no URL carries a second host, a prefix or a signature.
2. **Acceleration where the community is in the path, and only there.** A core reached on
   loopback, over the home network or at its own domain is served by the core, exactly as
   without any of this — the community is not there to help, and nothing assumes it is.
3. **Basic protection, not more.** Only the owner's signed-in devices can read a mirrored
   object; no long-lived storage key is ever on a person's machine; no key a person can see
   opens anyone else's bytes.

## The shape

```
GET https://ana.hi-agent.xyz/api/media/file/2026-09-22/14/03-22.jpg
    Cookie: hi_surface=<id>.<exp>.<sig>                 ← the one session cookie, as today
  │
  └─ the edge function (in front of /api/media/* and /api/attachments/*)
       ├─ signature valid for "ana", not expired, and the object is in the bucket
       │     → the bytes, from cos://…/cache/ana/api/media/file/2026-09-22/14/03-22.jpg
       └─ anything else
             → on to the relay → the core, as today
                   the core checks the session, serves the bytes,
                   and puts the object up if the edge should have had it
```

**The edge never refuses.** A missing, expired or foreign signature, or an object not up
there yet, sends the request on unchanged, and whatever answers is the core's. The worst the
edge can do is be as slow as last week; a `401` only ever comes from the core.

**The edge only fronts paths where a copy can exist**: `/api/media/*` and `/api/attachments/*`.
Everything else on the hostname — the page, the API, the SSE streams — goes to the relay
untouched, so a miss costs a bucket lookup only where one could have hit.

**A core the edge is not in front of is simply the second branch forever**: same code, same
URLs, correct in every deployment. A self-hosted core never mirrors anything, because nothing
grants it a bucket.

## The credential: one cookie

**`hi_surface` is the credential for `ana.hi-agent.xyz`, and the only one.** It keeps its
name, its attributes (`HttpOnly; Secure; SameSite=Lax; Path=/`, host-only) and its meaning.
What changes is that it carries its own proof, so the edge can check it without the core:

    hi_surface = <id>.<exp>.<hex hmac-sha256(K_ana, "<id>.<exp>")>

- **The core checks `<id>`, as it checks the whole token today**: a row in its session table,
  looked up by the hash of the id. The signature means nothing to the core. Revoking a device
  removes the row, and the core refuses it at once.
- **The edge checks the signature and the expiry**, with the key for the handle its hostname
  names. It has no session table and needs none.

**Renewal moves `<exp>`, so the token changes — and that breaks nothing.** The core renews a
session once it is a day into its life, as it does now. The old token stays valid at both ends
— the core looks up the same `<id>`, the edge checks an old `<exp>` that has not passed — so
requests already in flight with it all succeed, and whichever `Set-Cookie` a browser sees
last is fine. That was the only reason the token did not rotate before.

**A token without a signature is still a session.** A token with no dots is all `<id>` — which
is every session from before this, and every session on a core that holds no key. The core
accepts it as it always did; the edge passes it on. Whenever a request's token is not signed
under the core's current key and the core has one, the response carries a signed token for the
same session. So an old browser, a key that arrived after login, or a key that rotated heals on
its next request, and nobody signs in again.

**No second cookie, no second meaning.** A visitor of a shared view holds no session, so holds
no signature, and is served by the core as before. The edge does not know shares exist.

## Keys

| Key | Held by | Does | Reaches |
|---|---|---|---|
| **master** | the broker, the edge function (as a secret) | derives every handle's key | never leaves the community |
| **`K_handle` = HMAC-SHA256(master, handle)** | that handle's core | signs its session cookies | reads of its own `cache/<handle>/` at the edge |
| **COS read key** | the edge function (as a secret) | reads the bucket | `cache/*`, read-only |
| **STS write credential** | the core, ~1 hour, refreshed | puts objects | `cache/<handle>/*`, write-only |

**A person can see only their own keys, and their own keys open only their own bytes.** A core
runs on its owner's machine, so its owner can read `K_handle` and the STS credential out of
it — and with them sign cookies for, and write objects under, the handle they already own. No
key on any person's machine opens another handle; the edge derives `K_handle` from the host,
so a cookie signed for `bob` is checked with `bob`'s key and fails on `ana.hi-agent.xyz`.

**The community could forge an edge-valid cookie, and it gains nothing by it.** It holds the
master, so it can sign for any handle — which reads only the bucket it already hosts. The core
never accepts a cookie on its signature, so no forged cookie reaches a core: **invariant 3
holds** — the edge checking a signature a core's key made is enforcement, not a decision.

Both halves of a core's keys arrive in one answer to `POST <community>/api/cache/credential
{handle}`, presented with the account's token for a handle the account owns — the check the
tunnel makes. With them come the bucket, the prefix and the fronted paths. A core asks when it
has something to upload or no key in hand, never on a clock, and keeps nothing of it on disk.
**A refusal withdraws both**: the core stops signing and stops uploading.

## What may be mirrored

The property that makes bytes safe to copy is that **the bytes at this path never change**,
and every route that serves such bytes already says so:

| Route | Declares | Mirrored |
|---|---|---|
| `/api/media/{ref}`, a signal's own blob | `private, max-age=31536000, immutable` | yes |
| `/api/media/{ref}`, the keepsake a faded day left | `private, max-age=31536000` | no — the ref named the original first |
| `/api/attachments/*` | `private, max-age=31536000, immutable` | yes — content-addressed, uploaded at placement ([showing.md](showing.md#serving)) |
| `/views/_shots/*` | `private, max-age=31536000` | no — re-taken in place |
| `/views/_compiled/*` | `private, max-age=31536000, immutable` | not fronted (Open 4) |
| `/assets/*` | `private, max-age=31536000, immutable` | not fronted (Open 4) |
| `/views/*` (source), `/api/drive/file/*` | `no-store`, `no-cache` | no |

**So: a response under a fronted path, marked `immutable`, at a plain-ASCII path, is copied;
nothing else is.** Not a list of directories — the one judgment a person has to make, *do these
bytes ever change?*, is the one the header already asks, and a route that forgets to say it
costs acceleration and nothing else.

**`immutable` vouches for the path, not the URL**, because the path is the key. A route
re-taken in place and told apart by a query string must not say it, and neither may a route
that answers one path with two sets of bytes over its life. A test pins every such case.

**The key is the request path**, under the handle: `cache/<handle>/<request path>`. The edge
builds it from the host and the path it was asked for; the core from the path it served. No
translation table, and the disk layout stops mattering. Plain ASCII, so the two agree byte for
byte.

## Uploading

**The trigger is the core writing the file, not someone asking for it.** A file a person hands
over, and an attachment at placement, are queued as they are written: a thing handed over is
looked at, usually from the device that handed it over, so its bytes should be at the edge
before then. Something the core perceived — a camera still, a mic clip — waits for its first
look.

**A miss is the backstop.** A request that reaches the core carrying a token signed with this
core's own key, for an immutable fronted path, is one the edge looked for and did not find — so
the core puts it up. That catches whatever predates the feature, failed to upload, or expired
from the bucket, and it means the backlog is never migrated: everything older crosses over the
first time someone looks at it. The core remembers what it uploaded recently, so a missing edge
rule does not become an upload per look.

**The uploader fetches each object through the core's own router**, as a loopback request, and
puts exactly that response in the bucket, `Content-Type` and `Cache-Control` included. It goes
straight from the core to the bucket — never through the tunnel — one object at a time, in parts
above 8 MB so a dropped connection loses a part and not a video. A failed upload is a miss,
which is today's behaviour.

## The bucket

**The cache is a prefix, `cache/`, not a bucket of its own.** It shares one with the release
downloads, and nothing else about them is shared: a core writes only `cache/<handle>/`, the
edge reads only `cache/`, and the lifecycle rule — the one setting that could hurt `downloads/`
— is filtered to `cache/`. Objects expire 30 days after upload.

**Nothing is ever only there.** Emptying `cache/` loses nothing: every object misses, is served
by its core, and goes back up.

## What this accepts

- **Revocation reaches the edge when the cookie runs out.** A revoked device is refused by the
  core at once, and can still read mirrored objects at the edge until its `<exp>` — up to a
  session's life, 30 days.
- **Forgetting does not reach the bucket.** A faded day or a deleted object stops being served
  by the core at once; a copy already in the bucket can still be read at the edge, by the
  owner's own signed-in devices, until the lifecycle drops it.
- **A core that is asleep still has its mirrored pictures served.** The edge does not ask the
  core, so a phone keeps seeing what is up there while the laptop is closed.

## Decisions

| Decision | Reasoning |
|---|---|
| **Content leaves the tunnel; control stays in it** | A small machine can carry anyone's conversation and nobody's photographs |
| **The URL never changes** | A redirect or a second host is the prefix bug rebuilt — every relative reference, every CORS rule, every stored URL. Local, self-hosted and relayed are one set of URLs |
| **One cookie, carrying its own proof** | The edge cannot reach the core's session table. A signed session is the same credential in a form both can check; a second cookie would be a second meaning |
| **The core checks the id; the edge checks the signature** | The core stays the authority — revocation is immediate there — and the edge needs neither a database nor a round trip |
| **A key per handle, derived from a master** | A core's key is visible to its owner. Derived per handle, it opens only what its owner already owns, and the edge derives it from the host with nothing stored per handle |
| **The edge never refuses** | It can only ever be slower, never wrong about access |
| **Mirror ahead of demand** | A cache in front of the tunnel speeds up only the second fetch, and in a life record the first is the common case |
| **`immutable` decides, under the paths the edge fronts** | The annotation already asserts what a copy needs; the fronted paths are where a copy could ever be served |
| **Write credentials are STS, per handle, an hour** | No long-lived storage key on a person's machine, and declining to mint is revocation |

## Open

1. **Edge caching.** The function reads the bucket on every request it answers; the browser's
   own cache absorbs a second look on one device. Caching at the edge node would serve a second
   device without the bucket.
2. **Forgetting that reaches the bucket.** A delete and a fade could remove their copies too,
   closing the 30 days above.
3. **A size floor.** Below some size the bucket's round trip costs more than the tunnel's bytes.
   It has to be measured, and the rule must skip *small* objects, never large ones.
4. **The bundle and compiled views.** `/assets/*` is identical for every core and nobody's
   personal data, so one public copy per release — not a copy per core — is the better shape,
   and it has the largest effect on the cold path. Compiled views would be one more fronted
   path. Once `/assets/*` is served at the edge, `VIEW_PRELOAD_SPECIFIERS`' exclusion of
   `motion/react` — correct at 11 Mbps — should reverse.
5. **The drive.** It is mutable in place, so it is not mirrored — and generated images and video
   land there, so they are the largest objects this leaves on the tunnel.
