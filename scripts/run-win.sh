#!/usr/bin/env bash
# Build Pantry for Windows from WSL, copy the exe to %LOCALAPPDATA%\pantry-dev and start it.
set -euo pipefail
cd "$(dirname "$0")/.."
source ~/.cargo/env
npm run -s build >/dev/null
cargo xwin build --release --target x86_64-pc-windows-msvc --manifest-path src-tauri/Cargo.toml
LAD=$(wslpath "$(/mnt/c/Windows/System32/cmd.exe /c 'echo %LOCALAPPDATA%' 2>/dev/null | tr -d '\r')")
mkdir -p "$LAD/pantry-dev"
cp src-tauri/target/x86_64-pc-windows-msvc/release/pantry.exe "$LAD/pantry-dev/pantry.exe"
(cd "$LAD/pantry-dev" && /mnt/c/Windows/System32/cmd.exe /c start "" pantry.exe "$@")
