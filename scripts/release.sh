#!/usr/bin/env bash
# Publishes the version in tauri.conf.json as a GitHub Release; installed copies update to it (#39).
# Usage: scripts/release.sh <notes.md>   (Markdown, shown on GitHub and in the app's "What's new")
set -euo pipefail
cd "$(dirname "$0")/.."
die() { echo "release: $*" >&2; exit 1; }

notes=${1:-}
[ -n "$notes" ] && [ -s "$notes" ] || die "usage: scripts/release.sh <notes.md> (the notes file must not be empty)"
[ "$(git branch --show-current)" = main ] || die "not on main"
git fetch -q origin main
[ -z "$(git status --porcelain)" ] || die "uncommitted changes"
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] || die "main isn't the same as origin/main"

v=$(node -p "require('./src-tauri/tauri.conf.json').version")
tag="v$v"
[ -z "$(git ls-remote --tags origin "refs/tags/$tag")" ] || die "$tag already exists; bump the version first"

scripts/build-win.sh
dir=src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis
src="$dir/Spicetify Pantry_${v}_x64-setup.exe"
[ -f "$src" ] && [ -f "$src.sig" ] || die "installer or signature missing in $dir"

# GitHub turns spaces in asset names into dots, so name it without any.
asset="Spicetify-Pantry_${v}_x64-setup.exe"
out=$(mktemp -d)
cp "$src" "$out/$asset"
cp "$src.sig" "$out/$asset.sig"
url="https://github.com/VoidCrowned/spicetify-pantry/releases/download/$tag/$asset"
python3 -I - "$v" "$notes" "$url" "$out/$asset.sig" > "$out/latest.json" <<'PY'
import datetime, json, sys
v, notes, url, sig = sys.argv[1:]
print(json.dumps({
    "version": v,
    "notes": open(notes, encoding="utf-8").read().strip(),
    "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "platforms": {"windows-x86_64": {"signature": open(sig).read().strip(), "url": url}},
}, indent=2, ensure_ascii=False))
PY

gh release create "$tag" --target main --title "Spicetify Pantry $v" --notes-file "$notes" "$out/$asset" "$out/$asset.sig" "$out/latest.json"

# The feed installed copies read must now point at this release.
feed=$(curl -fsSL https://github.com/VoidCrowned/spicetify-pantry/releases/latest/download/latest.json)
[ "$(printf '%s' "$feed" | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["version"])')" = "$v" ] || die "the feed doesn't show $v yet"
echo "released $tag: https://github.com/VoidCrowned/spicetify-pantry/releases/tag/$tag"
echo "installer: $src"
