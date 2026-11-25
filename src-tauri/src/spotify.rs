//! Starting and closing Spotify, and running the Spicetify CLI. Spotify is never killed: it's closed with
//! CDP `Browser.close` (what File → Exit does) and Pantry waits until the process is gone.

use crate::cdp;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    Closed,
    /// Running with Pantry's debugging port: Pantry can read, write and close it.
    Connected,
    /// Running without the port: the user has to exit it first (File → Exit).
    RunningWithoutPort,
}

#[cfg(windows)]
fn hidden(c: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    c.creation_flags(0x0800_0000) // CREATE_NO_WINDOW
}
#[cfg(not(windows))]
fn hidden(c: &mut Command) -> &mut Command {
    c
}

pub fn is_running() -> bool {
    if cfg!(not(windows)) {
        return false;
    }
    let out = hidden(Command::new("tasklist").args(["/FI", "IMAGENAME eq Spotify.exe", "/NH", "/FO", "CSV"])).output();
    out.map(|o| String::from_utf8_lossy(&o.stdout).to_ascii_lowercase().contains("\"spotify.exe\"")).unwrap_or(false)
}

pub fn state() -> State {
    if cdp::port_open(cdp::PORT) {
        State::Connected
    } else if is_running() {
        State::RunningWithoutPort
    } else {
        State::Closed
    }
}

pub fn launch(exe: &Path) -> std::io::Result<()> {
    use std::process::Stdio;
    Command::new(exe).arg(format!("--remote-debugging-port={}", cdp::PORT)).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|_| ())
}

/// Waits until the xpui page answers and Spicetify is loaded (after `spicetify apply`).
pub fn wait_ready(timeout: Duration) -> Result<(), String> {
    wait_for(crate::js::READY, timeout).map_err(|_| format!("Spotify started, but Spicetify didn't load within {} s", timeout.as_secs()))
}

/// Waits until the xpui page has loaded, with or without Spicetify.
pub fn wait_page(timeout: Duration) -> Result<(), String> {
    wait_for(crate::js::PAGE_READY, timeout).map_err(|_| format!("Spotify didn't finish loading within {} s", timeout.as_secs()))
}

fn wait_for(expr: &str, timeout: Duration) -> Result<(), ()> {
    let t0 = Instant::now();
    while t0.elapsed() < timeout {
        if let Ok(serde_json::Value::Bool(true)) = cdp::eval_json(cdp::PORT, expr) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(700));
    }
    Err(())
}

/// Is Spicetify running in the page?
pub fn spicetify_loaded() -> bool {
    matches!(cdp::eval_json(cdp::PORT, crate::js::READY), Ok(serde_json::Value::Bool(true)))
}

/// Closes Spotify gracefully and waits for the process to end.
pub fn close(timeout: Duration) -> Result<(), String> {
    if !is_running() {
        return Ok(());
    }
    cdp::browser_close(cdp::PORT).map_err(|e| format!("couldn't ask Spotify to close: {e}"))?;
    let t0 = Instant::now();
    while t0.elapsed() < timeout {
        if !is_running() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err("Spotify is still running. Exit it via File → Exit, then try again.".into())
}

pub struct CliOutput {
    pub ok: bool,
    pub text: String,
}

/// Runs `spicetify <args>` without a console window.
pub fn spicetify(exe: &Path, args: &[&str]) -> std::io::Result<CliOutput> {
    let o = hidden(Command::new(exe).args(args)).output()?;
    let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&o.stderr));
    Ok(CliOutput { ok: o.status.success(), text: strip_ansi(&text) })
}

pub fn spicetify_version(exe: &Path) -> Option<String> {
    let o = spicetify(exe, &["-v"]).ok()?;
    o.text.lines().map(str::trim).rfind(|l| !l.is_empty()).map(str::to_owned)
}

pub fn strip_ansi(s: &str) -> String {
    let re = regex::Regex::new(r"\x1b\[[0-9;]*[A-Za-z]").unwrap();
    re.replace_all(s, "").into_owned()
}

#[cfg(test)]
mod tests {
    #[test]
    fn strips_colour_codes() {
        assert_eq!(super::strip_ansi("\x1b[1mspicetify v2.46.7\x1b[0m"), "spicetify v2.46.7");
    }
}
