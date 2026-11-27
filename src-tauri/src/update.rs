//! Self-updating (#39): the feed the updater reads, and checks on it for the self-test.
//! Releases are published by `scripts/release.sh`; the app itself updates through `tauri-plugin-updater`.

use base64::Engine;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::time::Duration;

pub const FEED: &str = "https://github.com/VoidCrowned/spicetify-pantry/releases/latest/download/latest.json";
const CONFIG: &str = include_str!("../tauri.conf.json");

#[derive(Debug, Deserialize)]
pub struct Feed {
    pub version: String,
    #[serde(default)]
    pub notes: String,
    pub platforms: BTreeMap<String, Platform>,
}

#[derive(Debug, Deserialize)]
pub struct Platform {
    pub signature: String,
    pub url: String,
}

/// `0.5.1` or `v0.5.1` → (0, 5, 1).
pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut it = v.trim().trim_start_matches('v').split('.').map(|p| p.parse::<u64>().ok());
    let t = (it.next()??, it.next()??, it.next()??);
    it.next().is_none().then_some(t)
}

/// Problems with a feed, or an empty list when the updater can use it.
pub fn feed_problems(f: &Feed) -> Vec<String> {
    let mut bad = vec![];
    if parse_version(&f.version).is_none() {
        bad.push(format!("version {:?} isn't x.y.z", f.version));
    }
    match f.platforms.get("windows-x86_64") {
        None => bad.push("no windows-x86_64 entry".into()),
        Some(p) => {
            if !p.url.starts_with("https://github.com/VoidCrowned/spicetify-pantry/releases/download/") {
                bad.push(format!("unexpected download URL {}", p.url));
            }
            if p.signature.trim().is_empty() {
                bad.push("empty signature".into());
            }
        }
    }
    bad
}

/// The updater's public key from the built-in config: a base64 minisign key file.
pub fn pubkey_ok() -> Result<String, String> {
    let c: serde_json::Value = serde_json::from_str(CONFIG).map_err(|e| e.to_string())?;
    let k = c.pointer("/plugins/updater/pubkey").and_then(|v| v.as_str()).ok_or("no updater pubkey in tauri.conf.json")?;
    let b64 = base64::engine::general_purpose::STANDARD;
    let text = String::from_utf8(b64.decode(k).map_err(|_| "pubkey isn't base64")?).map_err(|_| "pubkey isn't text")?;
    let key = text.lines().nth(1).ok_or("pubkey has no key line")?;
    let raw = b64.decode(key.trim()).map_err(|_| "key line isn't base64")?;
    // minisign: 2 bytes algorithm "Ed", 8 bytes key id, 32 bytes key.
    if raw.len() == 42 && &raw[..2] == b"Ed" {
        Ok(format!("key id {}", raw[2..10].iter().rev().map(|b| format!("{b:02X}")).collect::<String>()))
    } else {
        Err(format!("unexpected key ({} bytes)", raw.len()))
    }
}

/// What's new after an update: written just before installing, read (and removed) on the next start.
#[derive(Debug, serde::Serialize, Deserialize, PartialEq)]
pub struct JustUpdated {
    pub version: String,
    pub notes: String,
}

pub fn save_just_updated(data_dir: &std::path::Path, j: &JustUpdated) -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(data_dir.join("just-updated.json"), serde_json::to_vec(j)?)
}

/// The notes to show if this start is the first of the version they belong to. The file is removed either way.
pub fn take_just_updated(data_dir: &std::path::Path, running: &str) -> Option<JustUpdated> {
    let p = data_dir.join("just-updated.json");
    let j: Option<JustUpdated> = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let _ = std::fs::remove_file(&p);
    j.filter(|j| j.version == running)
}

pub enum FeedCheck {
    Ok(String),
    Bad(String),
    /// Offline, or nothing published yet: not a failure of this PC.
    Skip(String),
}

/// Downloads the feed and compares it with the running version.
pub fn check_feed(current: &str) -> FeedCheck {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(15))).build().into();
    let text = match agent.get(FEED).call() {
        Ok(mut r) => match r.body_mut().with_config().limit(1 << 20).read_to_string() {
            Ok(t) => t,
            Err(e) => return FeedCheck::Bad(format!("couldn't read the feed: {e}")),
        },
        Err(ureq::Error::StatusCode(404)) => return FeedCheck::Skip("no release published yet".into()),
        Err(e) => return FeedCheck::Skip(format!("couldn't reach GitHub ({e})")),
    };
    let f: Feed = match serde_json::from_str(&text) {
        Ok(f) => f,
        Err(e) => return FeedCheck::Bad(format!("latest.json isn't valid: {e}")),
    };
    let bad = feed_problems(&f);
    if !bad.is_empty() {
        return FeedCheck::Bad(bad.join("; "));
    }
    match (parse_version(&f.version), parse_version(current)) {
        (Some(a), Some(b)) if a > b => FeedCheck::Ok(format!("{} available (running {current})", f.version)),
        _ => FeedCheck::Ok(format!("up to date ({current}; latest release {})", f.version)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(v: &str, url: &str, sig: &str) -> Feed {
        serde_json::from_value(serde_json::json!({
            "version": v, "notes": "x", "pub_date": "2026-11-27T12:00:00Z",
            "platforms": { "windows-x86_64": { "signature": sig, "url": url } }
        }))
        .unwrap()
    }

    #[test]
    fn versions() {
        assert_eq!(parse_version("0.5.1"), Some((0, 5, 1)));
        assert_eq!(parse_version("v1.10.0"), Some((1, 10, 0)));
        assert!(parse_version("0.5").is_none() && parse_version("0.5.1.2").is_none() && parse_version("0.5.x").is_none());
        assert!(parse_version("0.10.0") > parse_version("0.9.9"));
    }

    #[test]
    fn feed_rules() {
        let url = "https://github.com/VoidCrowned/spicetify-pantry/releases/download/v0.5.0/Spicetify-Pantry_0.5.0_x64-setup.exe";
        assert!(feed_problems(&feed("0.5.0", url, "c2ln")).is_empty());
        assert_eq!(feed_problems(&feed("0.5", url, "c2ln")).len(), 1);
        assert_eq!(feed_problems(&feed("0.5.0", "https://example.com/x.exe", "")).len(), 2);
    }

    #[test]
    fn whats_new_shows_once_and_only_for_its_version() {
        let t = tempfile::tempdir().unwrap();
        let j = JustUpdated { version: "0.5.1".into(), notes: "- x".into() };
        save_just_updated(t.path(), &j).unwrap();
        assert_eq!(take_just_updated(t.path(), "0.5.1"), Some(j));
        assert_eq!(take_just_updated(t.path(), "0.5.1"), None);
        save_just_updated(t.path(), &JustUpdated { version: "0.5.1".into(), notes: String::new() }).unwrap();
        assert_eq!(take_just_updated(t.path(), "0.5.0"), None); // install failed: old version started
        assert!(!t.path().join("just-updated.json").exists());
    }

    #[test]
    fn built_in_key_is_valid() {
        assert!(pubkey_ok().is_ok(), "{:?}", pubkey_ok());
    }
}
