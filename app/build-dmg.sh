#!/usr/bin/env bash
# Build CookieUse.app (Release) and package a .dmg.
#
#   app/build-dmg.sh              ad-hoc signed (local testing)
#   SIGNING_IDENTITY="Developer ID Application" app/build-dmg.sh
#                                 Developer ID + hardened runtime, then the app and
#                                 the dmg are notarized and stapled, so the dmg opens
#                                 on a double-click with no Gatekeeper warning.
#
# Notarizing needs an App Store Connect API key: ASC_KEY_ID, ASC_ISSUER_ID and
# ASC_KEY_PATH (a .p8). SIGNING_KEYCHAIN optionally names the keychain holding the
# identity (CI imports it into a throwaway one).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT="${SCRIPT_DIR}/CookieUse.xcodeproj"
SCHEME="CookieUse"
APP_NAME="CookieUse"
DERIVED="${SCRIPT_DIR}/.build/release"
OUT="${SCRIPT_DIR}/build"
APP="${OUT}/${APP_NAME}.app"
DMG="${OUT}/${APP_NAME}.dmg"
ENTITLEMENTS="${SCRIPT_DIR}/CookieUse/App/CookieUse.entitlements"
IDENTITY="${SIGNING_IDENTITY:-}"
TEAM_ID="${APPLE_TEAM_ID:-6ZPXG4KVVS}"

if command -v xcodegen >/dev/null; then (cd "${SCRIPT_DIR}" && xcodegen generate >/dev/null); fi
mkdir -p "${OUT}"
rm -rf "${APP}" "${DMG}"

SIGN_ARGS=()
[[ -n "${SIGNING_KEYCHAIN:-}" ]] && SIGN_ARGS+=(--keychain "${SIGNING_KEYCHAIN}")

if [[ -n "${IDENTITY}" ]]; then
	for v in ASC_KEY_ID ASC_ISSUER_ID ASC_KEY_PATH; do
		[[ -n "${!v:-}" ]] || { echo "error: ${v} is required to notarize" >&2; exit 1; }
	done
	# Xcode builds unsigned; we sign once, explicitly, so the identity, hardened
	# runtime, secure timestamp and entitlements are exactly what notarization checks.
	xcodebuild -project "${PROJECT}" -scheme "${SCHEME}" -configuration Release \
		-destination 'platform=macOS' -derivedDataPath "${DERIVED}" \
		CODE_SIGNING_ALLOWED=NO build >/dev/null
	ditto "${DERIVED}/Build/Products/Release/${APP_NAME}.app" "${APP}"
	codesign --force --options runtime --timestamp --entitlements "${ENTITLEMENTS}" \
		--sign "${IDENTITY}" "${SIGN_ARGS[@]}" "${APP}"
	codesign --verify --deep --strict "${APP}"
	info="$(codesign --display --verbose=4 "${APP}" 2>&1)"
	[[ "${info}" == *"TeamIdentifier=${TEAM_ID}"* && "${info}" == *"Runtime Version="* ]] \
		|| { echo "error: app is not Developer ID signed for ${TEAM_ID} with hardened runtime" >&2; exit 1; }

	NOTARY=(--key "${ASC_KEY_PATH}" --key-id "${ASC_KEY_ID}" --issuer "${ASC_ISSUER_ID}" --wait)
	ZIP="${OUT}/${APP_NAME}-notarize.zip"
	ditto -c -k --keepParent "${APP}" "${ZIP}"
	xcrun notarytool submit "${ZIP}" "${NOTARY[@]}"
	rm -f "${ZIP}"
	xcrun stapler staple "${APP}"
else
	xcodebuild -project "${PROJECT}" -scheme "${SCHEME}" -configuration Release \
		-destination 'platform=macOS' -derivedDataPath "${DERIVED}" build >/dev/null
	ditto "${DERIVED}/Build/Products/Release/${APP_NAME}.app" "${APP}"
fi

TMP="${OUT}/.dmg-root"
rm -rf "${TMP}"
mkdir -p "${TMP}"
ditto "${APP}" "${TMP}/${APP_NAME}.app"
ln -s /Applications "${TMP}/Applications"
hdiutil create -volname "${APP_NAME}" -srcfolder "${TMP}" -ov -format UDZO "${DMG}" >/dev/null
rm -rf "${TMP}"

if [[ -n "${IDENTITY}" ]]; then
	codesign --force --timestamp --sign "${IDENTITY}" "${SIGN_ARGS[@]}" "${DMG}"
	xcrun notarytool submit "${DMG}" "${NOTARY[@]}"
	xcrun stapler staple "${DMG}"
	# What a user's Mac will check on first launch.
	xcrun stapler validate "${APP}"
	xcrun stapler validate "${DMG}"
	spctl --assess --type execute -vv "${APP}"
	spctl --assess --type open --context context:primary-signature -vv "${DMG}"
fi

echo "app: ${APP}"
echo "dmg: ${DMG}"
