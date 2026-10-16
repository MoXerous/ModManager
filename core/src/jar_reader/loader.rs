//! Loader identity + guarded fallback detection.
//!
//! Launcher/pack metadata is authoritative. Jar inspection is only a
//! fallback: `if loaders.is_empty() { scan_jars() }` — verbatim shape,
//! because NeoForge-compatible archives may also contain the legacy
//! Forge metadata path and would otherwise create a false conflict.

use super::cache::ScanCache;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Loader {
    Fabric,
    Quilt,
    NeoForge,
    Forge,
}

impl Loader {
    pub fn as_str(self) -> &'static str {
        match self {
            Loader::Fabric => "fabric",
            Loader::Quilt => "quilt",
            Loader::NeoForge => "neoforge",
            Loader::Forge => "forge",
        }
    }
}

/// Strict first-wins table (spec §1-2): order is load-bearing.
pub const FIRST_WINS: [(&str, Loader); 4] = [
    ("fabric.mod.json", Loader::Fabric),
    ("quilt.mod.json", Loader::Quilt),
    ("META-INF/neoforge.mods.toml", Loader::NeoForge),
    ("META-INF/mods.toml", Loader::Forge),
];

/// Sorted `*.jar` paths, capped at 30. Symlinks skipped; unreadable dirs
/// and bad zips are the caller's concern (detect logs + skips).
pub fn scan_jars_for_loader(mods_dir: &Path) -> Vec<PathBuf> {
    let entries = match std::fs::read_dir(mods_dir) {
        Ok(rd) => rd,
        Err(_) => return Vec::new(),
    };
    let mut jars: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|x| x.to_str())
                .map(|x| x.eq_ignore_ascii_case("jar"))
                .unwrap_or(false)
                && !is_link(p)
        })
        .collect();
    jars.sort();
    jars.truncate(30);
    jars
}

fn is_link(p: &Path) -> bool {
    std::fs::symlink_metadata(p)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// Guarded detection: authoritative loaders win untouched; jar scan runs
/// only when empty (spec §3-2). Bad zips log `info` and are skipped.
pub fn detect_loaders(
    authoritative: &[Loader],
    mods_dir: &Path,
    cache: &mut ScanCache,
) -> Vec<Loader> {
    if !authoritative.is_empty() {
        return authoritative.to_vec();
    }
    let mut out: Vec<Loader> = Vec::new();
    for jar in scan_jars_for_loader(mods_dir) {
        match cache.get(&jar) {
            Some(meta) => {
                for l in meta.loaders {
                    if !out.contains(&l) {
                        out.push(l);
                    }
                }
            }
            None => tracing::info!("Skipped non-zip jar during loader scan: {}", jar.display()),
        }
    }
    out
}
