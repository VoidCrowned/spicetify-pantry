#!/usr/bin/env bash
# Builds the Windows installer from WSL. The updater signs it with the key in ~/.tauri (#39),
# unless TAURI_SIGNING_PRIVATE_KEY is already set.
set -euo pipefail
cd "$(dirname "$0")/.."
source ~/.cargo/env
if [ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
  key=~/.tauri/spicetify-pantry.key
  [ -f "$key" ] || { echo "No signing key at $key: create one with 'npx tauri signer generate -w $key'." >&2; exit 1; }
  export TAURI_SIGNING_PRIVATE_KEY="$key" TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
fi
npm run build:win
