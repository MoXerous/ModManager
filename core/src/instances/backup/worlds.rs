//! Read-only worlds preview (F-118).
//!
//! Enumerating what is under `saves/` is a read: no folder is created, no file
//! is opened for write, and a link is listed but never followed. The old app's
//! English sentences are replaced by codes the UI translates.

use std::path::{Path, PathBuf};

use crate::safety::guard;

/// What a direct child of `saves/` turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldStatus {
    Detected,
    MissingLevelDat,
    Symlink,
}

impl WorldStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            WorldStatus::Detected => "detected",
            WorldStatus::MissingLevelDat => "missing-level-dat",
            WorldStatus::Symlink => "symlink",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldEntry {
    pub name: String,
    pub status: WorldStatus,
    pub datapack_count: u64,
    pub has_level_dat: bool,
    /// Stable code the UI translates; never an English sentence.
    pub message_code: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldsPreview {
    pub saves_dir: PathBuf,
    pub saves_exists: bool,
    pub worlds: Vec<WorldEntry>,
    pub message_code: &'static str,
}

/// A read-only inventory of the worlds in one instance.
///
/// Direct children only, dot-prefixed names excluded, plain files skipped, and
/// the list sorted without regard to case so two filesystems agree on the order.
pub fn preview_worlds(root: &Path) -> WorldsPreview {
    let saves_dir = root.join("saves");
    let saves_is_dir = std::fs::symlink_metadata(&saves_dir)
        .map(|meta| meta.is_dir())
        .unwrap_or(false);
    if !saves_is_dir {
        return WorldsPreview {
            saves_dir,
            saves_exists: false,
            worlds: Vec::new(),
            message_code: "worlds.no-saves",
        };
    }

    let mut worlds = Vec::new();
    if let Ok(read) = std::fs::read_dir(&saves_dir) {
        for child in read.flatten() {
            let name = child.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let path = child.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            // A link is listed and never opened: the preview does not count
            // what it does not own.
            if guard::is_link_or_reparse(&path) {
                worlds.push(WorldEntry {
                    name,
                    status: WorldStatus::Symlink,
                    datapack_count: 0,
                    has_level_dat: false,
                    message_code: "worlds.symlink-skipped",
                });
                continue;
            }
            if !meta.is_dir() {
                continue;
            }
            let has_level_dat = std::fs::symlink_metadata(path.join("level.dat"))
                .map(|meta| meta.is_file())
                .unwrap_or(false);
            let datapack_count = count_direct_children(&path.join("datapacks"));
            worlds.push(WorldEntry {
                name,
                status: if has_level_dat {
                    WorldStatus::Detected
                } else {
                    WorldStatus::MissingLevelDat
                },
                datapack_count,
                has_level_dat,
                message_code: if has_level_dat {
                    "worlds.level-dat-found"
                } else {
                    "worlds.level-dat-missing"
                },
            });
        }
    }
    worlds.sort_by_key(|world| world.name.to_lowercase());
    let message_code = if worlds.is_empty() {
        "worlds.empty"
    } else {
        "worlds.read-only"
    };
    WorldsPreview {
        saves_dir,
        saves_exists: true,
        worlds,
        message_code,
    }
}

/// Direct children of `datapacks/`, dot-prefixed ones excluded. A link or a
/// missing folder counts zero: this is a read, and a read through a link is a
/// read we do not perform.
fn count_direct_children(folder: &Path) -> u64 {
    if guard::is_link_or_reparse(folder) {
        return 0;
    }
    let Ok(read) = std::fs::read_dir(folder) else {
        return 0;
    };
    read.flatten()
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .count() as u64
}
