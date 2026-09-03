# Windows App

The Windows app lives at `app/windows`. It is a standalone .NET 8 / WinUI 3
build that speaks the core's documented HTTP API and links no Rust — the same
independence [`android.md`](android.md) and [`apple-ios.md`](apple-ios.md)
describe between themselves.

It differs from both in one way that matters more than any UI detail: **it hosts
a core as well as reaching one.** "Host and client are capabilities of an app
instance, never properties of a platform"
([`../arch/topology.md`](../arch/topology.md#app)) — a phone answers no to
hosting and a desktop answers yes.

## Why this one starts where macOS is trying to get to

macOS has ~4,900 lines of app code inside the engine and is migrating it out
([`apple-macos.md`](apple-macos.md)). Windows has none: every macOS-native crate
is `cfg`-gated, every OS capability has a `bail!("not supported")` stub, and
`main.rs` routes a non-macOS start to the plain server path. There has never
been a Windows shell to migrate.

So this is written directly in the target shape — **the shell owns the process
and the engine is its child** — and it costs nothing extra to do that, because
the alternative would have meant writing the main-thread inversion first in
order to delete it later. What macOS reaches at Phase 2 is where Windows begins.

That is worth knowing when reading both documents: where they disagree about who
owns `main`, neither is wrong; they are at different points on the same path.

## Ownership

The Windows app owns:

- its roster and current-core selection;
- the long-lived credential in Windows Credential Manager;
- session exchange and health checks;
- the WebView2 lifecycle;
- **the engine process** — starting it, restarting it, and killing it;
- the notification-area icon, which is the app's presence when no window is open.

The core remains the authority for identity, credential issuance and revocation,
memory, cognition, and channel behavior. Supervision is not management: the
engine owns its data directory and its own runtime provisioning, and the shell
starts it, watches it, and starts it again.

## Hosting the engine

`Core/LocalCore.cs`. The shell launches `hi-agent.exe` from its own directory —
the installer puts both in one place, so the pair find each other with no
configuration — and passes two flags.

**`--data-dir` is passed explicitly, and has to be.** `default_data_dir` in
`src/main.rs` reaches for the OS data directory only when `bundle::resources_dir()`
says it is inside a macOS `.app`; everywhere else it falls back to `./data`,
relative to the working directory. An installed Windows engine would therefore
write the person's memory into `%LOCALAPPDATA%\Programs\Hi Agent`, where the
uninstaller's promise to leave user data alone stops being true. The shell passes
`%APPDATA%\human-interface\hi-agent\data` — the same path
`directories::ProjectDirs` would pick — so the two agree rather than one guessing.

**`--port` is 12358 when it is free.** When it is not, the shell first asks
`GET /healthz`: an engine already answering there is *adopted* rather than
duplicated, because two engines over one data directory is the failure worth
avoiding and is worse than not being the one who started it. That also makes
`cargo run` plus the shell a working dev setup by accident rather than by
special-casing. Anything else on the port and the shell takes an ephemeral one.

**A job object with `KILL_ON_JOB_CLOSE` is what stops the engine outliving the
shell.** Windows has no process groups and no orphan reaping — a child whose
parent dies is re-parented and keeps running. For a process holding the agent's
data directory that is the worst shape of failure: invisible, still writing, and
in the way of the next start. The job holds through a crash, a kill, and Task
Manager's *End Task*.

Shutdown is a kill. There is no console attached, so there is no Ctrl+C to send
and no windowless equivalent of `SIGTERM` — and the engine is built to be killed,
which is what makes resuming after a restart work at all.

## Connection

For a remote core, identical to iOS and Android, because the wire is the same:
present the credential to `POST /api/session`, install the returned short-lived
cookie in the WebView2 profile, then load the core's address.

**For the local engine there is no session exchange, and that is not an
omission.** The core's loopback listener is ungated by construction
([`../arch/topology.md`](../arch/topology.md#what-is-gated)), so exchanging a
credential to reach `127.0.0.1` would be the shell authenticating to an open
door. That reasoning is what deleted `crates/hi-app`; repeating the exchange here
would repeat the mistake in a second language.

The credential never enters JavaScript, the roster file, or WebView2's storage.

## Where Windows differs from the phones

Four things do not port, and each is solved in one named place.

**A cookie cannot be handed over verbatim.** iOS and Android install the raw
`Set-Cookie` line into their cookie stores unmodified, precisely so the core keeps
ownership of `Path`, `Max-Age` and `SameSite`. WebView2's `CookieManager` has no
raw-header entry point — only `CreateCookie(name, value, domain, path)` and
properties — so `CoreWebView.BuildCookie` parses the header and carries every
attribute across. The raw line is kept on the session so what was parsed can be
compared with what arrived. Anything the core adds later that is not read there
is silently dropped, which is why that method carries the longest comment in the
file.

**Autoplay is a browser flag, not a webview setting.** The media-gesture trap
that cost both phones a microphone exists here too — Chromium gates
`AudioContext`, the graph the mic runs through and the agent's voice comes out
of, behind a user gesture the face does not have on load. The fix is
`--autoplay-policy=no-user-gesture-required` in `CoreWebView2EnvironmentOptions`,
which means it must be set when the *environment* is created and cannot be
changed afterwards.

**A main-frame status is readable.** `CoreWebView2NavigationCompletedEventArgs`
reports `HttpStatusCode`, so a 401 is met by re-exchanging the credential rather
than by showing a rendered "unauthorized" body. Android has to reconstruct this
from `onReceivedHttpError`; here it is simply available.

**There is no per-app camera permission to check first.** Android grants capture
only for permissions the app itself holds; Windows privacy settings are a
system-wide switch the person owns, and a denial surfaces as a failed capture
rather than as something to pre-empt. So origin is the whole question at that
rung: camera and microphone are allowed for the attached core's exact scheme,
host and port, and denied otherwise.

## What is deliberately not here

**The mechanisms.** Screen capture, input synthesis, the accessibility tree,
`desktop_context`, the global hotkey and the press-hold gesture are all absent,
on purpose: they have nothing to talk back through until
[`../arch/mechanisms.md`](../arch/mechanisms.md) is built. That is one connection
— `WS /api/mechanisms`, dialed by the app — and when it exists this shell is the
natural first client, because it already owns the process that would answer.
Adding a Windows twin of each capability *before* the seam exists would mean
building them where they cannot be reached.

Until then a Windows install has no hands, which is a normal state rather than a
broken one: `available()` becomes a fact about who is attached, and a core with
no hands says it cannot see the screen right now.

**Notifications, a start-at-login entry, and Authenticode signing.** None are
built. Signing is the external gate — the Windows analog of the Developer ID
requirement — and it needs a certificate before it needs code.

## Build

```sh
make win-app     # publish the shell   (Windows host + .NET SDK 8 only)
make exe         # cross-compile the engine    (Mac mini / Linux)
make installer   # NSIS Setup.exe, carrying the shell when one has been built
```

`make installer` produces a working install either way. Without a shell the
shortcuts start `hi-agent.exe` and the person gets a headless core they open in
a browser; with one they start `HiAgent.exe` and the engine becomes its child.
One installer, two payload tiers.

The WebView2 Evergreen runtime is the only prerequisite left. It ships with
Windows 11 and arrived on Windows 10 with Edge, so it is rarely absent — and when
it is, the shell says so by name rather than reporting a generic failure to
start.

## Verification

**None of this has been compiled.** There is no Windows machine and no .NET SDK
on any host this repo is developed from, so the shell has never been through a
compiler, let alone run: the C# and XAML here are written the way the Phase 1
SwiftUI window was, blind and fix-forward. `make exe` and `make installer` are
verified to *build* on the Mac mini and have never been run on Windows either.

Two consequences worth stating rather than discovering. The package versions in
`HiAgentWindows.csproj` are wildcards, not pins — nothing has ever restored them,
and an exact version would be a guess that reads like a decision; pin them the
first time a Windows box succeeds. And the two API details most likely to be
wrong are `TaskbarIcon.IconSource` (H.NotifyIcon's image type) and
`CoreWebView2Environment.CreateWithOptionsAsync`'s signature, both flagged where
they are used.

## See also

[`../arch/topology.md`](../arch/topology.md) for the three roles and what an app
owns · [`../api/client.md`](../api/client.md) for the wire ·
[`../arch/mechanisms.md`](../arch/mechanisms.md) for the seam this shell is
waiting on · [`apple-macos.md`](apple-macos.md) for the same destination
approached from the other direction.
