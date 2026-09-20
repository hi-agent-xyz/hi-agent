#!/usr/bin/env bash
#
# Build the Windows installer: a per-user NSIS Setup.exe wrapping the
# cross-compiled hi-agent.exe. The Windows analog of scripts/make-dmg.sh.
#
# This is the "it runs" tier: the installer carries the binary (+ icon), and the
# WinUI shell when one has been built. On first launch hi-agent.exe
# auto-provisions its managed runtime (Node + codex + esbuild + ffmpeg +
# recognition models) into the OS cache. A fully hermetic,
# offline-from-first-launch installer (the .dmg's bundling parity) is a later
# increment that needs a real Windows host to stage + verify.
#
# Runs on any host that can both cross-compile the exe (`make exe` — needs the
# cargo-xwin toolchain, set up on the Mac mini) and run `makensis`. No Windows
# box required — but the shell is left out when built that way, because WinUI 3
# has no cross build.
#
#   SKIP_BUILD=1   reuse an existing target/<win>/release/hi-agent.exe
#   SHELL_DIR=…    publish output of `make win-app`, to include the shell
#
# Output: target/installer/hi-agent-<version>-windows-x64.exe
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(pwd)"
./scripts/check-version.sh

# --- prerequisites ----------------------------------------------------------
if ! command -v makensis >/dev/null 2>&1; then
  echo "error: makensis (NSIS) not found on PATH." >&2
  echo "       macOS: brew install makensis   |   Debian/Ubuntu: apt-get install nsis" >&2
  exit 1
fi

WIN_TARGET="x86_64-pc-windows-msvc"
WIN_EXE="$ROOT/target/$WIN_TARGET/release/hi-agent.exe"
ICON="$ROOT/app/windows/HiAgentWindows/Assets/HiAgent.ico"
[ -f "$ICON" ] || { echo "error: $ICON not found" >&2; exit 1; }

# The WinUI shell, when there is one. It can only be built on Windows
# (`make win-app`), and this script runs on the Mac mini, so its absence is the
# normal case rather than a failure: the installer then ships the engine alone
# and its shortcuts start that. Point SHELL_DIR at a publish output to include
# it, or let the default path find one built in place.
SHELL_DIR="${SHELL_DIR:-$ROOT/app/windows/HiAgentWindows/bin/Release/net8.0-windows10.0.19041.0/win-x64/publish}"

VERSION="$(cat VERSION)"
VERSION4="${VERSION}.0"   # NSIS VIProductVersion wants a four-part numeric

OUT="$ROOT/target/installer"
SETUP="$OUT/hi-agent-$VERSION-windows-x64.exe"
mkdir -p "$OUT"

# --- 1. cross-compile the Windows binary (+ embedded SPA) -------------------
if [ "${SKIP_BUILD:-}" != "1" ]; then
  echo ">> cross-compiling hi-agent.exe (make exe)…"
  make exe
fi
[ -f "$WIN_EXE" ] || { echo "error: $WIN_EXE not found; run without SKIP_BUILD" >&2; exit 1; }

# --- 2. compile the installer ----------------------------------------------
# **NSIS reads backslashes only.** Its `File` and `OutFile` directives parse a
# forward-slash path as garbage and report `no files found` for a file that is
# demonstrably there — which is how the first native Windows run of this script
# failed, one line from the end, after an 8-minute engine build. It never bit on
# the Mac mini because POSIX builds of makensis accept either separator, so this
# is the one place in the repo where the *host running the packager* changes what
# a path has to look like. `cygpath` ships with the Git Bash that GitHub's
# `shell: bash` uses on Windows, and exists nowhere else — which makes it both
# the converter and the test for whether conversion is needed.
nsis_path() {
  if command -v cygpath >/dev/null 2>&1; then cygpath -w "$1"; else printf '%s' "$1"; fi
}

# The hermetic payload: the managed runtime, the three recognition models, the
# static ffmpeg and the headless browser, laid out by the engine provisioning
# *itself* (`--provision-into`) so what ships is byte-for-byte what a first run
# would have downloaded. `bundle::resources_beside_exe` looks for `resources`
# next to `hi-agent.exe`, which is where the .nsi puts this.
#
# **Only a Windows host can stage it, and that is not a policy.** The flag
# provisions the platform it is *running on*, so the Mac mini — which cannot
# execute a win32 binary at all — would stage a macOS tree under a Windows
# installer. So the test is whether this host can run what it just built, and
# a Mac simply produces the non-hermetic tier it always has: an installer whose
# first launch downloads ~721 MB into the OS cache. That tier is not deprecated;
# it is what `make installer` means anywhere but Windows.
RES_DEFINE=()
RES_DIR="$OUT/resources"
if [ "${OS:-}" = "Windows_NT" ]; then
  echo ">> provisioning the hermetic payload into $RES_DIR (large download, cached between runs)…"
  rm -rf "$RES_DIR"
  mkdir -p "$RES_DIR"
  "$WIN_EXE" --provision-into "$RES_DIR"
  # The engine reports success by exiting 0, but an empty tree would sail
  # through NSIS and ship an installer that silently still downloads. Check the
  # one subdirectory every resolver looks for.
  [ -d "$RES_DIR/runtime" ] && [ -d "$RES_DIR/models" ] && [ -d "$RES_DIR/ffmpeg" ] && [ -d "$RES_DIR/browser" ] || {
    echo "error: --provision-into left an incomplete tree in $RES_DIR" >&2
    ls -la "$RES_DIR" >&2
    exit 1
  }
  echo ">> payload staged: $(du -sh "$RES_DIR" 2>/dev/null | cut -f1)"
  RES_DEFINE=("-DRESDIR=$(nsis_path "$RES_DIR")\\")
else
  echo ">> not a Windows host — no hermetic payload staged (first launch will provision)"
fi

SHELL_DEFINE=()
if [ -f "$SHELL_DIR/HiAgent.exe" ]; then
  echo ">> including the WinUI shell from $SHELL_DIR"
  # The trailing separator is what tells `File /r` to copy the directory's
  # *contents* rather than the directory itself, and it has to be the separator
  # NSIS is reading — so it is appended here, where the platform is known,
  # rather than in the .nsi where it would have to be one or the other.
  if command -v cygpath >/dev/null 2>&1; then
    SHELL_DEFINE=("-DSHELLDIR=$(nsis_path "$SHELL_DIR")\\")
  else
    SHELL_DEFINE=("-DSHELLDIR=$SHELL_DIR/")
  fi
else
  echo ">> no WinUI shell at $SHELL_DIR — building the engine-only installer"
  echo "   (build it on a Windows host with 'make win-app', then re-run with SKIP_BUILD=1)"
fi

echo ">> building installer with makensis…"
# makensis on macOS aborts (std::bad_alloc) under a C/POSIX locale — the case
# in a bare SSH/CI shell. Force a UTF-8 locale just for the build so it works
# regardless of the caller's environment.
export LC_ALL="en_US.UTF-8"
makensis -V2 \
  "-DVERSION=$VERSION" \
  "-DVERSION4=$VERSION4" \
  "-DSRCEXE=$(nsis_path "$WIN_EXE")" \
  "-DICON=$(nsis_path "$ICON")" \
  "${SHELL_DEFINE[@]+"${SHELL_DEFINE[@]}"}" \
  "${RES_DEFINE[@]+"${RES_DEFINE[@]}"}" \
  "-DOUTFILE=$(nsis_path "$SETUP")" \
  "$ROOT/scripts/hi-agent.nsi"

echo ""
echo "done:"
echo "  installer: $SETUP"
du -h "$SETUP" 2>/dev/null || ls -l "$SETUP"
