//! The list of recently opened vaults, kept in the user's configuration folder so the
//! welcome screen can offer them again.

use std::path::{Path, PathBuf};

const MAX: usize = 12;

/// Where Rustsidian keeps per-user data. `RUSTSIDIAN_CONFIG_DIR` overrides the platform default.
pub fn config_dir() -> Option<PathBuf> {
    let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(dir) = var("RUSTSIDIAN_CONFIG_DIR") {
        return Some(dir);
    }
    if cfg!(windows) {
        var("APPDATA").map(|d| d.join("Rustsidian"))
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|d| d.join("Library/Application Support/Rustsidian"))
    } else {
        var("XDG_CONFIG_HOME")
            .or_else(|| var("HOME").map(|d| d.join(".config")))
            .map(|d| d.join("rustsidian"))
    }
}

/// The user's home folder.
pub fn home_dir() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"]
        .iter()
        .find_map(|k| std::env::var_os(k).filter(|v| !v.is_empty()))
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

fn file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("vaults.json"))
}

/// Recently opened vaults that still exist, most recent first.
pub fn load() -> Vec<PathBuf> {
    let Some(text) = file().and_then(|f| std::fs::read_to_string(f).ok()) else { return Vec::new() };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else { return Vec::new() };
    value
        .get("vaults")
        .and_then(serde_json::Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str()).map(PathBuf::from).filter(|p| p.is_dir()).collect())
        .unwrap_or_default()
}

fn save(vaults: &[PathBuf]) {
    let Some(file) = file() else { return };
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let list: Vec<String> = vaults.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    let _ = std::fs::write(file, serde_json::json!({ "vaults": list }).to_string());
}

/// Put a vault at the top of the list.
pub fn remember(vault: &Path) {
    let vault = std::fs::canonicalize(vault).unwrap_or_else(|_| vault.to_path_buf());
    let vault = clean(&vault);
    let mut list = load();
    list.retain(|p| *p != vault);
    list.insert(0, vault);
    list.truncate(MAX);
    save(&list);
}

pub fn forget(vault: &Path) {
    let mut list = load();
    list.retain(|p| p != vault);
    save(&list);
}

/// Drop the `\\?\` prefix Windows adds to canonical paths; it is noise when shown to people.
pub fn clean(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => p.to_path_buf(),
    }
}
