#!/bin/sh
# CookieUse.app installer (the macOS GUI).
#   curl -fsSL https://raw.githubusercontent.com/leeguooooo/cookie-use/main/install-app.sh | sh
# The dmg is Developer ID signed and notarized by Apple; this checks its sha256
# and that Gatekeeper accepts the app before replacing /Applications/CookieUse.app.
set -eu
REPO="leeguooooo/cookie-use"; APP="CookieUse"
BASE="https://github.com/${REPO}/releases/latest/download"
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
pkill -x "$APP" 2>/dev/null || true
echo "Installing to /Applications (you may be asked for your password)…"
sudo rm -rf "/Applications/${APP}.app"
sudo ditto "$MNT/${APP}.app" "/Applications/${APP}.app"
echo ""
echo "✅ ${APP} installed to /Applications. Launch it from Spotlight, or press ⌥⌘K."
