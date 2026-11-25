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

## 4. Marketplace records (#4)

From a redacted sample of my main PC (Spotify 1.3.10.2586, Spicetify 2.46.7, Marketplace v1.0.12, Hazy theme, 14 Marketplace extensions, 4 snippets; [`spike/sample.ps1`](../spike/sample.ps1)). All records sit in IndexedDB `spicetify-marketplace` → `settings` as `{ key, value }`, values are strings.

| Key | Value |
|---|---|
| `marketplace:installed-extensions` | JSON array of per-item keys |
| `marketplace:installed-themes` | JSON array of per-item keys |
| `marketplace:installed-snippets` | JSON array of per-item keys |
| `marketplace:theme-installed` | per-item key of the active theme (absent: no Marketplace theme) |
| `marketplace:installed:<user>/<repo>/<path of main>` | extension record (JSON) |
| `marketplace:installed:<user>/<repo>/<cssURL>` | theme record (JSON) |
| `marketplace:installed:snippet:<Title-With-Dashes>` | snippet record (JSON) |
| `marketplace:local-theme`, `:tabs`, `:active-tab`, `:sort`, `:themeDevTools`, `:albumArtBasedColors*`, `:colorShift`, … | Marketplace settings and UI state |
| `internal:local-storage-migrated` | `"1"`: migration sentinel (not a `marketplace:` key) |

Record fields:
- **Extension**: `type: "extension"`, `manifest` (the repo's manifest entry), `title`, `subtitle`, `authors`, `user`, `repo`, `branch`, `imageURL`, `extensionURL` (what Marketplace loads on every start), `readmeURL`, `stars`, `lastUpdated`, `created`.
- **Theme**: the same plus `cssURL`, `schemesURL`, `include` (theme JS URLs, e.g. Hazy's `hazy.js`), `tags`, `schemes` (all colour schemes, parsed), `activeScheme`. **Theme and colour scheme come from here**: the ini only says `current_theme = marketplace`. `Spicetify.Config` reports the live pair (`Hazy` / `Base`).
- **Snippet**: `code` (CSS), `title`, `description`, `imageURL`.

Other observations:
- `Spicetify.Config.extensions` lists everything that runs: the ini's local files, Marketplace extensions and theme JS (17 here, while the ini lists only `gradientBars.js`). This is the list for the post-restore self-check.
- `Extensions\` can hold files the ini no longer lists (6 here). Pantry captures only the files the ini lists and shows the others as "not enabled".
- Hazy's settings use unprefixed keys (`HideMerch`, `blurAmount`, …), so attribution needs the static scan of theme JS (`include`), not just prefixes.
- Values can contain API tokens (`more-lyrics.api-token`). Profiles stay on the PC; Pantry warns before a profile is shared.

### Extensions with their own IndexedDB

Sort Play keeps 125 settings in its own database (`SortPlayDB` → `settings`), next to cache stores with thousands of records. `keyval-store` is Spotify's own query cache.

Decision: capture every IndexedDB database except Spotify's (`keyval-store`) over CDP, with a tagged encoding for values that aren't plain JSON (Date, ArrayBuffer, typed arrays, Blob, Map, Set). Restore writes them back over CDP. Stores whose names look like caches are captured but not restored by default.
, read-only (#5)

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

## 7. Smart App Control blocks unsigned builds

Windows 11's Smart App Control (`HKLM\SYSTEM\CurrentControlSet\Control\CI\Policy` → `VerifiedAndReputablePolicyState`: 0 off, 1 on, 2 evaluation) blocked unsigned Pantry builds with "blocked by your organization's Device Guard policy" (Code Integrity events 3033/3077/3118). The first build ran, the next ones were blocked. On my dev laptop it's now off. Every user with Smart App Control on will hit this until Pantry is signed with a certificate from a Microsoft-trusted CA (#28).
