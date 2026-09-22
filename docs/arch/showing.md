# Showing

How what the agent made to be looked at reaches the person: the object, the places it is
put, the one way every place draws it, who decides when, and what all of it costs.

This is the goal state, in the present tense, as design here always is. It says nothing about
what is built; the phases at the end are the order it gets built in.

## Goal

**What would change what the person thinks reaches them when it exists, wherever they are
looking, at a cost that does not grow with how much the agent made.**

Three clauses, and each one is a property the current arrangement lacks:

- *When it exists* — not when a view has been built around it.
- *Wherever they are looking* — the task they opened, Home, the screen, the conversation, a
  link they sent someone, on the desktop or the phone.
- *At a cost that does not grow* — one copy of the bytes, one address, one route, one way to
  draw it, and a read path that does no work per picture.

## What opened this

On 2026-09-22 a court-calibration task wrote, at 13:28, *the court lines are drawn back onto
the frame from the model and sit on the painted lines*. The picture that showed it,
`work/figures/pose_899.png`, was already on disk. Home drew its first picture of the task at
14:31, and only because the worker had by then finished a whole 30-second video **and a page
to play it in**. Had that session died the way the task's first one did — a stream
disconnect after one 90-minute turn — nothing would ever have been drawn.

The ball-tracking task beside it is the same failure in the other direction. At 13:54 its line
said *the picture is in `work/single-…`*, a path nobody reading the card can open; at 14:03 a
`waiting` line asked the person to decide how to proceed; at 14:28 they typed into the row's
reply box *拿出来我看看效果先* — "bring it out so I can see how it looks first". The one thing
the row was waiting on was something they could not see.

Both of those asks were made at 12:33 on 09-20, in one breath: *mark it on a new video so I
can see how it looks*, and *calibrate the court, let me see it*. The first was on screen 21
hours later, the second about 42.

It is not two tasks. Measured on the live store (2026-09-22, read-only):

- Of 482 progress lines written since 09-15, **111 speak of a picture, a clip, a frame or how
  something looks**, in 29 of 53 tasks — and only 17 of those tasks ever had a view made for
  them. 26 lines name a media file by path, which nobody reading the card can open.
- The person asked to be shown something about **25 times in 22 days**: 8 *put it up now*, 8
  *make me something to look at*, and **9 *send it to me on Feishu*** — a surface this
  arrangement does not reach at all. Of the 8 *put it up now*, 4 had something on screen
  within ten minutes.
- **The person looks far more than the agent shows.** In September they went to a view 425
  times — `factory/tasks`, `factory/home` and `factory/workers` most of all — against 73 shows
  by the agent. Evidence that exists only where it is pushed misses where they actually look.
- Eight views are nothing but a frame around one medium, three of them for tasks and all three
  since 09-20, written by general workers as `fetch → blob → <video>` because `/views/*` serves
  mp4 as `text/plain`. Beside the views sit **4,672 pictures and clips, 1.37 GB** (measured
  2026-09-23). Only 1,543 of them (170 MB) are byte-identical to a file elsewhere in the data
  dir, and 1,448 of *those* are browser-profile caches workers created inside the views tree;
  the other 3,129 were made or downloaded there, so the cost is not duplicate bytes but
  results with no address of their own, each reachable only through the one page built
  around it.

**The cause is structural, not a lapse.** From a file to anything the person can see there is
exactly one road — build a view, render it with `hi_review_view`, which writes `made` on the
task, which Home joins to the view's shot. That road is priced for a deliverable: a React
module written, compiled, rendered in headless Chromium and reviewed by a model. Over 3,482
review calls in the frame logs, a view builder's run took **16 minutes at the median and 153 at
p90**, rendered its view **8 times at the median** (one ref 265 times), and spent **4.5 M
tokens** a thread at the median, 95% of it cached. A picture that already exists pays it
too, so workers rationally leave it to the end — the calibration worker's own plan put the
view fifth of eight steps. And the one rule that would have got the picture out
earlier, *Keep a current best, in case they ask* in `workers/general.md`, has its exit at "name
the file in your report", which reaches Cognition and stops. Meanwhile the record gate,
correctly, refuses a file path in a line as machinery — so the path goes, and the claim stays
without its evidence.

Underneath, the machinery for showing has grown one path per consumer:

| | Today |
|---|---|
| Ways to address a thing to show | view ref · compiled module URL · shot URL (two forms) · drive ref · signal ref · task-folder path · a view's own asset path |
| Routes serving bytes | `/views/*`, `/api/media`, `/api/drive/file`, `/api/tasks/{s}/files`, `/api/people/…` — and the same drive bytes under two of them. `/api/media/drive/…` was served `immutable` until 2026-09-22, a claim a file edited in place cannot keep and one the edge mirror would have believed; it is `no-cache` now, like `/api/drive/file` |
| MIME tables | four; the one on `/views/*` serves mp4 and fonts as `text/plain` |
| Range, ETag, streaming | streaming and ranges on `/api/media` and `/api/drive/file` since `server::disk_file`; the task-files route and `/views/*` still read whole files into memory; no `ETag` anywhere |
| Headless renders | four paths (review ×2 skins, show shot, ref shot, share check), each a fresh Chromium process, one of them under a lock |
| Content keys | `DefaultHasher`, which a toolchain update is free to change |
| Pictures on Home | a join of the row's `made` refs against `/api/views` (polled every 8 s for it), dropping any view without a shot — and a view that was only ever reviewed has none, so a task's result is hidden until somebody opens it |

None of that is anybody's mistake; each was the cheapest way to serve the consumer in front of
it. It is the shape that a capability expected to grow for years cannot keep.

## The model

Three things, and the separation between them is the design.

**An attachment is a thing made to be looked at**: a picture, a clip, a recording, a document.
Its bytes are immutable and addressed by their content. It knows only facts about itself —
kind, type, size, dimensions, duration, frame rate, codec — and nothing about who made it,
why, or where it has been put.

**A view is a composed thing**: live, laid out, sometimes interactive, addressed by its ref.
It is the one thing shown that has to be *built*, and that is exactly why it must never be the
only way to show anything.

Everywhere either is named, it is named the same way: `att:<id>` or `view:<ref>`.

**A placement is the act of putting one of them where the person looks**, and it is recorded
by the place it was put, in the record that place already keeps:

| Placed | Drawn on | Recorded in | Verb | Rung | Kind |
|---|---|---|---|---|---|
| on a task's line | the task's panel · its tiles on Home | the task's timeline | `hi_task_note(…, attach)` | worker | pull |
| made for a task | the same | the task's timeline (`made`) | host, witnessing `hi_review_view` | host | pull |
| on the screen | the stage · the trail | the appearance history | `hi_show` | Reaction | push |
| handed over in the conversation | a message | the journal | `hi_say(…, attach)` | Reaction | push |
| on a share | a page somebody else opens | `views/_shares` | `hi_share` | worker | outward |
| sent through an app | Feishu, WeChat, mail | the task's `delivered` line | the app's own tool, then `hi_task_note(delivered, attach)` | worker | outward |
| inside a view | wherever the view is drawn | the view's source | `hi_add_attachment`, then `<Attachment>` | view builder | — |

**Context lives on the placement, content lives on the attachment.** The caption is the line or
the message that carries it; the time, the task and the session are the placement's. One
picture put on three lines is one attachment and three claims about it, which is what it is.

**There is no catalogue of attachments.** Each surface's own record is the index of what is on it
— the rule [`views/`](data.md#views) already keeps (*nothing indexes it*) and the one
`referenced_files` stated while it existed (*not a listing of the folder*). An attachment no record places is
drawn nowhere, which is correct: nobody decided it was worth looking at.

**Presentation is one set of components** — a preview and a viewer — that every place draws
through, so a picture looks and behaves the same on a tile, in a panel, on the screen, in a
bubble and on a shared page, and improving the video player improves it everywhere.

## Principles

1. **Placing is cheap; composing is expensive, and neither pays for the other.** A picture is
   placed in the time it takes to copy it. A view is built when there is something to
   compose — a report, a comparison, an interactive tool — and never as a frame for one
   medium.
2. **A claim and its evidence travel together.** The line that says what a picture shows
   carries the picture; the message that hands something over carries the thing. That is the
   seam [legibility.md](legibility.md#the-surfaces) listed as missing (*a file handed over — no
   seam*), and it is the sentence's own seam rather than a new one.
3. **Minds point; the host witnesses.** A mind names a path or an id. The host copies, hashes,
   probes and writes the reference. No mind ever types a hash, a URL or a MIME type.
4. **Bytes are immutable; everything else is derived.** Previews, playable proxies and view
   shots are derivations keyed by the attachment and a named spec, kept in one disposable cache.
   Changing how a thumbnail is made is a spec version, never a migration.
5. **One address, one route, one type table.** Every byte the face fetches for an attachment comes
   from one handler that streams, answers ranges, and declares `immutable` — which is true by
   construction, and is what [topology.md](topology.md#content) uses to mirror content to the
   edge before anybody asks for it.
6. **Pull is free to fill; push is judgment.** A worker puts evidence on its task's record
   without asking anyone — that surface is visited, not imposed, and visited more than the
   screen is pushed: 425 moves by the person in September against 73 shows. Taking the
   screen or a message is Reaction's, by [invariant 1](arch.md#invariants) and by
   [*the screen answers to the conversation*](stage.md#the-screen-answers-to-the-conversation-in-both-directions),
   and the one thing the host decides about a push is where it lands: a show onto a page
   they only just got goes into their list with a dot instead
   ([*a show leaves a page being read alone*](stage.md#a-show-leaves-a-page-being-read-alone)).
   No queue, no importance field.
7. **The read path does no work per picture.** A surface that polls reads records it already
   reads; a picture on it costs one immutable fetch the browser caches forever.

## The attachment

### Identity and storage

The id is the first 16 hex digits of the SHA-256 of the bytes; the object keeps the full
digest beside it. SHA-256 rather than `DefaultHasher` because an address that a compiler
upgrade may change is not an address.

    data/attachments/
      objects/<2>/<id>.<ext>        the bytes, never rewritten
      objects/<2>/<id>.json         the probe: kind, mime, bytes, width, height,
                                    duration, fps, codec, color, sha256
      derived/<spec>/<2>/<id>.<ext> previews, proxies — disposable, re-derivable

**The pen is the host's.** It is written at a seam, the way the log is, and nothing else
writes under it ([the authorship rule](arch.md#the-authorship-rule)). It is not `drive/`,
which is the agents' own cabinet and is edited in place, and not `memory/raw/`, whose media
[fades](data.md#forgetting): a picture put in front of the person is a record of what they
were shown, kept like text is kept.

**Copied, not referenced.** A line says what was true when it was written. The worker will
overwrite `pose_899.png` on its next iteration, and `/tmp` does not outlive the session that
wrote there — the calibration task's first shift left all its work in `/tmp/cc`. A reference to
the working file would silently change what the line's evidence shows; a copy cannot. Where
the filesystem clones (APFS, btrfs, XFS with reflink) the copy costs no space until the
original changes; elsewhere it costs the bytes, and the same picture placed twice is still one
object.

**Kept.** Objects are not collected: keep-biased, as the rest of `data/` is ([Forgetting](data.md#forgetting)).
Removing one is an explicit forget, and a forget reaches the edge's mirror too
([topology.md § Open](topology.md#open)). The derived cache has a size bound and evicts least
recently used entries **when it writes**, never on a timer.

### What the host does at placement

On the tool call, before it returns:

1. Resolve the path — absolute as given, or relative to the task's folder, then the data dir
   ([invariant 11](arch.md#invariants)). Refuse anything under `drive/accounts/secrets/`.
2. Copy it in (a clone, where the filesystem has them), then hash and probe the copy — so what
   is hashed, probed and kept is one set of bytes, even if the worker rewrites its file meanwhile.
3. Probe it — an image header decode, `ffprobe` for time-based media. **What the bytes are is
   decided by decoding them, not by the extension.** A file that does not decode as a picture, clip,
   recording or document is refused with the reason, and nothing is written.
4. Draw the preview from the frame the probe already decoded.
5. Write the placement into the record, and answer with the id and the probe in one line
   (`att:3f9a… · clip 1920×1080 · 0:30 · 30 fps`), which is what the mind needs to refer to it
   later and nothing more.

Then, off the call: the proxy if one is needed, and the mirror upload.

**The preview is made on the call, not queued**, because it is the tile — the one thing the line
is for — and it costs almost nothing once the probe has decoded a frame: measured on the
court-calibration figures, a 2.4 MB picture attached in 39 ms and a 9.4 MB clip with a picture
beside it in 210 ms, previews included.

**Budget:** under 300 ms for a 5 MB image, under a second for a 200 MB clip — the hash and the
probe, not the copy, dominate once the copy is a clone. Refused above **2 GiB**, with the
reason, as a starting ceiling.

### Derivations

Each is a named, versioned spec, and its key is `(spec, id)`:

| Spec | What | When |
|---|---|---|
| `preview.v1` | a still fitted whole inside 960×540, JPEG — PNG where the picture has transparency — the tile rule of [stage.md](stage.md#the-frame-is-a-surface-and-a-view-goes-up-before-it-is-finished) | on placement; for video, a frame from the first tenth that is not black; for a document, its first page |
| `proxy.v1` | H.264 High, SDR BT.709, ≤1080p, `faststart`, AAC | on placement, only when the probe says a browser this product ships in cannot play the original (HEVC outside WebKit, HDR transfer curves, exotic containers) |
| `shot.v1` | a view's picture, in the views tree's own `_shots/` | as today: on a show, on an open, for a `made` view that has none, and when one is published |

**A preview is the tile's size, not the file's.** Workers write figures at full resolution —
`pose_899.png` is 2.4 MB — and a Home drawing twenty of them would fetch tens of megabytes to
fill 240×135 boxes; fitted to 960×540 and stored as JPEG a picture is on the order of 100 KB.
JPEG and not lossy WebP, which would save perhaps another quarter: the only encoder for it is
libwebp, one more native library on five platforms, and the viewer opens the original anyway.

**One derivation queue, one browser.** Everything that renders a view — review, shot, share
check — goes through one long-lived headless Chromium with a queue in front of it, rather than
launching a fresh process per render on four separate paths. Media derivations run on their
own bounded pool (two jobs), so a long transcode never holds up a preview. Every job is
single-flight per key.

**A missing derivation is a floor, not an error.** A tile whose preview is not there yet draws
the kind and the duration in words (*video · 0:30*), exactly as a view without a shot drew its
mark before pictures existed; the preview replaces it when it lands.

### Serving

    GET /api/attachments/{id}                 the bytes
    GET /api/attachments/{id}/{spec}          a derivation (preview, proxy), or what it is (about)

- `Cache-Control: private, max-age=31536000, immutable`, `ETag` the digest — both true forever,
  and the second is what makes a revalidation cost nothing.
- **Range, and streaming**, through `server::disk_file` like the media and drive routes — a
  39 MB clip is never held in memory, and WebKit seeks it.
- The type comes from the probe, from **one** type table that every byte route in the server
  shares; `X-Content-Type-Options: nosniff` on all of them.
- **Nothing an attachment contains can run.** SVG and PDF are served with
  `Content-Security-Policy: sandbox`; an SVG's preview is a raster. Something meant to run is a
  view, and views have their own path.
- **Mirrored like everything immutable.** Placement enqueues the upload
  ([topology.md § Uploading](topology.md#uploading-a-queue-with-that-second-branch-as-its-backstop)),
  so by the time the person opens Home on their phone the bytes are at the edge. Through the
  tunnel a 39 MB clip is 28 seconds at the measured 11 Mbps; from the edge it is not the
  tunnel's problem.

## Placements

### On a task's record

    hi_task_note(kind, text, attach?: [path | att:id, …])        workers, at most four

The line is still one line and still judged at the seam
([legibility.md § M](legibility.md#m-the-gate--on-a-line-not-on-a-record)); the store writes
the attachments after the text in its own marker, ` ⟨attached att:… att:…⟩`, which no mind
types and every reader strips. A hand-typed marker naming an id the store never
placed is ignored on read, the way a `made` ref to a view that is not on disk is.

**The kind of the line says what the picture is**, which is the first thing in any record able
to answer home.md's open question of which result a task is *for*: on `delivered` it is what
they have now, on `update` it is how the work is going, on `waiting` it is what they are being
asked to judge. Nothing is ranked by it yet (§ *Open*); it is recorded, which costs nothing.

**`made` stays**, and stays the host's: a view rendered for a task is placed on it,
witnessed rather than said. The row carries both kinds together, newest first by the time of
the line that placed each, still six at most on Home.

**What reads it:**

- **The row** (`GET /api/tasks`) carries `attached: [{ref, preview, kind, at, line}]`,
  newest first, computed from the timeline the row is already built from — no directory is
  listed and nothing is statted per picture. Home draws its tiles from it and **stops polling
  `/api/views`**, which it read only to join shots; a `made` view with no shot is enqueued for
  one instead of being dropped.
- **The panel** draws each line's attachments under its sentence — previews, lazily loaded,
  pressed open into the viewer — and the `made` line draws the view's picture instead of its
  ref. The record becomes a log with its evidence in it.
- **A tile** opens what it is a picture of: a view is gone to, as before; an attachment opens
  the task's panel at the line that carried it.
- **The projection** into Cognition's and Reaction's windows (`render_projection`) appends the
  newest line's attachments as ids, names and probes —
  `· attached att:3f9a… pose_899.png (picture 1920×1080)` — so the rung holding the
  conversation knows there is something it can put up, and can answer *let me see it* in the
  same turn instead of dispatching anyone.

### On the stage

    hi_show(op, id, ref: view:<ref> | att:<id>)                  Reaction

An attachment on the stage is drawn by the face's **own viewer** — the component in `@hi/core`
that the panel and the conversation draw it with — for the reason the conversation is bundled
([stage.md § 1](stage.md#1-a-views-source-is-bundled-or-compiled)): showing a picture must not
depend on the view compiler, the module cache, or a builder having been right. The stage mounts
it through a module the host writes rather than compiles, `/api/attachments/<id>/stage.v1.mjs`
— three lines naming the object and importing that component by the bare specifiers the import
map resolves — so an attachment is a destination like any view's: in the trail, under the
cursor, restored after a restart, its card's picture its preview and its label what it is.
Everything else about the stage — one screen, one cursor, *a show takes the window with it* and
the one case it does not — is unchanged.

This is what turns *拿出来我看看* into seconds. Reaction already holds the id from the
projection; putting it up needs no worker, no view and no review.

### In the conversation

    hi_say(text, attach?: [att:id, …])                            Reaction

[message.md](message.md#both-ends-can-hand-over-a-file) has said since it was written that
`From::Agent` with `Content::File` is a message like any other, and nothing has ever minted
one. This mints it. One content per message still holds: the words are one message and each
thing handed over is one more, enqueued together as one arrival, and **each counts toward the
three messages between one of theirs and the next** ([legibility.md § F](legibility.md#f-delivery)) —
a picture in the conversation is read like one. A show points at something and takes it back;
a hand-over is kept in the conversation; which one a moment wants is Reaction's call, and the
object is the same either way.

**A view is shown, not handed over**: it is live, and there are no bytes in it to keep.

`FileRef.ref` takes either name; the face resolves `att:` to the attachment route and anything
else to `/api/media` as today. **The person's own handed files stay signals** — they are the
log's, with the log's retention — and are drawn by the same viewer.

### On a share

    hi_share(ref: att:id | view:ref, …)                                  workers

A shared attachment is its bytes and a page that is the bundled viewer, at `/att/<id>`, with
`og:image` the preview at an absolute URL. It needs none of a view's share check: it fetches
nothing, so there is nothing that could render half-empty. A shared page's scope grows by exactly
the attachments its check saw it request — the check lets the attachment routes through while it
refuses the rest of `/api/*`, and records each id it saw. [sharing.md](sharing.md) has the rest.

### Through an app

Nine of the twenty-five times the person asked to be shown something, they asked for it on
Feishu. **Apps stay a worker's tools, and nothing here becomes a connector** — reaching Feishu
is a skill and a CLI, the way [surfaces.md](surfaces.md#surfaces) has it. What changes is that
there is one thing to send: an attachment's object is an ordinary file with a stable path, so any
tool that uploads a picture uploads it, and a view goes as its share link with its preview
beside it. The send is then said after, by [invariant 9](arch.md#invariants), as a `delivered`
line carrying the same attachment — so the record shows exactly what went out, as a picture, not
a sentence about one.

### Inside a view

    import { Attachment } from "@hi/core";   <Attachment id="att:3f9a…" />

A view embeds an attachment through the same component the host draws it with, instead of
keeping the file in its folder or writing a `fetch → blob → <video>` player. The ball
tracker's player view is what that looked like; it is the part of the view builder's job that
stops existing. 44 of the 125 composed views embed a picture or a clip; an attachment is one
copy however many views show it, has an address a line, a message and a share can carry as
well, and a shared view's scope names exactly the ones it uses.

**The view names the id and nothing else.** `<Attachment>` asks
`/api/attachments/<id>/about.v1` what the thing is — the same description the stage module is
written from, so the two cannot disagree — and draws a picture at its own shape from its
preview until its box is wider than the preview is sharp, then from the original; a clip
through the playable route, so a clip no browser decodes plays from its copy here too. An id
the core does not hold is said on the page and reported as an error, which a review reads.

**An id without a line is `hi_add_attachment`**, a worker's verb that copies files into the
store and answers with their ids, writing nothing anywhere: the placement is the `<Attachment>`
in the view's source, which is its record. It is the builder's only way to an id for a file no
line carries, and it is not a way to show anything — an id nobody embeds or carries is drawn
nowhere. **A view's own material stays its own**: a photograph found
for a poster is part of the composition, not a result, and lives in the view's folder as it
always has. What moves is the work's evidence, the thing the view is *about*.

## Presentation

Two components, in the face, exported through `@hi/core`:

- **`Preview`** — the 16:9 tile: the preview when it exists, the kind and duration in words
  until it does. For a view, its shot.
- **`Viewer`** — the thing opened: a picture fitted whole and zoomable to 1:1; a clip with
  frame stepping at the probed rate, speed, and a loop between two marks — review tools, because
  what a person does with a result clip is look at one moment of it; a recording; a document
  through `@open-file-viewer/core`, which `factory/drive` already loads and which moves up
  into the viewer rather than living in one view; a view, mounted as the stage mounts it.

Every face is the same web face in a native webview — WKWebView on iOS and macOS, WebView2,
WebKitGTK, Android's — so this is written once. The television draws the same viewer and its
focus follows the remote the way [the room's does](stage.md#the-television-is-the-room-and-the-focus-is-the-cursor).

## Who decides

**The worker decides what is worth placing, by the rule it already has.** *Keep a current
best, in case they ask* moves its exit from the report to the record: when the work has
something that would change how the person thinks about it — a direction that held or fell, a
first version that can be looked at, a result only their eyes can judge — the line that says so
carries it. Unverified is a label in the caption, not a reason to hold, which is that section's
own sentence. What a worker looks at to find its way — a mask, a zoomed crop, a residual plot —
stays in its folder: those are its working files, not the work, and the difference is the
worker's judgment, never a filter on file names.

**The record is not the screen.** A brief that says *leave the screen alone* governs a push;
placing on the record is what the person visits. `cognition.md` says so where it writes briefs.

**Reaction decides what is pushed**, under the rules that already govern the stage and the
conversation. Nothing new decides *when*.

**Nothing in code infers any of it.** A picture in a task's folder is not placed because it is
there — one live task's folder holds 2,101 of them — and a line that names a file is not given
it. What reaches the person is what a mind handed over.

## Cost

| | Budget | Bought by |
|---|---|---|
| placing a picture | ≤ 300 ms | a clone, a streamed hash, a header decode |
| placing a clip ≤ 200 MB | ≤ 1 s | the same, and one `ffprobe` |
| preview ready | with the call | the frame the probe already decoded |
| a Home poll | no read per picture | previews computed from the timeline the row is already built from |
| a tile, second time | 0 bytes | `immutable` |
| a clip opened on the phone | the edge's bandwidth | mirrored at placement |
| server memory per request | one chunk | streaming |
| a view render | one queue, one browser | no process launch per render |
| a mind's context | one line per attachment | ids, never bytes or URLs |

## Measurement

Server-side, beside the rest of [legibility's numbers](legibility.md#i-the-number), and read
per day:

- **Show-me messages** — the person asking to be shown something that was only described. The
  primary number, and the reception read that already runs on their next message
  ([legibility.md § G](legibility.md#g-audit)) is what counts it: a keyword scan of the
  baseline found about 70% of them and matched as many messages that were the person asking
  the *agent* to look at something, so only a reader of the words can count it. Baseline
  about 25 in 22 days; target zero.
- **Time to first evidence** — from a task's first `update` to its first placement, and the
  share of open tasks past an hour with lines and no placement.
- **Lines that name a picture by path** — the failure this replaces: 26 since 09-15, should go
  to zero.
- Placements per surface; derivation latency p50/p95 and failures by spec; bytes stored and
  bytes deduplicated; mirror lag.

A change to the rules in `general.md` that decide what gets placed goes through `make
eval-records` first, scored on whether the replayed line carries what it describes.

## What this deletes

In the phase that replaces each, never after it:

- Home's `/api/views` poll and its client-side join of refs to shots.
- `referenced_files`, `/api/tasks/{s}/files/*` and the panel's file-token linking — a line
  that hands something over carries it. **What it costs:** inline-code file names on records
  written before this stop being links; the text stays, as retired mechanisms' text always
  does.
- Four of the five type tables — a `.mov` and an `.m4a` went out of `/api/media` as
  `application/octet-stream`, and `/views/*` served an `.mp4` as `text/plain`, which is why
  the views built to play a clip fetched it as a blob instead of naming it in a `<video src>`.
- The whole-file reads in every byte route but one. **`/views/*` keeps its**, deliberately:
  it is the one route inside a compression layer, and a compressed `206` would carry a
  `Content-Range` describing bytes that are not the bytes in the body. What it serves is
  compiled modules — tens of kilobytes of text where compression is the win and a range is
  never asked for.
- A fresh Chromium per render, and the four paths that launch one.
- `DefaultHasher` as a content key, in `_compiled/` and `_shots/` both: a compiled module
  is named by the first 16 hex digits of its source's SHA-256, and a view's picture takes its
  name from that.
- Agents writing into `views/_shots/`, a host directory: 561 of its 627 files (115 MB on
  2026-09-23) are review screenshots workers saved there themselves, in subdirectories of
  their own. A review's pictures come back in the call; a worker that wants to keep one
  places it. Said in `view-builder.md`, which is where the habit came from — the files
  already there are left where they are, as retired mechanisms' leavings always are.
- `drive/generated/` as the landing place for `hi_text_to_image` and its siblings, which
  produce attachments at birth; the drive keeps what somebody decides to keep, which is what the
  drive is. With it goes `/api/media` answering `drive/…` refs at all: drive bytes are served by
  `/api/drive/file`, revalidated, and only an attachment claims never to change. **A generated
  picture answers with its `att:` id**, which is what `hi_image_to_image`, `hi_image_to_video`,
  `hi_image_text_to_text` and a group's `icon` all take — and what puts it on the screen with
  nothing built around it.

## Phases

Each phase compiles, ships, and is worth having without the next.

1. **The object and the record.** The store, the probe, `preview.v1`, the route with Range and
   streaming; `hi_task_note(attach)`; the row's `attached` and the panel drawing them; Home's tiles
   from the row, the `/api/views` poll deleted, a shot enqueued for a `made` view without one;
   the projection carrying ids; `general.md`, `cognition.md` and `judges/record.md` re-aimed;
   the legibility table's rows. *This is the phase that would have drawn `pose_899` at 13:28.*
   A clip in a codec no browser plays is refused with the command that makes one it can, until
   `proxy.v1` makes that copy itself.
2. **The stage and the conversation.** `hi_show(att:)` and the bundled viewer; the agent's
   `Content::File` minted by `hi_say(attach)` and drawn by the chat; `proxy.v1`; `reaction.md`;
   the preview and viewer moved from the board's view into `@hi/core`, which is the loan phase 1
   takes and names; the measurements, with show-me messages counted by the reception read and
   the rest computed from the records.
3. **Pages embed, and shares carry.** `<Attachment>` in `@hi/core`; `view-builder.md` told a view
   is for composition; `hi_share` for attachments, with the scope and an absolute `og:image`.
4. **One of everything.** One browser behind one queue; content keys under SHA-256;
   generated media as attachments; `referenced_files` and the task-files route deleted; one type
   table and streaming across every byte route, `/api/media` and `/api/drive/file` included.

## What this amends

- [`home.md`](home.md) § *Internal mapping*: a task's pictures are the attachments and views its
  record places, not only the views it made; § *Handing off*: a tile opens the panel at its line.
- [`legibility.md`](legibility.md) § *The surfaces*: rows for an attachment on a task line, on the
  stage, and handed over in a message; *a file handed over* leaves § *Open*.
- [`data.md`](data.md): `attachments/` in the tree and in the authorship table; `made` joined by
  `⟨attached …⟩` in *Tasks*; `drive/` no longer where generated media land.
- [`surfaces.md`](surfaces.md) § *Channels*: the rich-content row takes an attachment or a view.
- [`message.md`](message.md): an agent's `Content::File` exists.
- [`sharing.md`](sharing.md): a share is of an attachment or a view.
- [`topology.md`](topology.md) § *Content*: the attachment routes in the mirrored table.
- [`stage.md`](stage.md) § 1: the attachment viewer is bundled, beside the conversation.
- [`arch.md`](arch.md) § *Contents*: this document; [`../data-dir-layout.md`](../data-dir-layout.md): the tree.

## Decisions

| Decision | Reasoning |
|---|---|
| **An attachment is separate from a view** | Everything shown had to be a view, so a picture that already existed paid for a React module, a compile, a headless render and a review before anyone could see it — a builder run is 16 minutes at the median and 153 at p90, for work that was done. Eight views exist only to frame one medium |
| **Content-addressed and copied at placement** | A line's evidence must not change after the line is written, and the working file will; `/tmp` does not survive its session. Content-addressing also makes `immutable` true, which is what lets the edge mirror it ahead of demand |
| **The host's pen, in its own subtree** | Written at a seam like the log, kept like text. `drive/` is edited in place by agents; `memory/raw/` media fade |
| **Context on the placement, content on the object** | One picture on three lines is three claims. A caption on the object would be a second text surface with no seam |
| **No catalogue** | Each surface's record already indexes what is on it; a catalogue beside them is bookkeeping that drifts, the argument `views/` and `referenced_files` already made |
| **Placing on the record is the worker's, pushing is Reaction's** | Pull surfaces are visited; push surfaces interrupt. The one mouth stays one, and a worker needs nobody's leave to show its own evidence where its record is read |
| **No inference from folders or prose** | 2,101 pictures in one task's folder are its working files, not its results; a path in a line is a mention without a verb |
| **One route, streamed, ranged, immutable** | Five byte routes and four type tables, of which two stream and answer ranges; `immutable` is now what decides what leaves the machine, so it has to be true by construction rather than by care. The route rides `server::disk_file`, which already carries ranges and streaming for the other two |
| **A view's picture stays in the views tree** | It is not a derivation *of an attachment*. Its key is either a compiled module's content, which lives in `views/_compiled/`, or a named surface's ref — which is not content at all and is rewritten in place, which is what its `?v=` is for. Putting it under the attachment store's pen would file a non-attachment there and rename what the face, Home, a share's scope and three journeys already speak. What phase 4 wanted from it — no content key a toolchain may redefine — is the SHA-256 above |
| **Derivations are named specs in one disposable cache** | Thumbnail quality and codec choices change for years; a spec bump re-derives lazily and needs no migration |
| **Previews at the tile's size, in JPEG** | A tile is looked at, not edited; fitting it is an order of magnitude off every Home, and lossy WebP's further quarter costs a native library on every platform |
| **One browser, one queue** | Four render paths each launching Chromium, one of them locked, is the cost that grows with use |
| **`made` stays beside `attach`** | It is witnessed, not said; a builder that never writes a line still has its view placed on its task |
| **A hand-over counts toward the three messages** | A picture in the conversation is read like one |

## Open

- **Judging the picture, not only its caption.** The seam reads the line; nothing yet reads
  whether the attachment is legible, or is the one the caption describes. A vision read at the
  seam is the obvious shape, and its cost has to be measured first.
- **Whether a `delivered` attachment should outrank progress on Home.** Newest-first is what the
  row does; the kind is recorded so the question can be answered from data when a task's six
  tiles first push its deliverable off.
- **What the viewer grows.** Compare two clips in lock-step, a filmstrip of a task's placements,
  a waveform — each added when a task needs it, in the one viewer.
- **The ceiling.** 2 GiB is a starting value.
- **Inbound and outbound in one store.** A handed file stays a signal with the log's retention;
  whether the two converge is a question about forgetting, not about showing.
