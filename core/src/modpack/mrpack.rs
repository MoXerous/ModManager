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

use crate::safety::path::{safe_member_name, NameError};
use serde::Deserialize;
use thiserror::Error;

/// The only entry name the format defines, at the zip root.
pub const INDEX_ENTRY: &str = "modrinth.index.json";
/// The format version this build reads.
pub const SUPPORTED_FORMAT_VERSION: u64 = 1;

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
        override_layers.push(name);
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
