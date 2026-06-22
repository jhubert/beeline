#!/usr/bin/env bash
#
# Build a signed + notarized Beeline.app and DMG.
#
# Signing: uses the AppCamp "Developer ID Application" cert (override with
# APPLE_SIGNING_IDENTITY). Tauri signs the app + sidecar with a hardened runtime.
#
# Notarization: done here with notarytool against a stored keychain profile
# (the same mechanism LegalMessageExport uses) — no password in any file. Reuses
# the existing `LMR-notary` profile by default; override with NOTARY_PROFILE, or
# create one once with:
#     xcrun notarytool store-credentials "beeline-notary" \
#         --apple-id "you@appcamp.com" --team-id "6ULL56D9UV" \
#         --password "app-specific-password"
# Set NO_NOTARIZE=1 to stop after signing (fast pipeline check).
#
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TRIPLE="$(rustc -Vv | sed -n 's/host: //p')"

export APPLE_SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:-Developer ID Application: Jeremy Hubert (6ULL56D9UV)}"
NOTARY_PROFILE="${NOTARY_PROFILE:-LMR-notary}"

echo "==> Building mailagent helper (release)"
cargo build --release --manifest-path "$ROOT/Cargo.toml" -p mailagent-cli

echo "==> Staging helper as a Tauri sidecar: binaries/mailagent-$TRIPLE"
mkdir -p "$ROOT/apps/desktop/src-tauri/binaries"
cp "$ROOT/target/release/mailagent" \
   "$ROOT/apps/desktop/src-tauri/binaries/mailagent-$TRIPLE"

cd "$ROOT/apps/desktop"

echo "==> Installing frontend deps (Tauri CLI)"
pnpm install

if [[ ! -f src-tauri/icons/icon.icns ]]; then
  echo "==> Generating app icons from assets/logo-icon-macos.png"
  pnpm tauri icon "$ROOT/assets/logo-icon-macos.png"
fi

echo "==> tauri build (sign + dmg)"
# externalBin is injected only for the release bundle (so the helper is shipped
# beside the app); it's omitted from the base config so `tauri dev` doesn't
# require the staged sidecar. We let Tauri SIGN only — notarization is below.
pnpm tauri build --config '{"bundle":{"externalBin":["binaries/mailagent"]}}'

APP="$(find src-tauri/target/release/bundle/macos -maxdepth 1 -name '*.app' | head -1)"
DMG="$(find src-tauri/target/release/bundle/dmg -maxdepth 1 -name '*.dmg' | head -1)"
[[ -n "$DMG" ]] || { echo "✗ DMG not found under bundle/dmg" >&2; exit 1; }

if [[ -n "${NO_NOTARIZE:-}" ]]; then
  echo "==> NO_NOTARIZE set — signed but not notarized. DMG: $DMG"
  exit 0
fi

echo "==> Notarizing DMG with profile '$NOTARY_PROFILE' (a few minutes)…"
xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait

echo "==> Stapling…"
# Staple the standalone .app (for zip distribution) and the DMG (for the installer).
[[ -n "$APP" ]] && xcrun stapler staple "$APP" || true
xcrun stapler staple "$DMG"

echo
echo "==> Gatekeeper check (want: accepted / source=Notarized Developer ID):"
spctl -a -t open --context context:primary-signature -vvv "$DMG" 2>&1 | sed 's/^/    /' || true
echo
echo "==> Done. DMG: $DMG"
