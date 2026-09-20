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
- the notification-area icon, which is the app's presence when no window is open
  — and which says what state the agent is in, because the window that also says
  so can be closed.

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

**Adding a remote agent leads with its name**, matching iOS, Android and the GTK shell:
`AddAgentWindow` takes a name, calls `POST /api/access/request`, shows the six-digit code it
gets back, and polls until somebody approves it on the agent. The address-and-code form is
folded into an `Expander` for a self-hosted core with no name in the default zone. One
button drives both, because only one of the two is ever the thing being filled in.

Written 2026-09-11 and, like everything else here, **never compiled** — see § *Verification*.

## Settings

`Views/SettingsWindow.xaml` and `Core/SettingsClient.cs`. Before it there was
nowhere on Windows to change the credential mode, enter a key, turn the relay on
or read the version: the engine's config store was reachable only by editing it
by hand.

It is a **client of the engine's config API** — `GET /api/settings` and four
small PUTs, the boundary [`../core-shell-config-api.md`](../core-shell-config-api.md)
defines — which is the same thing `HiSettings.swift` is on macOS, and the panes
are that window's panes: General, Account, Reach, About, in that order and in its
words. It holds no state of its own, so there is no second copy of a setting to
drift from `config.db`; a key can be written and never read back, because the
read surface carries `configured: bool` and no secret. Nothing about it is a
Windows invention except the sidebar, which is a `NavigationView` because that is
what a settings window is here.

**It configures the agent on this computer, and only that one.** The config
routes are loopback-gated at the engine, and rightly: they read and write
credentials. So the window points at `LocalCore`'s address rather than at
whichever core the face happens to be showing — a remote agent's credentials,
energy and reachability belong to the machine hosting it. With no local engine
there is nothing to open, and the tray greys the item rather than opening an
empty window.

One thing it does beyond persisting: **theme.** "Core persists, shell applies" —
the engine stores the choice and `Ui/Theme.cs` puts it on each window's root
element. That is not decoration. The face paints `--bg-1` directly under the
title bar and `MainWindow.ApplyTitleBarTheme` matches the bar to it off the
*system* theme, so a stored "dark" under a light Windows would draw a pale strip
across the top of a dark page — the exact seam that code exists to close. The
shell therefore reads the stored theme once the engine answers, at every start.

## What the tray says

macOS puts text beside the menu-bar icon when the engine will not come up —
`⚠ needs setup`, `⚠ startup failed` ([`../../src/lib.rs`](../../src/lib.rs)). A
notification-area icon has no text beside it, so the same fact arrives three
ways: the tooltip (`Hi Agent — starting the agent…`), the first line of the menu,
and the icon itself. Hovering is a choice; the colour is the part that is not.

**Three states, the menu bar's three:** the mark in colour while the agent
answers, drained of colour while it does not, and knocked out of coral while it
has its ear open — somebody is holding the attention key, which on this platform
is the right Ctrl. The third wins over the other two, because an agent that is
hearing you is plainly there, and while you are holding a key the only question
you have is whether it is listening.

All three are **one mark**, the other two derived from the first by
[`../../scripts/make-tray-icos.py`](../../scripts/make-tray-icos.py) at every size
the icon carries, so changing the logo cannot leave the states showing different
marks. Neither derivation draws anything: grey is the mark desaturated and
lifted, listening is the same silhouette knocked out of the brand coral — a
filled tile where the other two are white, which is the one difference that
survives 16px, where a badge or an outline does not.

**Where listening comes from is `GET /api/listening`**, held open by
`Core/ListeningWatch.cs` against the engine *on this machine* — the key is on this
keyboard and no remote agent can hear it. It is a subscription rather than a poll
because a hold lasts a second or two, and it is a fact read off the core rather
than a push at this app because a menu bar, a notification area and a browser face
are all asking the same question
([`../arch/mechanisms.md`](../arch/mechanisms.md) Open 2, now answered).

Listening is the only state that is not also in the menu, and deliberately: a hold
cannot be held while opening a menu, so a header for it would be a line nobody
could ever see. It is in the icon and the tooltip.

The sentence is the one the window would show, which is where it comes from:
`AppModel.StageDetail` when the model has one — it is already written for a
person — and the stage's own words otherwise. Nothing is worded twice.

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

**The mechanisms.** Screen capture, input synthesis, the accessibility tree and
`desktop_context` are absent from this shell because they are absent from the
core: all four were deleted rather than re-homed, and driving a machine is now a
note over the tools that machine already has
([`../arch/mechanisms.md`](../arch/mechanisms.md) § *Computer use does not cross
this seam*). So a Windows install has no hands, which is a normal state rather
than a broken one.

**The attention gesture is not on that list any more, and only two thirds of it
works.** The key tap and the microphone are in the engine — `WH_KEYBOARD_LL` on
the right Ctrl, cpal on WASAPI — so **press-and-hold listens on Windows today**,
and the tray says so. What does not work is the other two gestures, because both
end in something only this shell holds:

| Gesture | On Windows | Waiting on |
|---|---|---|
| press-and-hold → listen | works | — |
| single tap → open the chat | recognized, lands on nothing | raising a window is the shell's |
| double tap → hand over a screenshot | recognized, logged, lands on nothing | a screen grab is the shell's |

Both are one connection away — `WS /api/mechanisms`, dialed by the app — and when
it exists this shell is the natural first client, because it already owns the
process, the window and the window server. The key tap moving here with them is a
question rather than a plan: it needs no grant on Windows, which is the whole
reason the rule puts a mechanism in the shell (`mechanisms.md` Open 4).

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

**Not yet compiled end to end — but no longer untouched.** There is no Windows
machine among the hosts this repo is developed from, so the C# and XAML here
were written the way the Phase 1 SwiftUI window was: blind and fix-forward.
What changed on 2026-09-10/11 is that `.github/workflows/release.yml` builds
this project on a hosted `windows-latest` runner, so the v0.1.0 release runs
became the first Windows box ever to try. Each run got one stage further, and
each failure is recorded here because a release is an expensive place to
discover them.

| Run | Reached | Stopped on |
|---|---|---|
| 1 | NuGet restore | `NU1202` — `H.NotifyIcon.WinUI 2.*` had floated to 2.4.1, which ships only net10.0 |
| 2 | C# compile | `CS1729` — `CoreWebView2EnvironmentOptions` takes no constructor arguments |
| 3 | packaging | `MSB4062` — `Microsoft.Build.Packaging.Pri.Tasks.dll` not found under SDK 10.0.400 |

**Windows was not in 0.1.0.** Its job was commented out rather than held for,
because what stopped it being worth a release delay is not the three errors
above, which are ordinary; it is that nobody knows how many stages sit behind
packaging, on a surface that has never produced a running window.

**It was taken back in on 2026-09-20, on a dry run rather than a release.** The
run-3 fix — the repo-root `global.json` pinning the SDK 8.0 band — had still
never been tried, so the next run starts at packaging rather than at the top,
and `workflow_dispatch` with `publish=false` is the way to learn that without
tagging anything: no tag, no draft, each platform job asserting only that its
artifact exists. Run 4 onward is recorded in the table above.

Deciding that also cleared up what the Mac mini can and cannot stand in for.
`make installer` there is verified again as of `fc61c11` / 0.1.2 — the engine
cross-compiles and links in 48 s and NSIS produces a 22 MB Setup.exe — but it
prints `no WinUI shell … building the engine-only installer` and means it. That
installer's shortcuts start `hi-agent.exe`, so it installs a headless core to
open in a browser. **The Mac mini cannot host this job**: `make win-app` needs
a real Windows host, and every error runs 1–3 found was in that half.

Three lessons, each now fixed in the general form rather than the specific one.

**Wildcards retarget an app nobody compiles.** `2.*` moved to a release that
dropped net8.0 and the project went with it, silently, until a release run
said so. All three package versions are pinned now — `H.NotifyIcon.WinUI` to
2.3.2, the newest still shipping a net8.0 lib.

**The bundled WebView2 is not the standalone one.** The four-argument
`CoreWebView2EnvironmentOptions` constructor belongs to
`Microsoft.Web.WebView2`; the copy inside the Windows App SDK has a
parameterless constructor and `AdditionalBrowserArguments` as a property. The
`XamlCompiler.exe` MSB3073 that rode along behind this error turned out to be
a cascade — it cleared when the C# did.

**The SDK the runner happens to ship is not the SDK this targets.**
`windows-latest` preinstalls the current .NET SDK (10.0.400 on run 3), and
`dotnet` picks the highest one installed unless told otherwise. WindowsAppSDK
1.6's packaging targets look for their task assembly where an SDK 8 lays it
out. The repo root now carries a `global.json` pinning the 8.0 band, which
binds a local `make win-app` as much as CI, and the workflow installs that
band so the pin can be satisfied.

**What is verified is therefore: restore succeeds, the C# compiles, and the
XAML compiles** — as of run 3, and of the code that existed then. Packaging
does not, yet. Nothing has linked, nothing has run, and no window has ever
appeared. `TaskbarIcon.IconSource` — the other API flagged as probably wrong
where it is used — remains unsettled, along with every runtime question behind
it: whether the tray appears, whether the WebView loads the face, whether the
engine child is adopted and dies with its parent. `make exe` and `make
installer` are verified to *build* on the Mac mini and have never been run on
Windows either.

**Settings, the tray's state, and the listening state added after them have not
met even that much** — all were written after run 3 and no run has happened since.
Four things about them are worth knowing before the next one:

- `TaskbarIcon.IconSource` is now load-bearing rather than cosmetic: it is
  assigned on every state change, not once at startup. If that property turns
  out to be the wrong shape, what breaks is the state signal and not just the
  picture.
- `ms-appx:///Assets/…` is resolved three times now, for three icons. It has never
  been resolved once — the app is unpackaged (`WindowsPackageType=None`), where
  WinUI maps those URIs to the install directory, and nothing has confirmed that.
- The Settings window's controls are filled from the engine's own snapshot, so
  the first real question it can answer is whether `GET /api/settings` reaches a
  Windows client at all — one call, before any of the writes matter.
- `ListeningWatch` is the first thing here that holds a connection open rather
  than making a request, and it keeps its own `HttpClient` to do it, because
  `CoreClient.Http`'s 20-second timeout would abort the subscription on a clock.
  Nothing has confirmed that `HttpClient` streams SSE the way this assumes, or
  that `ReadLineAsync` sees a line before the buffer it is reading from fills.

**The engine half is a different story and is verified further.** `make exe`
cross-compiles and *links* a real `x86_64-pc-windows-msvc` binary on the Mac mini,
and as of 2026-09-18 that binary contains the keyboard hook, cpal's WASAPI capture
and `GET /api/listening`. So the Rust side is known to compile, link and typecheck
its own Windows-only tests; what is unknown is everything that happens when it
runs — whether the hook receives an edge, whether WASAPI opens the default input,
and whether a person holding the right Ctrl is heard.

## See also

[`../arch/topology.md`](../arch/topology.md) for the three roles and what an app
owns · [`../api/client.md`](../api/client.md) for the wire ·
[`../arch/mechanisms.md`](../arch/mechanisms.md) for the seam this shell is
waiting on · [`apple-macos.md`](apple-macos.md) for the same destination
approached from the other direction.
