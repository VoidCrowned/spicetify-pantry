# Spike #1/#2: detect Spotify (regular installer and Microsoft Store) and Spicetify. Read-only.
$ErrorActionPreference = 'SilentlyContinue'
$r = [ordered]@{}
$exe = Join-Path $env:APPDATA 'Spotify\Spotify.exe'
$uninst = Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Spotify'
$r.regular = [ordered]@{
  exe = if (Test-Path $exe) { $exe } else { $null }
  version = (Get-Item $exe).VersionInfo.FileVersion
  uninstallKey = [ordered]@{ DisplayVersion = $uninst.DisplayVersion; InstallLocation = $uninst.InstallLocation; DisplayIcon = $uninst.DisplayIcon }
  prefs = if (Test-Path "$env:APPDATA\Spotify\prefs") { "$env:APPDATA\Spotify\prefs" } else { $null }
  localData = if (Test-Path "$env:LOCALAPPDATA\Spotify") { "$env:LOCALAPPDATA\Spotify" } else { $null }
  localStorage = if (Test-Path "$env:LOCALAPPDATA\Spotify\Browser\Local Storage\leveldb") { "$env:LOCALAPPDATA\Spotify\Browser\Local Storage\leveldb" } else { $null }
}
$pkg = Get-AppxPackage -Name 'SpotifyAB.SpotifyMusic'
$r.store = if ($pkg) { [ordered]@{ version = $pkg.Version; installLocation = $pkg.InstallLocation; family = $pkg.PackageFamilyName
  data = "$env:LOCALAPPDATA\Packages\$($pkg.PackageFamilyName)\LocalState\Spotify" } } else { $null }
$sp = (Get-Command spicetify).Source
$ud = if ($sp) { (& spicetify path userdata) | Select-Object -Last 1 } else { $null }
$ini = if ($ud) { Join-Path $ud 'config-xpui.ini' } else { $null }
$sections = [ordered]@{}; $cur = $null
if ($ini -and (Test-Path $ini)) { foreach ($line in Get-Content $ini) {
  if ($line -match '^\[(.+)\]') { $cur = $Matches[1]; $sections[$cur] = [ordered]@{} }
  elseif ($cur -and $line -match '^\s*([^;=]+?)\s*=\s*(.*)$') { $sections[$cur][$Matches[1]] = $Matches[2] } } }
$list = { param($d) if (Test-Path $d) { @(Get-ChildItem $d | ForEach-Object Name) } else { $null } }
$r.spicetify = [ordered]@{
  exe = $sp; version = if ($sp) { (& spicetify -v) | Select-Object -Last 1 } else { $null }
  userdata = $ud; ini = $ini; iniSections = $sections
  extensions = & $list "$ud\Extensions"; themes = & $list "$ud\Themes"; customApps = & $list "$ud\CustomApps"
  spotifyRunning = [bool](Get-Process Spotify)
}
$r | ConvertTo-Json -Depth 6
