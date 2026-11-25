//! Finds Spotify and Spicetify on disk (docs/findings.md §1, §2). Read-only.

use crate::ini::Ini;
use serde::Serialize;
use std::path::PathBuf;

/// The folders detection starts from. On Windows they come from the environment; tests pass their own.
#[derive(Clone, Debug)]
pub struct Roots {
    pub appdata: PathBuf,
    pub localappdata: PathBuf,
}

impl Roots {
    pub fn from_env() -> Option<Roots> {
        Some(Roots {
            appdata: std::env::var_os("APPDATA")?.into(),
            localappdata: std::env::var_os("LOCALAPPDATA")?.into(),
        })
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Spotify {
    pub exe: PathBuf,
    /// From prefs `app.last-launched-version`, e.g. `1.3.10.2586.g81a118a22460`.
    pub version: Option<String>,
    pub prefs: PathBuf,
    pub local_storage: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Spicetify {
    pub exe: Option<PathBuf>,
    pub userdata: PathBuf,
    pub ini: PathBuf,
    /// `[Backup] with`: the Spicetify version that last applied. The live version comes from `spicetify -v`.
    pub backup_with: Option<String>,
    /// `[Backup] version`: the Spotify build Spicetify backed up.
    pub backup_spotify: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detection {
    pub spotify: Option<Spotify>,
    /// A Microsoft Store install was found. Spicetify can't patch it, so Pantry only reports it.
    pub store_spotify: bool,
    pub spicetify: Option<Spicetify>,
}

pub fn detect(r: &Roots) -> Detection {
    let exe = r.appdata.join("Spotify").join("Spotify.exe");
    let prefs = r.appdata.join("Spotify").join("prefs");
    let spotify = exe.is_file().then(|| Spotify {
        version: std::fs::read_to_string(&prefs).ok().and_then(|p| prefs_version(&p)),
        exe,
        prefs,
        local_storage: local_storage_dir(r),
    });
    let store_spotify = std::fs::read_dir(r.localappdata.join("Packages"))
        .map(|d| d.flatten().any(|e| e.file_name().to_string_lossy().starts_with("SpotifyAB.SpotifyMusic_")))
        .unwrap_or(false);
    let userdata = r.appdata.join("spicetify");
    let ini_path = userdata.join("config-xpui.ini");
    let spicetify = ini_path.is_file().then(|| {
        let ini = Ini::load(&ini_path).unwrap_or_default();
        let cli = r.localappdata.join("spicetify").join("spicetify.exe");
        Spicetify {
            exe: cli.is_file().then_some(cli),
            backup_with: ini.get("Backup", "with").filter(|s| !s.is_empty()).map(str::to_owned),
            backup_spotify: ini.get("Backup", "version").filter(|s| !s.is_empty()).map(str::to_owned),
            userdata,
            ini: ini_path,
        }
    });
    Detection { spotify, store_spotify, spicetify }
}

pub fn local_storage_dir(r: &Roots) -> PathBuf {
    r.localappdata.join("Spotify").join("Browser").join("Local Storage").join("leveldb")
}

/// `app.last-launched-version="1.3.10.2586.g81a118a22460"` → `1.3.10.2586.g81a118a22460`.
pub fn prefs_version(prefs: &str) -> Option<String> {
    prefs.lines().find_map(|l| {
        let v = l.strip_prefix("app.last-launched-version=")?;
        Some(v.trim().trim_matches('"').to_owned())
    })
}

/// `1.3.10.2586.g81a118a22460` → `1.3.10.2586`, so builds compare by number only.
pub fn short_version(v: &str) -> String {
    v.split('.').take_while(|p| p.chars().all(|c| c.is_ascii_digit())).collect::<Vec<_>>().join(".")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_version_is_read() {
        let p = "storage.size=1024\napp.last-launched-version=\"1.3.10.2586.g81a118a22460\"\n";
        assert_eq!(prefs_version(p).as_deref(), Some("1.3.10.2586.g81a118a22460"));
        assert_eq!(prefs_version("x=1"), None);
    }

    #[test]
    fn short_version_drops_hash() {
        assert_eq!(short_version("1.3.10.2586.g81a118a22460"), "1.3.10.2586");
        assert_eq!(short_version("2.46.7"), "2.46.7");
    }

    #[test]
    fn detects_a_fake_install() {
        let t = tempfile::tempdir().unwrap();
        let r = Roots { appdata: t.path().join("Roaming"), localappdata: t.path().join("Local") };
        std::fs::create_dir_all(r.appdata.join("Spotify")).unwrap();
        std::fs::create_dir_all(r.appdata.join("spicetify")).unwrap();
        std::fs::create_dir_all(r.localappdata.join("Packages/SpotifyAB.SpotifyMusic_zpdnekdrzrea0")).unwrap();
        std::fs::write(r.appdata.join("Spotify/Spotify.exe"), b"").unwrap();
        std::fs::write(r.appdata.join("Spotify/prefs"), "app.last-launched-version=\"1.3.10.2586.gabc\"\n").unwrap();
        std::fs::write(r.appdata.join("spicetify/config-xpui.ini"), "[Backup]\nversion = 1.3.10.2586.gabc\nwith    = 2.46.7\n").unwrap();
        let d = detect(&r);
        assert_eq!(d.spotify.unwrap().version.as_deref(), Some("1.3.10.2586.gabc"));
        assert!(d.store_spotify);
        let s = d.spicetify.unwrap();
        assert_eq!(s.backup_with.as_deref(), Some("2.46.7"));
        assert!(s.exe.is_none());
    }
}
