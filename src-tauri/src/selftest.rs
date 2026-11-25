//! The built-in self-test (#14). The button copies only the summary line and failed checks.

use crate::attribute::{self, Attributor, Candidate, Owner};
use crate::capture::{self, Progress};
use crate::detect::{self, Roots};
use crate::profile::{self, Profile};
use crate::restore::{self, Options};
use crate::{cdp, js, spotify};
use serde::Serialize;
use std::path::Path;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    pub id: usize,
    pub name: String,
    /// "pass", "fail" or "skip".
    pub status: &'static str,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelfTest {
    pub summary: String,
    /// The summary line plus one line per failed check: what the button copies.
    pub clipboard: String,
    pub results: Vec<CheckResult>,
}

struct T(Vec<CheckResult>);
impl T {
    fn add(&mut self, name: &str, r: Result<String, String>) {
        let (status, detail) = match r {
            Ok(d) => ("pass", d),
            Err(d) => ("fail", d),
        };
        self.0.push(CheckResult { id: self.0.len() + 1, name: name.into(), status, detail });
    }
    fn skip(&mut self, name: &str, why: &str) {
        self.0.push(CheckResult { id: self.0.len() + 1, name: name.into(), status: "skip", detail: why.into() });
    }
}

fn ok_if(cond: bool, yes: impl Into<String>, no: impl Into<String>) -> Result<String, String> {
    if cond { Ok(yes.into()) } else { Err(no.into()) }
}

pub fn run(roots: &Roots, dir: &Path, progress: Progress) -> SelfTest {
    let mut t = T(vec![]);
    let det = detect::detect(roots);

    progress("Checking Spotify and Spicetify…");
    t.add("Spotify (regular installer) found", det.spotify.as_ref().map(|s| s.exe.display().to_string()).ok_or_else(|| "not found in %APPDATA%\\Spotify".into()));
    t.add("Spotify version readable", det.spotify.as_ref().and_then(|s| s.version.clone()).ok_or_else(|| "no app.last-launched-version in prefs".into()));
    t.add("Local Storage folder found", det.spotify.as_ref().map(|s| s.local_storage.clone()).filter(|p| p.is_dir()).map(|p| p.display().to_string()).ok_or_else(|| "missing".into()));
    t.add("Not a Microsoft Store install only", ok_if(det.spotify.is_some() || !det.store_spotify, "ok", "only the Microsoft Store version is installed; Spicetify needs the regular installer"));
    t.add("config-xpui.ini found", det.spicetify.as_ref().map(|s| s.ini.display().to_string()).ok_or_else(|| "not found in %APPDATA%\\spicetify".into()));
    let cli = det.spicetify.as_ref().and_then(|s| s.exe.clone());
    t.add("Spicetify CLI runs", cli.as_ref().and_then(|e| spotify::spicetify_version(e)).ok_or_else(|| "spicetify -v failed".into()));

    progress("Checking profiles…");
    let probe = dir.join(".pantry-selftest");
    t.add(
        "Profiles folder writable",
        std::fs::create_dir_all(dir).and_then(|_| std::fs::write(&probe, b"x")).and_then(|_| std::fs::remove_file(&probe)).map(|_| dir.display().to_string()).map_err(|e| e.to_string()),
    );
    let tmp = std::env::temp_dir().join(format!("pantry-selftest-{}", std::process::id()));
    let mut p = Profile { format: profile::FORMAT, id: "selftest".into(), ..Default::default() };
    p.storage.local_storage.insert("k".into(), "v ✓".into());
    p.files.insert("Extensions/a.js".into(), capture::file_entry(b"a"));
    t.add("Profile save and load round trip", p.save(&tmp).and_then(|f| Profile::load(&f)).map_err(|e| e.to_string()).and_then(|q| ok_if(q == p, "identical", "differs after loading")));
    let _ = std::fs::remove_dir_all(&tmp);
    t.add("SHA-256", ok_if(capture::sha256_hex(b"abc") == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad", "known vector matches", "wrong digest"));

    let mut bad = vec![];
    let list = profile::list(dir);
    for s in &list {
        match Profile::load(&dir.join(format!("{}.json", s.id))) {
            Err(e) => bad.push(format!("{}: {e}", s.id)),
            Ok(q) => {
                if !q.ungrouped_keys().is_empty() {
                    bad.push(format!("{}: {} keys in no group", q.id, q.ungrouped_keys().len()));
                }
                for (rel, f) in &q.files {
                    if capture::decode_file(f).map(|b| capture::sha256_hex(&b)) != Some(f.sha256.clone()) {
                        bad.push(format!("{}: {rel} checksum", q.id));
                    }
                }
            }
        }
    }
    t.add("Saved profiles intact", ok_if(bad.is_empty(), format!("{} profiles, files and key groups verified", list.len()), bad.join("; ")));

    let km = attribute::keymap();
    t.add("Key map has my extensions", ok_if(["gradient-progress-bar", "gradient-connect-bar", "gradient-bars"].iter().all(|k| km.extensions.contains_key(*k)), format!("{} entries", km.extensions.len()), "entries missing"));
    let c = vec![Candidate { id: "x".into(), source: "gradientBars.js".into(), code: Some("'catjam-settings.'".into()) }];
    let a = Attributor { keymap: &km, candidates: &c, username: None };
    t.add(
        "Key attribution rules",
        ok_if(
            a.owner_of_key("gpb:settings") == Owner::Extensions(vec!["x".into()])
                && a.owner_of_key("catjam-settings.x") == Owner::Extensions(vec!["x".into()])
                && a.owner_of_key("ES|1") == Owner::Spotify
                && a.owner_of_key("abcdefghijklmnopqrstuvwxy:x") == Owner::Spotify
                && a.owner_of_key("nobody") == Owner::Unassigned,
            "key map, scan, Spotify and unassigned",
            "a rule gave the wrong owner",
        ),
    );
    let mut rp = Profile::default();
    rp.storage.local_storage.insert("ES|1".into(), "x".into());
    rp.storage.local_storage.insert("ext".into(), "x".into());
    rp.spotify_keys.push("ES|1".into());
    let sel = restore::select(&rp, &Options { include_spotify: true, acknowledged: false }, "indexeddb", true);
    t.add("Telemetry is never restored", ok_if(!sel.local_storage.contains_key("ES|1") && sel.local_storage.contains_key("ext"), "ok", "ES| key selected"));
    t.add("LevelDB read-only decode", leveldb_fixture());

    progress("Checking the connection to Spotify…");
    match spotify::state() {
        spotify::State::RunningWithoutPort => {
            for n in ["Spotify connection", "Page round trip", "Capture script", "Marketplace storage"] {
                t.skip(n, "Spotify runs without Pantry's connection; exit it and run the self-test again");
            }
        }
        _ => match capture::connect(&det, progress) {
            Err(e) => {
                t.add("Spotify connection", Err(e.to_string()));
                for n in ["Page round trip", "Capture script", "Marketplace storage"] {
                    t.skip(n, "no connection");
                }
            }
            Ok(()) => {
                t.add("Spotify connection", cdp::eval_json(cdp::PORT, js::READY).map_err(|e| e.to_string()).and_then(|v| ok_if(v == true, "Spicetify loaded", "Spicetify not loaded")));
                t.add(
                    "Page round trip",
                    cdp::eval_json(cdp::PORT, &js::selftest_roundtrip("pantry:selftest")).map_err(|e| e.to_string()).and_then(|v| {
                        ok_if(v["codec"] == true && v["wrote"] == true && v["removed"] == true, "codec, write and cleanup ok", v.to_string())
                    }),
                );
                t.add(
                    "Capture script",
                    cdp::eval_json(cdp::PORT, &js::capture()).map_err(|e| e.to_string()).and_then(|v| {
                        let l = capture::parse_live(&v);
                        ok_if(!l.local_storage.is_empty() && !l.local_storage.contains_key("pantry:selftest"), format!("{} keys, {} databases", l.local_storage.len(), l.indexed_db.len()), "no keys or leftover self-test key")
                    }),
                );
                t.add("Marketplace storage", cdp::eval_json(cdp::PORT, js::MARKETPLACE_STATE).map_err(|e| e.to_string()).map(|v| if v["indexedDb"] == true { "IndexedDB".to_string() } else if v["legacyKeys"] == true { "localStorage (old Marketplace)".to_string() } else { "no Marketplace data".to_string() }));
            }
        },
    }

    let pass = t.0.iter().filter(|r| r.status == "pass").count();
    let skip = t.0.iter().filter(|r| r.status == "skip").count();
    let total = t.0.len() - skip;
    let mut summary = format!("[pantry] self-test: {pass}/{total} passed");
    if skip > 0 {
        summary.push_str(&format!(", {skip} skipped"));
    }
    let mut clipboard = summary.clone();
    for r in t.0.iter().filter(|r| r.status == "fail") {
        clipboard.push_str(&format!("\n#{} {}: {}", r.id, r.name, r.detail));
    }
    SelfTest { summary, clipboard, results: t.0 }
}

fn leveldb_fixture() -> Result<String, String> {
    use rusty_leveldb::{Options, DB};
    let dir = std::env::temp_dir().join(format!("pantry-selftest-ldb-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    {
        let mut db = DB::open(&dir, Options::default()).map_err(|e| e.to_string())?;
        db.put(b"_https://xpui.app.spotify.com\0\x01gpb:settings", b"\x01{}").map_err(|e| e.to_string())?;
        db.flush().map_err(|e| e.to_string())?;
    }
    let r = crate::leveldb::read_local_storage(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    let m = r?;
    ok_if(m.get("gpb:settings").map(String::as_str) == Some("{}"), "decoded from a copy", format!("got {m:?}"))
}
