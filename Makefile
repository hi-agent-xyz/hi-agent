# One version for the binary, web app, desktop metadata, and published artifacts.
# VERSION is the source of truth; `make bump-version V=x.y.z` synchronizes the
# committed files that cannot read it directly at build time.
VERSION := $(shell cat VERSION)
VERSIONED_FILES := VERSION Cargo.toml Cargo.lock \
                   src/appearance/web/package.json src/appearance/web/package-lock.json \
                   app/apple/macos/Info.plist \
                   app/apple/ios/HiAgentIOS.xcodeproj/project.pbxproj \
                   app/android/app/build.gradle.kts \
                   app/windows/HiAgentWindows/HiAgentWindows.csproj

# Keep the former `VERSION=x.y.z` spelling working for callers while matching
# Abacad's public `V=x.y.z` interface.
BUMP_VERSION := $(strip $(if $(V),$(V),$(if $(filter command line,$(origin VERSION)),$(VERSION))))

.PHONY: help check-version build dev run test docker dmg app ios android android-apk exe win-app installer bump-version version

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

# The face's half on its own. Not a shortcut for the impatient: the client shells
# under `app/` change the face without touching a line of Rust, and the hosts they
# are developed on cannot always build the core — `cargo test` wants a toolchain
# and tens of gigabytes of `target/`, and a disk that runs out mid-build reports
# success while producing nothing.
test-web: ## run the web tests alone (no Rust toolchain needed)
# Guarded on the runner itself, not on `node_modules/` being present: a fresh
# worktree has no deps at all and a half-installed tree has the directory, so a
# `test -d` here is the check that passes while the thing it guards is missing.
	@test -x src/appearance/web/node_modules/.bin/vitest || (cd src/appearance/web && npm ci)
	cd src/appearance/web && npm test

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

# `make exe` is a Windows *build check*: it cross-compiles the binary from a
# mac/linux host (proving the Windows code paths compile + link) without running
# it. One-time toolchain on the host:
#   rustup target add x86_64-pc-windows-msvc
#   cargo install cargo-xwin        # fetches the MSVC CRT + Windows SDK on first build
#   brew install llvm ninja         # macOS: clang-cl/lld-link/llvm-lib + ninja (knf-rs's cmake)
#                                    # Linux: install clang, lld, llvm + ninja from your distro
# Workaround baked in below: upstream knf-rs-sys's build.rs picks the C++ stdlib by
# *host* cfg!() — a bug under cross-compile that emits `-lc++` (libc++) even for the
# MSVC target. The MSVC CRT already auto-links the C++ runtime, so we satisfy the
# spurious reference with an empty c++.lib placed on the linker search path.
exe: check-version ## cross-compile a Windows .exe build check (see WIN_TARGET; needs cargo-xwin)
	@test -d src/appearance/web/dist || (cd src/appearance/web && npm ci && npm run build)
	@mkdir -p $(WIN_SHIM)
	PATH="$(WIN_LLVM_BIN):$$PATH" llvm-lib /llvmlibempty "/out:$(WIN_SHIM)/c++.lib"
	PATH="$(WIN_LLVM_BIN):$$PATH" RUSTFLAGS="-Lnative=$(WIN_SHIM)" XWIN_ACCEPT_LICENSE=1 \
		cargo xwin build --release --target $(WIN_TARGET)
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

bump-version: ## set the committed version everywhere (usage: make bump-version V=x.y.z)
	@test -n "$(BUMP_VERSION)" || { echo "usage: make bump-version V=x.y.z" >&2; exit 1; }
	@./scripts/bump-version.sh "$(BUMP_VERSION)"

# Cut a release from an up-to-date branch. This follows Abacad's workflow:
# propose the next patch version, synchronize all version files, commit them,
# tag v<version>, and push both the commit and tag.
version: ## bump, commit, tag, and push a release
	@cur=$$(cat VERSION); \
	major=$${cur%%.*}; rest=$${cur#*.}; minor=$${rest%%.*}; patch=$${rest##*.}; \
	def="$$major.$$minor.$$((patch + 1))"; \
	printf 'Current version: %s\n' "$$cur"; \
	printf 'New version [%s]: ' "$$def"; \
	read v; v=$${v:-$$def}; \
	case "$$v" in [0-9]*.[0-9]*.[0-9]*) ;; *) echo "error: not an x.y.z version: $$v" >&2; exit 1;; esac; \
	if git rev-parse -q --verify "refs/tags/v$$v" >/dev/null 2>&1; then echo "error: tag v$$v already exists" >&2; exit 1; fi; \
	"$${MAKE:-make}" --no-print-directory bump-version V="$$v" && \
	git add $(VERSIONED_FILES) && \
	git commit -m "release v$$v" && \
	git tag "v$$v" && \
	git push origin HEAD && \
	git push origin "v$$v" && \
	echo "Pushed v$$v"
