# One version for the binary, web app, desktop metadata, and published artifacts.
# VERSION is the source of truth; `make bump-version V=x.y.z` synchronizes the
# committed files that cannot read it directly at build time.
VERSION := $(shell cat VERSION)
VERSIONED_FILES := VERSION Cargo.toml Cargo.lock \
                   src/appearance/web/package.json src/appearance/web/package-lock.json \
                   app/apple/macos/Info.plist \
                   app/apple/ios/HiAgentIOS.xcodeproj/project.pbxproj \
                   app/android/app/build.gradle.kts \
                   app/windows/HiAgentWindows/HiAgentWindows.csproj \
                   app/linux/Cargo.toml app/linux/Cargo.lock

# Keep the former `VERSION=x.y.z` spelling working for callers while matching
# Abacad's public `V=x.y.z` interface.
BUMP_VERSION := $(strip $(if $(V),$(V),$(if $(filter command line,$(origin VERSION)),$(VERSION))))

.PHONY: help check-version build dev run test test-live test-views docker dmg app ios android android-apk exe win-app installer linux-app deb manifest bump-version version

# Windows target for the `exe` build check. MSVC (not gnu) because `ort`'s
# prebuilt ONNX Runtime ships for MSVC only.
WIN_TARGET := x86_64-pc-windows-msvc
WIN_SHIM   := $(CURDIR)/target/winshim
# Homebrew's LLVM (clang-cl / lld-link / llvm-lib) is keg-only, so prepend it on
# macOS; empty/harmless on Linux (use the distro's clang + lld + llvm there).
WIN_LLVM_BIN := $(shell brew --prefix llvm 2>/dev/null)/bin

help:
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-8s %s\n", $$1, $$2}'

check-version: ## verify every committed version stamp matches VERSION
	@./scripts/check-version.sh

build: check-version ## install web deps, build SPA, build release binary
	cd src/appearance/web && npm ci && npm run build
	cargo build --release

dev: ## run rust + vite dev servers (Ctrl-C stops both, incl. every child proc)
	./scripts/dev.sh

run: ## run the release binary
	./target/release/hi-agent

test: ## run rust + web tests
	cargo test
	$(MAKE) test-web
	$(MAKE) test-views

test-live: ## run the tests that need a real server (see each test for its env)
# Kept out of `test` rather than skipped inside it: these reach a third party over the
# network, so what they answer is whether a mechanism has ever been watched running —
# and that is a question someone asks deliberately, not one a test sweep answers by
# accident. Findings go in `docs/user-journeys/`.
#
# `--nocapture` is not a debugging flag here. What these tests answer is what a real
# server actually did, and cargo swallows a passing test's output — so without it the
# run reports "ok" and shows nothing, which is the same green this repo has been
# burned by. The observation is the point; it has to reach the person who asked.
	cargo test -- --ignored --nocapture

# The face's half on its own. Not a shortcut for the impatient: the client shells
# under `app/` change the face without touching a line of Rust, and the hosts they
# are developed on cannot always build the core — `cargo test` wants a toolchain
# and tens of gigabytes of `target/`, and a disk that runs out mid-build reports
# success while producing nothing.
test-views: ## run the factory views' pure read models (no browser, no Rust toolchain)
# These were written to be run and then were not reachable from any target, so nothing ran
# them: `home.test.mjs` evaluates everything above `export default function Home` in a bare
# VM and exercises the real layout engine. A test no target runs goes stale next to the code
# it guards, which is the whole argument for it being here.
#
# Same guard as `test-web`, for the same reason: they resolve `d3-flextree` out of the web
# app's `node_modules`, so a fresh worktree installs first.
	@test -d src/appearance/web/node_modules/d3-flextree || (cd src/appearance/web && npm ci)
	node --test src/mind/views/factory/*.test.mjs

test-web: ## run the web tests alone (no Rust toolchain needed)
# Guarded on the runner itself, not on `node_modules/` being present: a fresh
# worktree has no deps at all and a half-installed tree has the directory, so a
# `test -d` here is the check that passes while the thing it guards is missing.
	@test -x src/appearance/web/node_modules/.bin/vitest || (cd src/appearance/web && npm ci)
	cd src/appearance/web && npm test

# Not a test — it reads a live instance's wire log and reports what a week of
# conversation actually cost, which is the one thing `cargo test` can never
# answer. Lives here because `make help` is where this repo keeps its verbs.
measure: ## report reply latency and reply shape from a data dir's wire log (DATA=./data)
	python3 scripts/measure-replies.py $(if $(DATA),$(DATA),data)

docker: ## build the docker image
	docker build -t hi-agent:dev .

dmg: check-version ## build hi-agent-<version>-macos-apple-silicon.dmg
	./scripts/make-dmg.sh

app: ## wrap the dev binary in a minimal ad-hoc-signed Hi Agent.app for local mic/camera testing (macOS)
	./scripts/make-app.sh

ios: ## build the iPhone/iPad client for the simulator (requires macOS and Xcode)
	xcodebuild -project app/apple/ios/HiAgentIOS.xcodeproj \
		-scheme HiAgentIOS \
		-sdk iphonesimulator \
		-configuration Debug \
		CODE_SIGNING_ALLOWED=NO \
		build

android: check-version ## build + unit-test the Android handset client (debug APK; requires the Android SDK)
	cd app/android && ./gradlew --no-daemon assembleMobileDebug testMobileDebugUnitTest

android-apk: check-version ## build the unsigned handset release APK for self-hosted distribution
	cd app/android && ./gradlew --no-daemon assembleMobileRelease

android-tv: check-version ## build + unit-test the Android TV client (debug APK; requires the Android SDK)
	cd app/android && ./gradlew --no-daemon assembleTvDebug testTvDebugUnitTest

android-tv-apk: check-version ## build the unsigned Android TV release APK for self-hosted distribution
	cd app/android && ./gradlew --no-daemon assembleTvRelease

# `make exe` builds the Windows engine binary. What that means depends on the
# host, and the target hides the difference on purpose — `installer` asks for
# the .exe, not for a particular way of producing one.
#
# On a **Windows host** (the release CI runner) it is an ordinary native build:
# the MSVC toolchain is already the host toolchain, so none of the cross-compile
# scaffolding below applies.
#
# Everywhere else it is a cross-compile, and a *build check*: it proves the
# Windows code paths compile and link from a mac/linux box without running them.
# One-time toolchain on such a host:
#   rustup target add x86_64-pc-windows-msvc
#   cargo install cargo-xwin        # fetches the MSVC CRT + Windows SDK on first build
#   brew install llvm ninja         # macOS: clang-cl/lld-link/llvm-lib + ninja (knf-rs's cmake)
#                                    # Linux: install clang, lld, llvm + ninja from your distro
# Workaround baked into the cross branch: upstream knf-rs-sys's build.rs picks the
# C++ stdlib by *host* cfg!() — a bug under cross-compile that emits `-lc++`
# (libc++) even for the MSVC target. The MSVC CRT already auto-links the C++
# runtime, so we satisfy the spurious reference with an empty c++.lib placed on
# the linker search path. Natively there is no such reference to satisfy.
#
# Windows is the one OS that announces itself in the environment (`OS=Windows_NT`,
# set by the OS itself, not by a shell), which is why the branch reads that rather
# than shelling out to `uname` — under a Git Bash `make` the latter says MINGW64
# and under a cmd-hosted one it says nothing at all.
exe: check-version ## build the Windows engine .exe (native on Windows, cross-compiled elsewhere)
	@test -d src/appearance/web/dist || (cd src/appearance/web && npm ci && npm run build)
ifeq ($(OS),Windows_NT)
	cargo build --release --target $(WIN_TARGET)
else
	@mkdir -p $(WIN_SHIM)
	PATH="$(WIN_LLVM_BIN):$$PATH" llvm-lib /llvmlibempty "/out:$(WIN_SHIM)/c++.lib"
	PATH="$(WIN_LLVM_BIN):$$PATH" RUSTFLAGS="-Lnative=$(WIN_SHIM)" XWIN_ACCEPT_LICENSE=1 \
		cargo xwin build --release --target $(WIN_TARGET)
endif
	@echo "built target/$(WIN_TARGET)/release/hi-agent.exe"

# The Windows shell — the app, as opposed to `exe`, which is the engine. Unlike
# every other target here this one needs a real Windows host: WinUI 3 links the
# Windows App SDK and its XAML compiler runs nowhere else. There is no cross
# build to fall back on, which is why the shell is the one part of this repo
# with no build check on the machines it is written from.
WIN_SHELL_PROJECT := app/windows/HiAgentWindows/HiAgentWindows.csproj
WIN_SHELL_RID     := win-x64

win-app: check-version ## publish the Windows shell (requires Windows + .NET SDK 8)
	dotnet publish $(WIN_SHELL_PROJECT) -c Release -r $(WIN_SHELL_RID)

installer: check-version ## build hi-agent-<version>-windows-x64.exe
	./scripts/make-installer.sh

# The Linux shell. Like `win-app` this needs its own platform — GTK4,
# libadwaita and WebKitGTK have no cross build — but unlike it, that platform is
# an ordinary Debian or Ubuntu box rather than a machine nobody here has. Build
# dependencies beyond the engine's own cmake + libclang-dev:
#   apt install libgtk-4-dev libadwaita-1-dev libwebkitgtk-6.0-dev libsecret-1-dev
LINUX_SHELL_DIR := app/linux

linux-app: check-version ## build the Linux shell (Debian/Ubuntu host + GTK4 dev packages)
	cd $(LINUX_SHELL_DIR) && cargo build --release

linux-test: ## run the Linux shell's tests
	cd $(LINUX_SHELL_DIR) && cargo test

deb: check-version ## package shell + engine as hi-agent_<version>_<arch>.deb
	./scripts/make-deb.sh

# The published release's index: one JSON file naming every artifact of this
# version with its URL, size and SHA-256, attached to the GitHub Release under
# the unversioned name `manifest.json` so that
# https://github.com/<owner>/<repo>/releases/latest/download/manifest.json is a
# permanent address for "what is current and where do I get it".
#
# DIST is a directory holding the already-built artifacts — the release workflow
# fills it from the per-platform build jobs. The script derives the filenames it
# expects from VERSION and fails on any that is missing or empty, so this is also
# the check that a packaging step which reported success actually produced
# something.
DIST ?= target/dist

manifest: ## write manifest.json describing the built artifacts (usage: make manifest DIST=dir)
	@DIST="$(DIST)" ./scripts/make-manifest.sh

bump-version: ## set the committed version everywhere (usage: make bump-version V=x.y.z)
	@test -n "$(BUMP_VERSION)" || { echo "usage: make bump-version V=x.y.z" >&2; exit 1; }
	@./scripts/bump-version.sh "$(BUMP_VERSION)"

# Cut a release: stamp the version everywhere, tag it, push it, and hand the
# rest to CI. The tag push is the handoff — release.yml triggers on `v*` and
# does the building, signing, notarizing and publishing. This target builds
# nothing, which is why it is fast and why a failure here is never a build
# failure.
version: ## bump, tag, and push a release (CI builds and publishes it)
	@VERSIONED_FILES="$(VERSIONED_FILES)" V="$(BUMP_VERSION)" ./scripts/cut-release.sh
