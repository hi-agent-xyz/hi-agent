#!/usr/bin/env bash
#
# Build the Linux package: a .deb carrying the engine, the GTK shell, a desktop
# entry, an icon, and the optional `systemd --user` unit. The Linux analog of
# scripts/make-dmg.sh and scripts/make-installer.sh.
#
# Hermetic since 2026-09-20: the engine's payload — codex, esbuild, ffmpeg, the
# headless browser, the ONNX models — ships inside the package.
#
# This reverses what stood here before, so the old argument is worth answering
# rather than deleting. It said: notarization forces the .dmg's hermetic layout,
# Linux has no such requirement, first-run provisioning is the platform norm,
# and it keeps the package near 30 MB instead of near a gigabyte. Three of those
# are still true. The one that decided it is that *why the .dmg is hermetic* and
# *what being hermetic is worth* are different questions: signing is why macOS
# had to, but a first launch that downloads 721 MB is a first launch that fails
# on a metered connection, on a plane, or behind a firewall that dislikes
# GitHub — on every platform equally. A 500 MB download the person chose beats a
# 721 MB one sprung on them at the worst moment.
#
# The fourth argument — that first-run provisioning is the best-tested path —
# was the real one, and it is preserved rather than traded away: every Docker
# core still takes it, and SKIP_PAYLOAD=1 builds a package that does too.
#
# Runs on a Debian 13 / Ubuntu 26.04 host with the GTK4 development packages.
# There is no cross build: the shell links GTK4, libadwaita and WebKitGTK.
#
#   SKIP_ENGINE=1  package the shell alone (an install that only ever attaches
#                  to a core somewhere else — the shell shows a stage message
#                  instead of starting one)
#   SKIP_BUILD=1   reuse whatever is already built
#   SKIP_PAYLOAD=1 leave the payload out; first launch provisions it as before.
#                  A ~25 MB package again — for a quick local build, or a
#                  channel where the download size is the binding constraint.
#
# Output: target/linux/hi-agent_<version>_<arch>.deb
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(pwd)"
./scripts/check-version.sh

VERSION="$(tr -d '\r\n' < VERSION)"
ARCH="$(dpkg --print-architecture)"
SHELL_DIR="$ROOT/app/linux"
OUT="$ROOT/target/linux"
STAGE="$OUT/hi-agent_${VERSION}_${ARCH}"
DEB="$OUT/hi-agent_${VERSION}_${ARCH}.deb"

command -v dpkg-deb >/dev/null 2>&1 || {
  echo "error: dpkg-deb not found. This target builds on Debian/Ubuntu." >&2
  exit 1
}

# --- build ------------------------------------------------------------------
if [ -z "${SKIP_BUILD:-}" ]; then
  echo ">> building the shell …"
  (cd "$SHELL_DIR" && cargo build --release)
  if [ -z "${SKIP_ENGINE:-}" ]; then
    echo ">> building the engine …"
    make build
  fi
fi

SHELL_BIN="$SHELL_DIR/target/release/hi-agent-shell"
ENGINE_BIN="$ROOT/target/release/hi-agent"
[ -x "$SHELL_BIN" ] || { echo "error: $SHELL_BIN not built" >&2; exit 1; }
if [ -z "${SKIP_ENGINE:-}" ] && [ ! -x "$ENGINE_BIN" ]; then
  echo "error: $ENGINE_BIN not built — run 'make build', or SKIP_ENGINE=1 for a client-only package." >&2
  exit 1
fi

# --- stage ------------------------------------------------------------------
rm -rf "$STAGE"
mkdir -p \
  "$STAGE/DEBIAN" \
  "$STAGE/usr/bin" \
  "$STAGE/usr/share/applications" \
  "$STAGE/usr/share/icons/hicolor/scalable/apps" \
  "$STAGE/usr/share/doc/hi-agent" \
  "$STAGE/usr/lib/systemd/user"

# Both binaries live in /usr/lib/hi-agent, with /usr/bin symlinks, and the
# payload sits beside them. That layout is what makes the bundle resolvable
# without a line of Rust: `bundle::resources_dir` derives `resources` from the
# *canonicalized* executable path, and on Linux `current_exe()` is already
# /proc/self/exe — fully resolved — so an engine launched as /usr/bin/hi-agent
# (the systemd unit does exactly that) finds /usr/lib/hi-agent/resources.
#
# "The shell finds the engine beside itself" still holds and is still the
# lookup: `engine_bin()` resolves its own exe first, so both real files being
# in one directory is what matters, not which directory.
mkdir -p "$STAGE/usr/lib/hi-agent"
install -m 0755 "$SHELL_BIN" "$STAGE/usr/lib/hi-agent/hi-agent-shell"
ln -sf ../lib/hi-agent/hi-agent-shell "$STAGE/usr/bin/hi-agent-shell"
if [ -z "${SKIP_ENGINE:-}" ]; then
  install -m 0755 "$ENGINE_BIN" "$STAGE/usr/lib/hi-agent/hi-agent"
  ln -sf ../lib/hi-agent/hi-agent "$STAGE/usr/bin/hi-agent"

  # The hermetic payload — the managed runtime, the three recognition models,
  # the static ffmpeg and the headless browser — laid out by the engine
  # provisioning itself, so what ships is byte-for-byte what a first run would
  # have fetched. This is the package's whole size: ~25 MB becomes ~500 MB.
  #
  # It is staged here rather than downloaded at launch so an install works on a
  # machine that is offline, metered, or behind a firewall that does not like
  # GitHub — the same reason the .dmg does it. First-run provisioning is not
  # deleted and is not unexercised: every Docker core still takes that path,
  # and so does any build made with SKIP_PAYLOAD=1.
  if [ -z "${SKIP_PAYLOAD:-}" ]; then
    echo ">> provisioning the hermetic payload (large download, cached between runs)…"
    "$ENGINE_BIN" --provision-into "$STAGE/usr/lib/hi-agent/resources"
    for d in runtime models ffmpeg browser; do
      [ -d "$STAGE/usr/lib/hi-agent/resources/$d" ] || {
        echo "error: --provision-into left no $d/ — the package would silently still download" >&2
        exit 1
      }
    done
    echo ">> payload staged: $(du -sh "$STAGE/usr/lib/hi-agent/resources" | cut -f1)"
  else
    echo ">> SKIP_PAYLOAD=1 — first launch will provision"
  fi
fi

install -m 0644 "$SHELL_DIR/data/dev.human-interface.HiAgent.desktop" \
  "$STAGE/usr/share/applications/"
install -m 0644 "$SHELL_DIR/data/dev.human-interface.HiAgent.svg" \
  "$STAGE/usr/share/icons/hicolor/scalable/apps/"
install -m 0644 "$SHELL_DIR/data/hi-agent.service" \
  "$STAGE/usr/lib/systemd/user/"
install -m 0644 "$ROOT/LICENSE" "$STAGE/usr/share/doc/hi-agent/copyright" 2>/dev/null || true

# Stated rather than derived with dpkg-shlibdeps, which would need a full
# debian/ source tree for five names that the target table in
# docs/platforms/linux.md already pins. Both targets carry all five.
cat > "$STAGE/DEBIAN/control" <<CONTROL
Package: hi-agent
Version: $VERSION
Section: utils
Priority: optional
Architecture: $ARCH
Depends: libgtk-4-1 (>= 4.18), libadwaita-1-0 (>= 1.7), libwebkitgtk-6.0-4 (>= 2.48), libsecret-1-0, libsoup-3.0-0
Maintainer: Hi Agent <hi@hi-agent.xyz>
Homepage: https://hi-agent.xyz
Description: Your agent, on this computer
 Hi Agent is an agent that lives with you: it remembers, it acts, and it is
 reachable from the machines you already use.
 .
 This package installs the engine and the GTK shell that hosts it. The shell
 starts the engine and supervises it, or attaches to one already running —
 including one managed by the bundled systemd user unit.
 .
 The managed runtime (codex, esbuild, ffmpeg, the headless browser and the
 recognition models) ships inside this package, so a fresh install works
 without downloading anything.
CONTROL

cat > "$STAGE/DEBIAN/postinst" <<'POSTINST'
#!/bin/sh
set -e
# The desktop entry and the icon are only seen once the caches know about them.
if [ "$1" = "configure" ]; then
  [ -x /usr/bin/update-desktop-database ] && update-desktop-database -q /usr/share/applications || true
  [ -x /usr/bin/gtk-update-icon-cache ] && gtk-update-icon-cache -q -f /usr/share/icons/hicolor || true
  # The unit is installed, never enabled. Stock GNOME has no tray, so this is
  # what keeps the agent alive with no window open — but starting a background
  # process that holds a person's data on their behalf is their decision:
  #   systemctl --user enable --now hi-agent.service
  systemctl --system daemon-reload >/dev/null 2>&1 || true
fi
POSTINST
chmod 0755 "$STAGE/DEBIAN/postinst"

cat > "$STAGE/DEBIAN/postrm" <<'POSTRM'
#!/bin/sh
set -e
if [ "$1" = "remove" ] || [ "$1" = "purge" ]; then
  [ -x /usr/bin/update-desktop-database ] && update-desktop-database -q /usr/share/applications || true
  [ -x /usr/bin/gtk-update-icon-cache ] && gtk-update-icon-cache -q -f /usr/share/icons/hicolor || true
fi
# The agent's data directory is never touched. ~/.local/share/hi-agent is the
# person's memory, not this package's state.
POSTRM
chmod 0755 "$STAGE/DEBIAN/postrm"

# --- build the package ------------------------------------------------------
dpkg-deb --build --root-owner-group "$STAGE" "$DEB" >/dev/null
rm -rf "$STAGE"

echo "built $DEB"
dpkg-deb --info "$DEB" | sed -n '1,12p'
