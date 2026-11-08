//! Read-only update preview over the instance lock (F-120, F-128, F-129).
//!
//! The check reads `instance.lock.json`, asks the provider about the recorded
//! hashes, and compares the answer with the recorded identity. It writes
//! nothing: no journal, no lock rewrite, no database row. Installing a newer
//! version is a separate flow that needs a plan, a backup and a re-check, and
//! it is deliberately not part of this module.
//!
//! SHA-512 is asked for first because it is the hash the downloader verified
//! and the only one that is still collision-resistant. SHA-1 is a fallback for
//! locks written before `sha512` existed, never the first question. A provider
//! that has no hash lookup at all is reported as `Unsupported`; falling
//! through to "no update" would turn "I cannot tell" into "you are current".

use std::collections::HashMap;
use std::path::Path;

use crate::instances::lock::{read_lock, LockEntry, LockError, LOCK_FILE_NAME};
use crate::providers::{HashAlgo, LatestFilter, ProjectFile, Provider, ProviderError, Version};

/// What the check can say about one recorded file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateStatus {
    /// The provider's latest version for this file is a different one.
    Available,
    /// The provider's latest version for this file is the installed one.
    UpToDate,
    /// The provider answered, but not about this file.
    Unidentified,
    /// The provider cannot answer this question at all.
    Unsupported,
}

/// One locked file and what the provider says about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCandidate {
    pub filename: String,
    pub provider: String,
    pub project_id: String,
    pub current_version_id: String,
    pub current_sha1: String,
    pub current_sha512: String,
    pub latest_version_id: Option<String>,
    pub latest_version_number: Option<String>,
    pub latest_filename: Option<String>,
    pub latest_sha1: Option<String>,
    pub latest_sha512: Option<String>,
    pub status: UpdateStatus,
}

/// A fact about the check itself, not about one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateWarning {
    /// The provider has no hash lookup; every candidate is `Unsupported`.
    Unsupported,
    /// There is no lock file in this instance.
    EmptyLock,
    /// The lock exists but cannot be read; the check cannot start.
    LockUnreadable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePreview {
    /// How many locked files were eligible for this check.
    pub checked: usize,
    pub candidates: Vec<UpdateCandidate>,
    pub warnings: Vec<UpdateWarning>,
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("an update check needs a concrete loader and Minecraft version")]
    NeedsConcreteTarget,
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Lock(#[from] LockError),
}

/// Previews the updates for the instance at `instance_root` using `provider`.
///
/// Only files recorded with this provider (or with no provider at all, which
/// old locks mean as "the only one there was") are considered: asking one
/// catalogue about another catalogue's files would report a false
/// `Unidentified` for every one of them.
///
/// A missing lock is not an error; it is `EmptyLock`, because an instance with
/// nothing recorded yet is a normal state. A newer lock schema is an error:
/// interpreting it would be guessing (§5.10).
pub async fn check_updates(
    provider: &dyn Provider,
    instance_root: &Path,
    game_version: &str,
    loader: &str,
) -> Result<UpdatePreview, UpdateError> {
    let game_version = game_version.trim();
    let loader = loader.trim();
    if game_version.is_empty() || !is_concrete_loader(loader) {
        return Err(UpdateError::NeedsConcreteTarget);
    }

    let mut warnings = Vec::new();
    let lock_path = instance_root.join(LOCK_FILE_NAME);
    let lock = match read_lock(&lock_path) {
        Ok(lock) => lock,
        Err(LockError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(UpdatePreview {
                checked: 0,
                candidates: Vec::new(),
                warnings: vec![UpdateWarning::EmptyLock],
            });
        }
        // The typed refusal the rest of the app already translates; a newer
        // lock must never be read as "nothing to update".
        Err(e @ LockError::NewerSchema { .. }) => return Err(e.into()),
        Err(_) => {
            return Ok(UpdatePreview {
                checked: 0,
                candidates: Vec::new(),
                warnings: vec![UpdateWarning::LockUnreadable],
            });
        }
    };

    let provider_id = provider.provider_id();
    let eligible: Vec<&LockEntry> = lock
        .entries
        .iter()
        .filter(|e| e.provider.is_empty() || e.provider.eq_ignore_ascii_case(provider_id))
        .filter(|e| is_recordable_file(&e.filename))
        .filter(|e| !e.sha512.is_empty() || !e.sha1.is_empty())
        .collect();
    let checked = eligible.len();
    if checked == 0 {
        return Ok(UpdatePreview {
            checked: 0,
            candidates: Vec::new(),
            warnings,
        });
    }

    let filter = LatestFilter {
        loaders: vec![loader.to_lowercase()],
        game_versions: vec![game_version.to_string()],
    };

    // `None` means "this provider has no hash lookup", which is not the same
    // as "the provider knows no version for these hashes".
    let mut found: HashMap<usize, Version> = HashMap::new();
    let mut unsupported = false;

    let sha512_hashes: Vec<(usize, String)> = eligible
        .iter()
        .enumerate()
        .filter(|(_, e)| !e.sha512.is_empty())
        .map(|(i, e)| (i, e.sha512.to_lowercase()))
        .collect();
    if !sha512_hashes.is_empty() {
        match lookup(provider, &sha512_hashes, HashAlgo::Sha512, &filter).await? {
            Some(map) => absorb(&mut found, &sha512_hashes, map),
            None => unsupported = true,
        }
    }

    // The fallback is only for records the provider did not answer for; a
    // provider without a hash lookup is never asked twice.
    if !unsupported {
        let sha1_hashes: Vec<(usize, String)> = eligible
            .iter()
            .enumerate()
            .filter(|(i, _)| !found.contains_key(i))
            .filter(|(_, e)| !e.sha1.is_empty())
            .map(|(i, e)| (i, e.sha1.to_lowercase()))
            .collect();
        if !sha1_hashes.is_empty() {
            match lookup(provider, &sha1_hashes, HashAlgo::Sha1, &filter).await? {
                Some(map) => absorb(&mut found, &sha1_hashes, map),
                None => unsupported = true,
            }
        }
    }

    if unsupported {
        warnings.push(UpdateWarning::Unsupported);
    }

    let candidates = eligible
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let latest = found.get(&index);
            let status = match latest {
                Some(version) if recorded_file_is_current(entry, version) => UpdateStatus::UpToDate,
                Some(_) => UpdateStatus::Available,
                None if unsupported => UpdateStatus::Unsupported,
                None => UpdateStatus::Unidentified,
            };
            let file = latest.and_then(primary_file);
            UpdateCandidate {
                filename: entry.filename.clone(),
                provider: if entry.provider.is_empty() {
                    provider_id.to_string()
                } else {
                    entry.provider.clone()
                },
                project_id: entry.project_id.clone(),
                current_version_id: entry.version_id.clone(),
                current_sha1: entry.sha1.clone(),
                current_sha512: entry.sha512.clone(),
                latest_version_id: latest.map(|v| v.id.clone()),
                latest_version_number: latest.map(|v| v.version_number.clone()),
                latest_filename: file.map(|f| f.filename.clone()),
                latest_sha1: file.and_then(|f| f.sha1.clone()),
                latest_sha512: file.and_then(|f| f.sha512.clone()),
                status,
            }
        })
        .collect();

    Ok(UpdatePreview {
        checked,
        candidates,
        warnings,
    })
}

/// `any` is the old app's "no filter" sentinel, not a loader; `unknown` is
/// this app's "could not detect" one. Neither can be sent to a provider as if
/// it were a concrete loader.
fn is_concrete_loader(loader: &str) -> bool {
    !loader.is_empty()
        && !loader.eq_ignore_ascii_case("any")
        && !loader.eq_ignore_ascii_case(crate::db::instances::UNKNOWN)
}

/// The files an update check may ask about.
fn is_recordable_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".jar") || lower.ends_with(".jar.disabled")
}

/// `Ok(None)` is `ProviderError::Unsupported`; every other failure propagates.
async fn lookup(
    provider: &dyn Provider,
    wanted: &[(usize, String)],
    algo: HashAlgo,
    filter: &LatestFilter,
) -> Result<Option<HashMap<String, Version>>, ProviderError> {
    let hashes: Vec<String> = wanted.iter().map(|(_, hash)| hash.clone()).collect();
    match provider
        .get_versions_by_hashes(&hashes, algo, Some(filter))
        .await
    {
        Ok(map) => Ok(Some(map)),
        Err(ProviderError::Unsupported { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

fn absorb(
    found: &mut HashMap<usize, Version>,
    wanted: &[(usize, String)],
    answer: HashMap<String, Version>,
) {
    for (index, hash) in wanted {
        if let Some(version) = answer.get(hash) {
            found.insert(*index, version.clone());
        }
    }
}

/// The version is the installed one if the ids agree, or if any of its files
/// carries the hash we recorded. Ids are the cheap answer; the hash covers
/// locks that predate `version_id` and versions whose id a provider reissued.
fn recorded_file_is_current(entry: &LockEntry, latest: &Version) -> bool {
    if !entry.version_id.is_empty() && entry.version_id == latest.id {
        return true;
    }
    latest.files.iter().any(|file| {
        let sha512 = file.sha512.as_deref().unwrap_or_default();
        let sha1 = file.sha1.as_deref().unwrap_or_default();
        (!entry.sha512.is_empty() && sha512.eq_ignore_ascii_case(&entry.sha512))
            || (!entry.sha1.is_empty() && sha1.eq_ignore_ascii_case(&entry.sha1))
    })
}

/// The file a version presents as its main artifact. `files` is never empty
/// for a real version, but the UI is not the place to discover a provider
/// quirk, so the first file is a defensible fallback.
fn primary_file(version: &Version) -> Option<&ProjectFile> {
    version
        .files
        .iter()
        .find(|f| f.primary)
        .or_else(|| version.files.first())
}

#[cfg(test)]
mod tests;
