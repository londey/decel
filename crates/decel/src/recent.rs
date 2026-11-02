// SPDX-License-Identifier: MIT OR Apache-2.0

//! Recent files and per-user paths.

use std::path::{Path, PathBuf};

use directories::ProjectDirs;

const MAX_RECENT: usize = 12;

fn dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", "decel")
}

/// Where untitled workbooks live until Save As.
pub fn untitled_dir() -> PathBuf {
    dirs()
        .map(|d| d.data_dir().join("untitled"))
        .unwrap_or_else(|| std::env::temp_dir().join("decel-untitled"))
}

pub fn config_dir() -> Option<PathBuf> {
    dirs().map(|d| d.config_dir().to_path_buf())
}

/// A fresh path for a new untitled workbook.
pub fn new_untitled_path() -> std::io::Result<PathBuf> {
    let dir = untitled_dir();
    std::fs::create_dir_all(&dir)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut n = 0u32;
    loop {
        let p = dir.join(format!("Untitled-{stamp}-{n}.decel"));
        if !p.exists() {
            return Ok(p);
        }
        n += 1;
    }
}

/// Deletes untitled workbooks that were never edited (and aren't open).
pub fn prune_untitled() {
    let Ok(entries) = std::fs::read_dir(untitled_dir()) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "decel") {
            continue;
        }
        // Opening for write takes the lock, so a workbook open elsewhere is skipped.
        let empty = decel_store::Store::open(&p)
            .and_then(|s| s.txns())
            .is_ok_and(|t| t.is_empty());
        if empty {
            for suffix in ["", "-wal", "-shm", ".lock"] {
                let mut f = p.clone().into_os_string();
                f.push(suffix);
                let _ = std::fs::remove_file(f);
            }
        }
    }
}

pub fn is_untitled(path: &Path) -> bool {
    path.starts_with(untitled_dir())
}

fn recent_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("recent.json"))
}

pub fn load() -> Vec<PathBuf> {
    recent_file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|s| serde_json::from_str::<Vec<PathBuf>>(&s).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.exists())
        .collect()
}

/// Moves `path` to the front of the recent list and saves it.
pub fn touch(path: &Path) -> Vec<PathBuf> {
    let mut list = load();
    list.retain(|p| p != path);
    list.insert(0, path.to_path_buf());
    list.truncate(MAX_RECENT);
    if let Some(f) = recent_file() {
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(&list) {
            let _ = std::fs::write(f, json);
        }
    }
    list
}
