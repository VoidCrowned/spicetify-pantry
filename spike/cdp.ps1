# Spike #3: minimal CDP client in PowerShell 5.1. Dot-source it.
$Port = 9222
function Get-CdpTargets { $list = (Invoke-WebRequest -UseBasicParsing "http://127.0.0.1:$Port/json/list" -TimeoutSec 3).Content | ConvertFrom-Json; $list | ForEach-Object { $_ } }  # PS 5.1 doesn't unroll JSON arrays
function Get-XpuiPage { Get-CdpTargets | Where-Object { $_.type -eq 'page' -and $_.url -like '*xpui.app.spotify.com*' } | Select-Object -First 1 }
function Invoke-Cdp([string]$WsUrl, [string]$Method, $Params = @{}) {
  $ws = New-Object System.Net.WebSockets.ClientWebSocket
  $ws.Options.Proxy = $null
  $ct = [Threading.CancellationToken]::None
  $ws.ConnectAsync([Uri]$WsUrl, $ct).Wait()
  $msg = @{ id = 1; method = $Method; params = $Params } | ConvertTo-Json -Depth 10 -Compress
  $bytes = [Text.Encoding]::UTF8.GetBytes($msg)
  $ws.SendAsync([ArraySegment[byte]]$bytes, 'Text', $true, $ct).Wait()
  $buf = New-Object byte[] 65536; $ms = New-Object IO.MemoryStream
  while ($true) {
    $ms.SetLength(0)
    do { $res = $ws.ReceiveAsync([ArraySegment[byte]]$buf, $ct); $res.Wait(); $ms.Write($buf, 0, $res.Result.Count) } until ($res.Result.EndOfMessage -or $res.Result.MessageType -eq 'Close')
    if ($res.Result.MessageType -eq 'Close') { return $null }
    $reply = [Text.Encoding]::UTF8.GetString($ms.ToArray()) | ConvertFrom-Json
    if ($reply.id -eq 1) { break }
  }
  try { $ws.CloseAsync('NormalClosure', '', $ct).Wait() } catch {}
  $reply
}
function Invoke-Js([string]$Expr) {
  $p = Get-XpuiPage
  $r = Invoke-Cdp $p.webSocketDebuggerUrl 'Runtime.evaluate' @{ expression = $Expr; returnByValue = $true; awaitPromise = $true }
  $r.result.result.value
}
function Close-SpotifyGracefully {
  $v = (Invoke-WebRequest -UseBasicParsing "http://127.0.0.1:$Port/json/version" -TimeoutSec 3).Content | ConvertFrom-Json
  try { Invoke-Cdp $v.webSocketDebuggerUrl 'Browser.close' | Out-Null } catch {}
  for ($i = 0; $i -lt 60; $i++) { if (-not (Get-Process Spotify -ErrorAction SilentlyContinue)) { return $true }; Start-Sleep -Milliseconds 500 }
  $false
}
