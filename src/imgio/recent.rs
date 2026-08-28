//! Recently opened folders, shown on the welcome screen. Persisted next to
//! the processing settings in %APPDATA%\photo-editor\recent.json.

use std::path::PathBuf;

const MAX_RECENT: usize = 8;

fn path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("photo-editor").join("recent.json"))
}

pub fn load() -> Vec<PathBuf> {
    let Some(p) = path() else {
        return Vec::new();
    };
    let list: Vec<PathBuf> = std::fs::read_to_string(p)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    // Drop folders that no longer exist.
    list.into_iter().filter(|p| p.is_dir()).collect()
}

/// Move `dir` to the front of the list and persist. Failures are ignored —
/// recents are a convenience, never worth an error dialog.
pub fn remember(list: &mut Vec<PathBuf>, dir: PathBuf) {
    list.retain(|p| *p != dir);
    list.insert(0, dir);
    list.truncate(MAX_RECENT);
    if let Some(p) = path() {
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(list) {
            let _ = std::fs::write(p, json);
        }
    }
}
