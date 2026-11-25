# Findings (Milestone 1)

Tested on Windows 11, **Spotify 1.3.10.2586** (regular installer), **Spicetify 2.46.7**, WebView2 158. The kickoff named Spotify 1.3.6.1921 / Spicetify 2.46.4; those are no longer installed, so everything below is verified on the current versions.

Spike scripts: [`spike/`](../spike), read-only unless noted. They target Windows PowerShell 5.1 (`powershell.exe`, on every Windows PC) and also run in PowerShell 7 (`pwsh`). Pantry itself does all of this in Rust.

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

## 3. localStorage over CDP (#3)

Works on Spotify 1.3.10.2586 / Spicetify 2.46.7 ([`spike/dump.ps1`](../spike/dump.ps1), [`spike/cdp.ps1`](../spike/cdp.ps1)).

- Launch `%APPDATA%\Spotify\Spotify.exe --remote-debugging-port=9222` directly. No `config-xpui.ini` change needed.
- `GET http://127.0.0.1:9222/json/list` → one `page` target, `https://xpui.app.spotify.com/index.html`. The page appeared within 10 s; `Spicetify.Platform` and `Spicetify.LocalStorage` exist soon after. Allow 40–60 s on a cold start.
- `Runtime.evaluate` with `returnByValue` + `awaitPromise` on the page's `webSocketDebuggerUrl` reads everything: no `--remote-allow-origins` needed for a client that sends no `Origin` header.
- Close with `Browser.close` on the browser target (`/json/version` → `webSocketDebuggerUrl`). Spotify exited within seconds in all 3 runs.
- Pitfalls: .NET's `ClientWebSocket` uses the system proxy (set `Options.Proxy = $null`); PowerShell 5.1 doesn't unroll a JSON array from `ConvertFrom-Json`.

Key families seen (130 keys on a fresh Spicetify + Marketplace setup):

| Family | Example | Owner | Restore by default |
|---|---|---|---|
| `<userId>:*` | `<userId>:ylx-expanded-folders` | Spotify, per account | No (opt-in; warn if the account differs) |
| `ES\|*` | `ES\|…\|EVENT\|…`, `INSTALLATION_ID` | Spotify telemetry queue | Never |
| `com.spotify.*` | `com.spotify.single.item.cache:desktop-ui` | Spotify cache | No (opt-in) |
| `spicetify-exp-features` | | Spicetify (experimental feature toggles) | Yes |
| anything else | `gpb:settings` | extensions | Yes |

`<userId>` is the account's `Spicetify.Platform.username`. Decision: profiles store it as a SHA-256 hash, enough to warn on an account mismatch.

**Marketplace no longer uses localStorage.** Since Marketplace PR #1181 (July 2026; installed here: v1.0.12, the latest), every `marketplace:*` key lives in IndexedDB: database `spicetify-marketplace` (Dexie, IndexedDB version 20), object store `settings`, records `{ key, value }` with string values. On first start it copies legacy `marketplace:*` localStorage keys into IndexedDB, writes the sentinel `internal:local-storage-migrated = 1`, then **deletes** them from localStorage. After that, `marketplace:*` keys written to localStorage are deleted on the next start, not imported.

Decision: Pantry captures both stores and restores `marketplace:*` into whichever one the installed Marketplace uses (IndexedDB if the database and sentinel exist, localStorage otherwise). Writes still go only through CDP. Reading IndexedDB over CDP must never create a database: check `indexedDB.databases()` first and open without a version.

## 5. Local Storage LevelDB, read-only (#5)

- Folder: `%LOCALAPPDATA%\Spotify\Browser\Local Storage\leveldb`. Pantry never opens it in place: with Spotify closed it copies the folder to a temp dir, drops `LOCK`, and opens the copy with `rusty-leveldb` (default comparator works).
- Key: `_https://xpui.app.spotify.com` `\0` + a type byte + the key; value: a type byte + the value. Type byte `1` = Latin-1 (one byte per character), `0` = UTF-16LE. All 130 keys here were Latin-1. Other entries: `VERSION`, `META:https://xpui.app.spotify.com`, `METAACCESS:…`.
- Result: **130/130 keys and values identical** to the CDP dump taken just before Spotify closed.
- Not covered: Marketplace's IndexedDB on disk (`Browser\IndexedDB\https_xpui.app.spotify.com_0.indexeddb.leveldb`). It uses Chromium's `idb_cmp1` comparator and V8-serialised values. Decoding it moves to the emergency-recovery issue (#18); normal capture reads it over CDP.

## 6. Key map (#6)

[`keymap.json`](../keymap.json) maps extensions to their localStorage keys, key prefixes, file names and window globals. First entries: my own extensions, taken from their source (spicetify-gradient-bars):

| Extension | File | Keys | Global |
|---|---|---|---|
| Gradient Progress Bar | `gradientProgressBar.js` | `gpb:settings`, `gpb:presets` | `window.__gpb` |
| Gradient Connect Bar | `gradientConnectBar.js` | `gcb:settings`, `gcb:presets` | `window.__gcb` |
| Gradient Bars | `gradientBars.js` | both bars' keys (no keys of its own) | `window.__gb` (+ `__gpb`, `__gcb`) |

Self-tests use `gpb:selftest:*` / `gcb:selftest:*` while they run; the `gpb:` / `gcb:` prefixes cover them. A key can belong to more than one extension (Gradient Bars shares both bars' keys), so attribution assigns it to every installed extension that claims it.
