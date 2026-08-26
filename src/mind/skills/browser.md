---
purpose: drive a real Chrome — open a page, read what actually rendered, screenshot it, and script clicks and typing over DevTools
use: browser
---

`browser` is this machine's Chrome under a stable name. It is a thin wrapper: it works out
which browser this machine actually has — one you already had installed, or a build hi-agent
downloads on first use — and passes everything you give it straight through, adding
`--headless` only if that particular binary needs it. So you never have to know which browser
you got, and this note stays true on a machine that has a different one.

It publishes no interface of its own. `browser --help` is Chrome's own help, and that is the
argument list to trust.

## What it is for

Reaching a page as a browser sees it, not as an HTTP client does. Anything rendered by
JavaScript, anything behind a login you are already signed into, anything where the markup
you get from a plain fetch is an empty shell — that is this.

Web search and plain fetches are still the right tool for a page that is just text. Reach for
this when the page has to actually *run*.

## Getting started

Read a rendered page:

    browser --dump-dom "https://example.com"

Look at it the way a person would:

    browser --screenshot=/tmp/page.png --window-size=1280,900 "https://example.com"

Both of those are one-shot: Chrome starts, does the thing, exits. They are enough for a
surprising amount of work, and they are where to start.

## When you need to interact

Clicking, typing, waiting for something to appear, or doing several steps in one page needs
the DevTools protocol, because a one-shot invocation cannot hold state between steps:

    browser --remote-debugging-port=9222 "about:blank" &

Then drive it over that port — CDP is a WebSocket speaking JSON, and every language you have
can talk to it. `Page.navigate`, `Runtime.evaluate`, `Input.dispatchMouseEvent` and
`Page.captureScreenshot` cover most of what an errand needs.

**If you find yourself doing this more than once, write yourself a driver** — a small script
that wraps the steps you keep repeating — put it on your PATH, and leave a note beside this
one describing it. A capability you had to reconstruct twice should have been a tool the
second time.

## Traps worth knowing

- **A page is not done when it loads.** Content that arrives by fetch is not in the DOM yet
  when navigation completes. Wait for the element you actually want, not for the page.
- **Some sites refuse an obviously automated browser**, and the tell is usually the user
  agent or a missing window size. This is a thing to notice rather than fight: if a site
  clearly does not want to be driven, say so instead of escalating.
- **Being signed in is worth protecting.** A browser profile carrying live logins is not
  something a note can rebuild — only the person can sign in again. Keep any profile you rely
  on somewhere durable under `drive/`, and pass it explicitly with `--user-data-dir`. Never
  keep it under `bin/`, which is disposable.
- **A screenshot is evidence and prose is not.** If you are reporting what a page said, look
  at the page.

## Perishable

Everything above the traps is durable. These rot, so re-check rather than trusting them:

- **Chrome's own flags.** They change between versions, and `--headless` in particular has
  changed meaning more than once. `browser --help` is the truth; this note is not.
- **What any specific site looks like.** Layout, selectors, and whether a flow needs a login
  at all are all facts about this month.

## If it is missing

`bin/browser` is written by hi-agent at every start, so a missing one means hi-agent has not
started since the file was removed — nothing to rebuild by hand. The browser it points at is
resolved the first time you actually run it, which is also when a download happens if this
machine has no Chrome of its own. So the first call can be slow and later ones are not.

On a slim Linux box the download alone is not enough: Chromium needs its own shared libraries
(`libnss3`, `libexpat1`, `libfontconfig1`, and fonts) before it will start at all. The launch
error quotes the browser's own stderr, and installing the system Chromium package is usually
the shorter road than fixing the download.
