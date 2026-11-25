//! Restore (#12): version check → automatic capture → Spicetify files and ini entries → `spicetify -n apply`
//! → start Spotify with the debugging port → write keys over CDP → reload → check each extension is back.
//! Spotify's LevelDB files are never touched; every storage write goes through CDP.

use crate::capture::{self, decode_file, sha256_hex, Progress};
use crate::detect::{short_version, Detection, Roots};
use crate::ini::Ini;
use crate::profile::*;
use crate::{attribute, cdp, js, spotify};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    /// Also restore Spotify's own keys (never telemetry; per-account keys only for the same account).
    #[serde(default)]
    pub include_spotify: bool,
    /// The user saw the version warnings and wants to go ahead.
    #[serde(default)]
    pub acknowledged: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub warnings: Vec<String>,
    pub blockers: Vec<String>,
}

pub fn check(p: &Profile, det: &Detection, spicetify_now: Option<&str>) -> Check {
    let mut c = Check::default();
    match &det.spotify {
        None => c.blockers.push("Spotify (regular installer) isn't installed.".into()),
        Some(s) => {
            if let (Some(a), Some(b)) = (&p.versions.spotify, &s.version) {
                if short_version(a) != short_version(b) {
                    c.warnings.push(format!("Spotify version differs: profile {}, installed {}.", short_version(a), short_version(b)));
                }
            }
        }
    }
    if det.spicetify.as_ref().and_then(|s| s.exe.as_ref()).is_none() {
        c.blockers.push("Spicetify isn't installed. Install it first, then restore.".into());
    } else if let (Some(a), Some(b)) = (&p.versions.spicetify, spicetify_now) {
        if a != b {
            c.warnings.push(format!("Spicetify version differs: profile {a}, installed {b}."));
        }
    }
    if p.source == "leveldb" {
        c.warnings.push("This profile was read with Spotify closed: it has no Marketplace data or extension databases.".into());
    }
    c
}

/// What gets written into the page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pub local_storage: BTreeMap<String, String>,
    pub indexed_db: BTreeMap<String, Database>,
    pub skipped_spotify: usize,
    pub skipped_account: usize,
}

/// `live_mp` is where the running Marketplace keeps its data: "indexeddb", "localStorage" or "none".
pub fn select(p: &Profile, o: &Options, live_mp: &str, same_account: bool) -> Selection {
    let mut s = Selection::default();
    let mut mp: BTreeMap<String, String> = BTreeMap::new();
    for (k, v) in &p.storage.local_storage {
        if attribute::is_never_restored(k) {
            s.skipped_spotify += 1;
            continue;
        }
        if k.starts_with("marketplace:") {
            mp.insert(k.clone(), v.clone());
            continue;
        }
        if p.spotify_keys.contains(k) {
            if !o.include_spotify {
                s.skipped_spotify += 1;
                continue;
            }
            let per_account = !k.starts_with("com.spotify.");
            if per_account && !same_account {
                s.skipped_account += 1;
                continue;
            }
        }
        s.local_storage.insert(k.clone(), v.clone());
    }
    if let Some(db) = p.storage.indexed_db.get("spicetify-marketplace") {
        let (_, m) = capture::marketplace_records(&capture::Live { indexed_db: [("spicetify-marketplace".to_string(), db.clone())].into(), ..Default::default() });
        mp.extend(m);
    }
    mp.remove("internal:local-storage-migrated");
    match live_mp {
        "indexeddb" if !mp.is_empty() => {
            let records = mp.iter().map(|(k, v)| json!({ "key": k, "value": { "key": k, "value": v } })).collect();
            let store = ObjectStore { key_path: json!("key"), records, ..Default::default() };
            let version = p.storage.indexed_db.get("spicetify-marketplace").map(|d| d.version).unwrap_or(20);
            s.indexed_db.insert("spicetify-marketplace".into(), Database { version, stores: [("settings".to_string(), store)].into(), spotify: false });
        }
        "localStorage" => s.local_storage.extend(mp),
        _ => {}
    }
    for (name, db) in &p.storage.indexed_db {
        if db.spotify || name == "spicetify-marketplace" {
            continue;
        }
        let stores: BTreeMap<String, ObjectStore> = db.stores.iter().filter(|(_, st)| !st.cache).map(|(n, st)| (n.clone(), st.clone())).collect();
        if stores.values().any(|st| !st.records.is_empty()) {
            s.indexed_db.insert(name.clone(), Database { version: db.version, stores, spotify: false });
        }
    }
    s
}

pub fn payload(s: &Selection) -> Value {
    json!({ "localStorage": s.local_storage, "indexedDB": s.indexed_db })
}

/// Writes the profile's Spicetify files and ini entries. Returns (files written, files already identical).
pub fn write_spicetify(p: &Profile, userdata: &Path) -> Result<(usize, usize), String> {
    let (mut wrote, mut same) = (0, 0);
    for (rel, f) in &p.files {
        if rel.split('/').any(|seg| seg == ".." || seg.is_empty()) {
            return Err(format!("refusing unsafe path {rel}"));
        }
        let path = userdata.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        if std::fs::read(&path).is_ok_and(|b| sha256_hex(&b) == f.sha256) {
            same += 1;
            continue;
        }
        let bytes = decode_file(f).ok_or_else(|| format!("{rel}: damaged in the profile"))?;
        if sha256_hex(&bytes) != f.sha256 {
            return Err(format!("{rel}: checksum doesn't match the profile"));
        }
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("{rel}: {e}"))?;
        }
        std::fs::write(&path, &bytes).map_err(|e| format!("{rel}: {e}"))?;
        wrote += 1;
    }
    let ini_path = userdata.join("config-xpui.ini");
    let mut ini = Ini::load(&ini_path).map_err(|e| format!("config-xpui.ini: {e}"))?;
    ini.set_list("AdditionalOptions", "extensions", &p.ini.extensions);
    ini.set_list("AdditionalOptions", "custom_apps", &p.ini.custom_apps);
    if !p.ini.current_theme.is_empty() {
        ini.set("Setting", "current_theme", &p.ini.current_theme);
    }
    ini.set("Setting", "color_scheme", &p.ini.color_scheme);
    ini.save(&ini_path).map_err(|e| format!("config-xpui.ini: {e}"))?;
    Ok((wrote, same))
}

/// Everything the self-check looks for after the reload.
pub fn expectations(p: &Profile) -> (Vec<String>, Vec<String>) {
    let mut globals: Vec<String> = p.extensions.iter().flat_map(|e| e.globals.iter().cloned()).collect();
    globals.sort();
    globals.dedup();
    let scripts = p
        .extensions
        .iter()
        .filter(|e| e.kind == "local" || e.kind == "marketplace")
        .filter_map(|e| e.source.as_deref())
        .map(|s| s.rsplit('/').next().unwrap_or(s).to_owned())
        .collect();
    (globals, scripts)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub ok: bool,
    /// Set when the restore stopped before doing anything because the user must confirm.
    pub needs_confirmation: Option<Check>,
    pub backup_id: Option<String>,
    pub steps: Vec<Step>,
}

impl Report {
    fn step(&mut self, name: &str, ok: bool, detail: impl Into<String>) -> bool {
        self.steps.push(Step { name: name.into(), ok, detail: detail.into() });
        ok
    }
}

fn wait_marketplace(expect: bool) -> String {
    if !expect {
        return "none".into();
    }
    let t0 = std::time::Instant::now();
    while t0.elapsed() < Duration::from_secs(30) {
        if let Ok(v) = cdp::eval_json(cdp::PORT, js::MARKETPLACE_STATE) {
            if v["indexedDb"] == true {
                return "indexeddb".into();
            }
        }
        std::thread::sleep(Duration::from_millis(700));
    }
    // An old Marketplace (before PR #1181) never creates the database.
    "localStorage".into()
}

fn reload_and_wait() -> Result<(), String> {
    cdp::eval_json(cdp::PORT, js::RELOAD).map_err(|e| e.to_string())?;
    std::thread::sleep(Duration::from_secs(2));
    spotify::wait_ready(Duration::from_secs(60))?;
    std::thread::sleep(Duration::from_secs(8));
    Ok(())
}

pub fn restore(roots: &Roots, dir: &Path, id: &str, o: &Options, progress: Progress) -> Report {
    let mut r = Report::default();
    let p = match Profile::load(&dir.join(format!("{id}.json"))) {
        Ok(p) => p,
        Err(e) => {
            r.step("Load profile", false, e.to_string());
            return r;
        }
    };
    let det = crate::detect::detect(roots);
    let cli = det.spicetify.as_ref().and_then(|s| s.exe.clone());
    let now = cli.as_ref().and_then(|e| spotify::spicetify_version(e));
    let c = check(&p, &det, now.as_deref());
    if !c.blockers.is_empty() {
        r.step("Version check", false, c.blockers.join(" "));
        return r;
    }
    if !c.warnings.is_empty() && !o.acknowledged {
        r.needs_confirmation = Some(c);
        return r;
    }
    r.step("Version check", true, if c.warnings.is_empty() { "Versions match.".to_string() } else { format!("Continued despite: {}", c.warnings.join(" ")) });

    progress("Capturing the current state first…");
    match capture::capture(roots, dir, "before-restore", progress) {
        Ok((b, _)) => {
            r.backup_id = Some(b.id.clone());
            r.step("Automatic capture", true, format!("Saved as {}.", b.id));
        }
        Err(e) => {
            r.step("Automatic capture", false, format!("{e} Nothing was changed."));
            return r;
        }
    }

    progress("Closing Spotify…");
    if let Err(e) = spotify::close(Duration::from_secs(30)) {
        r.step("Close Spotify", false, e);
        return r;
    }
    r.step("Close Spotify", true, "Closed normally.");

    let userdata = det.spicetify.as_ref().unwrap().userdata.clone();
    progress("Writing Spicetify files and settings…");
    match write_spicetify(&p, &userdata) {
        Ok((w, s)) => r.step("Spicetify files", true, format!("{w} written, {s} already identical; config-xpui.ini updated.")),
        Err(e) => return { r.step("Spicetify files", false, e); r },
    };

    progress("Running spicetify apply…");
    let cli = cli.unwrap();
    let mut out = spotify::spicetify(&cli, &["-n", "apply"]);
    let needs_backup = out.as_ref().is_ok_and(|o| !o.ok && o.text.contains("backup apply"));
    if needs_backup {
        progress("Spotify was updated: running spicetify backup apply…");
        out = spotify::spicetify(&cli, &["-n", "backup", "apply"]);
    }
    match out {
        Ok(o) if o.ok => r.step("spicetify apply", true, if needs_backup { "Backed up the new Spotify version and applied." } else { "Applied." }),
        Ok(o) => return { r.step("spicetify apply", false, o.text.lines().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n")); r },
        Err(e) => return { r.step("spicetify apply", false, e.to_string()); r },
    };

    progress("Starting Spotify…");
    if let Err(e) = spotify::launch(&det.spotify.as_ref().unwrap().exe).map_err(|e| e.to_string()).and_then(|_| spotify::wait_ready(Duration::from_secs(90))) {
        r.step("Start Spotify", false, e);
        return r;
    }
    r.step("Start Spotify", true, "Running with Pantry's connection.");

    let live_mp = wait_marketplace(p.ini.custom_apps.iter().any(|a| a == "marketplace"));
    let live = cdp::eval_json(cdp::PORT, "JSON.stringify(Spicetify.Platform.username || null)").ok();
    let same_account = match (&p.account_hash, live.as_ref().and_then(|v| v.as_str())) {
        (Some(h), Some(u)) => *h == sha256_hex(u.as_bytes()),
        _ => false,
    };
    let sel = select(&p, o, &live_mp, same_account);
    progress("Writing settings into Spotify…");
    match cdp::eval_json(cdp::PORT, &js::restore(&payload(&sel))) {
        Ok(v) => {
            let errors: Vec<String> = v["errors"].as_array().into_iter().flatten().filter_map(|e| e.as_str().map(str::to_owned)).collect();
            let mut d = format!("{} keys and {} database records written.", v["keys"], v["records"]);
            if sel.skipped_spotify > 0 {
                d.push_str(&format!(" {} Spotify keys left out.", sel.skipped_spotify));
            }
            if sel.skipped_account > 0 {
                d.push_str(&format!(" {} per-account keys left out (different Spotify account).", sel.skipped_account));
            }
            if !errors.is_empty() {
                d.push_str(&format!(" Errors: {}", errors.join("; ")));
            }
            r.step("Write settings", errors.is_empty(), d);
        }
        Err(e) => return { r.step("Write settings", false, e.to_string()); r },
    }

    progress("Reloading Spotify…");
    if let Err(e) = reload_and_wait() {
        r.step("Reload", false, e);
        return r;
    }
    // Extensions can rewrite their keys while the page unloads or starts; put back anything that changed, once.
    let after = cdp::eval_json(cdp::PORT, &js::capture()).map(|v| capture::parse_live(&v).local_storage).unwrap_or_default();
    let changed: BTreeMap<String, String> = sel.local_storage.iter().filter(|(k, v)| after.get(*k) != Some(*v)).map(|(k, v)| (k.clone(), v.clone())).collect();
    if changed.is_empty() {
        r.step("Reload", true, "All written keys survived the reload.");
    } else {
        let n = changed.len();
        let again = Selection { local_storage: changed, ..Default::default() };
        let ok = cdp::eval_json(cdp::PORT, &js::restore(&payload(&again))).is_ok() && reload_and_wait().is_ok();
        r.step("Reload", ok, format!("{n} keys were changed by an extension at startup and were written again."));
    }

    progress("Checking extensions…");
    let (globals, scripts) = expectations(&p);
    match cdp::eval_json(cdp::PORT, &js::check(&globals, &scripts)) {
        Ok(v) => {
            let missing: Vec<String> = ["globals", "scripts"]
                .iter()
                .flat_map(|g| v[*g].as_object().into_iter().flatten().filter(|(_, ok)| **ok != true).map(|(n, _)| n.clone()).collect::<Vec<_>>())
                .collect();
            let total = globals.len() + scripts.len();
            r.step("Extensions running", missing.is_empty(), if missing.is_empty() { format!("{total}/{total} found.") } else { format!("Missing: {}", missing.join(", ")) });
        }
        Err(e) => {
            r.step("Extensions running", false, e.to_string());
        }
    }
    r.ok = r.steps.iter().all(|s| s.ok);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> Profile {
        let mut p = Profile { source: "cdp".into(), ..Default::default() };
        for (k, v) in [("gpb:settings", "{}"), ("ES|x", "1"), ("3ccpqn2u57xfh5ggnu6cljg2d:queue-view", "1"), ("com.spotify.cache", "c"), ("who-knows", "1"), ("spicetify-exp-features", "{}")] {
            p.storage.local_storage.insert(k.into(), v.into());
        }
        p.spotify_keys = vec!["ES|x".into(), "3ccpqn2u57xfh5ggnu6cljg2d:queue-view".into(), "com.spotify.cache".into()];
        let mp = ObjectStore {
            key_path: json!("key"),
            records: vec![
                json!({"key": "internal:local-storage-migrated", "value": {"key": "internal:local-storage-migrated", "value": "1"}}),
                json!({"key": "marketplace:tabs", "value": {"key": "marketplace:tabs", "value": "[]"}}),
            ],
            ..Default::default()
        };
        p.storage.indexed_db.insert("spicetify-marketplace".into(), Database { version: 20, stores: [("settings".to_string(), mp)].into(), spotify: false });
        let ext = ObjectStore { key_path: json!("id"), records: vec![json!({"key": 1, "value": {"id": 1}})], ..Default::default() };
        let cache = ObjectStore { key_path: json!("id"), records: vec![json!({"key": 2, "value": {"id": 2}})], cache: true, ..Default::default() };
        p.storage.indexed_db.insert("SortPlayDB".into(), Database { version: 18, stores: [("settings".to_string(), ext), ("trackCache".to_string(), cache)].into(), spotify: false });
        p.storage.indexed_db.insert("keyval-store".into(), Database { version: 1, spotify: true, ..Default::default() });
        p
    }

    #[test]
    fn default_selection_leaves_spotify_out() {
        let s = select(&profile(), &Options::default(), "indexeddb", true);
        assert_eq!(s.local_storage.keys().collect::<Vec<_>>(), vec!["gpb:settings", "spicetify-exp-features", "who-knows"]);
        assert_eq!(s.skipped_spotify, 3);
        let mp = &s.indexed_db["spicetify-marketplace"].stores["settings"];
        assert_eq!(mp.records.len(), 1, "the migration sentinel stays Marketplace's own");
        assert_eq!(s.indexed_db["SortPlayDB"].stores.keys().collect::<Vec<_>>(), vec!["settings"], "caches stay out");
        assert!(!s.indexed_db.contains_key("keyval-store"));
    }

    #[test]
    fn including_spotify_still_skips_telemetry_and_other_accounts() {
        let o = Options { include_spotify: true, acknowledged: false };
        let s = select(&profile(), &o, "indexeddb", false);
        assert!(s.local_storage.contains_key("com.spotify.cache"));
        assert!(!s.local_storage.contains_key("ES|x"));
        assert!(!s.local_storage.contains_key("3ccpqn2u57xfh5ggnu6cljg2d:queue-view"));
        assert_eq!(s.skipped_account, 1);
        assert!(select(&profile(), &o, "indexeddb", true).local_storage.contains_key("3ccpqn2u57xfh5ggnu6cljg2d:queue-view"));
    }

    #[test]
    fn marketplace_goes_where_the_live_marketplace_reads() {
        let s = select(&profile(), &Options::default(), "localStorage", true);
        assert_eq!(s.local_storage.get("marketplace:tabs").map(String::as_str), Some("[]"));
        assert!(!s.indexed_db.contains_key("spicetify-marketplace"));
        let none = select(&profile(), &Options::default(), "none", true);
        assert!(!none.local_storage.contains_key("marketplace:tabs"));
    }

    #[test]
    fn writes_files_and_ini_and_refuses_bad_paths() {
        let t = tempfile::tempdir().unwrap();
        let ud = t.path();
        std::fs::write(ud.join("config-xpui.ini"), "[AdditionalOptions]\nextensions = old.js\ncustom_apps = \n[Setting]\ncurrent_theme = marketplace\ncolor_scheme = \n").unwrap();
        let mut p = Profile::default();
        p.files.insert("Extensions/a.js".into(), capture::file_entry(b"a"));
        p.files.insert("Themes/T/user.css".into(), capture::file_entry(b"css"));
        p.ini = IniEntries { extensions: vec!["a.js".into()], custom_apps: vec!["marketplace".into()], current_theme: "T".into(), color_scheme: "Dark".into(), raw: String::new() };
        assert_eq!(write_spicetify(&p, ud).unwrap(), (2, 0));
        assert_eq!(write_spicetify(&p, ud).unwrap(), (0, 2));
        let ini = Ini::load(&ud.join("config-xpui.ini")).unwrap();
        assert_eq!(ini.get("AdditionalOptions", "extensions"), Some("a.js"));
        assert_eq!(ini.get("Setting", "color_scheme"), Some("Dark"));
        assert_eq!(std::fs::read(ud.join("Themes/T/user.css")).unwrap(), b"css");
        p.files.insert("../evil.js".into(), capture::file_entry(b"x"));
        assert!(write_spicetify(&p, ud).is_err());
        let mut bad = Profile::default();
        let mut f = capture::file_entry(b"x");
        f.sha256 = "0".repeat(64);
        bad.files.insert("Extensions/b.js".into(), f);
        assert!(write_spicetify(&bad, ud).unwrap_err().contains("checksum"));
    }

    #[test]
    fn version_check() {
        let t = tempfile::tempdir().unwrap();
        let r = Roots { appdata: t.path().into(), localappdata: t.path().into() };
        let mut p = profile();
        p.versions.spotify = Some("1.3.8.620".into());
        let c = check(&p, &crate::detect::detect(&r), None);
        assert!(c.blockers.iter().any(|b| b.contains("Spotify")));
        let det = Detection {
            spotify: Some(crate::detect::Spotify { exe: "x".into(), version: Some("1.3.10.2586.gabc".into()), prefs: "p".into(), local_storage: "l".into() }),
            store_spotify: false,
            spicetify: Some(crate::detect::Spicetify { exe: Some("s".into()), userdata: "u".into(), ini: "i".into(), backup_with: None, backup_spotify: None }),
        };
        p.versions.spicetify = Some("2.46.4".into());
        let c = check(&p, &det, Some("2.46.7"));
        assert!(c.blockers.is_empty());
        assert_eq!(c.warnings.len(), 2);
        p.versions.spotify = Some("1.3.10.2586.gzzz".into());
        p.versions.spicetify = Some("2.46.7".into());
        assert!(check(&p, &det, Some("2.46.7")).warnings.is_empty());
    }
}
