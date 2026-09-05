# Linux App

`app/linux` is built, and — alone among the shells written in this shape — it
has been **run**. This document was written before the code the way
[`windows.md`](windows.md) was written before a compiler, so that the decisions
were settled in one place rather than rediscovered in a GTK callback.
§ *Verification* records which of them survived contact.

The Linux app is a standalone GTK4 / libadwaita build that speaks the core's
documented HTTP API. It is the **second shell written in the target shape** —
the shell owns the process and the engine is its child — after
[`windows.md`](windows.md), and for the same reason: there has never been a Linux
shell to migrate. Every macOS-native crate is `cfg`-gated, every OS capability
already answers `bail!("not supported")`, and `main.rs` routes a non-macOS start
to the plain server path.

So the engine half was not work. `Dockerfile` stage 2 is `rust:1-trixie` and its
runtime stage is `debian:trixie-slim` — **that is already a Debian 13 build of
the core**, minus a display. What was missing was a face, a package, and a way to
start; those are `app/linux`, `scripts/make-deb.sh` and
`data/hi-agent.service`.

## Targets

Latest-only, deliberately. The floor is Debian stable, which is older than the
current Ubuntu LTS.

| | Debian 13 trixie | Ubuntu 26.04 LTS |
|---|---|---|
| GNOME Shell | 48.7 | 50 |
| `xdg-desktop-portal` | 1.20.3 | 1.21+ |
| GTK4 / libadwaita | 4.18.6 / 1.7.6 | newer |
| WebKitGTK 6.0 | 2.52.6 | newer |
| Session | Wayland by default | **Wayland only** — the X11 GNOME session is gone from GDM |

Dropping Ubuntu 24.04 LTS is what buys the simplification: its GTK 4.14 and
WebKitGTK 2.44 were the only reason the shell would have needed version guards.
At a 4.18 / 2.52 floor there are none, and WebKitGTK's 2025 camera-portal work is
already in, so `getUserMedia` needs no GStreamer device-monitor fallback.

**Two GNOME versions still straddle, in exactly one place.** Portal 1.20 derives
an app's identity from its systemd scope; portal 1.21 requires
`Registry.Register`. That difference is only visible to `GlobalShortcuts`, which
belongs to a phase that cannot start yet — so for everything buildable today the
two targets are one.

**Portals or nothing.** Ubuntu 26.04 removed the X11 GNOME session, so an XCB
path would be code for a session that no longer exists on the newest LTS. A
capability with no portal is unavailable, which is what
[`../arch/mechanisms.md`](../arch/mechanisms.md) already says `available()` should
come to mean.

## The one way it differs from its siblings: it may link Rust

iOS, Android and Windows link no Rust and share nothing, on purpose. This shell
is Rust, and links exactly one crate: [`hi-wire`](../../crates/hi-wire), whose
whole reason for existing is that an app and a core must agree on `hi_surface`
and `x-hi-surface` and neither side can change them alone. Swift and Kotlin
retell those names because they cannot do otherwise; this is the first shell that
can share them instead.

**It links `hi-wire` and nothing else. Never `hi-agent`.** The hazard is
specific: a Rust shell makes "just call into the engine crate" a one-line change,
and that line would rebuild the macOS main-thread inversion in a new place after
we spent a refactor removing it. The process boundary is the design, not an
implementation detail, and a shared language must not be allowed to erode it.

Rust rather than C is a GTK-side judgment, not a consequence of the core's
language — GNOME supports C, C++, GJS, Python, Rust and Vala with no official
pick, and gtk-rs is a first-class binding hosted on GNOME's own infrastructure.
The honest argument for C was that it makes the boundary above self-enforcing;
the rule replaces it at lower cost than writing GTK in C.

## Ownership

The Linux app owns:

- its roster and current-core selection;
- the long-lived credential, in the Secret Service (GNOME Keyring via libsecret)
  — never a dotfile, never `localStorage`;
- session exchange and health checks for remote cores;
- the `WebKitWebView` lifecycle;
- **the engine process** — starting it, adopting it, restarting it, killing it;
- presence when no window is open, which on this platform is the hard part.

The core remains the authority for identity, credential issuance and revocation,
memory, cognition, and channel behaviour.

## Hosting the engine

**`--data-dir` is passed explicitly, for the reason `windows.md` gives.**
`default_data_dir` in `src/main.rs` reaches for the OS data directory only when
`bundle::resources_dir()` says it is inside a macOS `.app`; everywhere else it
falls back to `./data`, relative to the working directory. A `.desktop` launch
has no meaningful working directory, so an installed engine would scatter the
person's memory wherever the launcher happened to start. The shell passes
`~/.local/share/hi-agent` — the path `directories::ProjectDirs` picks on Linux —
so the two agree rather than one guessing.

**`--port` is 12358 when it is free, and an engine already answering `GET
/healthz` there is adopted rather than duplicated.** Two engines over one data
directory is the failure worth avoiding. This is the Windows rule unchanged, and
on Linux it earns a second job (below).

**`PR_SET_PDEATHSIG` is what stops the engine outliving the shell.** Windows
needed a job object because it has no orphan reaping; Linux has the signal built
in, so a child whose shell dies — crash, kill, or a clean quit — gets `SIGTERM`
without the shell having to survive long enough to send it. The engine is built
to be killed; that is what makes resuming after a restart work at all.

### The systemd option, and why adoption makes it not a fork

macOS keeps the agent alive with a menu-bar item. **Stock GNOME has no tray**, so
"close the window and the agent stops living" is a real failure mode here and not
a cosmetic one — pulses, duties and task resumption all stop with it.

The Linux answer is a `systemd --user` unit: started at login, restarted on
crash, alive with no window open. That looks like a divergence from the
shell-owns-the-process shape the other two desktops are converging on, and it is
not, because **adoption already reconciles them**. The shell's rule is: if
something is answering `/healthz`, attach to it; otherwise start one as a child
with `PR_SET_PDEATHSIG`. A systemd-managed engine is simply the first branch. The
shell never needs to know which world it is in, and the same code is a working
`cargo run` dev setup by accident rather than by special-casing.

The one thing that must not be got wrong: **`PR_SET_PDEATHSIG` is set only on an
engine this shell started.** Killing an adopted engine on window close would make
the unit pointless.

So the unit is optional and the shell works without it. It is the recommended
install because it is the answer to the tray, not because the architecture
changed.

## Connection

For a remote core, identical to iOS, Android and Windows, because the wire is the
same: present the credential to `POST /api/session`, install the returned
short-lived cookie in the `WebKitCookieManager`, then load the core's address.

**For the local engine there is no session exchange, and that is not an
omission.** The core's loopback listener is ungated by construction
([`../arch/topology.md`](../arch/topology.md#what-is-gated)), so exchanging a
credential to reach `127.0.0.1` would be the shell authenticating to an open
door. That reasoning is what deleted `crates/hi-app`; repeating the exchange here
would repeat the mistake in a third language.

**The cookie survives verbatim, which it does not on Windows.** WebView2's
`CookieManager` has no raw-header entry point, so `CoreWebView.BuildCookie` has
to parse `Set-Cookie` and carry each attribute across by hand — the longest
comment in that file, and a silent-drop risk whenever the core adds an attribute.
libsoup parses the header for us — `soup_cookies_from_response` does the whole
message at once, with the request URI as the cookie's origin — and
`webkit_cookie_manager_add_cookie` takes the resulting `SoupCookie`, so the core
keeps ownership of `Path`, `Max-Age` and `SameSite` exactly as it does on the
phones. This is the one place Linux has the easier job.

**Emptying the jar is where it pays that back.** Detaching from a core has to
clear the previous core's cookie, and WebKitGTK 6.0 has no bulk delete: the 2.x
`webkit_cookie_manager_delete_all_cookies` is gone and
`WebKitWebsiteDataManager` exposes no `clear` through the bindings, so it is
`all_cookies` then `delete_cookie` per entry. One line on Windows and both
phones; a loop here.

## Where Linux differs from the other desktops

**Presence is a window, not a tray.** GNOME has no built-in StatusNotifierItem
and has said it does not plan to support AppIndicators. Ubuntu ships and enables
the AppIndicator extension by default; Debian's stock GNOME does not — the
package exists (`gnome-shell-extension-appindicator`) but installing an extension
is not enabling it, and enabling it is a user action a `.deb` cannot perform. So
the systemd unit carries liveness, and desktop notifications — which are
universal — are what should carry the "come and see" acknowledgement that the
macOS tray flash carries today. **The tray-anchored popover does not port at
all**: there is nothing to anchor to.

**Two consequences settled while building it.** *Closing the window quits the
shell* — with no tray to retreat into, a held process with no window would be
invisible and unquittable, so the window is the presence and the unit is the
liveness. That costs nothing when the unit is installed, because the shell then
adopts rather than starts and quitting leaves the engine running. And *the tray
item is not built*: an SNI implementation that may find nothing listening on
either target is not worth writing, so the header bar's primary menu carries the
short list the macOS tray and `TrayIcon.cs` carry. **Notifications are not built
either**, and cannot be until the engine can originate a request to an app
([`../arch/mechanisms.md`](../arch/mechanisms.md)) — so the "come and see"
described above is a design position, not a shipped one.

**The media-gesture trap is here too, under a different name.** It cost both
phones a microphone and cost Windows an environment option: an engine gates
`AudioContext` — the graph the mic runs through and the agent's voice comes out
of — behind a user gesture the face does not have on load. On WebKitGTK the fix
is `WebKitSettings:media-playback-requires-user-gesture` set to `FALSE`. Cheap,
and invisible until someone wonders why the agent has no voice.

**An unhandled permission request is a denial.** `WebKitUserMediaPermissionRequest`
is denied by default if the `permission-request` signal goes unhandled, so
handling it is not a refinement — it is the entire mic and camera implementation.
There is no TCC and no per-app permission grant to check first, so origin is the
whole question at that rung: allow capture for the attached core's exact scheme,
host and port, and deny otherwise.

**A main-frame status is readable**, as on Windows and unlike Android: the main
resource's response carries the status code, so a 401 is met by re-exchanging the
credential rather than by rendering an "unauthorized" body.

## What is deliberately not here

**The mechanisms.** Screen capture, input synthesis, the accessibility tree and
`desktop_context` are absent for the reason `windows.md` gives: they have nothing
to talk back through until [`../arch/mechanisms.md`](../arch/mechanisms.md) is
built, and building them first would build them where they cannot be reached.

Worth recording for when that seam is designed, because Linux is the platform
that tests it hardest: **these are portal *sessions*, not calls.** ScreenCast and
RemoteDesktop are long-lived, user-consented, restore-token-carrying sessions,
where macOS could have got away with a stateless function behind a compile-time
`cfg`. Linux is what makes "capability is a fact about who is attached, and
mutable while running" unavoidable rather than merely correct.

Debian 13's `gnome.portal` declares every interface this will need —
`ScreenCast`, `RemoteDesktop`, `Screenshot`, `GlobalShortcuts`, `InputCapture`,
`Notification`, `Background`, `Settings` — so the platform is not the blocker;
the seam is. The one that stays hard afterwards is `desktop.context`: Wayland
gives an unprivileged client no way to ask which window has focus, and AT-SPI's
focused accessible is an approximation rather than an answer. AT-SPI is, on the
other hand, a better `ax.inspect` than macOS AX.

**The press-hold attention gesture, in its current form.** The
`GlobalShortcuts` portal does emit `Activated`/`Deactivated`, so press-and-hold
edges survive — but only for a *bound accelerator*, and a bare modifier is not
one. ⌘-held has no Linux spelling: Super belongs to the compositor. The gesture
returns as a real chord or not at all, and that is a product decision rather than
a port.

**A native Settings window.** macOS built one because Phase 1 needed a
presentational surface to prove the config API boundary, and because TCC grants
had to be brokered somewhere native. Neither reason exists here. Settings stay in
the web face, where they are already cross-platform.

## Packaging

A `.deb`, consumed by both targets. GUI dependencies come from apt
(`libgtk-4-1`, `libadwaita-1-0`, `libwebkitgtk-6.0-4`); the engine's payload —
codex, esbuild, ffmpeg, the headless browser, the ONNX models — is **downloaded
on first run, not bundled**.

That is the opposite of the `.dmg`, and the difference is not taste. macOS
bundles because notarization requires every Mach-O inside the `.app` to be
co-signed, so the hermetic layout is a consequence of signing rather than a
distribution choice. Linux has no such requirement, first-run provisioning is the
platform norm, it keeps the package near 30 MB instead of near a gigabyte, and it
is the best-tested path in the codebase — every Docker core already takes it.

`--provision-into` and `HI_AGENT_BUNDLE_DIR` remain available if that is ever
reconsidered: a Linux bundle would need **no new detection code**, because the
shell can point the engine at a staged tree directly instead of deriving it from
an executable path the way the `.app` does.

Not chosen, and why: **Snap** is Ubuntu-only and its confinement fights an engine
whose job includes reading the person's files and spawning a runtime. **Flatpak**
has the same fight and is not installed by default on Ubuntu. Both would cost
more than the audience they add.

```sh
make linux-app   # build the shell   (Debian/Ubuntu host + GTK4 dev packages)
make linux-test  # its tests
make deb         # package shell + engine
```

Build dependencies beyond the engine's own `cmake` + `libclang-dev`:
`libgtk-4-dev`, `libadwaita-1-dev`, `libwebkitgtk-6.0-dev`, `libsecret-1-dev`.

`SKIP_ENGINE=1 make deb` packages the shell alone — an install that only ever
attaches to a core somewhere else, where the shell shows a stage message instead
of starting one. The engine half is the ordinary `make build`.

`app/linux` is **its own cargo workspace**, not a member of the engine's. The
engine's workspace is resolved on macOS by `make test` and `make dmg`, and a
member there would drag GTK4 and WebKitGTK into a resolve that has no chance of
linking. `hi-wire` is reached by path across that boundary, which is all a path
dependency needs — and it is why `check-version.sh` has a second lockfile to
check.

## Verification

**The development box turned out to be the target.** It is Debian 13 trixie with
root, so the wall this section used to describe — "there is not yet a host on
which it could be attempted" — was never real. `apt install libgtk-4-dev
libadwaita-1-dev libwebkitgtk-6.0-dev libsecret-1-dev` produced exactly the
versions in the target table (4.18.6 / 1.7.6 / 2.52.6), and the shell compiles,
links, runs, and is screenshot-verifiable there. That makes this the **only shell
in the target shape that has been watched running**; `app/windows` has still
never met a compiler.

Watched on 2026-09-05, under `Xvfb` on Debian 13, against a stand-in core that
answers `/healthz` and serves a page:

- **Adoption.** An engine already answering on 12358 is attached to, and the log
  says so. No second engine starts.
- **The face loads with no session exchange** for the local core, and
  `WebKitWebView` renders it. The stage flips to the face only once the load
  finishes.
- **`PR_SET_PDEATHSIG`.** `SIGKILL` on the shell — where nothing orderly can run
  — leaves the engine it started dead and the port released.
- **An adopted engine survives the shell quitting**, which is the rule the
  `systemd --user` unit depends on and the one thing that must not be got wrong.
- **`--data-dir`** arrives as `~/.local/share/hi-agent`, and the four XDG
  directories are honoured separately (config / state / data / cache).
- **The header bar matches `--bg-1`** in both appearances — sampled, not
  asserted: `srgb(43,39,32)` at the header in dark, `#ffffff` in light.
- **Single instance.** The app ID takes `dev.human-interface.HiAgent` as a
  session-bus name and a second launch exits without constructing a model. The
  hyphen is fine: D-Bus forbids hyphens in *interface* names, not in well-known
  *bus* names.
- **The `.deb` installs and uninstalls**, and `/usr/bin/hi-agent-shell` behaves
  the same as the one in `target/`.

Two things the run corrected that reading could not have:

- `webkit_cookie_manager_delete_all_cookies` **does not exist in WebKitGTK 6.0**.
  It went with the 2.x API and `WebKitWebsiteDataManager` has no `clear` in the
  binding, so emptying the jar is `all_cookies` followed by `delete_cookie` per
  entry. Windows and both phones get this in one line.
- `g_date_time_format` is not `strftime`. It rejects `%.3f`, and the fallback
  turned that into a blank timestamp column in every log line — the exact shape
  of failure that looks like a field nobody set.

**Still not verified, and not verifiable here:**

- **The portals.** They need a real GNOME session with real consent dialogs,
  which is the same GUI-session wall that blocks screencast and hotkey testing on
  macOS. Nothing in `app/linux` touches them yet.
- **The Secret Service.** A headless box has no unlocked login keyring, so
  `credentials::save` / `load` have never round-tripped. Everything watched above
  used the local core, which needs no credential. **A remote core has therefore
  never been paired from this shell** — the code path is written and unexercised.
- **The mic and camera.** `set_media_playback_requires_user_gesture(false)` and
  the `permission-request` handler are the fix and the whole implementation
  respectively, and neither has been exercised against a real capture device.
- **Ubuntu 26.04.** Everything above is Debian 13 only.
- **The repo's pinned toolchain.** Every build above used stable 1.98.1, not the
  1.97.1 that `rust-toolchain.toml` pins — the pinned one would not finish
  downloading on that box. The highest `rust-version` among the locked
  dependencies is 1.92, so the pin has room, but *that* build has not been run.

One trap to record now rather than discover: on portal 1.20 the `GlobalShortcuts`
app identity is derived from the systemd scope, so **launching from a terminal
may behave differently from launching via the `.desktop` entry**. A shortcut that
works in one and not the other is that, not a bug in the registration.

## See also

[`../arch/topology.md`](../arch/topology.md) for the three roles and what an app
owns · [`../api/client.md`](../api/client.md) for the wire ·
[`../arch/mechanisms.md`](../arch/mechanisms.md) for the seam this shell is
waiting on · [`windows.md`](windows.md) for the same shape one platform earlier ·
[`apple-macos.md`](apple-macos.md) for the destination approached from the other
direction.
