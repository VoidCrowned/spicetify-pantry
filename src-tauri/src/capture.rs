//! Capture (#10): writes one profile from the live Spotify over CDP, or, with Spotify closed and unable to
//! start, from a read-only copy of Local Storage.

use crate::attribute::{self, Attributor, Candidate, Owner};
use crate::detect::{Detection, Roots};
use crate::ini::Ini;
use crate::profile::*;
use crate::{cdp, js, spotify};
use base64::Engine;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub type Progress<'a> = &'a (dyn Fn(&str) + Sync);

pub fn sha256_hex(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

pub fn file_entry(b: &[u8]) -> FileEntry {
    FileEntry { sha256: sha256_hex(b), size: b.len() as u64, base64: base64::engine::general_purpose::STANDARD.encode(b) }
}

/// Files under `dir`, recursively, keyed `<prefix>/<relative path>` with `/` separators.
fn collect_dir(dir: &Path, prefix: &str, out: &mut BTreeMap<String, FileEntry>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let key = format!("{prefix}/{name}");
        let p = e.path();
        if p.is_dir() {
            collect_dir(&p, &key, out);
        } else if let Ok(b) = std::fs::read(&p) {
            out.insert(key, file_entry(&b));
        }
    }
}

pub fn decode_file(f: &FileEntry) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD.decode(&f.base64).ok()
}

fn fetch_code(url: &str) -> Option<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(15))).build().into();
    let mut r = agent.get(url).call().ok()?;
    r.body_mut().with_config().limit(20 << 20).read_to_string().ok()
}

/// Script URLs on GitHub or jsDelivr that a small loader script pulls in (e.g. Sort Play's loader).
pub fn loader_urls(code: &str) -> Vec<String> {
    if code.len() > 64 << 10 {
        return vec![]; // a full extension, not a loader
    }
    let re = regex::Regex::new(r#"https://(?:raw\.githubusercontent\.com|cdn\.jsdelivr\.net)/[^"'`\s)]+?\.m?js"#).unwrap();
    let mut v: Vec<String> = re.find_iter(code).map(|m| m.as_str().to_owned()).collect();
    v.dedup();
    v.truncate(5);
    v
}

/// The code itself plus, one level deep, the scripts it loads.
fn follow_loader(code: String) -> Vec<String> {
    let mut out: Vec<String> = loader_urls(&code).iter().filter_map(|u| fetch_code(u)).collect();
    out.insert(0, code);
    out
}

/// The live page's data, as returned by `js::capture()`.
#[derive(Default)]
pub struct Live {
    pub spotify_version: Option<String>,
    pub username: Option<String>,
    pub config: Value,
    pub local_storage: BTreeMap<String, String>,
    pub indexed_db: BTreeMap<String, Database>,
}

pub fn parse_live(v: &Value) -> Live {
    let mut idb = BTreeMap::new();
    if let Some(dbs) = v["indexedDB"].as_object() {
        for (name, d) in dbs {
            let spotify = name == "keyval-store";
            let mut stores = BTreeMap::new();
            if let Some(ss) = d["stores"].as_object() {
                for (s, sd) in ss {
                    stores.insert(
                        s.clone(),
                        ObjectStore {
                            key_path: sd["keyPath"].clone(),
                            auto_increment: sd["autoIncrement"].as_bool().unwrap_or(false),
                            indexes: sd["indexes"].as_array().cloned().unwrap_or_default(),
                            records: sd["records"].as_array().cloned().unwrap_or_default(),
                            cache: s.to_ascii_lowercase().contains("cache") || name.to_ascii_lowercase().contains("cache"),
                        },
                    );
                }
            }
            idb.insert(name.clone(), Database { version: d["version"].as_u64().unwrap_or(1), stores, spotify });
        }
    }
    Live {
        spotify_version: v["spotifyVersion"].as_str().map(str::to_owned),
        username: v["username"].as_str().map(str::to_owned),
        config: v["config"].clone(),
        local_storage: v["localStorage"].as_object().map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_owned())).collect()).unwrap_or_default(),
        indexed_db: idb,
    }
}

/// `marketplace:*` records from IndexedDB (current Marketplace) or localStorage (before Marketplace PR #1181).
pub fn marketplace_records(live: &Live) -> (String, BTreeMap<String, String>) {
    if let Some(db) = live.indexed_db.get("spicetify-marketplace") {
        let mut m = BTreeMap::new();
        for r in db.stores.get("settings").map(|s| s.records.as_slice()).unwrap_or(&[]) {
            if let (Some(k), Some(v)) = (r["value"]["key"].as_str(), r["value"]["value"].as_str()) {
                m.insert(k.to_owned(), v.to_owned());
            }
        }
        return ("indexeddb".into(), m);
    }
    let m: BTreeMap<String, String> = live.local_storage.iter().filter(|(k, _)| k.starts_with("marketplace:")).map(|(k, v)| (k.clone(), v.clone())).collect();
    (if m.is_empty() { "none".into() } else { "localStorage".into() }, m)
}

struct Item {
    ext: Extension,
    cand: Candidate,
    urls: Vec<String>,
}

/// Builds the extension list and fetches remote code (in parallel) for the static scan.
fn candidates(userdata: &Path, ini: &Ini, mp: &BTreeMap<String, String>, theme: &str, config: &Value) -> Vec<Item> {
    let mut items = vec![];
    for f in ini.get_list("AdditionalOptions", "extensions") {
        let code = std::fs::read_to_string(userdata.join("Extensions").join(&f)).ok();
        items.push(Item {
            ext: Extension { id: format!("local:{f}"), name: f.trim_end_matches(".js").trim_end_matches(".mjs").into(), kind: "local".into(), source: Some(f.clone()), ..Default::default() },
            cand: Candidate { id: format!("local:{f}"), source: f.clone(), code },
            urls: vec![],
        });
    }
    let list = |k: &str| -> Vec<String> { mp.get(k).and_then(|v| serde_json::from_str(v).ok()).unwrap_or_default() };
    for key in list("marketplace:installed-extensions") {
        let rec: Value = mp.get(&key).and_then(|v| serde_json::from_str(v).ok()).unwrap_or(Value::Null);
        let url = rec["extensionURL"].as_str().unwrap_or("").to_owned();
        items.push(Item {
            ext: Extension { id: key.clone(), name: rec["title"].as_str().unwrap_or(&key).into(), kind: "marketplace".into(), source: Some(url.clone()), ..Default::default() },
            cand: Candidate { id: key.clone(), source: url.clone(), code: None },
            urls: if url.is_empty() { vec![] } else { vec![url] },
        });
    }
    if let Some(active) = mp.get("marketplace:theme-installed") {
        let rec: Value = mp.get(active).and_then(|v| serde_json::from_str(v).ok()).unwrap_or(Value::Null);
        let urls: Vec<String> = rec["include"].as_array().into_iter().flatten().filter_map(|u| u.as_str()).filter(|u| u.ends_with(".js") || u.ends_with(".mjs")).map(str::to_owned).collect();
        let name = rec["title"].as_str().unwrap_or("theme").to_owned();
        items.push(Item {
            ext: Extension { id: active.clone(), name, kind: "theme".into(), source: urls.first().cloned(), ..Default::default() },
            cand: Candidate { id: active.clone(), source: urls.first().cloned().unwrap_or_default(), code: None },
            urls,
        });
    } else if !theme.is_empty() && theme != "marketplace" {
        let code = std::fs::read_to_string(userdata.join("Themes").join(theme).join("theme.js")).ok();
        items.push(Item {
            ext: Extension { id: format!("theme:{theme}"), name: theme.into(), kind: "theme".into(), source: Some(format!("Themes/{theme}")), ..Default::default() },
            cand: Candidate { id: format!("theme:{theme}"), source: "theme.js".into(), code },
            urls: vec![],
        });
    }
    for app in ini.get_list("AdditionalOptions", "custom_apps") {
        if app == "marketplace" {
            continue; // Marketplace's own keys are grouped separately.
        }
        let mut files = BTreeMap::new();
        collect_dir(&userdata.join("CustomApps").join(&app), "", &mut files);
        let code: String = files.iter().filter(|(k, _)| k.ends_with(".js")).filter_map(|(_, f)| decode_file(f)).map(|b| String::from_utf8_lossy(&b).into_owned()).collect::<Vec<_>>().join("\n");
        items.push(Item {
            ext: Extension { id: format!("app:{app}"), name: app.clone(), kind: "custom-app".into(), source: Some(format!("CustomApps/{app}")), ..Default::default() },
            cand: Candidate { id: format!("app:{app}"), source: app.clone(), code: Some(code) },
            urls: vec![],
        });
    }
    for key in list("marketplace:installed-snippets") {
        let rec: Value = mp.get(&key).and_then(|v| serde_json::from_str(v).ok()).unwrap_or(Value::Null);
        items.push(Item {
            ext: Extension { id: key.clone(), name: rec["title"].as_str().unwrap_or(&key).into(), kind: "snippet".into(), scanned: true, ..Default::default() },
            cand: Candidate { id: key.clone(), source: String::new(), code: Some(String::new()) },
            urls: vec![],
        });
    }
    // Theme JS that Spicetify injects but no record names (e.g. a local theme's extension in Config.extensions)
    // is covered by the local theme above; anything else that runs shows up in the self-check.
    let _ = config;
    let codes: Vec<Vec<String>> = std::thread::scope(|s| {
        let handles: Vec<_> = items.iter().map(|it| s.spawn(|| it.urls.iter().filter_map(|u| fetch_code(u)).flat_map(follow_loader).collect::<Vec<_>>())).collect();
        handles.into_iter().map(|h| h.join().unwrap_or_default()).collect()
    });
    for (it, code) in items.iter_mut().zip(codes) {
        if !it.urls.is_empty() && !code.is_empty() {
            it.cand.code = Some(code.join("\n"));
        }
    }
    for it in &mut items {
        if it.ext.kind != "snippet" {
            it.ext.scanned = it.cand.code.is_some();
        }
    }
    items
}

/// Assembles a profile from what was read. Pure apart from the network fetches, so tests can feed it.
pub fn build(det: &Detection, live: Live, spicetify_version: Option<String>, source: &str, reason: &str, now: chrono::DateTime<chrono::Utc>) -> Profile {
    let sp = det.spicetify.as_ref();
    let ini = sp.and_then(|s| Ini::load(&s.ini).ok()).unwrap_or_default();
    let userdata = sp.map(|s| s.userdata.clone()).unwrap_or_default();
    let (mp_storage, mp) = marketplace_records(&live);
    let current_theme = ini.get("Setting", "current_theme").unwrap_or("").to_owned();
    let mut items = candidates(&userdata, &ini, &mp, &current_theme, &live.config);

    let mut files = BTreeMap::new();
    for f in ini.get_list("AdditionalOptions", "extensions") {
        if let Ok(b) = std::fs::read(userdata.join("Extensions").join(&f)) {
            files.insert(format!("Extensions/{f}"), file_entry(&b));
        }
    }
    if !current_theme.is_empty() && current_theme != "marketplace" {
        collect_dir(&userdata.join("Themes").join(&current_theme), &format!("Themes/{current_theme}"), &mut files);
    }
    for app in ini.get_list("AdditionalOptions", "custom_apps") {
        collect_dir(&userdata.join("CustomApps").join(&app), &format!("CustomApps/{app}"), &mut files);
    }

    let km = attribute::keymap();
    let cands: Vec<Candidate> = items.iter().map(|i| i.cand.clone()).collect();
    let a = Attributor { keymap: &km, candidates: &cands, username: live.username.as_deref() };
    for it in &mut items {
        it.ext.globals = a.globals_for(&it.cand);
        if let Some(n) = a.name_for(&it.cand) {
            if it.ext.kind == "local" {
                it.ext.name = n;
            }
        }
    }
    let mut spicetify_ext = Extension { id: "spicetify".into(), name: "Spicetify".into(), kind: "spicetify".into(), scanned: true, ..Default::default() };
    let (mut spotify_keys, mut unassigned, mut mp_keys) = (vec![], vec![], vec![]);
    for k in live.local_storage.keys() {
        match a.owner_of_key(k) {
            Owner::Spotify => spotify_keys.push(k.clone()),
            Owner::Spicetify => spicetify_ext.keys.push(k.clone()),
            Owner::Marketplace => mp_keys.push(k.clone()),
            Owner::Unassigned => unassigned.push(k.clone()),
            Owner::Extensions(ids) => {
                for id in ids {
                    if let Some(it) = items.iter_mut().find(|i| i.ext.id == id) {
                        it.ext.keys.push(k.clone());
                    }
                }
            }
        }
    }
    for k in mp.keys() {
        if !mp_keys.contains(k) {
            mp_keys.push(k.clone());
        }
    }
    let mut notes = vec![];
    for name in live.indexed_db.keys() {
        match a.owner_of_database(name) {
            Owner::Extensions(ids) => {
                for id in ids {
                    if let Some(it) = items.iter_mut().find(|i| i.ext.id == id) {
                        it.ext.databases.push(name.clone());
                    }
                }
            }
            Owner::Unassigned => notes.push(format!("IndexedDB database \"{name}\" isn't linked to an extension; it's captured and restored anyway.")),
            _ => {}
        }
    }
    for it in &items {
        if !it.ext.scanned {
            notes.push(format!("Couldn't read the code of {}; its keys may be listed as unassigned.", it.ext.name));
        }
    }

    let (theme_name, scheme, theme_source) = if let Some(active) = mp.get("marketplace:theme-installed") {
        let rec: Value = mp.get(active).and_then(|v| serde_json::from_str(v).ok()).unwrap_or(Value::Null);
        (rec["title"].as_str().map(str::to_owned), rec["activeScheme"].as_str().map(str::to_owned), "marketplace")
    } else if !current_theme.is_empty() && current_theme != "marketplace" {
        (Some(current_theme.clone()), ini.get("Setting", "color_scheme").filter(|s| !s.is_empty()).map(str::to_owned), "local")
    } else {
        (None, None, "none")
    };
    let marketplace_version = crate::restore::marketplace_version(&userdata);

    let mut extensions: Vec<Extension> = items.into_iter().map(|i| i.ext).collect();
    if !spicetify_ext.keys.is_empty() {
        extensions.push(spicetify_ext);
    }
    let id = new_id(now);
    Profile {
        format: FORMAT,
        created_at: now.to_rfc3339(),
        id,
        reason: reason.into(),
        source: source.into(),
        versions: Versions {
            spotify: live.spotify_version.clone().or_else(|| det.spotify.as_ref().and_then(|s| s.version.clone())),
            spicetify: spicetify_version.or_else(|| sp.and_then(|s| s.backup_with.clone())),
            marketplace: marketplace_version,
        },
        account_hash: live.username.as_deref().map(|u| sha256_hex(u.as_bytes())),
        theme: Theme { name: theme_name, colour_scheme: scheme, source: theme_source.into() },
        ini: IniEntries {
            extensions: ini.get_list("AdditionalOptions", "extensions"),
            custom_apps: ini.get_list("AdditionalOptions", "custom_apps"),
            current_theme,
            color_scheme: ini.get("Setting", "color_scheme").unwrap_or("").into(),
            raw: ini.to_text(),
        },
        files,
        storage: Storage { local_storage: live.local_storage, indexed_db: live.indexed_db },
        marketplace: Marketplace { storage: mp_storage, keys: mp_keys },
        extensions,
        spotify_keys,
        unassigned,
        notes,
    }
}

#[derive(Debug)]
pub enum CaptureError {
    /// Spotify runs without the debugging port; the user has to exit it.
    NeedsExit,
    Other(String),
}
impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            CaptureError::NeedsExit => f.write_str("Spotify is running without Pantry's connection. Exit Spotify (⋯ → File → Exit), then try again."),
            CaptureError::Other(s) => f.write_str(s),
        }
    }
}

/// Makes sure Spotify runs with the debugging port: starts it if closed. Never closes a running Spotify.
pub fn connect(det: &Detection, progress: Progress) -> Result<(), CaptureError> {
    match spotify::state() {
        spotify::State::Connected => Ok(()),
        spotify::State::RunningWithoutPort => Err(CaptureError::NeedsExit),
        spotify::State::Closed => {
            let exe = &det.spotify.as_ref().ok_or(CaptureError::Other("Spotify isn't installed.".into()))?.exe;
            progress("Starting Spotify…");
            spotify::launch(exe).map_err(|e| CaptureError::Other(format!("couldn't start Spotify: {e}")))?;
            spotify::wait_page(Duration::from_secs(90)).map_err(CaptureError::Other)?;
            // Give Spicetify and extensions time to load; on stock Spotify this just times out.
            progress("Waiting for extensions to load…");
            if spotify::wait_ready(Duration::from_secs(20)).is_ok() {
                std::thread::sleep(Duration::from_secs(8));
            }
            Ok(())
        }
    }
}

pub fn capture(roots: &Roots, dir: &Path, reason: &str, progress: Progress) -> Result<(Profile, PathBuf), CaptureError> {
    let det = crate::detect::detect(roots);
    if det.spotify.is_none() {
        return Err(CaptureError::Other("Spotify (regular installer) isn't installed.".into()));
    }
    let cli_version = det.spicetify.as_ref().and_then(|s| s.exe.as_ref()).and_then(|e| spotify::spicetify_version(e));
    let (live, source) = match connect(&det, progress) {
        Ok(()) => {
            progress("Reading settings from Spotify…");
            let v = cdp::eval_json(cdp::PORT, &js::capture()).map_err(|e| CaptureError::Other(e.to_string()))?;
            let live = parse_live(&v);
            if live.config.is_null() {
                progress("Spicetify isn't applied right now; capturing what Spotify still has…");
            }
            (live, "cdp")
        }
        Err(CaptureError::Other(e)) if !spotify::is_running() => {
            progress("Spotify didn't start; reading a copy of its Local Storage instead…");
            let ls = crate::leveldb::read_local_storage(&det.spotify.as_ref().unwrap().local_storage).map_err(|e2| CaptureError::Other(format!("{e}; fallback failed too: {e2}")))?;
            (Live { local_storage: ls, ..Default::default() }, "leveldb")
        }
        Err(e) => return Err(e),
    };
    let applied = !live.config.is_null();
    progress("Sorting keys by extension…");
    let mut p = build(&det, live, cli_version, source, reason, chrono::Utc::now());
    if source == "leveldb" {
        p.notes.push("Read from a copy of Local Storage with Spotify closed: Marketplace data and extension databases (IndexedDB) aren't included.".into());
    }
    if source == "cdp" && !applied {
        p.notes.push("Spicetify wasn't applied when this was captured: settings and Marketplace data are included, extensions weren't running.".into());
    }
    let path = p.save(dir).map_err(|e| CaptureError::Other(format!("saving the profile: {e}")))?;
    Ok((p, path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fake(t: &Path) -> (Roots, Detection) {
        let r = Roots { appdata: t.join("Roaming"), localappdata: t.join("Local") };
        let ud = r.appdata.join("spicetify");
        std::fs::create_dir_all(ud.join("Extensions")).unwrap();
        std::fs::create_dir_all(ud.join("CustomApps/marketplace")).unwrap();
        std::fs::create_dir_all(r.appdata.join("Spotify")).unwrap();
        std::fs::write(r.appdata.join("Spotify/Spotify.exe"), b"").unwrap();
        std::fs::write(ud.join("Extensions/gradientBars.js"), "const K = \"gpb:settings\";").unwrap();
        std::fs::write(ud.join("Extensions/unused.js"), "x").unwrap();
        std::fs::write(ud.join("CustomApps/marketplace/extension.js"), "console.log(\"Initializing Spicetify Marketplace v1.0.12\");").unwrap();
        std::fs::write(
            ud.join("config-xpui.ini"),
            "[AdditionalOptions]\nextensions = gradientBars.js\ncustom_apps = marketplace\n[Setting]\ncurrent_theme = marketplace\ncolor_scheme = \n[Backup]\nversion = 1.3.10.2586.gabc\nwith = 2.46.7\n",
        )
        .unwrap();
        let d = crate::detect::detect(&r);
        (r, d)
    }

    fn live_json() -> Value {
        let theme = json!({"type":"theme","title":"Hazy","activeScheme":"Base","include":[],"schemes":{}}).to_string();
        json!({
            "spotifyVersion": "1.3.10.2586",
            "username": "3ccpqn2u57xfh5ggnu6cljg2d",
            "config": {"current_theme":"Hazy","color_scheme":"Base"},
            "localStorage": {
                "gpb:settings": "{}", "gcb:presets": "[]", "spicetify-exp-features": "{}",
                "3ccpqn2u57xfh5ggnu6cljg2d:queue-view": "1", "ES|x|EVENT|1": "e", "who-knows": "1"
            },
            "indexedDB": {
                "keyval-store": {"version": 1, "stores": {"keyval": {"keyPath": null, "autoIncrement": false, "indexes": [], "records": []}}},
                "spicetify-marketplace": {"version": 20, "stores": {"settings": {"keyPath": "key", "autoIncrement": false, "indexes": [], "records": [
                    {"key": "internal:local-storage-migrated", "value": {"key": "internal:local-storage-migrated", "value": "1"}},
                    {"key": "marketplace:theme-installed", "value": {"key": "marketplace:theme-installed", "value": "marketplace:installed:A/Hazy/app.css"}},
                    {"key": "marketplace:installed:A/Hazy/app.css", "value": {"key": "marketplace:installed:A/Hazy/app.css", "value": theme}},
                    {"key": "marketplace:installed-snippets", "value": {"key": "marketplace:installed-snippets", "value": "[\"marketplace:installed:snippet:Duck\"]"}},
                    {"key": "marketplace:installed:snippet:Duck", "value": {"key": "marketplace:installed:snippet:Duck", "value": "{\"title\":\"Duck\",\"code\":\"x\"}"}}
                ]}}},
                "SomeExtDB": {"version": 3, "stores": {"settings": {"keyPath": "id", "autoIncrement": false, "indexes": [], "records": [{"key": 1, "value": {"id": 1}}]}, "trackCache": {"keyPath": "id", "autoIncrement": false, "indexes": [], "records": []}}}
            }
        })
    }

    #[test]
    fn builds_a_complete_profile() {
        let t = tempfile::tempdir().unwrap();
        let (_r, det) = fake(t.path());
        let live = parse_live(&live_json());
        let p = build(&det, live, Some("2.46.7".into()), "cdp", "manual", chrono::Utc::now());
        assert_eq!(p.versions.spotify.as_deref(), Some("1.3.10.2586"));
        assert_eq!(p.versions.marketplace.as_deref(), Some("1.0.12"));
        assert_eq!(p.marketplace.storage, "indexeddb");
        assert!(p.marketplace.keys.contains(&"marketplace:theme-installed".to_string()));
        assert_eq!(p.theme, Theme { name: Some("Hazy".into()), colour_scheme: Some("Base".into()), source: "marketplace".into() });
        let gb = p.extensions.iter().find(|e| e.id == "local:gradientBars.js").unwrap();
        assert_eq!(gb.name, "Gradient Bars");
        assert_eq!(gb.keys, vec!["gcb:presets", "gpb:settings"]);
        assert_eq!(gb.globals, vec!["__gb", "__gpb", "__gcb"]);
        assert!(p.extensions.iter().any(|e| e.kind == "snippet" && e.name == "Duck"));
        assert_eq!(p.extensions.iter().find(|e| e.id == "spicetify").unwrap().keys, vec!["spicetify-exp-features"]);
        assert_eq!(p.spotify_keys.len(), 2);
        assert_eq!(p.unassigned, vec!["who-knows"]);
        assert!(p.ungrouped_keys().is_empty(), "never drop keys");
        assert!(p.files.contains_key("Extensions/gradientBars.js"));
        assert!(!p.files.contains_key("Extensions/unused.js"), "only enabled extensions");
        assert!(p.files.contains_key("CustomApps/marketplace/extension.js"));
        assert_eq!(p.files["Extensions/gradientBars.js"].sha256, sha256_hex(b"const K = \"gpb:settings\";"));
        assert_eq!(p.account_hash.as_deref().map(str::len), Some(64));
        assert!(!serde_json::to_string(&p.extensions).unwrap().contains("3ccpqn2u57xfh5ggnu6cljg2d"));
        assert!(p.storage.indexed_db["keyval-store"].spotify);
        assert!(p.storage.indexed_db["SomeExtDB"].stores["trackCache"].cache);
        assert!(!p.storage.indexed_db["SomeExtDB"].stores["settings"].cache);
        assert!(p.notes.iter().any(|n| n.contains("SomeExtDB")));
    }

    #[test]
    fn finds_loader_urls() {
        let code = "const u = \"https://raw.githubusercontent.com/hoeci/sort-play/main/sort-play.js\"; fetch(`https://cdn.jsdelivr.net/gh/a/b@main/x.mjs`); \"https://example.com/y.js\"";
        assert_eq!(loader_urls(code), vec!["https://raw.githubusercontent.com/hoeci/sort-play/main/sort-play.js", "https://cdn.jsdelivr.net/gh/a/b@main/x.mjs"]);
        assert!(loader_urls(&"x".repeat(70_000)).is_empty());
    }

    #[test]
    fn legacy_marketplace_in_local_storage() {
        let live = Live { local_storage: [("marketplace:installed-extensions".to_string(), "[]".to_string())].into(), ..Default::default() };
        let (s, m) = marketplace_records(&live);
        assert_eq!(s, "localStorage");
        assert_eq!(m.len(), 1);
        assert_eq!(marketplace_records(&Live::default()).0, "none");
    }
}
