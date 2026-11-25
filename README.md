# Pantry

Spicetify forgets. Pantry doesn't.

My Windows app backs up and restores Spicetify extensions **and their settings** from outside Spotify, so nothing is lost when Spotify updates, Spicetify breaks or Spicetify is removed. Spicetify's own backup keeps the extension list, but Marketplace installs and every extension setting live only inside Spotify's browser storage. Pantry captures those too.

## What it does

- **Capture** saves a profile: Spotify and Spicetify versions, theme and colour scheme, your `config-xpui.ini` entries, local extension files, Marketplace installs, and every setting, grouped by extension. Nothing is dropped: settings no extension claims go to "unassigned".
- **Restore** puts a profile back: it captures your current setup first, writes the Spicetify files, runs `spicetify apply`, starts Spotify and writes the settings back, then checks that every extension is running.

Pantry never writes to Spotify's storage files. It talks to Spotify through its debugging port while Spotify runs, and closes Spotify the normal way, never by force. Profiles stay on your PC in `%APPDATA%\Pantry\profiles`. They can contain API tokens that extensions store, so think before sharing one.

Needs the regular Spotify installer (not the Microsoft Store version) and Spicetify. Builds aren't code-signed yet: Windows SmartScreen asks first (More info → Run anyway), and Windows 11 with Smart App Control on blocks Pantry.

## Building

From WSL: `npm install`, then `npm run build:win` builds the Windows installer with cargo-xwin. Tests run as Windows binaries: `CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUNNER=$PWD/scripts/win-runner.sh cargo xwin test --target x86_64-pc-windows-msvc` in `src-tauri`.

`pantry.exe --cli self-test <out.json>` runs the self-test without the window (also `capture`, `list`, `restore <id>`).

Status: early development. See [ROADMAP.md](ROADMAP.md).

## Licence

[WTFPL](LICENSE)
