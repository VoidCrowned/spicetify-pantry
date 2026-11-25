# Spike #3: launch Spotify with the debugging port, dump localStorage, close Spotify gracefully.
# Never kills Spotify. Refuses to run while Spotify is open, unless -Attach (Spotify already has the port).
# Usage: dump.ps1 -Out <file.json> [-Attach]
param([Parameter(Mandatory)][string]$Out, [switch]$Attach)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'cdp.ps1')
if (-not $Attach) {
  if (Get-Process Spotify -ErrorAction SilentlyContinue) { throw 'Spotify is running. Exit it first (File > Exit).' }
  Start-Process "$env:APPDATA\Spotify\Spotify.exe" -ArgumentList "--remote-debugging-port=$Port"
}
$t0 = Get-Date; $page = $null
while (-not $page -and ((Get-Date) - $t0).TotalSeconds -lt 60) { Start-Sleep 1; try { $page = Get-XpuiPage } catch {} }
if (-not $page) { throw 'xpui page did not appear within 60 s' }
$ready = $false
while (-not $ready -and ((Get-Date) - $t0).TotalSeconds -lt 90) { Start-Sleep 1; try { $ready = Invoke-Js '!!(window.Spicetify && Spicetify.Platform && Spicetify.LocalStorage)' } catch {} }
# Marketplace loads extensions after its storage hydrates: wait up to 30 s for known globals, then 3 s for startup writes.
$t1 = Get-Date
while (((Get-Date) - $t1).TotalSeconds -lt 30) { try { if (Invoke-Js '!!(window.__gpb || window.__gcb || window.__gb)') { break } } catch {}; Start-Sleep 1 }
Start-Sleep 3
# IndexedDB: only open databases that already exist (open() on a missing name creates one), and without a version
# (Dexie stores version(2) as 20).
$js = @'
(async () => {
const dbs = indexedDB.databases ? await indexedDB.databases() : [];
const idb = {};
for (const { name, version } of dbs) {
  const db = await new Promise((ok, bad) => { const r = indexedDB.open(name); r.onsuccess = () => ok(r.result); r.onerror = () => bad(r.error); });
  const stores = {};
  for (const s of db.objectStoreNames) {
    stores[s] = await new Promise((ok, bad) => { const r = db.transaction(s, 'readonly').objectStore(s).getAll(); r.onsuccess = () => ok(r.result); r.onerror = () => bad(r.error); });
  }
  db.close();
  idb[name] = { version, stores };
}
return JSON.stringify({
  url: location.href,
  spotifyVersion: Spicetify.Platform && Spicetify.Platform.version,
  spicetifyVersion: Spicetify.Config && Spicetify.Config.version,
  config: Spicetify.Config,
  globals: Object.keys(window).filter(k => /^__/.test(k)),
  localStorage: Object.fromEntries(Object.keys(localStorage).sort().map(k => [k, localStorage.getItem(k)])),
  indexedDB: idb
});
})()
'@
$json = Invoke-Js $js
[IO.File]::WriteAllText($Out, $json, (New-Object Text.UTF8Encoding $false))
$closed = Close-SpotifyGracefully
@{ page = $page.url; secondsToReady = [int]((Get-Date) - $t0).TotalSeconds; bytes = $json.Length; closedGracefully = $closed } | ConvertTo-Json
