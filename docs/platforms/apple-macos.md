# Apple macOS App

The macOS app lives at `app/apple/macos`. Unlike its two siblings it is **not a
standalone build yet** — the directory holds the parts of the macOS app that have
already been separated from the engine, and the rest is still Rust inside the core
binary. Read that as a migration in progress, not as a description of a finished
client.

## What is here today

| Path | What it is |
|---|---|
| `HiSettings.swift` | The SwiftUI Settings window. Already a pure client of the engine's loopback config API — it holds no engine state and reaches into no Rust. |
| `Info.plist` | The `.app` bundle's identity: the version pair `make bump-version` stamps, and the `NSCamera`/`NSMicrophone` usage strings TCC needs before it will present a prompt. |
| `HiAgent.icns` | The app icon. |
| `hi-agent.entitlements` | What the signed app is allowed to do. |
| `dmg/` | The styled disk-image background. |

`scripts/make-dmg.sh` and `scripts/make-app.sh` assemble these into a bundle; they
stay in `scripts/` because they are how the app is built, not what it is.

## What is still in the engine

Everything needing the OS session: the tray, the popover, the face window, the
global hotkey tap, input synthesis, screencast, `desktop_context`, accessibility,
audio capture, and the press-hold gesture machine — `src/foundation/vendors/macos_*.rs`
and `src/body/gesture.rs`. AppKit owns the main thread through `run_with_tray`
(`src/lib.rs`), with the server on a background thread.

That is the arrangement `CLAUDE.md` § *UI architecture* is committed to undoing.

## Why `HiSettings.swift` moved first

It is the only piece already in the target shape. It talks to the engine over
`http://127.0.0.1`, not through FFI into engine state; the single `hi_settings_open`
entry point exists so a Rust-owned tray can open it, and it disappears when Swift
owns `NSApplication`.

So `build.rs` still compiles this file with `swiftc` and links the archive into the
core binary. **That link is the leftover, not the design** — the file is where it
belongs, and it is the Rust side of the seam that has yet to move.

## What has to exist before the rest can follow

The engine↔shell **perceive/act protocol**, which has no design. Config is already
request/response over the API in [core-shell-config-api.md](../core-shell-config-api.md);
what is missing is the streaming half — microphone PCM and screen frames — plus the
engine→shell direction for act requests. Until that exists the capability mechanisms
above cannot leave the engine, because they have nothing to talk back through.

## Not part of this app

- `crates/hi-app` is the cross-platform **app role** (roster + loopback proxy) that
  the core binary mounts on a second port. It is Rust, it is not macOS, and it does
  not move here. The shared name is unfortunate; see `docs/arch/topology.md`.
- `src/foundation/machine_id.rs`'s macOS arm reads `ioreg` over the CLI — no
  framework, no TCC — and is deliberately engine-side.
