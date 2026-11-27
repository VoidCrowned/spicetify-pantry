//! Export and import (#32): a profile is already one self-contained file, so export writes a copy (optionally
//! without private data) and import validates a file and adds it to the profile list.

use crate::profile::{self, Profile, FORMAT};
use serde::Serialize;
use std::path::Path;

/// Writes profile `id` from `dir` to `dest`. With `share`, Spotify's own keys, the account hash and the raw ini stay out.
pub fn export(dir: &Path, id: &str, dest: &Path, share: bool) -> Result<Profile, String> {
    let p = Profile::load(&profile::path_of(dir, id).map_err(|e| e.to_string())?).map_err(|e| format!("couldn't read profile {id}: {e}"))?;
    let out = if share { p.for_sharing() } else { p };
    let mut tmp = dest.as_os_str().to_owned();
    tmp.push(".tmp");
    let json = serde_json::to_vec_pretty(&out).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, json).and_then(|_| std::fs::rename(&tmp, dest)).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("couldn't write {}: {e}", dest.display())
    })?;
    Ok(out)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    pub id: String,
    /// The same profile was already in the list: nothing was written.
    pub already_there: bool,
    pub extensions: usize,
    pub theme: Option<String>,
    pub colour_scheme: Option<String>,
    pub shared: bool,
}

/// The profile without the import stamp, to compare a file with one already in the list.
fn comparable(p: &Profile) -> Profile {
    Profile { imported_at: None, ..p.clone() }
}

pub fn import(dir: &Path, src: &Path, now: chrono::DateTime<chrono::Utc>) -> Result<Imported, String> {
    let bytes = std::fs::read(src).map_err(|e| format!("couldn't read {}: {e}", src.display()))?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| "This isn't a Spicetify Pantry profile (not valid JSON).".to_string())?;
    match v.get("format").and_then(|f| f.as_u64()) {
        None => return Err("This isn't a Spicetify Pantry profile (no format number).".into()),
        Some(f) if f > FORMAT as u64 => return Err(format!("This profile was made by a newer Spicetify Pantry (format {f}). Update it, then import it.")),
        _ => {}
    }
    let mut p: Profile = serde_json::from_value(v).map_err(|e| format!("This profile is damaged: {e}"))?;
    if !profile::valid_id(&p.id) {
        return Err(format!("This profile has an invalid id ({:?}).", p.id));
    }
    // Old UTC ids (before #47) arrive under the local-time id they'd have been migrated to.
    let base = profile::localise_id(profile::id_base(&p.id).unwrap_or(&p.id), &chrono::Local);
    let summary = |p: &Profile, already_there| Imported {
        id: p.id.clone(),
        already_there,
        extensions: p.extensions.iter().filter(|x| x.kind != "spicetify").count(),
        theme: p.theme.name.clone(),
        colour_scheme: p.theme.colour_scheme.clone(),
        shared: p.shared,
    };
    // The same capture can be in the list under its own id or a `-N` one from an earlier import.
    for n in 1..10_000 {
        let id = if n == 1 { base.clone() } else { format!("{base}-{n}") };
        let path = profile::path_of(dir, &id).map_err(|e| e.to_string())?;
        match Profile::load(&path) {
            Ok(existing) => {
                if comparable(&existing) == comparable(&Profile { id: id.clone(), ..p.clone() }) {
                    return Ok(summary(&existing, true));
                }
            }
            Err(_) if path.exists() => continue,
            Err(_) => {
                p.id = id;
                p.imported_at = Some(now.to_rfc3339());
                p.save(dir).map_err(|e| format!("couldn't save the profile: {e}"))?;
                return Ok(summary(&p, false));
            }
        }
    }
    Err("Too many profiles with this id.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Extension;

    fn sample() -> Profile {
        let mut p = Profile { format: FORMAT, id: "2026-11-25_23-59-04".into(), reason: "manual".into(), source: "cdp".into(), account_hash: Some("h".into()), ..Default::default() };
        p.storage.local_storage.insert("gpb:settings".into(), "{}".into());
        p.storage.local_storage.insert("ES|x".into(), "1".into());
        p.spotify_keys.push("ES|x".into());
        p.extensions.push(Extension { id: "local:gradientBars.js".into(), kind: "local".into(), keys: vec!["gpb:settings".into()], ..Default::default() });
        p.theme.name = Some("Hazy".into());
        p.ini.raw = "prefs_path = C:\\Users\\VC".into();
        p
    }

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-11-26T10:00:00Z").unwrap().into()
    }

    #[test]
    fn round_trip_to_another_pc() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let p = sample();
        p.save(a.path()).unwrap();
        let f = a.path().join("out.json");
        export(a.path(), &p.id, &f, false).unwrap();
        let i = import(b.path(), &f, now()).unwrap();
        assert!(!i.already_there && i.id == p.id && i.extensions == 1 && i.theme.as_deref() == Some("Hazy"));
        let q = Profile::load(&b.path().join(format!("{}.json", p.id))).unwrap();
        assert_eq!(q.imported_at.as_deref(), Some("2026-11-26T10:00:00+00:00"));
        assert_eq!(comparable(&q), p);
    }

    #[test]
    fn reimport_and_clash() {
        let (d, x) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let p = sample();
        p.save(d.path()).unwrap();
        let f = x.path().join("out.json");
        export(d.path(), &p.id, &f, false).unwrap();
        assert!(import(d.path(), &f, now()).unwrap().already_there);
        // A shared copy differs: it arrives as -2, and importing it again finds that one.
        export(d.path(), &p.id, &f, true).unwrap();
        let i = import(d.path(), &f, now()).unwrap();
        assert_eq!((i.id.as_str(), i.already_there, i.shared), ("2026-11-25_23-59-04-2", false, true));
        assert!(import(d.path(), &f, now()).unwrap().already_there);
        assert_eq!(profile::list(d.path()).len(), 2);
    }

    #[test]
    fn old_utc_ids_are_converted_on_import() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let mut p = sample();
        p.id = "2026-11-25T22-59-04Z".into();
        p.save(a.path()).unwrap();
        let f = a.path().join("out.json");
        export(a.path(), &p.id, &f, false).unwrap();
        let i = import(b.path(), &f, now()).unwrap();
        let want = profile::localise_id(&p.id, &chrono::Local);
        assert_eq!(i.id, want);
        assert_eq!(Profile::load(&b.path().join(format!("{want}.json"))).unwrap().id, want);
        assert!(import(b.path(), &f, now()).unwrap().already_there);
    }

    #[test]
    fn shared_export_has_no_private_data() {
        let d = tempfile::tempdir().unwrap();
        let p = sample();
        p.save(d.path()).unwrap();
        let f = d.path().join("out.json");
        export(d.path(), &p.id, &f, true).unwrap();
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(!text.contains("ES|x") && !text.contains("C:\\\\Users") && !text.contains("\"accountHash\": \"h\""));
        assert!(text.contains("gpb:settings"));
    }

    #[test]
    fn bad_files_are_refused() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("x.json");
        let cases: [(serde_json::Value, &str); 3] = [
            (serde_json::json!({ "hello": 1 }), "no format number"),
            (serde_json::json!({ "format": 99 }), "newer Spicetify Pantry"),
            ({ let mut v = serde_json::to_value(sample()).unwrap(); v["id"] = "../../evil".into(); v }, "invalid id"),
        ];
        for (v, msg) in cases {
            std::fs::write(&f, v.to_string()).unwrap();
            let e = import(d.path(), &f, now()).unwrap_err();
            assert!(e.contains(msg), "{e}");
        }
        std::fs::write(&f, "not json").unwrap();
        assert!(import(d.path(), &f, now()).unwrap_err().contains("not valid JSON"));
        assert!(export(d.path(), "../x", &f, false).is_err());
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
    }
}
