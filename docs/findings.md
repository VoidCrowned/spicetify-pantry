# Findings (Milestone 1)

Tested on Windows 11, **Spotify 1.3.10.2586** (regular installer), **Spicetify 2.46.7**, WebView2 158. The kickoff named Spotify 1.3.6.1921 / Spicetify 2.46.4; those are no longer installed, so everything below is verified on the current versions.

Spike scripts: [`spike/`](../spike) (PowerShell 5.1, read-only unless noted).

## 1. Spotify installs (#1)

| | Regular installer | Microsoft Store |
|---|---|---|
| Exe | `%APPDATA%\Spotify\Spotify.exe` | `C:\Program Files\WindowsApps\SpotifyAB.SpotifyMusic_<ver>_x64__zpdnekdrzrea0\Spotify.exe` |
| Detect | `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Spotify` (`DisplayVersion`, `InstallLocation`), or the exe's file version | `Get-AppxPackage SpotifyAB.SpotifyMusic` |
| Version | exe `FileVersion` `1.3.10.2586`; uninstall key adds the build hash: `1.3.10.2586.g81a118a22460` | package `Version` |
| prefs | `%APPDATA%\Spotify\prefs` | `%LOCALAPPDATA%\Packages\SpotifyAB.SpotifyMusic_zpdnekdrzrea0\LocalState\Spotify\prefs` |
| Local data | `%LOCALAPPDATA%\Spotify` | `%LOCALAPPDATA%\Packages\SpotifyAB.SpotifyMusic_zpdnekdrzrea0\LocalCache\Spotify` |
| Local Storage | `%LOCALAPPDATA%\Spotify\Browser\Local Storage\leveldb` | under the package's `LocalCache\Spotify\Browser\…` |

Store column: **not tested here** (no Store install on this PC); paths come from the package layout. Spicetify doesn't support the Store version (its files are read-only), so Pantry treats a Store install as "detected, unsupported" and tells the user to switch to the regular installer.

## 2. Spicetify (#2)

- Exe: `%LOCALAPPDATA%\spicetify\spicetify.exe` (on `PATH`). Version: `spicetify -v` → `2.46.7`.
- User data: `spicetify path userdata` → `%APPDATA%\spicetify`. Contains `config-xpui.ini`, `Extensions\`, `Themes\`, `CustomApps\`.
- `config-xpui.ini` entries Pantry captures:
  - `[AdditionalOptions] extensions` and `custom_apps`: `|`-separated file / folder names (here: extensions empty, custom apps `marketplace`).
  - `[Setting] current_theme`, `color_scheme` (here: `marketplace`, empty), `spotify_launch_flags`.
  - `[Backup] version` / `with`: the Spotify build and Spicetify version the backup was made with. This is the most reliable "which versions does this setup belong to" pair.
- With the Marketplace, the theme is the `marketplace` placeholder folder: the real theme and its colour scheme live in localStorage (see section 4).
