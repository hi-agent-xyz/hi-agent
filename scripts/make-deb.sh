#!/usr/bin/env bash
#
# Build the Linux package: a .deb carrying the engine, the GTK shell, a desktop
# entry, an icon, and the optional `systemd --user` unit. The Linux analog of
# scripts/make-dmg.sh and scripts/make-installer.sh.
#
# Unlike the .dmg this is deliberately *not* hermetic. The engine's payload —
# codex, esbuild, ffmpeg, the headless browser, the ONNX models — is downloaded
# on first run. That is the opposite of macOS and the difference is not taste:
# notarization requires every Mach-O inside the .app to be co-signed, so the
# hermetic layout there is a consequence of signing rather than a distribution
# choice. Linux has no such requirement, first-run provisioning is the platform
# norm, it keeps the package near 30 MB instead of near a gigabyte, and it is
# the best-tested path in the codebase — every Docker core already takes it.
#
# Runs on a Debian 13 / Ubuntu 26.04 host with the GTK4 development packages.
# There is no cross build: the shell links GTK4, libadwaita and WebKitGTK.
#
#   SKIP_ENGINE=1  package the shell alone (an install that only ever attaches
#                  to a core somewhere else — the shell shows a stage message
#                  instead of starting one)
#   SKIP_BUILD=1   reuse whatever is already built
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

install -m 0755 "$SHELL_BIN" "$STAGE/usr/bin/hi-agent-shell"
# The shell finds the engine beside itself, so /usr/bin is not a convention
# here — it is the lookup.
[ -z "${SKIP_ENGINE:-}" ] && install -m 0755 "$ENGINE_BIN" "$STAGE/usr/bin/hi-agent"

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
 The managed runtime (codex, esbuild, ffmpeg and the recognition models) is
 downloaded on first run.
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
