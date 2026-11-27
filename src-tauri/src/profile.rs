//! The profile format (#7): one JSON file per capture. Values live once, in `storage`; `extensions`,
//! `spotify_keys` and `unassigned` are indexes into it, so every key is in at least one group and none is dropped.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const FORMAT: u32 = 1;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub format: u32,
    /// Local time of the capture, also the file name: `2026-11-25_23-59-04`, `-2`… on a clash (#47).
    /// Profiles from before #47 used UTC (`2026-11-25T22-59-04Z`); `migrate` renames them.
    pub id: String,
    /// My own name for it (#46); `None` shows the date and time instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub created_at: String,
    /// "manual" or "before-restore" (the automatic capture before every restore).
    pub reason: String,
    /// "cdp" (live Spotify) or "leveldb" (read-only copy, localStorage only).
    pub source: String,
    pub versions: Versions,
    /// SHA-256 of the Spotify account's username, to warn when restoring per-account keys to another account.
    pub account_hash: Option<String>,
    pub theme: Theme,
    pub ini: IniEntries,
    /// Spicetify userdata files, keyed by path relative to the userdata folder with `/` separators.
    pub files: BTreeMap<String, FileEntry>,
    pub storage: Storage,
    pub marketplace: Marketplace,
    pub extensions: Vec<Extension>,
    /// Spotify's own localStorage keys: kept in the capture, not restored by default.
    pub spotify_keys: Vec<String>,
    /// Keys no extension claimed. Restored by default, because they're most likely extension settings.
    pub unassigned: Vec<String>,
    pub notes: Vec<String>,
    /// When this profile was imported from a file (#32); `None` for profiles captured on this PC.
    #[serde(default)]
    pub imported_at: Option<String>,
    /// Exported "for sharing": no Spotify keys, account hash or raw ini.
    #[serde(default)]
    pub shared: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Versions {
    pub spotify: Option<String>,
    pub spicetify: Option<String>,
    pub marketplace: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Theme {
    pub name: Option<String>,
    pub colour_scheme: Option<String>,
    /// "marketplace", "local" or "none".
    pub source: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IniEntries {
    pub extensions: Vec<String>,
    pub custom_apps: Vec<String>,
    pub current_theme: String,
    pub color_scheme: String,
    /// The whole file as it was, for reference. Restore only writes the four fields above.
    pub raw: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub sha256: String,
    pub size: u64,
    pub base64: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Storage {
    pub local_storage: BTreeMap<String, String>,
    pub indexed_db: BTreeMap<String, Database>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Database {
    pub version: u64,
    pub stores: BTreeMap<String, ObjectStore>,
    /// Spotify's own database: schema only, no records.
    #[serde(default)]
    pub spotify: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ObjectStore {
    pub key_path: Value,
    pub auto_increment: bool,
    pub indexes: Vec<Value>,
    /// `{ key, value }` in Pantry's tagged encoding (see js.rs), so Dates, Blobs and typed arrays survive.
    pub records: Vec<Value>,
    /// Looks like a cache (name contains "cache"): captured, not restored by default.
    #[serde(default)]
    pub cache: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Marketplace {
    /// "indexeddb", "localStorage" or "none" (docs/findings.md §3).
    pub storage: String,
    /// Every `marketplace:*` key (and the migration sentinel), wherever it's stored.
    pub keys: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Extension {
    /// `local:<file>`, the Marketplace record key, `theme:<name>`, `app:<folder>` or `spicetify`.
    pub id: String,
    pub name: String,
    /// "local", "marketplace", "theme", "custom-app", "snippet" or "spicetify".
    pub kind: String,
    /// File name or URL the code comes from.
    pub source: Option<String>,
    pub keys: Vec<String>,
    pub databases: Vec<String>,
    /// `window` globals that must exist after a restore (from the key map).
    pub globals: Vec<String>,
    /// Whether the static scan could read this extension's code.
    pub scanned: bool,
}

impl Profile {
    pub fn file_name(&self) -> String {
        format!("{}.json", self.id)
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let p = dir.join(self.file_name());
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, &p)?;
        Ok(p)
    }

    pub fn load(p: &Path) -> std::io::Result<Profile> {
        let v: Profile = serde_json::from_slice(&std::fs::read(p)?)?;
        Ok(v)
    }

    /// A copy for other people: without Spotify's own keys, the account hash and the raw ini (it holds Windows paths).
    /// Everything Restore writes by default stays.
    pub fn for_sharing(&self) -> Profile {
        let mut p = self.clone();
        for k in std::mem::take(&mut p.spotify_keys) {
            p.storage.local_storage.remove(&k);
        }
        p.account_hash = None;
        p.ini.raw.clear();
        p.shared = true;
        p
    }

    /// Every localStorage key belongs to at least one group.
    pub fn ungrouped_keys(&self) -> Vec<&String> {
        self.storage
            .local_storage
            .keys()
            .filter(|k| {
                !self.spotify_keys.contains(k)
                    && !self.unassigned.contains(k)
                    && !self.marketplace.keys.contains(k)
                    && !self.extensions.iter().any(|e| e.keys.contains(k))
            })
            .collect()
    }
}

/// A short summary for the profile list.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: String,
    pub name: Option<String>,
    pub created_at: String,
    pub reason: String,
    pub source: String,
    pub versions: Versions,
    pub theme: Theme,
    pub extensions: usize,
    pub keys: usize,
    pub unassigned: usize,
    pub bytes: u64,
    pub imported_at: Option<String>,
    pub shared: bool,
}

pub fn list(dir: &Path) -> Vec<Summary> {
    let mut out: Vec<Summary> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| {
            let p = Profile::load(&e.path()).ok()?;
            Some(Summary {
                extensions: p.extensions.iter().filter(|x| x.kind != "spicetify").count(),
                keys: p.storage.local_storage.len() + p.storage.indexed_db.values().flat_map(|d| d.stores.values()).map(|s| s.records.len()).sum::<usize>(),
                unassigned: p.unassigned.len(),
                bytes: e.metadata().map(|m| m.len()).unwrap_or(0),
                imported_at: p.imported_at,
                shared: p.shared,
                id: p.id,
                name: p.name,
                created_at: p.created_at,
                reason: p.reason,
                source: p.source,
                versions: p.versions,
                theme: p.theme,
            })
        })
        .collect();
    // Newest first by the real time: ids are local time, which repeats an hour when the clocks go back.
    let at = |s: &Summary| chrono::DateTime::parse_from_rfc3339(&s.created_at).ok();
    out.sort_by(|a, b| at(b).cmp(&at(a)).then_with(|| b.id.cmp(&a.id)));
    out
}

/// Profile ids are file names: a local timestamp from `new_id` (or an old UTC one from before #47),
/// optionally with `-2`, `-3`… after a clash.
fn id_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"^(\d{4}-\d{2}-\d{2}_\d{2}-\d{2}-\d{2}|\d{4}-\d{2}-\d{2}T\d{2}-\d{2}-\d{2}Z)(-\d{1,4})?$").unwrap())
}

pub fn valid_id(id: &str) -> bool {
    id_re().is_match(id)
}

/// The timestamp part of an id, without its `-N`.
pub fn id_base(id: &str) -> Option<&str> {
    id_re().captures(id).and_then(|c| c.get(1)).map(|m| m.as_str())
}

/// `id` in the local-time scheme: an old UTC id becomes the local time it stood for (its `-N` dropped,
/// `free_id` adds a new one if needed); anything else comes back as it is.
pub fn localise_id<Tz: chrono::TimeZone>(id: &str, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    match id_base(id).filter(|b| b.ends_with('Z')).and_then(|b| chrono::NaiveDateTime::parse_from_str(b, "%Y-%m-%dT%H-%M-%SZ").ok()) {
        Some(t) => new_id(&t.and_utc().with_timezone(tz)),
        None => id.to_owned(),
    }
}

/// `base`, or `base-2`, `base-3`… if a profile with that id is already in `dir`.
pub fn free_id(dir: &Path, base: &str) -> String {
    (1..10_000).map(|n| if n == 1 { base.to_owned() } else { format!("{base}-{n}") }).find(|id| !dir.join(format!("{id}.json")).exists()).unwrap_or_else(|| base.to_owned())
}

/// Renames profiles from before #47 (UTC file names) to local time. Returns the old → new ids;
/// a file that can't be read or written stays as it is and is still listed.
pub fn migrate<Tz: chrono::TimeZone>(dir: &Path, tz: &Tz) -> Vec<(String, String)>
where
    Tz::Offset: std::fmt::Display,
{
    let mut done = vec![];
    let mut old: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json") && p.file_stem().and_then(|s| s.to_str()).and_then(id_base).is_some_and(|b| b.ends_with('Z')))
        .collect();
    old.sort();
    for path in old {
        let Ok(mut p) = Profile::load(&path) else { continue };
        let from = path.file_stem().unwrap().to_string_lossy().into_owned();
        p.id = free_id(dir, &localise_id(&from, tz));
        if p.save(dir).is_ok() && std::fs::remove_file(&path).is_ok() {
            done.push((from, p.id));
        }
    }
    done
}

/// The path of a profile in `dir`, or an error for an id that isn't a valid file name.
pub fn path_of(dir: &Path, id: &str) -> std::io::Result<PathBuf> {
    if valid_id(id) {
        Ok(dir.join(format!("{id}.json")))
    } else {
        Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("not a profile id: {id:?}")))
    }
}

pub const NAME_MAX: usize = 80;

/// Gives profile `id` a name; an empty (or all-space) name clears it. Returns the name as saved.
pub fn rename(dir: &Path, id: &str, name: &str) -> Result<Option<String>, String> {
    let name: String = name.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
    if name.chars().count() > NAME_MAX {
        return Err(format!("Names can be up to {NAME_MAX} characters."));
    }
    let path = path_of(dir, id).map_err(|e| e.to_string())?;
    let mut p = Profile::load(&path).map_err(|_| format!("There's no profile {id}."))?;
    p.name = (!name.is_empty()).then_some(name);
    p.save(dir).map_err(|e| format!("couldn't save the profile: {e}"))?;
    Ok(p.name)
}

/// Deletes profile `id` from `dir`: to the Recycle Bin, so a mis-click can be undone (permanently in unit tests).
pub fn delete(dir: &Path, id: &str) -> Result<(), String> {
    let p = path_of(dir, id).map_err(|e| e.to_string())?;
    if !p.is_file() {
        return Err(format!("There's no profile {id}."));
    }
    #[cfg(not(test))]
    return trash::delete(&p).map_err(|e| format!("couldn't move {id} to the Recycle Bin: {e}"));
    #[cfg(test)]
    return std::fs::remove_file(&p).map_err(|e| e.to_string());
}

/// Moves profiles from the old data folder (`%APPDATA%\Pantry`, before #49) to the new one. Never overwrites:
/// an identical file is just dropped, a different one with the same name is kept as `-2`, `-3`…
/// The old folder is removed once it's empty. Returns how many profiles moved.
pub fn move_data_dir(old: &Path, new: &Path) -> usize {
    let (from, to) = (old.join("profiles"), new.join("profiles"));
    if !from.is_dir() {
        return 0;
    }
    if !new.exists() && std::fs::rename(old, new).is_ok() {
        return std::fs::read_dir(&to).map(|d| d.flatten().count()).unwrap_or(0);
    }
    if std::fs::create_dir_all(&to).is_err() {
        return 0;
    }
    let mut moved = 0;
    for e in std::fs::read_dir(&from).into_iter().flatten().flatten() {
        let (src, name) = (e.path(), e.file_name());
        let dst = to.join(&name);
        let done = if !dst.exists() {
            std::fs::rename(&src, &dst).or_else(|_| std::fs::copy(&src, &dst).and_then(|_| std::fs::remove_file(&src))).is_ok()
        } else if std::fs::read(&src).ok() == std::fs::read(&dst).ok() {
            std::fs::remove_file(&src).is_ok()
        } else {
            // Same name, different content: keep both.
            match (Profile::load(&src), src.file_stem().and_then(|s| s.to_str()).filter(|s| valid_id(s))) {
                (Ok(mut p), Some(stem)) => {
                    p.id = free_id(&to, stem);
                    p.save(&to).is_ok() && std::fs::remove_file(&src).is_ok()
                }
                _ => false,
            }
        };
        moved += done as usize;
    }
    let _ = std::fs::remove_dir(&from);
    let _ = std::fs::remove_dir(old);
    moved
}

/// The id for a capture at `now`, in the time zone `now` carries: `2026-11-27_10-10-04`.
pub fn new_id<Tz: chrono::TimeZone>(now: &chrono::DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    now.format("%Y-%m-%d_%H-%M-%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Profile {
        let mut p = Profile { format: FORMAT, id: "2026-11-25_23-59-04".into(), reason: "manual".into(), source: "cdp".into(), ..Default::default() };
        p.storage.local_storage.insert("gpb:settings".into(), "{}".into());
        p.storage.local_storage.insert("ES|x".into(), "1".into());
        p.storage.local_storage.insert("mystery".into(), "1".into());
        p.extensions.push(Extension { id: "local:gradientBars.js".into(), keys: vec!["gpb:settings".into()], ..Default::default() });
        p.spotify_keys.push("ES|x".into());
        p.unassigned.push("mystery".into());
        p
    }

    #[test]
    fn round_trips_through_disk() {
        let t = tempfile::tempdir().unwrap();
        let p = sample();
        let path = p.save(t.path()).unwrap();
        assert_eq!(path.file_name().unwrap(), "2026-11-25_23-59-04.json");
        assert_eq!(Profile::load(&path).unwrap(), p);
        let l = list(t.path());
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].keys, 3);
        assert_eq!(l[0].unassigned, 1);
    }

    #[test]
    fn every_key_is_grouped() {
        let mut p = sample();
        assert!(p.ungrouped_keys().is_empty());
        p.storage.local_storage.insert("lost".into(), "1".into());
        assert_eq!(p.ungrouped_keys(), vec!["lost"]);
    }

    #[test]
    fn sharing_copy_leaves_out_private_data() {
        let mut p = sample();
        p.account_hash = Some("abc".into());
        p.ini.raw = "prefs_path = C:\\Users\\VC\\AppData\\Roaming\\Spotify\\prefs".into();
        p.ini.current_theme = "Hazy".into();
        let s = p.for_sharing();
        assert!(s.shared && s.account_hash.is_none() && s.ini.raw.is_empty() && s.spotify_keys.is_empty());
        assert!(!s.storage.local_storage.contains_key("ES|x"));
        assert!(s.storage.local_storage.contains_key("gpb:settings") && s.storage.local_storage.contains_key("mystery"));
        assert_eq!(s.ini.current_theme, "Hazy");
        assert!(s.ungrouped_keys().is_empty());
    }

    #[test]
    fn old_profiles_without_new_fields_load() {
        let mut v = serde_json::to_value(sample()).unwrap();
        v.as_object_mut().unwrap().remove("importedAt");
        v.as_object_mut().unwrap().remove("shared");
        v.as_object_mut().unwrap().remove("name");
        let p: Profile = serde_json::from_value(v).unwrap();
        assert!(p.imported_at.is_none() && !p.shared && p.name.is_none());
    }

    #[test]
    fn ids_are_checked() {
        assert!(valid_id("2026-11-25T22-59-04Z") && valid_id("2026-11-25T22-59-04Z-2"));
        for bad in ["../x", "2026-11-25T22-59-04Z/../../a", "", "2026-11-25T22-59-04Z.json", "a\\b"] {
            assert!(!valid_id(bad), "{bad}");
        }
        assert!(path_of(Path::new("d"), "../x").is_err());
    }

    #[test]
    fn delete_removes_only_valid_existing_profiles() {
        let t = tempfile::tempdir().unwrap();
        let p = sample();
        p.save(t.path()).unwrap();
        assert!(delete(t.path(), "../x").is_err());
        assert!(delete(t.path(), "2026-01-01T00-00-00Z").unwrap_err().contains("no profile"));
        delete(t.path(), &p.id).unwrap();
        assert!(list(t.path()).is_empty());
    }

    #[test]
    fn ids_are_file_safe() {
        let a = new_id(&chrono::DateTime::parse_from_rfc3339("2026-11-25T09:05:00Z").unwrap());
        assert_eq!(a, "2026-11-25_09-05-00");
        assert!(!a.contains(':') && valid_id(&a));
    }

    #[test]
    fn ids_are_local_time() {
        let cet = chrono::FixedOffset::east_opt(3600).unwrap();
        let t = chrono::DateTime::parse_from_rfc3339("2026-11-27T09:10:04Z").unwrap().with_timezone(&cet);
        assert_eq!(new_id(&t), "2026-11-27_10-10-04");
        assert_eq!(localise_id("2026-11-27T09-10-04Z", &cet), "2026-11-27_10-10-04");
        assert_eq!(localise_id("2026-11-27T09-10-04Z-3", &cet), "2026-11-27_10-10-04");
        assert_eq!(localise_id("2026-11-27_10-10-04-2", &cet), "2026-11-27_10-10-04-2");
        for ok in ["2026-11-27_10-10-04", "2026-11-27_10-10-04-2", "2026-11-27T09-10-04Z", "2026-11-27T09-10-04Z-12"] {
            assert!(valid_id(ok), "{ok}");
        }
        for bad in ["selftest", "../x", "2026-11-27_10-10-04Z", "2026-11-27 10-10-04"] {
            assert!(!valid_id(bad), "{bad}");
        }
        assert_eq!(id_base("2026-11-27_10-10-04-2"), Some("2026-11-27_10-10-04"));
    }

    #[test]
    fn clashing_ids_get_a_number() {
        let t = tempfile::tempdir().unwrap();
        let p = sample();
        p.save(t.path()).unwrap();
        assert_eq!(free_id(t.path(), &p.id), format!("{}-2", p.id));
        assert_eq!(free_id(t.path(), "2026-01-01_00-00-00"), "2026-01-01_00-00-00");
    }

    #[test]
    fn old_utc_names_are_migrated() {
        let t = tempfile::tempdir().unwrap();
        let cet = chrono::FixedOffset::east_opt(3600).unwrap();
        let mut a = sample();
        a.id = "2026-11-27T09-10-04Z".into();
        a.created_at = "2026-11-27T09:10:04+00:00".into();
        a.save(t.path()).unwrap();
        let mut b = a.clone();
        b.id = "2026-11-27T09-10-04Z-2".into();
        b.shared = true;
        b.save(t.path()).unwrap();
        let moved = migrate(t.path(), &cet);
        assert_eq!(moved.len(), 2);
        let l = list(t.path());
        let ids: Vec<&str> = l.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"2026-11-27_10-10-04") && ids.contains(&"2026-11-27_10-10-04-2"), "{ids:?}");
        assert!(!t.path().join("2026-11-27T09-10-04Z.json").exists());
        assert_eq!(Profile::load(&t.path().join("2026-11-27_10-10-04.json")).unwrap().id, "2026-11-27_10-10-04");
        assert!(migrate(t.path(), &cet).is_empty());
    }

    #[test]
    fn list_is_newest_first_by_time() {
        let t = tempfile::tempdir().unwrap();
        // Clocks go back: the later capture has the smaller local id.
        for (id, at) in [("2026-10-25_02-30-00", "2026-10-25T00:30:00+00:00"), ("2026-10-25_02-10-00", "2026-10-25T01:10:00+00:00")] {
            let mut p = sample();
            p.id = id.into();
            p.created_at = at.into();
            p.save(t.path()).unwrap();
        }
        assert_eq!(list(t.path())[0].id, "2026-10-25_02-10-00");
    }

    #[test]
    fn old_data_folder_is_moved() {
        let t = tempfile::tempdir().unwrap();
        let (old, new) = (t.path().join("Pantry"), t.path().join("Spicetify Pantry"));
        let p = sample();
        p.save(&old.join("profiles")).unwrap();
        assert_eq!(move_data_dir(&old, &new), 1);
        assert!(!old.exists() && new.join("profiles").join(p.file_name()).is_file());
        assert_eq!(move_data_dir(&old, &new), 0);
    }

    #[test]
    fn moving_into_an_existing_folder_keeps_both() {
        let t = tempfile::tempdir().unwrap();
        let (old, new) = (t.path().join("Pantry"), t.path().join("Spicetify Pantry"));
        let p = sample();
        p.save(&new.join("profiles")).unwrap();
        p.save(&old.join("profiles")).unwrap(); // identical: dropped
        let mut q = sample();
        q.id = "2026-01-02_00-00-00".into();
        q.save(&old.join("profiles")).unwrap(); // new name: moved
        let mut r = sample();
        r.shared = true;
        r.save(&old.join("profiles")).unwrap(); // overwrites the identical one in old/
        assert_eq!(move_data_dir(&old, &new), 2);
        let ids: Vec<String> = list(&new.join("profiles")).into_iter().map(|s| s.id).collect();
        assert_eq!(ids.len(), 3, "{ids:?}");
        assert!(ids.contains(&format!("{}-2", p.id)));
        assert!(!old.exists());
    }

    #[test]
    fn rename_sets_tidies_and_clears() {
        let t = tempfile::tempdir().unwrap();
        let p = sample();
        p.save(t.path()).unwrap();
        assert_eq!(rename(t.path(), &p.id, "  Before   Spotify\nupdate ").unwrap().as_deref(), Some("Before Spotify update"));
        assert_eq!(list(t.path())[0].name.as_deref(), Some("Before Spotify update"));
        assert_eq!(Profile::load(&t.path().join(p.file_name())).unwrap().id, p.id);
        assert!(rename(t.path(), &p.id, &"x".repeat(NAME_MAX + 1)).is_err());
        assert_eq!(rename(t.path(), &p.id, &"é".repeat(NAME_MAX)).unwrap().map(|n| n.chars().count()), Some(NAME_MAX));
        assert_eq!(rename(t.path(), &p.id, "   ").unwrap(), None);
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(t.path().join(p.file_name())).unwrap()).unwrap();
        assert!(v.get("name").is_none());
        assert!(rename(t.path(), "2020-01-01_00-00-00", "x").is_err());
        assert!(rename(t.path(), "../x", "x").is_err());
    }
}
