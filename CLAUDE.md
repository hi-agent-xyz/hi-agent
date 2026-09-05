# hi-agent

## Decide and proceed; don't gate on low-value questions

Bias to action. Make the engineering calls you can make yourself and start building in the same turn the approach is agreed — don't stack up confirmation questions or re-ask "go?". Just decide: sensible defaults, anything already implied by stated preferences or memory, and choices that are cheap to reverse. Reserve questions for forks that are genuinely consequential, hard to undo, or a matter of the user's preference — and batch those into a single ask. Never invent an option the user wouldn't want and then ask them to rule it out.

**`docs/arch/` is the goal state, not a description of what exists.** Never edit it to match the code. **A place where the code doesn't match it is the work, not a finding: align it and move on.** Do not write a paragraph explaining that the code does X while the design says Y; go change the code. Surface only three things: a conflict *inside* the design, something the design never specifies that the code must decide, or a change to the design itself (state it, then make it — in `docs/arch/`, not by diverging quietly).

**There is no status file, and `docs/arch/` still carries no status.** A design document that doubles as a status report goes stale in a way that makes readers distrust the design too — but the cure is not a second document describing the system beside the code, which is what `docs/status.md` became before it was deleted. **What is built is in `src/`; what it should be is in `docs/arch/`; how it got there is in `git log`.** A decision to *not* build something is design, so it belongs in the relevant `docs/arch/` file's § *Decisions* or § *Open* — not in a ledger of its own.

**The one thing none of those hold is whether a mechanism has ever been watched running**, and that now lives where it is acted on: [docs/user-journeys/](docs/user-journeys/), whose dated 实测 / 复测 sections are the record of what was observed against a live instance. Keep paying attention to the distinction — **built is not watched.** Nearly everything typechecks and passes tests; a dead soul seed, a write-only verb, a switchboard with no readers and a frame log that kept nothing all shipped green. **A mechanism that is described and absent typechecks.**

**Retired vocabulary — do not reintroduce.** arbiter · delegate · ask · surface · handoff · notify · spawn · see · alarm · WorkerId · ToolCallStub · FollowMailbox · `Address` (any form) · scene-as-address · Deliberation · `WORKER_SYSTEM_PROMPT` · `any_host` · the ACP session vocabulary · `pulse` · `check_in` · `back_in` · `(pulse)`. Each was retired **by deletion**, never by deprecation.

**And nothing in this host fires on a period or at a named time.** The last three names above were the timers, removed on their own measurements: the recurring glance-up (1819 self-given turns, 46% making no tool call at all), the check-in the host armed under an open-ended silence (399 firings, 11% producing speech), and `back_in`, the one Reaction set itself — 53 firings, and the work it waited on reported a median 1.2 minutes later, so what it bought was "still going, another five minutes" said just before the real answer. Every wake is now an event: input, mail, a worker's report, and one boot wake that is restart recovery. The single exception is the [upkeep sweep](docs/arch/host.md#the-upkeep-sweep), and the test it passes is the one to apply to anything else proposed here — **does the tick itself cost a turn, or only the case it finds?**

**Be specific, not literary.** Name the function, the file, and the failing condition — "nothing calls `take_pending(id)`, so messages sit in the inbox forever", not "it needs a drain". Metaphor is fine after the mechanism, never instead of it. And keep "the design says X" clearly separate from "the code currently does X".

**A fix is a change to a general idea, never a rule about the case that failed.** The cheap move after something goes wrong is to write down what should have happened *that time*. That rule fires on one shape, goes stale with it, and adds weight to exactly the documents — `src/identity/*.md` above all — that only work if they can be held whole. So before adding anything, go find out whether the idea is already there. On 2026-09-02, four rules drafted after a botched WeChat delivery turned out to be four narrower restatements of rules `cognition.md` already stated more generally — and that the same session had run straight past. **A rule already written and lost does not need a second copy; it needs to reach the case that beat it.** Re-aim or extend what states it, and fold enumerated instances into the axis they share rather than adding one more. When a rule genuinely is new, the axis is the rule and the incident is the evidence filed under it, never the other way round.

## What the refactor learned — rules that outlived it

Four disciplines paid for repeatedly, in mechanisms that looked done. Every one of them typechecked.

- **Delete rather than deprecate.** A compatibility path kept "just until the replacement lands" is the thing that quietly becomes permanent. Every mechanism this repo has retired went by deletion.
- **Real and unexercised is fine; described and absent is not.** Landing machinery before the thing it talks to exists is an acceptable intermediate state. A prompt, doc or comment naming a mechanism that does not exist is not — that is how a dead soul seed, a write-only verb, a switchboard with no readers and a frame log that kept nothing all shipped green.
- **Nothing may read as finished that isn't.** If something is on loan, wrong-shaped, or unverified, that fact belongs in the code comment **before** the commit lands, and in the journey doc if a live run is what would settle it. A wrong call that is written down is cheap to reverse; one that reads as done is not.
- **A temporarily wrong owner is allowed only if the loan is named.** Take it when the right owner has no code yet — but say so in the prompt, and name the item that takes it back. The loans that were never named are the ones that became permanent.

One build discipline goes with them: **each commit compiles standalone**, not just the final state of a branch. An intermediate that does not build is useless for bisecting, and that gap has been shipped here once already.

## Making changes: always in a worktree

Do all work for a task in its own fresh git worktree branched from `origin/main` — never edit the primary checkout directly. When the work is done and the user gives the go: commit, fetch + rebase, then push `<branch>:main`. Once the push lands, delete the worktree and its branch — never keep one around.

    git fetch origin
    # create a worktree off origin/main; make all changes there

    # --- when ready (after the user's "go") ---
    git fetch origin && git rebase origin/main
    git push origin <branch>:main

    # --- after the push lands: tear it down ---
    git worktree remove <path> && git branch -d <branch>

## Working alongside uncommitted changes

The working tree may hold the user's in-progress work that is unrelated to your task. Don't entangle with it: keep your changes in new files where possible, put additive config (e.g. a new Cargo dependency) in its own separate block rather than interleaved with theirs, and at commit time stage only the files/hunks your task owns — never `git add -A`. Leave their WIP untouched in the tree for them to commit.

## Building, running, testing: always through `make`

**The [Makefile](Makefile) is the only supported way to build, run, test, or package this repo.** Every such action has a target; use it. Do not hand-roll the underlying `cargo` / `npm` / `docker` / cross-compile invocation, and do not add a new way to build — if something needs building differently, add or change a target.

| Target | What it does |
|---|---|
| `make dev` | run rust + vite dev servers together (Ctrl-C stops both) |
| `make build` | `npm ci` + build SPA, then `cargo build --release` |
| `make run` | run the release binary |
| `make test` | `cargo test` + web tests |
| `make docker` | build the docker image |
| `make dmg` / `make app` | macOS `.dmg` / local ad-hoc-signed `.app` |
| `make exe` / `make installer` | Windows cross-compile build check / installer |
| `make win-app` | publish the WinUI shell — **needs a real Windows host** |
| `make linux-app` / `make linux-test` / `make deb` | the GTK shell, its tests, the `.deb` — **needs a Debian 13 / Ubuntu 26.04 host** (the dev box is one) |
| `make bump-version V=x.y.z` / `make version` | version stamps / cut a release |
| `make help` | the authoritative list — read it instead of guessing |

**A bare command failing is expected, and is not a bug to fix.** Each target does setup the raw command does not: `build`/`dmg`/`exe`/`installer` run `check-version` first; `cargo build --release` only embeds a *previously built* SPA, so on its own it silently produces a binary with a stale or empty `dist/`; `make exe` needs the empty-`c++.lib` shim, the Homebrew-LLVM `PATH`, and the `cargo xwin` env before the compiler will link. So if a hand-typed `cargo …` or `npm …` errors out, **that is the missing setup talking, not a defect** — don't investigate it, don't "repair" the tree, don't work around it. Re-run the `make` target. Only a failure of the `make` target itself is a real failure worth reporting or fixing.

Dev-server detail, since `make dev` is where most local work happens:
- **Rust backend** on `:12358` via `cargo watch -x 'run -- --port 12358'` (auto-rebuilds/restarts on Rust changes).
- **Vite dev server** on `:12359` (`npm run dev` in `src/appearance/web`) — this is the page you open in dev.
- The browser talks only to Vite (`:12359`), which proxies `/api/*` and `/generated/*` to the backend. Caveat: `cargo watch` restarts the backend on Rust edits, but **Vite config changes (`vite.config.ts`) are NOT hot-reloaded** — restart `make dev` (or just the Vite process) after editing it.

(Bare commands still appear below when describing *what the build does* — that is explanation of mechanism, not an instruction to run them.)

## Dev vs. prod serving (important)

The two environments serve the web app differently, and this asymmetry has bitten us before:
- **Prod**: `cargo build --release` bundles the built SPA (`src/appearance/web/dist/`) into the binary via `RustEmbed` ([src/appearance/embed.rs](src/appearance/embed.rs)). The Rust server serves `GET /`, `/assets/*`, and `/generated/*` all **same-origin**, and [index()](src/appearance/mod.rs) injects the import map into the HTML.
- **Dev**: Vite serves the page; the Rust `index()`/import-map injection does **not** run. Dev mirrors prod via the Vite proxy (`/generated`) + a serve-only import-map plugin ([vite.config.ts](src/appearance/web/vite.config.ts)). If a view 404s or its bare imports don't resolve in dev, suspect this seam first.

Agent views are NOT self-contained bundles: the compiled `.mjs` keeps bare imports (`react`, direct shadcn paths such as `@/components/ui/card`, `@hi/core`, `motion/react`) resolved via the page import map to the host's shared instances — required so host and view share one React instance (hooks/context cross the boundary). Do not bundle these deps into views. See the shims in [src/appearance/web/src/shared/](src/appearance/web/src/shared/) and the direct component entries in [src/appearance/web/vite.config.ts](src/appearance/web/vite.config.ts).

## Deployment shapes

A **core** installs in two shapes:
1. **Docker on a server** — `make docker` builds the image; users run it server-side. Note that a published port is not loopback, so a Docker deployment is gated from first run (see § *Testing user journeys live* for the first-boot credential).
2. **Bundled desktop app** — `make dmg` builds the hermetic macOS `.dmg`; `make app` wraps the dev binary in a minimal ad-hoc-signed `.app` for local mic/camera testing. `make exe` / `make installer` are the Windows pair (they build; they have never been run on Windows). No Tauri and no Electron: the binary is its own shell.

**Surfaces are a separate axis from install shapes.** `app/apple/ios` and `app/android` are native clients — API clients holding no engine state; they attach to a core, they are not one. (`app/android` covers two device shapes from one build, a `mobile` flavor and a `tv` one, sharing the whole client layer and differing only in the shell — see [docs/platforms/android-tv.md](docs/platforms/android-tv.md) for why that split is a flavor and the iOS/Android split is not.) They share no code and are not meant to: they speak the wire in [docs/api/client.md](docs/api/client.md) and are otherwise unrelated builds. Each does the same thing with a credential: hold it in the OS keychain, exchange it once at `POST <core>/api/session`, put the returned cookie in its webview, and load the core's address directly. There is no local proxy — `crates/hi-app` was one and is deleted; see the App section of [docs/arch/topology.md](docs/arch/topology.md) for why it lost.

`app/windows` is the desktop's other client, and the **first one written in the target shape**: a standalone .NET/WinUI 3 build that owns the process and runs `hi-agent.exe` as its supervised child. That costs nothing extra because Windows never had a shell to migrate — every macOS-native crate is `cfg`-gated and `main.rs` already routes a non-macOS start to the plain server path. It has **never been compiled**: there is no Windows machine and no .NET SDK on any host this repo is developed from. See [docs/platforms/windows.md](docs/platforms/windows.md).

`app/linux` is the same shape again, and — unlike every other shell — it **has been run**. Targets are latest-only (Debian 13 + Ubuntu 26.04, so no version guards), the toolkit is GTK4/libadwaita with WebKitGTK, and the shell is Rust: the first one that can link [crates/hi-wire](crates/hi-wire) instead of retelling the names. **It links `hi-wire` and never `hi-agent`**: a shared language makes rebuilding the main-thread inversion a one-line mistake. It is **its own cargo workspace** for the same reason — a member of the engine's would drag GTK4 into a resolve that runs on macOS. Three things are Linux-specific: stock GNOME has no tray, so closing the window quits the shell and a `systemd --user` unit carries liveness (reconciled with the shell-owns-the-process shape by the same `/healthz` adoption rule Windows uses, and `PR_SET_PDEATHSIG` is set only on an engine this shell *started*); the payload is downloaded on first run rather than bundled, because signing — not distribution — is what makes the `.dmg` hermetic; and Ubuntu 26.04 is Wayland-only, so it is portals or nothing.

**The development box is Debian 13 with root, so it is the target** — `make linux-app`, `make linux-test` and `make deb` all work there, and a GTK4 window under `Xvfb` is screenshot-verifiable. [docs/platforms/linux.md](docs/platforms/linux.md) § *Verification* is the record of what has actually been watched (adoption, `PR_SET_PDEATHSIG`, the face loading, the `.deb` installing) and what has not — the Secret Service, the portals, mic/camera, and Ubuntu. **No remote core has ever been paired from this shell**: a headless box has no unlocked keyring, so that path is written and unexercised.

`app/apple/macos` is the same slot for the Mac, and it is **the only one that is not a standalone build**: it holds the `.app` bundle definition and the SwiftUI Settings window, while every macOS capability that needs the OS session is still Rust inside the core binary. It fills up as Phase 2 proceeds — see [docs/platforms/apple-macos.md](docs/platforms/apple-macos.md) for what is there and what still is not.

The managed runtime (codex + esbuild) auto-installs into the OS cache on first run, so a bundled app needs no separate runtime install. Both are native binaries that merely ship through the npm registry, so provisioning is an HTTPS GET of each platform tarball plus `tar -x` — **there is no Node and no npm** in this path. On a dev box with a pin-matching `codex` on PATH, the **system runtime** is used instead (esbuild is then provisioned separately — see [runtime::ensure_view_esbuild](src/runtime/mod.rs)). The agent is `codex app-server`, a native binary hi-agent talks JSON-RPC to over stdio.

(Node is still a *build-time* dev dependency for the web SPA — `make build` runs `npm ci` in `src/appearance/web`. That is unrelated to what an end user's machine downloads at runtime.)

## macOS entry shape (tray vs. headless)

On macOS the binary's default shape is a **desktop app**: AppKit owns the main thread and shows a menu-bar status item (Open / Quit), while the HTTP server + reaction run on a background thread ([run_with_tray](src/lib.rs); status item in [vendors/macos_tray.rs](src/foundation/vendors/macos_tray.rs)). Everywhere else (Linux/Docker) tokio keeps the main thread as before. Still one binary — this is the main-thread inversion the distribution model accepted as the cost of a tray; no shell crate, no Tauri.

The tray **auto-skips when `SSH_CONNECTION` is set** (no window server over SSH) or with `--no-tray`, falling back to the server-owns-main-thread path. So the SSH journey-testing command below is unchanged. The visible icon can only be tested from a real desktop session (same GUI-session wall as screencast/hotkey); over SSH you can verify compile, tests, and that startup logs `tray skipped (headless)` and still binds.

## UI architecture: headless engine + web face + native shell

**Decision (2026-07-07): the long-term target is a headless Rust *engine* supervised by a per-platform native *shell* that owns the process. The shell (SwiftUI/AppKit on macOS, XAML/WinUI on Windows, GTK on Linux) owns `main`, the run loop, and everything that touches the OS session; the Rust engine is pure cross-platform cognition + state and touches no platform GUI/OS APIs.** We accept a per-platform native cost for best-in-class native UX and a genuinely headless core.

This is not a new mode: the headless engine is *exactly the shape the app already compiles to on Linux/Docker* (server owns the thread, macOS crates `cfg`-gated). The refactor makes that the shape everywhere and deletes the macOS main-thread inversion (`run_with_tray`) from Rust, re-homing it in the shell.

### The three parts, by what each *is*

1. **Headless engine (Rust).** All state + logic: config, credentials/mode, energy, memory, and *all cognition* — vision model calls, STT/diarization, the reflex recognizer, and the biometric pipeline (face `buffalo_l`, voiceprint `CAM++`, clustering, `hi_name_person`/`hi_merge_people`). **Pure Rust: no objc2, no Apple frameworks.** ("Pure" = no platform-GUI code; it still links portable native deps — ONNX Runtime, ffmpeg — and spawns the codex runtime. Those build the same on every OS.) Runs **out-of-process as a sidecar** the shell spawns and supervises.
2. **Web face (webview in the shell).** The main content-heavy, fast-moving UI. Talks to the engine over the local API. Write-once cross-platform. (Precedent: the popover face is a `WKWebView`; native and web chat were both tried and rejected in its favor.)
3. **Native shell (per platform).** Owns the process and everything needing the OS session, in two roles:
   - **App-shell primitives** — run loop, tray, global hotkey tap, native windows, popover. Move to the shell.
   - **Native-presentational surfaces** — Settings and future preference windows, built in the platform's native UI toolkit (SwiftUI first) as **clients of the engine's local API** — not in-process C-ABI FFI.

### Mechanism vs policy — the rule that keeps the engine pure

Every OS-integration *capability* splits: the raw OS touch (**mechanism**) lives in the shell; the cross-platform brain (**policy**) stays in the engine and calls the mechanism over the API. Platform-specific code was always going to be written per-OS — the only question is *which process*, and the answer is "the one holding the session + grants" = the shell.

| Capability | Mechanism → **shell** | Policy → **engine** |
|---|---|---|
| Vision | grab frames | Doubao vision call, when-to-see |
| Screen-control / reflex | screen pixels, post keystroke, read AX tree | reflex recognizer, fire policy |
| Face / voice ID | camera / mic bytes | `buffalo_l` / `CAM++` ONNX, clustering, recognition |
| desktop_context | focused app / window query | how context feeds cognition |

The biometric/ML layer is **already correctly engine-resident and cross-platform** — it does not move. Camera/mic bytes for it may even arrive via the **browser web face** (`getUserMedia` → POST), so that capture is cross-platform too. Only capabilities needing the **window-server** (screen capture, input synthesis, AX, desktop_context) *must* live in the shell.

### The engine's new interface

The engine's outbound API grows from config CRUD by exactly one direction: **the engine has to be able to ask the shell for something.** That is designed in [docs/arch/mechanisms.md](docs/arch/mechanisms.md), and it turned out much smaller than this section used to claim.

This paragraph previously called it a bidirectional *streaming* protocol — "frames, audio are continuous" — and named it the biggest design object in the refactor. Two thirds of that was wrong. **Audio needs nothing new**: `WS /api/in/audio/stream` already exists and the browser mic already uses it, so a shell streaming PCM is the same endpoint with a different client. **Frames are not continuous**: perception is pulled ([surfaces.md](docs/arch/surfaces.md)), a still frame is the capability's irreducible primitive, and a live encode is cast-to-view — separate work. What is genuinely missing is **initiative**: nothing in the system lets the core originate a request to an app. One inversion on the existing app↔core wire, not a new protocol.

### Permission model (macOS; analogous elsewhere)

- **Engine = POSIX-only, no TCC.** Runs as the same UID as the shell, so it inherits plain file access (its data dir, user-chosen paths) for free. It requires *nothing* TCC-gated — the split is load-bearing, TCC inheritance is not.
- **Shell holds all TCC grants** (Screen Recording, Accessibility, Camera, Microphone, protected folders) and brokers them over the API.
- **Bundle + co-sign the engine inside the `.app`** (same pattern already used for codex/esbuild/ffmpeg; mandatory for Developer-ID notarization anyway). Spawn it by **bundle-relative path** (not the OS-cache auto-install path the runtime uses) so it launches under the app's responsible-process — free TCC inheritance *if ever needed*, as a safety margin, not a dependency.
- **Mic capture → shell** (resolves the one open item): keeps the engine 100% TCC-free rather than dragging a Microphone grant into it. `cpal`-in-engine was the only capability that could have stayed; the permission story tips it to the shell.

### Sequencing — two phases, don't flip ownership first

- **Phase 1 — Settings in hosted SwiftUI, Rust still owns the process.** Host a SwiftUI Settings window (via `NSHostingView` in a Rust-created window) talking to the loopback config/energy/mode API. Needs no OS grants, touches none of the hard-won tray/hotkey/capture code — proves the core↔UI API boundary at near-zero risk. **Define that config/energy/mode API boundary cleanly first, then build the client** — the spec is [docs/core-shell-config-api.md](docs/core-shell-config-api.md).
- **Phase 2 — flip ownership.** Swift owns `NSApplication`; Rust demoted to sidecar; port app-shell primitives + capability mechanisms to Swift; stand up the streaming perceive/act API. This is the big, GUI-wall-bound phase — do it last.

**Boundary rule going forward:** a capability's *mechanism* (OS touch) belongs in the shell; its *policy* (cross-platform logic) belongs in the engine. A new surface is API-client-native only if it's presentational. When unsure which bucket something falls in, that's a consequential fork — ask.

**Status: Phase 1 is built — do not rebuild it.** [vendors/macos_swift_settings.rs](src/foundation/vendors/macos_swift_settings.rs) bridges a SwiftUI Settings window ([app/apple/macos/HiSettings.swift](app/apple/macos/HiSettings.swift)) that reads and writes settings **over the loopback config API**, not via FFI into engine state; the only FFI is the single `hi_settings_open` entry point, which `build.rs` compiles and links on macOS. The objc2 window it replaced — and the BYOK `NSAlert` that window opened, whose only caller it was — are deleted; the one survivor is `apply_app_theme`, now in [vendors/macos_window.rs](src/foundation/vendors/macos_window.rs) beside the window-level theme read it must stay in step with. The native iOS client proves the same boundary from the other side.

**On Windows the shell already owns the process**, because there was nothing to flip: `app/windows` starts `hi-agent.exe` as a child, passes it `--port` and `--data-dir`, and holds it in a job object so it cannot outlive the shell. It has no capability mechanisms — those wait on the same seam macOS does — so what it demonstrates is the ownership arrangement, not the mechanism calls. It is also unbuilt in the strongest sense: never compiled, no Windows host exists.

**Phase 2 on macOS — flipping process ownership to Swift — is not started.** The seam it needs is now designed ([docs/arch/mechanisms.md](docs/arch/mechanisms.md)) and nothing implements it; that doc's § *Open* carries the questions deliberately left unresolved. **Latency is not among them** — a loopback round trip measured 0.012 ms for a call and 0.29 ms for a 2 MB screen grab on an M4, three to four orders of magnitude under the mechanisms themselves, so the process boundary is never the thing to optimize on this seam. Keep `TCP_NODELAY` on; it is the one detail that turns those microseconds into tens of milliseconds.

## Testing user journeys live (Mac mini)

Journeys in [docs/user-journeys/](docs/user-journeys/) are specs of *intended* behavior — test them against a real running instance, not by code-reading. Standing setup: clone at `~/projects/hi-agent` on the Mac mini (`ssh macmini`), `make build`, run from the repo root. Model credentials are no longer in `.env`: the default `xiaoyuanzhu` mode auto-bootstraps a broker account and mints the LLM key OOTB, so a fresh box just works; to force BYOK keys (or tune agent behaviour) headlessly, write into the config store (`sqlite3 data/config.db` — the `app_settings` KV holds the mode flag + cognition tunables; `credential` rows hold vendor keys) or set them in Settings. The `.env` now carries only infra knobs (auth, dirs, `RUST_LOG`, etc.):

    nohup ./target/release/hi-agent --port 12358 > server.log 2>&1 &

Talk to it over the text channel — Claude plays the boss; the human is only pulled in for account-side steps (QR/device auth, credentials) and for observing effects in external apps (e.g. what actually landed in the Feishu group):

    curl -N localhost:12358/api/out/text               # current state, then replacements
    curl -X POST --data-binary "..." localhost:12358/api/in/text

Method — the parts that keep the test honest:

- **Don't lead the witness.** Speak like a terse, normal boss; never script journey-expected behaviors into the prompt. Test recovery by *creating the situation* (kill its processes, restart the host, plant a failure) and watching — not by mentioning it.
- **Trust but verify every claim.** Ground truth lives outside the conversation: `server.log`, `GET /api/sessions`, the raw wire frames (`GET /api/wire/frames/events`, kept per session under `data/memory/raw/`— every JSON-RPC line in both directions, so a tool call shows what it actually ran), and its workspace artifacts/ledgers.
- **Keep the harness out of the experiment.** A watcher whose own command line contains the probe string becomes a decoy (`pgrep -f "[f]oo"` avoids self-match). `/api/out/text` is a single current-state subscription: reconnecting receives one whole latest snapshot, never a replay or fragment continuation.
- **The conversation is an append-only message list, and delivery state stays dead.** [`docs/arch/text-transcript.md`](docs/arch/text-transcript.md) is the accepted contract: one backend-owned list, journal-seeded so it survives restart, message ids for scrollback — and still no client identity, cursor, acknowledgement, read receipt, or per-window bookmark.
- **There is no pulse to speed up any more.** The recurring glance-up was removed: Cognition wakes once shortly after the process starts (restart recovery) and otherwise only on input, mail or a worker's report. A journey that used to be paced by shortening `pulse` has to be driven by *sending something* instead, which is what a person would have done anyway. `reflect_every` still paces consolidation if that is what a test needs.
- Findings go back into the journey doc (实测缺口 / 复测 sections). When behavior and journey disagree, that's a bug in one or the other — resolve explicitly.
