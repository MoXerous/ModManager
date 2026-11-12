//! Instance folders: read-only scanning, lock file, enable/disable, safe
//! removal.
//!
//! Scanning never writes. The only writers here are [`toggle`] (rename
//! only), [`remove`] (move into a dated backup folder, never delete), and
//! [`lock`] (the lock file itself).

pub mod lock;
pub mod metadata;
pub mod record;
pub mod recovery;
pub mod registry;
pub mod remove;
pub mod toggle;

pub use lock::{read_lock, write_lock, LockEntry, LockError, LockFile, LOCK_FILE_NAME};
pub use record::{record_install, InstalledRecord, RecordError};

use crate::jar_reader::{classify_local_resource_type, ResourceType};
use std::path::{Path, PathBuf};

/// One scanned file: orthogonal facts, never a single enum (spec Â§5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalContentFact {
    pub path: PathBuf,
    /// Path inside the `mods` folder, always `/`-separated for IPC. Unlike
    /// `filename`, this keeps `disabled_mods/` intact, so a later toggle or
    /// removal addresses the file the user actually sees.
    pub rel_path: String,
    pub filename: String,
    pub size: u64,
    pub sha1: String,
    pub resource_type: ResourceType,
    pub disabled: bool,
}

/// Suffixes accepted by the scan (spec Â§4-1 + F-090 names).
fn scan_accepted(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".jar")
        || lower.ends_with(".jar.disabled")
        || lower.ends_with(".mrpack")
        || lower.ends_with(".zip")
}

/// Disabled = `.jar.disabled` suffix or any `disabled_mods` path segment
/// (both case-insensitive); the filename is the truth (F-067).
pub fn is_disabled_path(path: &Path) -> bool {
    if path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.to_lowercase().ends_with(".jar.disabled"))
        .unwrap_or(false)
    {
        return true;
    }
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .map(|s| s.to_lowercase() == "disabled_mods")
            .unwrap_or(false)
    })
}

/// Non-recursive scan files: top-level entries plus `disabled_mods/`,
/// deduped and casefold-sorted. Missing dir â‡’ [].
pub fn iter_local_mod_scan_files(folder: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut push_dir = |dir: &Path| {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if !p.is_file() {
                    continue;
                }
                let name = match p.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n,
                    None => continue,
                };
                if !scan_accepted(name) {
                    continue;
                }
                if seen.insert(p.to_string_lossy().into_owned()) {
                    out.push(p);
                }
            }
        }
    };
    push_dir(folder);
    push_dir(&folder.join("disabled_mods"));
    out.sort_by_key(|p| p.to_string_lossy().to_lowercase());
    out
}

fn has_launcher_metadata(dir: &Path) -> bool {
    ["mmc-pack.json", "instance.cfg", "modrinth.index.json"]
        .iter()
        .any(|n| dir.join(n).is_file())
}

/// Instance + minecraft roots: a `.minecraft`/`minecraft` child whose
/// parent carries launcher metadata â‡’ parent is the instance root
/// (Prism/MultiMC shape, spec Â§2-1).
pub fn detect_instance_roots(selected: &Path) -> (PathBuf, PathBuf) {
    if let Some(name) = selected.file_name().and_then(|n| n.to_str()) {
        let lower = name.to_lowercase();
        if lower == ".minecraft" || lower == "minecraft" {
            if let Some(parent) = selected.parent() {
                if parent != selected && has_launcher_metadata(parent) {
                    return (parent.to_path_buf(), selected.to_path_buf());
                }
            }
        }
    }
    let mc = [".minecraft", "minecraft"]
        .iter()
        .map(|n| selected.join(n))
        .find(|p| p.is_dir())
        .unwrap_or_else(|| selected.to_path_buf());
    (selected.to_path_buf(), mc)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    pub root: PathBuf,
    pub minecraft_root: PathBuf,
}

fn sha1_of(path: &Path) -> String {
    use sha1::Digest;
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return String::new(),
    };
    let mut h = sha1::Sha1::new();
    let mut reader = std::io::BufReader::new(file);
    let mut buf = [0u8; 64 * 1024];
    use std::io::Read;
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => h.update(&buf[..n]),
            Err(_) => return String::new(),
        }
    }
    hex_of(h.finalize())
}

fn hex_of(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// Read-only mod scan: facts only, no writes, no moves.
pub fn scan_mods(instance: &Instance) -> Vec<LocalContentFact> {
    let folder = instance.minecraft_root.join("mods");
    iter_local_mod_scan_files(&folder)
        .into_iter()
        .map(|p| {
            let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            let filename = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            let rel_path = p
                .strip_prefix(&folder)
                .map(relative_display)
                .unwrap_or_else(|_| filename.clone());
            LocalContentFact {
                path: p.clone(),
                rel_path,
                filename,
                size,
                sha1: sha1_of(&p),
                resource_type: classify_local_resource_type(&p),
                disabled: is_disabled_path(&p),
            }
        })
        .collect()
}

/// `/`-separated path for IPC, independent of the platform separator.
fn relative_display(rel: &Path) -> String {
    rel.components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
pub(crate) fn snapshot_tree(root: &Path) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(m) = e.metadata() {
                out.push((
                    p,
                    m.len(),
                    m.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                ));
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
pub(crate) mod tests;
