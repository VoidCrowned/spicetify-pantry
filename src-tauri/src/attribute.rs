//! Key attribution (#11): which extension owns which localStorage key and IndexedDB database.
//! Order: Spotify's own families → Spicetify → Marketplace → key map → static scan of extension code.
//! A key no rule claims goes to `unassigned`; nothing is ever dropped.

use serde::Deserialize;
use std::collections::BTreeMap;

pub const KEYMAP_JSON: &str = include_str!("../../keymap.json");

#[derive(Clone, Debug, Deserialize)]
pub struct KeyMap {
    pub extensions: BTreeMap<String, KeyMapEntry>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct KeyMapEntry {
    pub name: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(default)]
    pub prefixes: Vec<String>,
    #[serde(default)]
    pub globals: Vec<String>,
}

pub fn keymap() -> KeyMap {
    serde_json::from_str(KEYMAP_JSON).expect("keymap.json is valid")
}

/// An installed extension, theme or app, with its code if Pantry could read it.
#[derive(Clone, Debug, Default)]
pub struct Candidate {
    pub id: String,
    /// File name or URL; the key map matches on its last path segment.
    pub source: String,
    pub code: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Owner {
    /// Spotify's own: per-account (`<userId>:`), telemetry (`ES|`) or cache (`com.spotify.`).
    Spotify,
    Spicetify,
    Marketplace,
    Extensions(Vec<String>),
    Unassigned,
}

pub fn is_spotify_key(k: &str, username: Option<&str>) -> bool {
    if k.starts_with("ES|") || k.starts_with("com.spotify.") {
        return true;
    }
    if let Some(u) = username.filter(|u| !u.is_empty()) {
        if k.starts_with(&format!("{u}:")) {
            return true;
        }
    }
    // Spotify user IDs: 20+ lowercase letters and digits, then a colon.
    match k.split_once(':') {
        Some((id, _)) => id.len() >= 20 && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
        None => false,
    }
}

/// Telemetry and install IDs: never restored, even when Spotify's keys are included.
pub fn is_never_restored(k: &str) -> bool {
    k.starts_with("ES|")
}

fn file_tail(s: &str) -> &str {
    s.rsplit(['/', '\\']).next().unwrap_or(s)
}

fn quoted(code: &str, lit: &str) -> bool {
    ['"', '\'', '`'].iter().any(|q| code.contains(&format!("{q}{lit}{q}")))
}

fn quoted_prefix(code: &str, prefix: &str) -> bool {
    ['"', '\'', '`'].iter().any(|q| code.contains(&format!("{q}{prefix}")))
}

/// Prefixes of `k` that end at a separator, longest first, at least 3 characters: `a-b.c` → `a-b.`, `a-`.
fn separator_prefixes(k: &str) -> Vec<&str> {
    let mut v: Vec<&str> = k.char_indices().filter(|(i, c)| *i >= 2 && ":._-/|".contains(*c)).map(|(i, c)| &k[..i + c.len_utf8()]).filter(|p| p.len() < k.len()).collect();
    v.reverse();
    v
}

pub struct Attributor<'a> {
    pub keymap: &'a KeyMap,
    pub candidates: &'a [Candidate],
    pub username: Option<&'a str>,
}

impl Attributor<'_> {
    /// Key-map entries whose file is installed, with the candidate ids that match them.
    fn keymap_hits(&self, k: &str) -> Vec<String> {
        let mut out = vec![];
        for e in self.keymap.extensions.values() {
            let claims = e.keys.iter().any(|x| x == k) || e.prefixes.iter().any(|p| k.starts_with(p.as_str()));
            if !claims {
                continue;
            }
            for c in self.candidates {
                if e.files.iter().any(|f| f == file_tail(&c.source)) && !out.contains(&c.id) {
                    out.push(c.id.clone());
                }
            }
        }
        out
    }

    /// Static scan: exact quoted literal first; otherwise the longest quoted separator prefix.
    fn scan_hits(&self, k: &str) -> Vec<String> {
        let exact: Vec<String> = self.candidates.iter().filter(|c| c.code.as_deref().is_some_and(|code| quoted(code, k))).map(|c| c.id.clone()).collect();
        if !exact.is_empty() {
            return exact;
        }
        for p in separator_prefixes(k) {
            let hits: Vec<String> = self.candidates.iter().filter(|c| c.code.as_deref().is_some_and(|code| quoted_prefix(code, p))).map(|c| c.id.clone()).collect();
            if !hits.is_empty() {
                return hits;
            }
        }
        vec![]
    }

    pub fn owner_of_key(&self, k: &str) -> Owner {
        if is_spotify_key(k, self.username) {
            return Owner::Spotify;
        }
        if k.starts_with("spicetify") {
            return Owner::Spicetify;
        }
        if k.starts_with("marketplace:") || k == "internal:local-storage-migrated" {
            return Owner::Marketplace;
        }
        let km = self.keymap_hits(k);
        if !km.is_empty() {
            return Owner::Extensions(km);
        }
        let s = self.scan_hits(k);
        if !s.is_empty() {
            return Owner::Extensions(s);
        }
        Owner::Unassigned
    }

    /// IndexedDB databases: by exact quoted name in the code.
    pub fn owner_of_database(&self, name: &str) -> Owner {
        if name == "keyval-store" {
            return Owner::Spotify;
        }
        if name == "spicetify-marketplace" {
            return Owner::Marketplace;
        }
        let hits: Vec<String> = self.candidates.iter().filter(|c| c.code.as_deref().is_some_and(|code| quoted(code, name))).map(|c| c.id.clone()).collect();
        if hits.is_empty() { Owner::Unassigned } else { Owner::Extensions(hits) }
    }

    pub fn globals_for(&self, c: &Candidate) -> Vec<String> {
        self.keymap
            .extensions
            .values()
            .filter(|e| e.files.iter().any(|f| f == file_tail(&c.source)))
            .flat_map(|e| e.globals.iter().cloned())
            .collect()
    }

    pub fn name_for(&self, c: &Candidate) -> Option<String> {
        self.keymap.extensions.values().find(|e| e.files.iter().any(|f| f == file_tail(&c.source))).map(|e| e.name.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cands() -> Vec<Candidate> {
        vec![
            Candidate { id: "local:gradientBars.js".into(), source: "gradientBars.js".into(), code: Some("const STORAGE_KEY = \"gpb:settings\";".into()) },
            Candidate { id: "mp:catjam".into(), source: "https://raw.githubusercontent.com/BlafKing/x/main/marketplace/cat-jam.js".into(), code: Some("Spicetify.LocalStorage.get(\"catjam-settings.\" + id)".into()) },
            Candidate { id: "theme:Hazy".into(), source: "https://cdn.jsdelivr.net/gh/astromations/hazy/hazy.js".into(), code: Some("localStorage.getItem('HideMerch'); const db = indexedDB.open(\"HazyDB\")".into()) },
            Candidate { id: "mp:broken".into(), source: "https://example.invalid/x.js".into(), code: None },
        ]
    }

    #[test]
    fn spotify_families() {
        assert!(is_spotify_key("3ccpqn2u57xfh5ggnu6cljg2d:ylx-expanded-folders", None));
        assert!(is_spotify_key("ES|65b7|EVENT|1", None));
        assert!(is_spotify_key("com.spotify.single.item.cache:desktop-ui", None));
        assert!(is_spotify_key("bob:x", Some("bob")));
        assert!(!is_spotify_key("gpb:settings", None));
        assert!(!is_spotify_key("catjam-settings.catjam-webm-bpm", None));
        assert!(is_never_restored("ES|x") && !is_never_restored("com.spotify.x"));
    }

    #[test]
    fn attribution_order() {
        let km = keymap();
        let c = cands();
        let a = Attributor { keymap: &km, candidates: &c, username: None };
        assert_eq!(a.owner_of_key("gpb:presets"), Owner::Extensions(vec!["local:gradientBars.js".into()]), "key map prefix");
        assert_eq!(a.owner_of_key("gcb:settings"), Owner::Extensions(vec!["local:gradientBars.js".into()]), "Gradient Bars claims both bars' keys");
        assert_eq!(a.owner_of_key("HideMerch"), Owner::Extensions(vec!["theme:Hazy".into()]), "exact literal");
        assert_eq!(a.owner_of_key("catjam-settings.catjam-webm-bpm"), Owner::Extensions(vec!["mp:catjam".into()]), "separator prefix");
        assert_eq!(a.owner_of_key("spicetify-exp-features"), Owner::Spicetify);
        assert_eq!(a.owner_of_key("marketplace:tabs"), Owner::Marketplace);
        assert_eq!(a.owner_of_key("totally-unknown"), Owner::Unassigned);
        assert_eq!(a.owner_of_database("HazyDB"), Owner::Extensions(vec!["theme:Hazy".into()]));
        assert_eq!(a.owner_of_database("keyval-store"), Owner::Spotify);
        assert_eq!(a.owner_of_database("OtherDB"), Owner::Unassigned);
        assert_eq!(a.globals_for(&c[0]), vec!["__gb", "__gpb", "__gcb"]);
        assert_eq!(a.name_for(&c[0]).as_deref(), Some("Gradient Bars"));
    }

    #[test]
    fn keymap_needs_the_file_installed() {
        let km = keymap();
        let c = vec![Candidate { id: "x".into(), source: "other.js".into(), code: Some(String::new()) }];
        let a = Attributor { keymap: &km, candidates: &c, username: None };
        assert_eq!(a.owner_of_key("gpb:settings"), Owner::Unassigned);
    }

    #[test]
    fn prefixes() {
        assert_eq!(separator_prefixes("npv-ambience-blurAmount"), vec!["npv-ambience-", "npv-"]);
        assert_eq!(separator_prefixes("ab"), Vec::<&str>::new());
    }
}
