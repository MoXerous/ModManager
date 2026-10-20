//! `.mrpack` index reader — Modrinth pack format, `formatVersion` 1.
//!
//! The field names and their optionality come from Modrinth's own pack-format
//! documentation, not from memory of the format. What is *not* in that document
//! is the part that matters most here: `files[].path` is resolved against the
//! instance directory, which makes it exactly as untrusted as a ZIP member
//! name. It goes through the same `safe_member_name` gate, for the same
//! reason, because `AGENTS.md` §5.6 says modpacks are untrusted input.
//!
//! This module only *reads the index*. It does not fetch, verify, or place
//! anything: the download side already exists and takes hashes and URLs.
//!
//! shortcut: no dependency resolution at read time. The index says where a
//! file goes and what it must hash to; picking which Modrinth version supplies
//! it is `resolver`'s job, which already speaks in hashes. Upgrade path: hand
//! `PackFile.sha512` to `Provider::get_versions_by_hashes` and let it return
//! the URL list to use instead of trusting `downloads` first.
//!
//! shortcut: `server-overrides` / `client-overrides` are read and reported but
//! not merged. Applying the layer order (client, then overrides, then server)
//! belongs to the install step. Ceiling: a client-side install ignores the
//! server layer entirely. Upgrade path: the installer composes three passes
//! over the same extracted tree.

use crate::instances::metadata::PROFILE_FILE_NAME;
use crate::safety::path::{safe_member_name, NameError};
use serde::Deserialize;
use thiserror::Error;

/// The only entry name the format defines, at the zip root.
pub const INDEX_ENTRY: &str = "modrinth.index.json";
/// The format version this build reads.
pub const SUPPORTED_FORMAT_VERSION: u64 = 1;

/// The executor's scratch directory at the instance root.
///
/// It is defined here, next to the destination policy, rather than in `exec`:
/// the planner must refuse a pack that writes into the installer's own scratch
/// space, and two constants drifting apart would reopen exactly that hole.
pub const STAGING_DIR: &str = ".mm-staging";

/// Root-level files this application writes and later reads as its own state.
///
/// `profile.json` is the portable instance mirror (`instances::metadata`) and
/// `instance.lock.json` is the pinned install state (`instances::lock`). A pack
/// that names one would swap app state for pack content: the next start could
/// trust a forged profile, and a forged lock would rewrite what is pinned.
/// Only the root is protected; `config/profile.json` is the user's file and
/// the pack's business.
pub const INSTANCE_MANAGED_FILES: [&str; 2] = [PROFILE_FILE_NAME, "instance.lock.json"];

/// Why a path is reserved, when it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reserved {
    /// A root-level file the application owns.
    ManagedFile(&'static str),
    /// The executor's scratch directory or anything below it.
    Staging,
}

/// Returns why `safe_path` must not be a pack destination, if at all.
///
/// The comparison is ASCII case-insensitive because on Windows `PROFILE.JSON`
/// lands on the same file as `profile.json`; refusing it on Linux too keeps
/// one pack from installing on one system and replacing app state on another.
pub fn reserved_destination(safe_path: &str) -> Option<Reserved> {
    let (first, rest) = match safe_path.split_once('/') {
        Some((first, rest)) => (first, Some(rest)),
        None => (safe_path, None),
    };
    if first.eq_ignore_ascii_case(STAGING_DIR) {
        return Some(Reserved::Staging);
    }
    if rest.is_none() {
        for name in INSTANCE_MANAGED_FILES {
            if first.eq_ignore_ascii_case(name) {
                return Some(Reserved::ManagedFile(name));
            }
            // The `.part` suffix is reserved by the downloader's own
            // protocol (`safety::atomic`): `write_atomic` refuses to
            // replace a sibling `.part` it did not write, so a pack that
            // plants `<managed-name>.part` would wedge the application's
            // very next write of that managed file forever. The suffix is
            // ours; a pack may not claim it.
            let part = format!("{name}.part");
            if first.eq_ignore_ascii_case(&part) {
                return Some(Reserved::ManagedFile(name));
            }
        }
    }
    None
}

fn default_overrides() -> String {
    "overrides".to_string()
}

// --- on-disk shape, exactly as documented -----------------------------------

// The format is camelCase on disk — `formatVersion`, `versionId`,
// `serverOverrides`. Without this the parser rejects every real pack on the
// very first field, and the compiler is happy to let it ship.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawIndex {
    format_version: u64,
    game: String,
    version_id: String,
    name: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    files: Vec<RawFile>,
    #[serde(default)]
    dependencies: std::collections::BTreeMap<String, String>,
    #[serde(default = "default_overrides")]
    overrides: String,
    #[serde(default)]
    server_overrides: Option<String>,
    #[serde(default)]
    client_overrides: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawFile {
    path: String,
    hashes: RawHashes,
    #[serde(default)]
    env: Option<RawEnv>,
    #[serde(default)]
    downloads: Vec<String>,
    #[serde(default)]
    file_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawHashes {
    sha1: String,
    sha512: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RawSide {
    Required,
    Optional,
    Unsupported,
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct RawEnv {
    client: RawSide,
    server: RawSide,
}

// --- validated shape ------------------------------------------------------

/// Which environments a pack file applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Required,
    Optional,
    Unsupported,
}

/// A pack file that has already passed the path gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackFile {
    /// Destination relative to the instance root, already normalised and
    /// proven free of `..`, absolute prefixes and separators games.
    pub safe_path: String,
    pub sha1: String,
    pub sha512: String,
    pub downloads: Vec<String>,
    pub file_size: Option<u64>,
    pub client: Side,
    pub server: Side,
}

/// A parsed pack index, safe to hand to an installer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pack {
    pub format_version: u64,
    pub version_id: String,
    pub name: String,
    pub summary: Option<String>,
    pub dependencies: std::collections::BTreeMap<String, String>,
    pub files: Vec<PackFile>,
    /// Folder names to copy over the instance, in the order the format applies
    /// them. Empty when the pack ships no overrides folder.
    pub override_layers: Vec<String>,
}

#[derive(Debug, Error)]
pub enum IndexError {
    #[error("the pack index is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("pack format version {found} is not supported; this build reads {supported}")]
    FormatVersion { found: u64, supported: u64 },
    #[error("unsupported game {found:?}; only \"minecraft\" packs exist")]
    Game { found: String },
    // `NameError` carries no Display, only the variant names; debug form is
    // the readable one here (`Absolute`, `UnsafePath`, `TooDeep`, ...).
    #[error("pack file {path:?} has an unsafe destination: {reason:?}")]
    UnsafePath { path: String, reason: NameError },
    #[error("pack file {path:?} lists no download URL")]
    NoDownload { path: String },
    #[error("pack file {path:?} has an empty sha512")]
    EmptyHash { path: String },
    // A `sha512` is not a free-text field: the executor derives a staging
    // file name from it before any hash gate runs, so a value that is not
    // exactly 128 hexadecimal characters (`sha512_invalid` in the pack
    // spec, and anything `..` or `/` inside it can walk the staging file
    // out of the instance root) must be refused while the pack is still
    // text.
    #[error("pack file {path:?} has a sha512 of {found} characters; SHA-512 is exactly 128 hexadecimal characters")]
    InvalidHash { path: String, found: usize },
    // `safe_member_name` keeps a trailing slash because a ZIP member may be a
    // directory. An index entry names a file, so the same shape is a mistake
    // that would otherwise become a directory-shaped write target.
    #[error("pack file {path:?} ends with '/'; a pack file is never a directory")]
    DirectoryPath { path: String },
    #[error("pack file {path:?} would overwrite the application-managed file {name:?}")]
    ManagedPath { path: String, name: String },
    #[error("pack file {path:?} targets the installer's staging directory")]
    StagingPath { path: String },
}

/// Reads and validates `modrinth.index.json`.
///
/// Every rejection here happens before a single byte is written anywhere. The
/// whole point of reading a plan first is that a hostile pack gets refused
/// while it is still text.
pub fn parse_index(bytes: &[u8]) -> Result<Pack, IndexError> {
    let raw: RawIndex = serde_json::from_slice(bytes)?;

    if raw.format_version != SUPPORTED_FORMAT_VERSION {
        return Err(IndexError::FormatVersion {
            found: raw.format_version,
            supported: SUPPORTED_FORMAT_VERSION,
        });
    }
    if raw.game != "minecraft" {
        return Err(IndexError::Game { found: raw.game });
    }

    let mut files = Vec::with_capacity(raw.files.len());
    for f in &raw.files {
        let safe_path = safe_member_name(&f.path).map_err(|reason| IndexError::UnsafePath {
            path: f.path.clone(),
            reason,
        })?;
        if safe_path.ends_with('/') {
            return Err(IndexError::DirectoryPath {
                path: f.path.clone(),
            });
        }
        match reserved_destination(&safe_path) {
            Some(Reserved::ManagedFile(name)) => {
                return Err(IndexError::ManagedPath {
                    path: f.path.clone(),
                    name: name.to_string(),
                });
            }
            Some(Reserved::Staging) => {
                return Err(IndexError::StagingPath {
                    path: f.path.clone(),
                });
            }
            None => {}
        }
        if f.downloads.is_empty() {
            return Err(IndexError::NoDownload {
                path: f.path.clone(),
            });
        }
        if f.hashes.sha512.trim().is_empty() {
            return Err(IndexError::EmptyHash {
                path: f.path.clone(),
            });
        }
        if !is_sha512_hex(&f.hashes.sha512) {
            return Err(IndexError::InvalidHash {
                path: f.path.clone(),
                found: f.hashes.sha512.chars().count(),
            });
        }
        // Absent `env` means required on both sides: that is what an
        // unqualified file in a pack means, and guessing "optional" would let
        // an install silently skip a dependency.
        let (client, server) = match f.env {
            None => (Side::Required, Side::Required),
            Some(e) => (side(e.client), side(e.server)),
        };
        files.push(PackFile {
            safe_path,
            sha1: f.hashes.sha1.clone(),
            sha512: f.hashes.sha512.clone(),
            downloads: f.downloads.clone(),
            file_size: f.file_size,
            client,
            server,
        });
    }

    // Client first, plain overrides, then server: the order the format says the
    // layers apply in. `as_ref` flattens the two optionals to the same
    // `&String` as the defaulted one, so the three slots are one type.
    let layers: Vec<&String> = [
        raw.client_overrides.as_ref(),
        Some(&raw.overrides),
        raw.server_overrides.as_ref(),
    ]
    .into_iter()
    .flatten()
    .collect();

    let mut override_layers = Vec::with_capacity(layers.len());
    for layer in layers {
        let name = safe_member_name(layer).map_err(|reason| IndexError::UnsafePath {
            path: layer.clone(),
            reason,
        })?;
        // A layer is a folder. `safe_member_name` keeps the trailing slash it
        // allows for directory members, so strip exactly one, the same rule
        // `inspect_zip` uses for its collision key. Without this a pack that
        // says `"clientOverrides": "client-overrides/"` matches no member and
        // its whole layer vanishes with no error.
        override_layers.push(name.strip_suffix('/').unwrap_or(&name).to_string());
    }

    Ok(Pack {
        format_version: raw.format_version,
        version_id: raw.version_id,
        name: raw.name,
        summary: raw.summary,
        dependencies: raw.dependencies,
        files,
        override_layers,
    })
}

fn side(s: RawSide) -> Side {
    match s {
        RawSide::Required => Side::Required,
        RawSide::Optional => Side::Optional,
        RawSide::Unsupported => Side::Unsupported,
    }
}

/// SHA-512 is exactly 128 hexadecimal characters, upper or lower case;
/// anything else is not a digest value and must never reach any helper that
/// builds a path (staging or destination) out of it.
fn is_sha512_hex(s: &str) -> bool {
    s.len() == 128 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

// shortcut: `sha1` is propagated for by-hash provider lookups but carries no
// shape gate the way `sha512` does. Nothing derives a filesystem path from
// it; the downloader's own SHA-1 verification is the gate where it is used.
