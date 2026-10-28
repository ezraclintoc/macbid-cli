//! Where saved searches and watch state live on disk.
//!
//! Layout (respects XDG_CONFIG_HOME / %APPDATA% on Windows):
//!
//! ```text
//! <config>/macbid-cli/searches/<name>.json    saved search definitions
//! <config>/macbid-cli/state/<name>.json       last-seen lots for `diff`
//! ```
//!
//! Definitions are the serialized search arguments of `macbid save ...`;
//! state files are maps of lot ids seen in the last run plus an accumulating
//! ledger of lots that left the results, so they stay visible afterwards.

use std::fs;
use std::path::PathBuf;

/// Base directory for macbid-cli files (…/macbid-cli).
fn config_base() -> Result<PathBuf, String> {
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME")
        && !x.trim().is_empty()
    {
        return Ok(PathBuf::from(x).join("macbid-cli"));
    }
    if cfg!(windows)
        && let Ok(x) = std::env::var("APPDATA")
        && !x.trim().is_empty()
    {
        return Ok(PathBuf::from(x).join("macbid-cli"));
    }
    if let Ok(h) = std::env::var("HOME")
        && !h.trim().is_empty()
    {
        return Ok(PathBuf::from(h).join(".config").join("macbid-cli"));
    }
    Err("could not find a config directory (set $HOME or $XDG_CONFIG_HOME)".to_string())
}

fn searches_dir() -> Result<PathBuf, String> {
    let d = config_base()?.join("searches");
    fs::create_dir_all(&d).map_err(|e| format!("creating {d:?}: {e}"))?;
    Ok(d)
}

fn state_dir() -> Result<PathBuf, String> {
    let d = config_base()?.join("state");
    fs::create_dir_all(&d).map_err(|e| format!("creating {d:?}: {e}"))?;
    Ok(d)
}

/// Saved-search names may only contain letters, digits and `. - _ @`.
pub fn sanitize(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 64 {
        return Err("search name must be 1-64 characters".to_string());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '@'))
    {
        return Err("search name may only contain letters, digits, and . - _ @".to_string());
    }
    Ok(name.to_string())
}

fn search_path(name: &str) -> PathBuf {
    searches_dir()
        .ok()
        .unwrap_or_default()
        .join(format!("{name}.json"))
}

fn state_path(name: &str) -> PathBuf {
    state_dir()
        .ok()
        .unwrap_or_default()
        .join(format!("{name}.json"))
}

/// Write a saved-search definition (JSON text). Returns whether an
/// existing definition was overwritten.
pub fn save_search(name: &str, json: &str) -> Result<bool, String> {
    let path = search_path(name);
    let existed = path.exists();
    fs::write(&path, json).map_err(|e| format!("writing {path:?}: {e}"))?;
    Ok(existed)
}

/// Read a saved-search definition (JSON text).
pub fn load_search(name: &str) -> Result<String, String> {
    let path = search_path(name);
    fs::read_to_string(&path).map_err(|_| {
        format!(
            "no saved search `{name}` — create one with `macbid save {name} --category Televisions`"
        )
    })
}

/// Delete a saved search definition. Returns whether it existed.
pub fn delete_search(name: &str) -> bool {
    fs::remove_file(search_path(name)).is_ok()
}

/// Delete a saved search's watch state. Returns whether it existed.
pub fn delete_state(name: &str) -> bool {
    fs::remove_file(state_path(name)).is_ok()
}

/// All saved search names, sorted.
pub fn list_searches() -> Vec<String> {
    let Ok(dir) = searches_dir() else {
        return Vec::new();
    };
    let mut names: Vec<String> = fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
                .filter_map(|e| e.file_name().into_string().ok())
                .map(|f| f.trim_end_matches(".json").to_string())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// Write a watch-state file (JSON text).
pub fn save_state(name: &str, json: &str) -> Result<(), String> {
    let path = state_path(name);
    fs::write(&path, json).map_err(|e| format!("writing {path:?}: {e}"))?;
    Ok(())
}

/// Read a watch-state file (JSON text). `None` = never watched before.
pub fn load_state(name: &str) -> Option<String> {
    fs::read_to_string(state_path(name)).ok()
}
