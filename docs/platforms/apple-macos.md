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
global hotkey tap, audio capture, the ⌘-glance screen grab, and the press-hold gesture
machine — `src/foundation/vendors/macos_*.rs` and `src/body/gesture.rs`. AppKit owns the
main thread through `run_with_tray` (`src/lib.rs`), with the server on a background thread.

**Two of those are no longer Mac-shaped, and reading them as such is now a mistake.**
`src/body/gesture.rs` is the cross-platform gesture machine — Windows binds the right Ctrl
to the same three gestures — and audio capture is cpal
(`vendors/cpal_audio_capture.rs`), one portable dependency rather than a `macos_*` file.
What stays Mac-specific about them is the tap (`macos_hotkey.rs`) and the Microphone TCC
grant, which is the reason the mic is on Phase 2's list here and on nobody's list on
Windows.

**The list used to be longer, and shrank by deletion rather than by migration.** Input
synthesis, the accessibility tree and `desktop_context` are gone — driving a machine is a
note over that machine's own tools, so there is nothing left to re-home
([`../arch/mechanisms.md`](../arch/mechanisms.md#computer-use-does-not-cross-this-seam)).
The one capture that remains is not the agent looking: the person double-taps ⌘ and hands
over a screenshot, and `screencapture` is called straight from the gesture rather than
through a capability. Phase 2 is that much smaller.

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

**Mechanism calls** — the one seam where the core does the asking, designed in
[../arch/mechanisms.md](../arch/mechanisms.md) and built nowhere. Until an app can answer
`screen.grab` and `input.perform`, the capabilities above cannot leave the engine, because
they would have nothing to talk back through.

The half that is *not* missing is worth knowing before anyone plans for it: microphone and
camera already have cross-platform inbound endpoints (`WS /api/in/audio/stream`,
`WS /api/in/vision/stream`) that the web face uses today, so a shell that captures mic bytes
is a new client of an existing socket, not new protocol.

## Not part of this app

- The roster and the credential exchange, when the desktop grows them. Deleting
  `crates/hi-app` took the desktop's second port and its roster with it, so today
  the face loads `http://127.0.0.1:<port>/` and there is one core. Attaching to a
  remote core comes back as Swift, alongside the iOS one — `CoreClient.swift` is
  the shape to copy.
- `src/foundation/machine_id.rs`'s macOS arm reads `ioreg` over the CLI — no
  framework, no TCC — and is deliberately engine-side.
