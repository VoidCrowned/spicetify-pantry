//! Read-only fallback (docs/findings.md §5): decode Spotify's Local Storage from a copy, with Spotify closed.
//! The original folder is only ever read (copied); it's never opened as a database.

use std::collections::BTreeMap;
use std::path::Path;

const ORIGIN: &[u8] = b"_https://xpui.app.spotify.com\0";

fn decode(b: &[u8]) -> Option<String> {
    match b.first()? {
        1 => Some(b[1..].iter().map(|&c| c as char).collect()),
        0 => {
            let u: Vec<u16> = b[1..].chunks(2).map(|c| u16::from_le_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect();
            String::from_utf16(&u).ok()
        }
        _ => None,
    }
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        if e.file_type()?.is_file() && e.file_name() != "LOCK" {
            std::fs::copy(e.path(), to.join(e.file_name()))?;
        }
    }
    Ok(())
}

/// Copies `dir` to a temp folder and returns every xpui localStorage key.
pub fn read_local_storage(dir: &Path) -> Result<BTreeMap<String, String>, String> {
    let tmp = tempfile_dir()?;
    copy_dir(dir, &tmp).map_err(|e| format!("copying Local Storage: {e}"))?;
    let r = read_copy(&tmp);
    let _ = std::fs::remove_dir_all(&tmp);
    r
}

fn tempfile_dir() -> Result<std::path::PathBuf, String> {
    let p = std::env::temp_dir().join(format!("pantry-ldb-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    Ok(p)
}

pub fn read_copy(dir: &Path) -> Result<BTreeMap<String, String>, String> {
    use rusty_leveldb::{LdbIterator, Options, DB};
    let mut o = Options::default();
    o.create_if_missing = false;
    let mut db = DB::open(dir, o).map_err(|e| format!("opening the copy: {e}"))?;
    let mut it = db.new_iter().map_err(|e| e.to_string())?;
    let mut out = BTreeMap::new();
    while let Some((k, v)) = it.next() {
        if let Some(rest) = k.strip_prefix(ORIGIN) {
            if let (Some(k), Some(v)) = (decode(rest), decode(&v)) {
                out.insert(k, v);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(s: &str) -> Vec<u8> {
        let mut v = vec![0u8];
        for u in s.encode_utf16() {
            v.extend(u.to_le_bytes());
        }
        v
    }

    #[test]
    fn decodes_both_encodings() {
        assert_eq!(decode(b"\x01gpb:settings").as_deref(), Some("gpb:settings"));
        assert_eq!(decode(&utf16("Grüße ✓")).as_deref(), Some("Grüße ✓"));
        assert_eq!(decode(b"\x07x"), None);
    }

    #[test]
    fn reads_a_chromium_style_database_copy() {
        use rusty_leveldb::{Options, DB};
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("leveldb");
        {
            let mut db = DB::open(&src, Options::default()).unwrap();
            db.put(b"VERSION", b"1").unwrap();
            db.put(b"META:https://xpui.app.spotify.com", b"x").unwrap();
            let mut k = ORIGIN.to_vec();
            k.extend(b"\x01gpb:settings");
            db.put(&k, b"\x01{\"mode\":\"gradient\"}").unwrap();
            let mut k2 = ORIGIN.to_vec();
            k2.extend(utf16("ünï"));
            db.put(&k2, &utf16("välue")).unwrap();
            db.flush().unwrap();
        }
        let m = read_local_storage(&src).unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m["gpb:settings"], "{\"mode\":\"gradient\"}");
        assert_eq!(m["ünï"], "välue");
        assert!(src.join("CURRENT").exists(), "the original stays in place");
    }
}
