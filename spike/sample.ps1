# Pantry spike: collect a redacted sample of a modded Spotify setup for docs/findings.md (#3, #4).
# Run in PowerShell with Spotify closed:
#   irm https://raw.githubusercontent.com/VoidCrowned/spicetify-pantry/spike-cdp/spike/sample.ps1 | iex
# It starts Spotify with a local debugging port, reads localStorage and IndexedDB (read-only),
# closes Spotify normally and writes pantry-sample.json to your Desktop. It never kills Spotify and writes nothing to Spotify.
# Redacted: your Spotify user ID, Spotify's own values (key names and sizes only), telemetry (count only), Spotify's cache DB.
& {
$ErrorActionPreference = 'Stop'
$Port = 9222
function Get-Targets { $l = (Invoke-WebRequest -UseBasicParsing "http://127.0.0.1:$Port/json/list" -TimeoutSec 3).Content | ConvertFrom-Json; $l | ForEach-Object { $_ } }
function Get-Page { Get-Targets | Where-Object { $_.type -eq 'page' -and $_.url -like '*xpui.app.spotify.com*' } | Select-Object -First 1 }
function Invoke-Cdp([string]$WsUrl, [string]$Method, $Params = @{}) {
  $ws = New-Object System.Net.WebSockets.ClientWebSocket; $ws.Options.Proxy = $null
  $ct = [Threading.CancellationToken]::None
  $ws.ConnectAsync([Uri]$WsUrl, $ct).Wait()
  $b = [Text.Encoding]::UTF8.GetBytes((@{ id = 1; method = $Method; params = $Params } | ConvertTo-Json -Depth 10 -Compress))
  $ws.SendAsync([ArraySegment[byte]]$b, 'Text', $true, $ct).Wait()
  $buf = New-Object byte[] 65536; $ms = New-Object IO.MemoryStream
  while ($true) {
    $ms.SetLength(0)
    do { $r = $ws.ReceiveAsync([ArraySegment[byte]]$buf, $ct); $r.Wait(); $ms.Write($buf, 0, $r.Result.Count) } until ($r.Result.EndOfMessage -or $r.Result.MessageType -eq 'Close')
    if ($r.Result.MessageType -eq 'Close') { return $null }
    $reply = [Text.Encoding]::UTF8.GetString($ms.ToArray()) | ConvertFrom-Json
    if ($reply.id -eq 1) { break }
  }
  try { $ws.CloseAsync('NormalClosure', '', $ct).Wait() } catch {}
  $reply
}
function Invoke-Js([string]$e) { (Invoke-Cdp (Get-Page).webSocketDebuggerUrl 'Runtime.evaluate' @{ expression = $e; returnByValue = $true; awaitPromise = $true }).result.result.value }

if (Get-Process Spotify -ErrorAction SilentlyContinue) { Write-Host 'Spotify is running. Exit it first (... > File > Exit), then run this again.' -ForegroundColor Yellow; return }
$exe = Join-Path $env:APPDATA 'Spotify\Spotify.exe'
if (-not (Test-Path $exe)) { Write-Host "Spotify not found at $exe" -ForegroundColor Yellow; return }
Write-Host 'Starting Spotify with the debugging port...'
Start-Process $exe -ArgumentList "--remote-debugging-port=$Port"
$t0 = Get-Date; $ok = $false
while (-not $ok -and ((Get-Date) - $t0).TotalSeconds -lt 90) { Start-Sleep 1; try { $ok = Invoke-Js '!!(window.Spicetify && Spicetify.Platform && Spicetify.LocalStorage)' } catch {} }
if (-not $ok) { Write-Host 'Spotify did not become ready in 90 s. Close it normally and try again.' -ForegroundColor Yellow; return }
Write-Host 'Waiting for extensions to load...'
Start-Sleep 15
$js = @'
(async () => {
const user = (Spicetify.Platform && Spicetify.Platform.username) || '';
const red = s => user ? String(s).split(user).join('<userId>') : String(s);
const isSpotify = k => (user && k.startsWith(user + ':')) || /^[a-z0-9]{20,}:/.test(k) || k.startsWith('com.spotify.');
const ls = {}, spotifyKeys = {}; let telemetry = 0;
for (const k of Object.keys(localStorage).sort()) {
  const v = localStorage.getItem(k);
  if (k.startsWith('ES|')) telemetry++;
  else if (isSpotify(k)) spotifyKeys[red(k)] = v.length;
  else if (k === 'spicetify-exp-features') ls[k] = '<' + v.length + ' chars, Spicetify experimental features>';
  else ls[red(k)] = red(v);
}
const idb = {};
for (const { name, version } of (indexedDB.databases ? await indexedDB.databases() : [])) {
  const db = await new Promise((ok, bad) => { const r = indexedDB.open(name); r.onsuccess = () => ok(r.result); r.onerror = () => bad(r.error); });
  const stores = {};
  for (const s of db.objectStoreNames) {
    const rows = await new Promise((ok, bad) => { const r = db.transaction(s, 'readonly').objectStore(s).getAll(); r.onsuccess = () => ok(r.result); r.onerror = () => bad(r.error); });
    stores[s] = name === 'spicetify-marketplace' ? JSON.parse(red(JSON.stringify(rows))) : '<' + rows.length + ' records, not Spicetify>';
  }
  db.close(); idb[name] = { version, stores };
}
return JSON.stringify({
  spotifyVersion: Spicetify.Platform.version, spicetifyConfig: Spicetify.Config,
  globals: Object.keys(window).filter(k => /^__/.test(k) && !/^__(webpack|react|SENTRY|sentry|zod|@)/.test(k)),
  spicetifyGlobals: Object.keys(window.Spicetify || {}).length,
  localStorage: ls, spotifyKeys, telemetryKeys: telemetry, indexedDB: idb
});
})()
'@
$data = Invoke-Js $js | ConvertFrom-Json
$ud = Join-Path $env:APPDATA 'spicetify'
$ini = Join-Path $ud 'config-xpui.ini'
$list = { param($d) if (Test-Path $d) { @(Get-ChildItem $d | ForEach-Object { if ($_.PSIsContainer) { $_.Name + '\' } else { $_.Name } }) } else { @() } }
$mp = Join-Path $ud 'CustomApps\marketplace\manifest.json'
$sample = [ordered]@{
  pantrySample = 1
  spicetifyVersion = (& spicetify -v 2>$null | Select-Object -Last 1)
  ini = if (Test-Path $ini) { (Get-Content $ini -Raw).Replace($env:USERPROFILE, '%USERPROFILE%') } else { $null }
  extensionsFolder = & $list "$ud\Extensions"; themesFolder = & $list "$ud\Themes"; customAppsFolder = & $list "$ud\CustomApps"
  marketplaceManifest = if (Test-Path $mp) { Get-Content $mp -Raw | ConvertFrom-Json } else { $null }
  live = $data
}
$out = Join-Path ([Environment]::GetFolderPath('Desktop')) 'pantry-sample.json'
[IO.File]::WriteAllText($out, ($sample | ConvertTo-Json -Depth 12), (New-Object Text.UTF8Encoding $false))
Write-Host 'Closing Spotify normally...'
$v = (Invoke-WebRequest -UseBasicParsing "http://127.0.0.1:$Port/json/version" -TimeoutSec 3).Content | ConvertFrom-Json
try { Invoke-Cdp $v.webSocketDebuggerUrl 'Browser.close' | Out-Null } catch {}
for ($i = 0; $i -lt 60 -and (Get-Process Spotify -ErrorAction SilentlyContinue); $i++) { Start-Sleep -Milliseconds 500 }
Write-Host "Done: $out ($([int]((Get-Item $out).Length / 1KB)) KB)" -ForegroundColor Green
}
