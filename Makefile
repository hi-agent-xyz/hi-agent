# One version for the binary, web app, desktop metadata, and published artifacts.
# VERSION is the source of truth; `make bump-version V=x.y.z` synchronizes the
# committed files that cannot read it directly at build time.
VERSION := $(shell cat VERSION)
VERSIONED_FILES := VERSION Cargo.toml Cargo.lock scripts/Info.plist \
                   src/appearance/web/package.json src/appearance/web/package-lock.json

# Keep the former `VERSION=x.y.z` spelling working for callers while matching
# Abacad's public `V=x.y.z` interface.
BUMP_VERSION := $(strip $(if $(V),$(V),$(if $(filter command line,$(origin VERSION)),$(VERSION))))

.PHONY: help check-version build dev run test docker dmg app exe installer bump-version version

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
	cd src/appearance/web && npm test

docker: ## build the docker image
	docker build -t hi-agent:dev .

dmg: check-version ## build hi-agent-<version>-macos-apple-silicon.dmg
	./scripts/make-dmg.sh

app: ## wrap the dev binary in a minimal ad-hoc-signed Hi Agent.app for local mic/camera testing (macOS)
	./scripts/make-app.sh

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
