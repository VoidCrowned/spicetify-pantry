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
    /// UTC timestamp, also the file name: `2026-11-25T22-59-04Z`.
    pub id: String,
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
    pub created_at: String,
    pub reason: String,
    pub source: String,
    pub versions: Versions,
    pub theme: Theme,
    pub extensions: usize,
    pub keys: usize,
    pub unassigned: usize,
    pub bytes: u64,
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
                id: p.id,
                created_at: p.created_at,
                reason: p.reason,
                source: p.source,
                versions: p.versions,
                theme: p.theme,
            })
        })
        .collect();
    out.sort_by(|a, b| b.id.cmp(&a.id));
    out
}

pub fn new_id(now: chrono::DateTime<chrono::Utc>) -> String {
    now.format("%Y-%m-%dT%H-%M-%SZ").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Profile {
        let mut p = Profile { format: FORMAT, id: "2026-11-25T22-59-04Z".into(), reason: "manual".into(), source: "cdp".into(), ..Default::default() };
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
        assert_eq!(path.file_name().unwrap(), "2026-11-25T22-59-04Z.json");
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
    fn ids_sort_by_time_and_are_file_safe() {
        let a = new_id(chrono::DateTime::parse_from_rfc3339("2026-11-25T09:05:00Z").unwrap().into());
        assert_eq!(a, "2026-11-25T09-05-00Z");
        assert!(!a.contains(':'));
    }
}
