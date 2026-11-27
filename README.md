<img src="assets/icon/spicetify-pantry-icon.svg" alt="" width="128">

# Spicetify Pantry

Spicetify forgets. Pantry doesn't.

Spicetify Pantry is my Windows app. It backs up and restores Spicetify extensions **and their settings** from outside Spotify, so nothing is lost when Spotify updates, Spicetify breaks or Spicetify is removed. Spicetify's own backup keeps the extension list, but Marketplace installs and every extension setting live only inside Spotify's browser storage. Pantry backs those up too.

## What it does

- **Back up settings** saves a profile: Spotify and Spicetify versions, theme and colour scheme, your `config-xpui.ini` entries, local extension files, Marketplace installs, and every setting, grouped by extension. Nothing is dropped: settings no extension claims go to "unassigned".
- **Restore** puts a profile back: it backs up your current setup first, writes the Spicetify files, runs `spicetify apply`, starts Spotify and writes the settings back, then checks that every extension is running.
- **Export** saves a profile as one file, so I can set up another PC the same way: **Import profile…** there, then Restore. "For sharing with others" leaves out Spotify's own settings, the hashed account ID and Windows folder paths.
- **Delete** moves a profile to the Recycle Bin.

Pantry never writes to Spotify's storage files. It talks to Spotify through its debugging port while Spotify runs, and closes Spotify the normal way, never by force. Profiles stay on your PC in `%APPDATA%\Spicetify Pantry\profiles`. They can contain API tokens that extensions store, even in a copy for sharing, so think before sharing one.

Needs the regular Spotify installer (not the Microsoft Store version) and Spicetify. Builds aren't code-signed yet: Windows SmartScreen asks first (More info → Run anyway), and Windows 11 with Smart App Control on blocks Pantry.

## Building

From WSL: `npm install`, then `npm run build:win` builds the Windows installer with cargo-xwin. Tests run as Windows binaries: `CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUNNER=$PWD/scripts/win-runner.sh cargo xwin test --target x86_64-pc-windows-msvc` in `src-tauri`.

`pantry.exe --cli self-test <out.json>` runs the self-test without the window (also `capture`, `list`, `restore <id>`, `export <id> <file> [--share]`, `import <file>`, `delete <id>`, `close-spotify`).

Status: early development. See [ROADMAP.md](ROADMAP.md).

## Licence
Copyright © 2026 VoidCrowned. This program is free software. It comes without any warranty, to the extent permitted by applicable law. You can redistribute it and/or modify it under the terms of the Do What The Fuck You Want To Public License, Version 2, as published by Sam Hocevar. See [LICENSE](LICENSE) or http://www.wtfpl.net/ for more details.
