#!/usr/bin/env bash
#
# Build a signed + notarized Beeline.app and DMG.
#
# Notarization needs an Apple ID + app-specific password. Unlike the
# LegalMessageExport build (which uses a notarytool keychain profile), Tauri's
# bundler reads these from the environment, so put them in a gitignored
# notarize.env at the repo root and `source` it first, e.g.:
#
#   export APPLE_ID="jeremy@appcamp.com"
#   export APPLE_PASSWORD="abcd-efgh-ijkl-mnop"   # app-specific password
#
# The app-specific password is per-Apple-ID, not per-app: the same one behind
# the LMR-notary profile works here. If you don't have its value saved, make a
# fresh one at appleid.apple.com → Sign-In & Security → App-Specific Passwords.
#
# Signing identity + team default to the AppCamp Developer ID cert; override by
# exporting APPLE_SIGNING_IDENTITY / APPLE_TEAM_ID.
#
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TRIPLE="$(rustc -Vv | sed -n 's/host: //p')"

export APPLE_SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:-Developer ID Application: Jeremy Hubert (6ULL56D9UV)}"
export APPLE_TEAM_ID="${APPLE_TEAM_ID:-6ULL56D9UV}"

if [[ -z "${APPLE_ID:-}" || -z "${APPLE_PASSWORD:-}" ]]; then
  echo "warning: APPLE_ID / APPLE_PASSWORD not set — the build will be signed but NOT notarized." >&2
  echo "         (source a notarize.env with those to notarize + staple.)" >&2
fi

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
  echo "==> Generating app icons from assets/logo-icon.png"
  pnpm tauri icon "$ROOT/assets/logo-icon.png"
fi

echo "==> tauri build (sign$([[ -n "${APPLE_ID:-}" ]] && echo " + notarize") + dmg)"
# externalBin is injected only for the release bundle (so the helper is shipped
# beside the app); it's omitted from the base config so `tauri dev` doesn't
# require the staged sidecar.
pnpm tauri build --config '{"bundle":{"externalBin":["binaries/mailagent"]}}'

echo
echo "==> Done. Bundles:"
find src-tauri/target/release/bundle -maxdepth 2 -name '*.dmg' -o -name '*.app' 2>/dev/null | sed 's/^/    /'
