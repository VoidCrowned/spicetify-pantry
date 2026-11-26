pub mod attribute;
pub mod capture;
pub mod cdp;
pub mod detect;
pub mod ini;
pub mod js;
pub mod leveldb;
pub mod profile;
pub mod restore;
pub mod selftest;
pub mod spotify;
pub mod transfer;

use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};

fn roots() -> Result<detect::Roots, String> {
    detect::Roots::from_env().ok_or_else(|| "APPDATA / LOCALAPPDATA not set".into())
}

/// `%APPDATA%\Pantry\profiles`
pub fn profiles_dir(r: &detect::Roots) -> PathBuf {
    r.appdata.join("Pantry").join("profiles")
}

fn progress(app: &AppHandle) -> impl Fn(&str) + Sync + '_ {
    move |m: &str| {
        let _ = app.emit("progress", m.to_owned());
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    detection: detect::Detection,
    spotify_state: spotify::State,
    spicetify_version: Option<String>,
    profiles_dir: PathBuf,
}

#[tauri::command]
async fn status() -> Result<Status, String> {
    let r = roots()?;
    blocking(move || {
        let detection = detect::detect(&r);
        let spicetify_version = detection.spicetify.as_ref().and_then(|s| s.exe.as_ref()).and_then(|e| spotify::spicetify_version(e));
        Status { spotify_state: spotify::state(), spicetify_version, profiles_dir: profiles_dir(&r), detection }
    })
    .await
}

#[tauri::command]
async fn list_profiles() -> Result<Vec<profile::Summary>, String> {
    let r = roots()?;
    blocking(move || profile::list(&profiles_dir(&r))).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct KeyInfo {
    key: String,
    size: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtensionDetails {
    #[serde(flatten)]
    ext: profile::Extension,
    key_info: Vec<KeyInfo>,
    files: Vec<(String, u64, String)>,
    records: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Details {
    id: String,
    created_at: String,
    reason: String,
    source: String,
    versions: profile::Versions,
    theme: profile::Theme,
    marketplace: profile::Marketplace,
    extensions: Vec<ExtensionDetails>,
    spotify_keys: Vec<KeyInfo>,
    unassigned: Vec<KeyInfo>,
    notes: Vec<String>,
}

/// A profile without values or file contents, for the details view.
#[tauri::command]
async fn profile_details(id: String) -> Result<Details, String> {
    let r = roots()?;
    blocking(move || {
        let p = profile::path_of(&profiles_dir(&r), &id).and_then(|f| profile::Profile::load(&f)).map_err(|e| e.to_string())?;
        let info = |keys: &[String]| keys.iter().map(|k| KeyInfo { key: k.clone(), size: p.storage.local_storage.get(k).map(String::len).unwrap_or(0) }).collect::<Vec<_>>();
        let extensions = p
            .extensions
            .iter()
            .map(|e| {
                let src = e.source.clone().unwrap_or_default();
                let files = p
                    .files
                    .iter()
                    .filter(|(k, _)| (e.kind == "local" && k.as_str() == format!("Extensions/{src}")) || (e.kind != "local" && !src.is_empty() && k.starts_with(&format!("{src}/"))))
                    .map(|(k, f)| (k.clone(), f.size, f.sha256.clone()))
                    .collect();
                let records = e.databases.iter().filter_map(|d| p.storage.indexed_db.get(d)).flat_map(|d| d.stores.values()).map(|s| s.records.len()).sum();
                ExtensionDetails { key_info: info(&e.keys), files, records, ext: e.clone() }
            })
            .collect();
        Ok(Details {
            spotify_keys: info(&p.spotify_keys),
            unassigned: info(&p.unassigned),
            id: p.id.clone(),
            created_at: p.created_at.clone(),
            reason: p.reason.clone(),
            source: p.source.clone(),
            versions: p.versions.clone(),
            theme: p.theme.clone(),
            marketplace: p.marketplace.clone(),
            notes: p.notes.clone(),
            extensions,
        })
    })
    .await?
}

#[tauri::command]
async fn capture(app: AppHandle) -> Result<profile::Summary, String> {
    let r = roots()?;
    blocking(move || {
        let dir = profiles_dir(&r);
        let (p, _) = capture::capture(&r, &dir, "manual", &progress(&app)).map_err(|e| e.to_string())?;
        profile::list(&dir).into_iter().find(|s| s.id == p.id).ok_or_else(|| "saved, but couldn't list it".to_string())
    })
    .await?
}

#[tauri::command]
async fn restore_check(id: String) -> Result<restore::Check, String> {
    let r = roots()?;
    blocking(move || {
        let p = profile::path_of(&profiles_dir(&r), &id).and_then(|f| profile::Profile::load(&f)).map_err(|e| e.to_string())?;
        let det = detect::detect(&r);
        let now = det.spicetify.as_ref().and_then(|s| s.exe.as_ref()).and_then(|e| spotify::spicetify_version(e));
        Ok(restore::check(&p, &det, now.as_deref()))
    })
    .await?
}

#[tauri::command]
async fn restore(app: AppHandle, id: String, options: restore::Options) -> Result<restore::Report, String> {
    let r = roots()?;
    blocking(move || restore::restore(&r, &profiles_dir(&r), &id, &options, &progress(&app))).await
}

#[tauri::command]
async fn export_profile(id: String, path: PathBuf, share: bool) -> Result<(), String> {
    let r = roots()?;
    blocking(move || transfer::export(&profiles_dir(&r), &id, &path, share).map(|_| ())).await?
}

#[tauri::command]
async fn import_profile(path: PathBuf) -> Result<transfer::Imported, String> {
    let r = roots()?;
    blocking(move || transfer::import(&profiles_dir(&r), &path, chrono::Utc::now())).await?
}

#[tauri::command]
async fn self_test(app: AppHandle) -> Result<selftest::SelfTest, String> {
    let r = roots()?;
    blocking(move || selftest::run(&r, &profiles_dir(&r), &progress(&app))).await
}

#[tauri::command]
async fn open_profiles_folder(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = profiles_dir(&roots()?);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    app.opener().open_path(dir.to_string_lossy(), None::<&str>).map_err(|e| e.to_string())
}

/// Headless mode for automated checks: `pantry.exe --cli <command> [args] <out.json>`.
/// Commands: `self-test`, `capture`, `list`, `restore <id> [--include-spotify] [--acknowledge]`,
/// `export <id> <file> [--share]`, `import <file>`.
/// The release build has no console, so results go to the JSON file; progress lines go to `<out.json>.log`.
pub fn cli(args: &[String]) -> i32 {
    let Some(out) = args.last().filter(|a| a.ends_with(".json")) else {
        eprintln!("usage: pantry --cli <self-test|capture|list|restore <id>|export <id> <file>|import <file>> <out.json>");
        return 2;
    };
    let log = std::sync::Mutex::new(std::fs::File::create(format!("{out}.log")).ok());
    let progress = |m: &str| {
        if let Some(f) = log.lock().unwrap().as_mut() {
            use std::io::Write;
            let _ = writeln!(f, "{} {m}", chrono::Local::now().format("%H:%M:%S"));
        }
    };
    let r = match roots() {
        Ok(r) => r,
        Err(e) => {
            let _ = std::fs::write(out, serde_json::json!({ "error": e }).to_string());
            return 1;
        }
    };
    let dir = profiles_dir(&r);
    let (code, v) = match args[0].as_str() {
        "self-test" => {
            let t = selftest::run(&r, &dir, &progress);
            (if t.results.iter().any(|c| c.status == "fail") { 1 } else { 0 }, serde_json::to_value(t).unwrap())
        }
        "capture" => match capture::capture(&r, &dir, "manual", &progress) {
            Ok((p, path)) => (0, serde_json::json!({ "id": p.id, "path": path, "extensions": p.extensions, "unassigned": p.unassigned, "spotifyKeys": p.spotify_keys.len(), "notes": p.notes, "marketplace": p.marketplace, "theme": p.theme, "versions": p.versions })),
            Err(e) => (1, serde_json::json!({ "error": e.to_string() })),
        },
        "list" => (0, serde_json::to_value(profile::list(&dir)).unwrap()),
        "restore" if args.len() >= 3 => {
            let o = restore::Options { include_spotify: args.iter().any(|a| a == "--include-spotify"), acknowledged: args.iter().any(|a| a == "--acknowledge") };
            let rep = restore::restore(&r, &dir, &args[1], &o, &progress);
            (if rep.ok { 0 } else { 1 }, serde_json::to_value(rep).unwrap())
        }
        "export" if args.len() >= 4 => match transfer::export(&dir, &args[1], std::path::Path::new(&args[2]), args.iter().any(|a| a == "--share")) {
            Ok(p) => (0, serde_json::json!({ "id": p.id, "file": args[2], "shared": p.shared })),
            Err(e) => (1, serde_json::json!({ "error": e })),
        },
        "import" if args.len() >= 3 => match transfer::import(&dir, std::path::Path::new(&args[1]), chrono::Utc::now()) {
            Ok(i) => (0, serde_json::to_value(i).unwrap()),
            Err(e) => (1, serde_json::json!({ "error": e })),
        },
        other => (2, serde_json::json!({ "error": format!("unknown command {other}") })),
    };
    let _ = std::fs::write(out, serde_json::to_string_pretty(&v).unwrap());
    code
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![status, list_profiles, profile_details, capture, restore_check, restore, self_test, open_profiles_folder, export_profile, import_profile])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
