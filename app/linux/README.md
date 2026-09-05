# Hi Agent Linux

The native Linux app: a client of a core, and the host of the one on this
machine.

This target owns the Linux shell and client state:

- the process, the GTK main loop, and the window;
- **the engine** — `hi-agent` runs as this app's supervised child, or is adopted
  when something is already answering on 12358;
- pairing and the local roster;
- long-lived credentials in the Secret Service (GNOME Keyring, via libsecret);
- short-lived `hi_surface` session cookies;
- health checks;
- the `WebKitWebView` that renders the core face.

It talks to a core over its HTTP API. It links exactly one crate from this
repository — [`hi-wire`](../../crates/hi-wire), the names an app and a core have
to agree on — and **never `hi-agent`**: a shared language makes "just call into
the engine crate" a one-line change, and that line would rebuild the macOS
main-thread inversion in a new place after a refactor spent removing it. It
shares no code with the Apple, Android or Windows clients.

See [`docs/platforms/linux.md`](../../docs/platforms/linux.md) for the target
table, why the shell may be Rust when its siblings link none, and the three
things that are Linux-specific.

## Build

```sh
make linux-app    # the shell
make linux-test   # its tests
make deb          # shell + engine, packaged
```

Build dependencies beyond the engine's own `cmake` + `libclang-dev`:

```sh
apt install libgtk-4-dev libadwaita-1-dev libwebkitgtk-6.0-dev libsecret-1-dev
```

**A Debian 13 or Ubuntu 26.04 host is not optional.** GTK4, libadwaita and
WebKitGTK have no cross build. Unlike `make win-app`, that is an ordinary
machine rather than one nobody here has.

The floor is deliberately high — GTK 4.18, libadwaita 1.7, WebKitGTK 2.52 — and
that is what buys the absence of version guards anywhere in this crate. Dropping
Ubuntu 24.04 LTS is what pays for it.

## Running it from a checkout

```sh
cargo run --manifest-path app/linux/Cargo.toml
```

There is no engine beside the shell in a checkout, so this finds `hi-agent` on
`$PATH` — or, when `make dev` is already running, adopts the engine answering on
12358 and starts nothing. The adoption path is the dev setup by accident rather
than by special-casing.

## First slice

1. Starting, supervising and shutting down the local engine, with its data
   directory outside the install location.
2. Adopting an engine already answering on 12358 — a `systemd --user` unit, a
   `make dev`, or a crashed shell's orphan — instead of starting a second.
3. `PR_SET_PDEATHSIG` on an engine this shell started, and never on an adopted
   one.
4. Loading the local core's face with no session exchange, because loopback is
   ungated.
5. Adding a remote core by address and pairing code.
6. Keeping the credential in the Secret Service.
7. Checking whether cores answer, and saying so when one stops.
8. Switching between cores from the primary menu.
9. Renewing rejected or expired web sessions without exposing the credential to
   JavaScript.
10. Granting camera and microphone capture only to the attached core's exact web
    origin.
11. A header bar that matches the face's own background in both appearances.

## What is deliberately not here

**A tray icon.** GNOME has no built-in StatusNotifierItem and has said it does
not plan to support AppIndicators; Ubuntu enables the AppIndicator extension by
default and Debian's stock GNOME does not. So closing the window quits the
shell, the primary menu is the app's presence while it is open, and liveness
with no window open is the `systemd --user` unit's job. A tray built on a
D-Bus SNI implementation that may find nothing listening is not worth writing
blind.

**Desktop notifications.** They are the universal replacement for the "come and
see" the macOS tray flash carries — but nothing in the engine can ask an app for
anything yet. That is [`docs/arch/mechanisms.md`](../../docs/arch/mechanisms.md),
and it is unbuilt.

**Every capability mechanism** — screen capture, input synthesis, the
accessibility tree, `desktop_context`, global shortcuts — for the same reason
`app/windows` omits them: they have nothing to talk back through. Worth
recording for when that seam is designed: on Linux these are portal *sessions*,
not calls, which is what makes "capability is a fact about who is attached, and
mutable while running" unavoidable rather than merely correct.

**The press-hold attention gesture.** `GlobalShortcuts` emits
`Activated`/`Deactivated`, so press-and-hold edges survive — but only for a
bound accelerator, and a bare modifier is not one. ⌘-held has no Linux spelling:
Super belongs to the compositor. That is a product decision, not a port.

**A native Settings window.** macOS built one because Phase 1 needed a
presentational surface to prove the config API boundary and because TCC grants
had to be brokered somewhere native. Neither reason exists here. Settings stay
in the web face, where they are already cross-platform.

## Status

Compiled and **run** on Debian 13 (GTK 4.18.6 / libadwaita 1.7.6 / WebKitGTK
2.52.6), which makes this the only shell in the target shape that has been
watched working. See [`docs/platforms/linux.md`](../../docs/platforms/linux.md)
§ *Verification* for exactly what was watched and what was not — the split
matters here. In particular: **no remote core has ever been paired from this
shell**, because a headless box has no unlocked keyring, so
[`src/core/credentials.rs`](src/core/credentials.rs) and the exchange path in
[`src/model.rs`](src/model.rs) are written and unexercised. Mic and camera are
in the same state.
