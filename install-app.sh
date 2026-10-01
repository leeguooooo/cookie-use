#!/bin/sh
# CookieUse.app installer (the macOS GUI). Re-run it to upgrade.
#   curl -fsSL https://raw.githubusercontent.com/leeguooooo/cookie-use/main/install-app.sh | sh
# The dmg is Developer ID signed and notarized by Apple; this checks its sha256
# and that Gatekeeper accepts the app before installing it.
#
# No sudo: admins can write /Applications, and the app ends up owned by you so
# the next upgrade needs no password either. sudo is used once, only to remove a
# root-owned copy left by an older version of this script.
# COOKIE_USE_APP_DIR overrides the install folder (default /Applications).
set -eu
REPO="leeguooooo/cookie-use"; APP="CookieUse"
BASE="https://github.com/${REPO}/releases/latest/download"
DEST="${COOKIE_USE_APP_DIR:-/Applications}"
[ "$(uname -s)" = "Darwin" ] || { echo "error: macOS only." >&2; exit 1; }
TMP="$(mktemp -d)"; MNT=""
cleanup() { [ -n "$MNT" ] && hdiutil detach "$MNT" -quiet 2>/dev/null || true; rm -rf "$TMP"; }
trap cleanup EXIT

echo "Downloading the latest ${APP}…"
curl -fsSL -o "$TMP/${APP}.dmg" "${BASE}/${APP}.dmg"
curl -fsSL -o "$TMP/${APP}.dmg.sha256" "${BASE}/${APP}.dmg.sha256"
want="$(awk '{print $1}' "$TMP/${APP}.dmg.sha256")"
got="$(shasum -a 256 "$TMP/${APP}.dmg" | awk '{print $1}')"
[ -n "$want" ] && [ "$want" = "$got" ] || { echo "error: checksum mismatch for ${APP}.dmg" >&2; exit 1; }
MNT="$(hdiutil attach -nobrowse -noautoopen "$TMP/${APP}.dmg" | grep -o '/Volumes/.*' | head -1)"
spctl --assess --type execute "$MNT/${APP}.app" 2>/dev/null \
  || { echo "error: ${APP}.app is not notarized — refusing to install" >&2; exit 1; }

mkdir -p "$DEST"
[ -w "$DEST" ] || { echo "error: can't write to $DEST (set COOKIE_USE_APP_DIR=\$HOME/Applications)" >&2; exit 1; }
TARGET="$DEST/${APP}.app"
pkill -x "$APP" 2>/dev/null || true
if [ -e "$TARGET" ]; then
  if [ "$(stat -f %u "$TARGET")" != "$(id -u)" ]; then
    echo "Removing an old root-owned ${APP}.app (one-time; asks for your password)…"
    sudo rm -rf "$TARGET"
  else
    rm -rf "$TARGET"
  fi
fi
ditto "$MNT/${APP}.app" "$TARGET"
ver="$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$TARGET/Contents/Info.plist" 2>/dev/null || echo '?')"
echo ""
echo "✅ ${APP} ${ver} installed to ${DEST}."
open "$TARGET" 2>/dev/null || true
echo "   It lives in the menu bar — press ⌥⌘K to search and switch accounts."
