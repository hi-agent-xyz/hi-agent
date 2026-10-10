# Sharing

How one of the agent's views, or one attachment, becomes a page somebody else can open. Nothing
here changes what either *is*; it says what happens when one is handed to a person who is not
the owner.

This is the goal state, in the present tense throughout, as design here always is. It says
nothing about what is built.

## Goal

Let a person hand over one thing the agent made, without handing over the agent.

Everything below follows from taking both halves literally. *One thing* — a share is a view or
an [attachment](showing.md), never an account, a conversation or a door into the core. *Hand over* — what the other person
receives is a page, not a guest seat.

## Decisions

| Decision | Reasoning |
|---|---|
| **A shared view is an ordinary web page** | It has to be readable by a person on a phone, by an agent with `curl`, by a link preview in a chat app, and by a crawler if the owner made it public. Every one of those reads HTML. A thing that is only a page for a logged-in browser is not shareable, it is visitable |
| **The page carries its content as HTML, and the live view mounts over it** | A compiled view is an ESM module: `curl` on a React mount point returns an empty `<div>`, which fails the audience above by more than half. The rendered DOM is already produced by the check every share has to pass, so serving it costs nothing and is thrown away otherwise |
| **A share grants reading one view, never the API** | Its `<ref>.assets/` folder, registered resource names, and compiled module define its files. The server resolves names to attachment bytes without opening `/api/*` to the visitor. A browser's first render cannot define this grant: lazy images may load later |
| **A view is checked before it may be shared, with the API blocked** | A view that fetches its data renders half-empty under that scope, and the owner does not find out — the person they sent it to does. The check is the only thing standing between "share" and a silent bad impression |
| **Only a named view may be shared** | The same rule bookmarks already keep. An inline view is the disposable artifact a turn compiled; it has no name to put in a URL and nothing to come back to |
| **An attachment may be shared too, and needs no check** | A picture or a clip is its bytes and a page that is the host's own viewer, with its preview as `og:image`. A view uses its registered resource names regardless of when they load ([showing.md](showing.md#on-a-share)) |
| **Sharing is per-view and opt-in, and the default is unshared** | A share is a decision about one artifact, so it is stored beside the view and made once. Nothing becomes shareable by being shown |
| **Revocation is not instant** | A shared page is served to callers with no credential, so it is cacheable, and an edge holds it. Short `max-age` and honesty in the UI beat a mechanism that pretends otherwise |
| **No interaction, no live data, in the first shape** | Both are real wants and both cost the isolation this design does not currently need. A still, self-contained page is what the default view already is ([`overlay-presentation-model`](stage.md)) |

## Why this is small now

An earlier draft of this design carried a sandbox, an opaque origin, a second hostname and a
scoped session that had to be defended against other people's cores. All of it existed for one
reason: **relayed cores shared an origin**, so a view authored by one person's agent could run
in another person's browser and reach *their* core with *their* cookie attached.

That is gone. [`topology.md`](topology.md) moved addressing to one origin per core, and
`iloahz.hi-agent.xyz` cannot see `ningshen.hi-agent.xyz` at all. **Sharing was the trigger for
that reversal, and the reversal is what makes sharing ordinary.** What remains here is a read
grant and a page, not a security perimeter.

One exposure survives and is accepted: a shared view runs on its *own* core's origin, so if the
owner opens their own shared link while signed in, that code runs with their session. It already
does — every view they show does. Sharing adds no case.

## The address

A shared view is at its ref, on the core's own origin:

```
https://iloahz.hi-agent.xyz/agent-arch
https://iloahz.hi-agent.xyz/badminton-top10/leader     a two-segment ref is a two-segment path
https://iloahz.hi-agent.xyz/agent-arch?key=<token>     unlisted
https://iloahz.hi-agent.xyz/att/3f9a0c11d2e4b5a6       an attachment, at its id
```

**A share name is refused at creation if it collides with a core route** — `api`, `views`,
`assets`, `generated`, `up`, `render`, `auth`, `account`, `inspect`, `healthz`, `mcp`, `att`
(where attachments are shared), and whatever is added later. This is the username-versus-route trap for the third time in this
system (handles against community routes, then handles against DNS, now view names against the
core's own paths), and the same rule applies: over-broad, and only cheap before the first link
is handed out. Unlike a handle a name here *can* be released, so the list may be narrower than
the registry's — but a link already sent cannot be un-sent, which is why it is still checked at
creation and not at serve time.

## Two kinds of share

| | Who may open it | How |
|---|---|---|
| **public** | anyone with the URL, and a crawler | the path is open |
| **unlisted** | anyone with the URL *and* the key | `?key=` is exchanged once for a share cookie |

The key is a bearer token in a URL, which is exactly what it looks like: **whoever holds the
link holds the access.** It defeats enumeration and nothing else, and the UI should say so
rather than implying a permission.

**`?key=` is exchanged for a cookie, for the reason `POST /api/session` exists**: the page's
sub-resources — its module, its images — are requested by the browser without the query string,
so the grant has to live somewhere the browser sends by itself. It is the same seam with a
different scope: a session says *this surface may reach me*, a share cookie says *this caller
may read these paths*.

## Where the owner shares from

Two places, one mechanism. The agent shares with `hi_share` when asked to; the person shares
from **the card's menu** in the views tab ([stage.md](stage.md)) — a view's card, or an
attachment's. Both call the same check and are handed the link by the same rule: under the
core's claimed name, or, with none, a bare path that only works on this machine and says so.

The sheet the menu opens:

- heads with the view's picture — what is about to go out;
- says, before anything is made, that **anyone with the link can open it**;
- offers unlisted or public, **unlisted by default**;
- shows progress while the check renders, then the link with a copy button — or the check's
  refusals, as the core worded them;
- for something already shared, offers *copy link* on a public share, *new link* on an
  unlisted one — its key is kept only as a hash, so the link cannot be shown again, and a
  new one retires the old key — and *stop sharing*, noting that a stopped link can keep
  working for about a minute.

## The scope

A share grants exactly:

```
/views/_compiled/<the one hash>.mjs     this view's compiled module
/views/<ref>.assets/**                  the view's resource folder — including delayed images
/views/<ref>.assets/_resources/<name>/*  names registered when the view is published
/assets/*                               React, the stylesheet: the same build every install ships
```

and nothing else. A shared attachment's scope is its own `/api/attachments/<id>/**` — the
bytes, the preview, the playable copy and the module the viewer mounts from — and `/assets/*`. Never `/api/*`, never another view's folder or JSX, never the `views/` root —
`/views/{*path}` is one wildcard route with every view's source and every build artifact behind
it, and opening that would hand over every view the agent has built to share one poster.

For local files, save only visitor-visible resources in `views/<ref>.assets/` and use
`/views/<ref>.assets/<filename>` URLs. The directory is a deliberate publication
boundary: adding a file to it makes that file readable while the share exists. A
project folder may hold multiple views and private sources, so it is never granted whole.
Older shared views that reference files directly under `/views/<project>/` must move
those files into their own `.assets/` directory, update the JSX and republish.

This applies to every static resource, not only pictures: JSON/CSV, fonts, audio,
and custom-player video files follow the same rule. URLs carried inside JSON must
also use the view-owned path, including alternate clips and delayed loads. Generators
must write these paths too; a later data rebuild must not restore project-global URLs.
Never migrate a whole project folder automatically: it may contain private sources.

Local `hi_review_view` reuses the share scope's request diagnosis and warns builders
about observed out-of-scope URLs before handoff, without publishing anything or
adding a second render. System dashboards are exempt from this warning because they
deliberately read the API. These are warnings, not certification: a local review does
not block the API, validate all bindings, or exercise every interaction. Publication
still runs the visitor check. Refusals name both the offending path and the view-owned
directory to use, including the need to update embedded and delayed URLs.

`views/<ref>.resources.json` maps stable names to `att:<id>` references. A view uses
`<Attachment view="<ref>" resource="hero" />`, locally and when shared. The server
reads the named binding and streams the object or its preview from the existing
attachment store; publishing never copies the bytes. The share record fixes the set
of allowed names when published, including names used only after a click. Updating
the attachment behind an allowed name changes what every version using that name
displays; adding a new name requires republishing. These mutable paths must not be
cached as immutable. A view must not reference the global attachment API directly.
Existing share records retain their recorded legacy attachment grants.

The gate enforces these paths. The browser gets `Content-Security-Policy: connect-src 'self'`:
same-origin requests may reach the gate, including requests made after the initial render;
the gate alone decides which paths a visitor may read. External origins remain blocked.

## The check, and the two things it produces

Before a view may be shared it is rendered once through the headless browser already used for
review and for the band's tiles — same harness, one difference: **`/api/*` is refused for the
duration**, so the render sees exactly what a visitor will. Registered resources are
served under the view path. The check does not infer grants from observed requests.

An attachment is not checked. It fetches nothing, so there is nothing that could render
half-empty.

That run answers two questions:

1. **May this be shared at all?** A view that renders blank, throws, or fails to load its module
   is refused. `window.__hiRender` already collects failed loads and console errors, and *a
   blank screenshot with an empty error list means the view really did render blank* — which is
   the case that most needs catching, because it is the one that looks fine in source.
2. **What is the page's HTML?** `document.documentElement.outerHTML` after settle. This is what
   an agent reads without running anything, and what a link preview scrapes. The pictures stay
   as paths rather than being inlined: the view's resource folder is in the share's scope, and the
   whole views tree is already served `no-store` or `private`, so there is no gated asset for an
   edge to hold and nothing a data URI would buy but weight.

**The owner sees the result before it goes out.** The picture `view_shots` already renders is
the confirmation: *this is what you are about to publish*. That matters more than it sounds,
because a check that passes does not mean the content is meant to be public — the render bakes
in whatever the view was holding.

## The page

What a visitor's browser receives, in one document:

- the checked HTML as the initial body — so the content is there before any script runs;
- `<meta name="description">` and `og:description`, written by the agent at share time, saying
  what this is in a sentence;
- `og:image` pointing at the view's own `view_shots` picture — taken when it is published if it
  has none, because a view that was only ever reviewed has none and the link would arrive as a
  bare line of text — at an absolute URL, because a link preview is fetched by a server that never loaded
  the page, and carrying the key for an unlisted share, because that server holds no cookie;
- the import map and the module, so React mounts the live view over the static body. The module
  is named in a `<meta>`, because the page is at the share's own address and not at the render
  page's `?module=`.

A shared attachment's page is the same render page, written when it is asked for rather than
captured: the thing itself as an `<img>` or a `<video>` in the body, its preview as `og:image`,
and the stage module the host writes for it, so a browser gets the same viewer the person's own
screen draws it with.

An agent fetching the URL reads the content and the description and never runs anything. A
person gets the real view, animated. **Neither is a degraded mode of the other**, which is the
property that made this shape worth choosing over serving a snapshot or serving a mount point.

## Not in this shape

Named here so they are decisions rather than omissions:

- **An interactive share.** A view whose point *is* the interaction — filter it, scrub it, open
  a row — needs the API, and the API is the whole core. That wants a scope on the data and not
  just on the paths, which is a different design.
- **Live data.** The page is what the view was when it was shared. A share that follows the
  owner's data would make every link a standing read of their core.
- **Handing a view to another agent's core.** The natural next thing — *"发给宁神看看"* — and
  deliberately after this. The link is the whole payload, and a capability URL under the
  sender's own handle is already a bearer proof that the sender shared it, so it needs no
  signature. Mail carrying *content* rather than a link is what makes keypairs non-optional
  ([`topology.md`](topology.md)), and this does not.
- **A directory of shared views.** A share is a link a person hands to a person. A list of them
  is a profile page, which is a different product.

## Invariants

1. **A share reads one view or one attachment, never the API.** No path outside the derived
   list is served under a share grant, whatever the caller presents — and the only attachments
   in that list are the one the share is of, legacy grants, or the view's registered names at publication.
2. **No view is shared that has not rendered.** The check is the only path to a view's share
   record.
3. **A view is shared by its ref, an attachment by its id**, and a view's name never collides
   with a core route.
4. **A share cookie is not a session.** It authorizes paths, never the person, and cannot be
   exchanged upward.
5. **The page's content is in its HTML.** A shared view that only exists once JavaScript runs is
   a bug, not a variant.

## See also

[`topology.md`](topology.md) for the per-core origin this rests on and the auth seam it reuses ·
[`stage.md`](stage.md) for what a view is and how one gets on screen ·
[`surfaces.md`](surfaces.md) for how the world reaches a core
