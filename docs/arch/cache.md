# Cache — a core's pictures served from the edge, at the core's own paths

A core mirrors its bytes — photos, video, audio, attachments, and the files its views and its
drive show — into a bucket. When an app reaches that core by its name, the edge in front of
`<handle>.hi-agent.xyz` answers those paths from the bucket instead of sending the bytes down
the tunnel. Nothing about the URL changes, and the core still decides who may read and whether
a copy is current.

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
  └─ the edge function
       ├─ no valid signature for "ana"      → on to the relay → the core, as today
       │
       ├─ /api/media/*, /api/attachments/*  — bytes that never change
       │     object in the bucket  → the bytes, from cos://…/cache/ana/api/media/file/…/03-22.jpg
       │     not there             → on to the core, which serves it and puts it up
       │
       └─ /views/*, /api/drive/file/*       — bytes that change in place
             ask the core first, headers only (x-hi-cache: ask)
             304 + x-hi-cache: bucket  → the bytes, from the bucket
             anything else             → that answer, as the core gave it
```

**The edge never refuses.** A missing, expired or foreign signature, or an object not up
there yet, sends the request on unchanged, and whatever answers is the core's. The worst the
edge can do is be as slow as last week; a `401` only ever comes from the core.

**The edge only fronts paths where a copy can exist**: `/api/media/*`, `/api/attachments/*`,
`/views/*` and `/api/drive/file/*`. Everything else on the hostname — the page, the API, the
SSE streams — goes to the relay untouched.

## Two ways a copy is trusted

**Bytes that never change are served on the edge's word alone.** A response marked
`immutable` names bytes that will be at that path for as long as the path exists, so a copy is
correct forever and the edge serves it without asking anyone — which is also why these keep
being served while the core is asleep.

**Bytes that change in place are served on the core's word, each time.** A view's clip, a
picture a build worker downloaded, a video in the drive: an agent rewrites these under the same
name, with a shell, not through any route the core sees, so the core cannot know when a copy
went stale — and a copy nobody can invalidate is wrong for up to its whole life. So for these
paths the edge **asks the core first**, with the browser's own request plus `x-hi-cache: ask`,
and the core answers the one question only it can:

- the route answers as it always does — a `304` to the browser's own `If-None-Match`, a `401`,
  a `404` — and **that answer goes back unchanged**;
- the bytes it would send carry an `ETag`, and the copy it last put in the bucket carried the
  same one → **`304` with `x-hi-cache: bucket`** and the `ETag`, and no body: the edge reads the
  bytes from the bucket and hands them over under that `ETag`;
- otherwise the core sends the bytes, as today, and puts the current version up behind them.

The round trip is headers only, so what crosses the tunnel is a few hundred bytes where it was
the file. **A stale copy cannot be served**, because the core compares versions on every
request; **revocation is immediate** on these paths, because the core checks the session on
every request; and **the core's `ETag` is the one the browser keeps**, so a second look on one
device is a plain `304` from the core and no bytes from anywhere.

If the bucket turns out not to hold what the core vouched for — the lifecycle ran early, or
somebody emptied `cache/` — the edge asks again with `x-hi-cache: missing`; the core forgets
its record of the object, serves the bytes and puts them back up.

**The `ETag` is the file's length and modification time**, weak (`W/"…"`) because a compressed
and an uncompressed response share it. Every file route serves one and honours
`If-None-Match`.

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
| **the community's COS key** | the broker, the edge function | mints STS credentials; reads the bucket | the whole bucket — never on a person's machine |
| **STS write credential** | the core, ~1 hour, refreshed | puts objects | `cache/<handle>/*`, write-only |

**The edge reads with the community's own COS key, not a read-only key of its own.** Both
places it lives — the broker's box and the edge function — are inside the community's cloud
account, and whoever can read a secret out of one can reach the other; a second, narrower key
would guard nothing an attacker in either place does not already have.

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

| Route | Declares | Mirrored |
|---|---|---|
| `/api/media/{ref}`, a signal's own blob | `private, max-age=31536000, immutable` | yes, trusted by the edge |
| `/api/media/{ref}`, the keepsake a faded day left | `private, max-age=31536000` | no — the ref named the original first |
| `/api/attachments/*` | `private, max-age=31536000, immutable` | yes, trusted by the edge — content-addressed, uploaded at placement ([showing.md](showing.md#serving)) |
| `/views/*`, a view's own files | `no-cache` + `ETag` | yes, checked with the core — from 256 KiB |
| `/views/_shots/*` | `private, max-age=31536000` + `ETag` | yes, checked with the core — from 256 KiB |
| `/views/_compiled/*` | `private, max-age=31536000, immutable` | no — tens of kilobytes, under the floor |
| `/api/drive/file/*` | `no-cache` + `ETag` | yes, checked with the core — from 256 KiB |
| `/assets/*` | `private, max-age=31536000, immutable` | not fronted (Open 4) |

**Under the paths the edge trusts, a response marked `immutable`, at a plain-ASCII path, is
copied; nothing else is.** Not a list of directories — the one judgment a person has to make,
*do these bytes ever change?*, is the one the header already asks, and a route that forgets to
say it costs acceleration and nothing else. **`immutable` vouches for the path, not the URL**,
because the path is the key: a route re-taken in place and told apart by a query string must
not say it.

**Under the paths the edge checks, a response with an `ETag`, of at least 256 KiB, at a
plain-ASCII path, is copied.** Nothing about it has to be true forever, because nothing trusts
it past the next request. The floor is there because each of these costs a round trip to the
core *and* a bucket read: below it, the tunnel's own bytes are cheaper (Open 3). A drive file
whose name is not ASCII is not copied at all, which is most of what a Chinese-speaking owner
keeps there; that is Open 5.

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
core's own key, for an immutable path the edge trusts, is one the edge looked for and did not
find — so the core puts it up. That catches whatever predates the feature, failed to upload, or expired
from the bucket, and it means the backlog is never migrated: everything older crosses over the
first time someone looks at it. The core remembers what it uploaded recently, so a missing edge
rule does not become an upload per look.

**A changing file goes up the first time each version is looked at.** Nothing sees an agent
write it, so there is no earlier moment: the edge's question is the trigger. The first look at a
new version crosses the tunnel as it always did, and every look after that — on any device —
comes from the bucket. The core records each upload's `ETag` beside its path, which is what it
compares when it is asked.

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
- **A core that is asleep still has its mirrored pictures served** — the immutable ones. The
  edge does not ask the core about those, so a phone keeps seeing them while the laptop is
  closed. A view's files and the drive are not: the edge asks the core about every one, and a
  core that cannot answer serves nothing, exactly as without a cache.
- **A changing file's first look per version is as slow as before.** The copy exists only once
  someone has asked for that version.

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
| **`immutable` decides, under the paths the edge trusts** | The annotation already asserts what a copy needs; the fronted paths are where a copy could ever be served |
| **Files that change in place are checked with the core, not trusted** | Agents rewrite them with a shell, so nothing can tell the edge a copy went stale. Asking the core costs a headers-only round trip — hundreds of bytes where the file was — and makes a stale copy impossible. It is where the largest bytes are: a view's clips, the drive's videos |
| **Write credentials are STS, per handle, an hour** | No long-lived storage key on a person's machine, and declining to mint is revocation |

## Open

1. **Edge caching.** The function reads the bucket on every request it answers; the browser's
   own cache absorbs a second look on one device. Caching at the edge node would serve a second
   device without the bucket.
2. **Forgetting that reaches the bucket.** A delete and a fade could remove their copies too,
   closing the 30 days above.
3. **The size floor.** 256 KiB for checked paths is a guess, not a measurement: it has to be set
   by timing a checked hit against the tunnel at a few sizes. Immutable paths have none.
4. **The bundle and compiled views.** `/assets/*` is identical for every core and nobody's
   personal data, so one public copy per release — not a copy per core — is the better shape,
   and it has the largest effect on the cold path. Compiled views would be one more fronted
   path. Once `/assets/*` is served at the edge, `VIEW_PRELOAD_SPECIFIERS`' exclusion of
   `motion/react` — correct at 11 Mbps — should reverse.
5. **Paths that are not ASCII.** The key must be the same bytes at the edge and at the core, so
   only plain ASCII paths are copied, and a drive file named in Chinese is not. Percent-encoding
   the key on both sides the same way would close it.
