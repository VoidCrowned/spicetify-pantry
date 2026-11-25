//! A minimal Chrome DevTools Protocol client for Spotify's xpui page (docs/findings.md §3).
//! Plain HTTP for target discovery, one WebSocket per call. Localhost only, no proxy.

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;
use tungstenite::{Message, WebSocket};

pub const PORT: u16 = 9222;
const XPUI: &str = "xpui.app.spotify.com";

#[derive(Debug)]
pub struct CdpError(pub String);
impl std::fmt::Display for CdpError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for CdpError {}
pub type Result<T> = std::result::Result<T, CdpError>;
fn err<T>(m: impl Into<String>) -> Result<T> {
    Err(CdpError(m.into()))
}

fn http_get(port: u16, path: &str) -> Result<String> {
    let mut s = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_secs(2))
        .map_err(|e| CdpError(format!("no debugging port {port}: {e}")))?;
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    write!(s, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").map_err(|e| CdpError(e.to_string()))?;
    // The DevTools server may keep the connection open, so read until the body is complete, not until EOF.
    let mut buf = Vec::new();
    let mut chunk = [0u8; 16384];
    loop {
        if let Some(h) = find(&buf, b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..h]).to_ascii_lowercase();
            let len = head.lines().find_map(|l| l.strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok()));
            if len.is_some_and(|n| buf.len() >= h + 4 + n) {
                return Ok(String::from_utf8_lossy(&buf[h + 4..h + 4 + len.unwrap()]).into_owned());
            }
        }
        match s.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if buf.is_empty() => return err(format!("no answer on port {port}: {e}")),
            Err(_) => break,
        }
    }
    let text = String::from_utf8_lossy(&buf);
    Ok(text.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("").to_owned())
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

/// Is anything answering CDP on the port?
pub fn port_open(port: u16) -> bool {
    http_get(port, "/json/version").is_ok_and(|b| b.contains("webSocketDebuggerUrl"))
}

pub fn targets(port: u16) -> Result<Vec<Value>> {
    let body = http_get(port, "/json/list")?;
    serde_json::from_str::<Vec<Value>>(&body).map_err(|e| CdpError(format!("bad /json/list: {e}")))
}

/// The page target's WebSocket URL, if the xpui page is up.
pub fn xpui_ws(port: u16) -> Result<String> {
    let t = targets(port)?;
    pick_xpui(&t).ok_or_else(|| CdpError("Spotify's xpui page isn't open yet".into()))
}

pub fn pick_xpui(targets: &[Value]) -> Option<String> {
    targets
        .iter()
        .find(|t| t["type"] == "page" && t["url"].as_str().is_some_and(|u| u.contains(XPUI)))
        .and_then(|t| t["webSocketDebuggerUrl"].as_str().map(str::to_owned))
}

pub fn browser_ws(port: u16) -> Result<String> {
    let v: Value = serde_json::from_str(&http_get(port, "/json/version")?).map_err(|e| CdpError(e.to_string()))?;
    v["webSocketDebuggerUrl"].as_str().map(str::to_owned).ok_or_else(|| CdpError("no browser target".into()))
}

fn connect(ws_url: &str) -> Result<WebSocket<TcpStream>> {
    let rest = ws_url.strip_prefix("ws://").ok_or_else(|| CdpError(format!("unexpected URL {ws_url}")))?;
    let host = rest.split('/').next().unwrap_or("");
    let stream = TcpStream::connect(host).map_err(|e| CdpError(format!("connect {host}: {e}")))?;
    stream.set_read_timeout(Some(Duration::from_secs(120))).ok();
    let mut cfg = tungstenite::protocol::WebSocketConfig::default();
    cfg.max_message_size = Some(512 << 20);
    cfg.max_frame_size = Some(512 << 20);
    let (ws, _) = tungstenite::client::client_with_config(ws_url, stream, Some(cfg)).map_err(|e| CdpError(format!("WebSocket: {e}")))?;
    Ok(ws)
}

/// Sends one command and waits for its reply (events in between are skipped).
pub fn call(ws_url: &str, method: &str, params: Value) -> Result<Value> {
    let mut ws = connect(ws_url)?;
    ws.send(Message::text(json!({ "id": 1, "method": method, "params": params }).to_string()))
        .map_err(|e| CdpError(e.to_string()))?;
    loop {
        match ws.read() {
            Ok(Message::Text(t)) => {
                let v: Value = serde_json::from_str(&t).map_err(|e| CdpError(e.to_string()))?;
                if v["id"] == 1 {
                    let _ = ws.close(None);
                    if let Some(e) = v.get("error") {
                        return err(format!("{method}: {e}"));
                    }
                    return Ok(v["result"].clone());
                }
            }
            Ok(Message::Close(_)) => return err(format!("{method}: connection closed")),
            Ok(_) => {}
            Err(e) => return err(format!("{method}: {e}")),
        }
    }
}

/// Evaluates an expression in the xpui page (promises awaited) and returns its JSON value.
pub fn eval(port: u16, expr: &str) -> Result<Value> {
    let ws = xpui_ws(port)?;
    let r = call(&ws, "Runtime.evaluate", json!({ "expression": expr, "returnByValue": true, "awaitPromise": true }))?;
    if let Some(ex) = r.get("exceptionDetails") {
        let msg = ex["exception"]["description"].as_str().or(ex["text"].as_str()).unwrap_or("exception");
        return err(format!("page error: {msg}"));
    }
    Ok(r["result"]["value"].clone())
}

/// Evaluates a JS expression that resolves to a JSON string, and parses it.
pub fn eval_json(port: u16, expr: &str) -> Result<Value> {
    match eval(port, expr)? {
        Value::String(s) => serde_json::from_str(&s).map_err(|e| CdpError(format!("bad JSON from page: {e}"))),
        other => err(format!("expected a JSON string from the page, got {other}")),
    }
}

/// Asks Spotify to close the way File → Exit does. Doesn't wait.
pub fn browser_close(port: u16) -> Result<()> {
    let ws = browser_ws(port)?;
    match call(&ws, "Browser.close", json!({})) {
        Ok(_) => Ok(()),
        // Spotify may drop the connection before replying; that still means it's closing.
        Err(e) if e.0.contains("closed") || e.0.contains("reset") || e.0.contains("Connection") => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_xpui_page() {
        let t: Vec<Value> = serde_json::from_str(r#"[
            {"type":"service_worker","url":"https://xpui.app.spotify.com/sw.js","webSocketDebuggerUrl":"ws://a"},
            {"type":"page","url":"https://xpui.app.spotify.com/index.html","webSocketDebuggerUrl":"ws://127.0.0.1:9222/devtools/page/X"}
        ]"#).unwrap();
        assert_eq!(pick_xpui(&t).as_deref(), Some("ws://127.0.0.1:9222/devtools/page/X"));
        assert_eq!(pick_xpui(&[]), None);
    }

    #[test]
    fn closed_port_is_reported() {
        // Port 1 is never a CDP endpoint.
        assert!(!port_open(1));
        assert!(xpui_ws(1).is_err());
    }
}
