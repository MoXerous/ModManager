//! F-097..F-100: what the files in an instance really are.
//!
//! Every scanned file gets one [`LocalContentFact`] and four *independent*
//! axes on it: ownership (managed vs external/unknown), activation
//! (`disabled`), duplication (two files sharing one identity) and
//! compatibility with the instance's own target. They are fields, never one
//! enum (F-098): a file is routinely managed *and* disabled *and* a duplicate
//! *and* incompatible at the same time, and collapsing that into a single
//! value throws away everything the user needs to decide what to do.
//!
//! [`ContentClass`] is the one derived label the screen shows. Its precedence
//! follows the old app: disabled → duplicate → incompatible → managed →
//! external/unknown.
//!
//! Reading only. Nothing here writes, moves or renames a file.

use super::lock::{LockEntry, LockFile};
use super::LocalContentFact;

/// Where a file came from: the lock's record of it, or nowhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ownership {
    /// No lock record matched by SHA-1 or by provider + version identity.
    External,
    /// Matched a lock record by SHA-1: the bytes are the identity.
    ManagedBySha1,
    /// Matched a lock record by provider + version id only. The file is ours,
    /// but it is not the byte-for-byte file we installed (F-100).
    ManagedByVersion,
}

impl Ownership {
    pub fn is_managed(self) -> bool {
        !matches!(self, Ownership::External)
    }

    /// The axis value the screen translates: `managed` | `external`.
    pub fn as_str(self) -> &'static str {
        if self.is_managed() {
            "managed"
        } else {
            "external"
        }
    }

    /// How ownership was proven: `sha1` | `provider_version` | `` (none).
    pub fn evidence(self) -> &'static str {
        match self {
            Ownership::External => "",
            Ownership::ManagedBySha1 => "sha1",
            Ownership::ManagedByVersion => "provider_version",
        }
    }
}

/// Why a file is (or cannot be proven) compatible with the instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompatibilityReason {
    /// The instance has no game version and no concrete loader to compare to.
    NoTarget,
    /// The archive declares nothing that could settle the question, or only
    /// ranges. `>=1.20` is a claim about a set, not a list, so it never proves
    /// an exclusion.
    Unproven,
    /// The archive's declared versions do not include the instance's.
    GameVersion { target: String },
    /// The archive's declared loader is not the instance's.
    Loader { target: String },
}

impl CompatibilityReason {
    /// The stable code the UI translates.
    pub fn code(&self) -> &'static str {
        match self {
            CompatibilityReason::NoTarget => "noTarget",
            CompatibilityReason::Unproven => "unproven",
            CompatibilityReason::GameVersion { .. } => "gameVersion",
            CompatibilityReason::Loader { .. } => "loader",
        }
    }

    /// The value the reason is about, when it has one.
    pub fn target(&self) -> Option<&str> {
        match self {
            CompatibilityReason::GameVersion { target }
            | CompatibilityReason::Loader { target } => Some(target),
            CompatibilityReason::NoTarget | CompatibilityReason::Unproven => None,
        }
    }
}

/// F-097: the compatibility axis. Three states, because "not proven" is not
/// the same answer as "proven incompatible".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compatibility {
    Compatible,
    Incompatible(CompatibilityReason),
    Unknown(CompatibilityReason),
}

impl Compatibility {
    /// The axis value the screen translates: `compatible` | `incompatible` |
    /// `unknown`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Compatibility::Compatible => "compatible",
            Compatibility::Incompatible(_) => "incompatible",
            Compatibility::Unknown(_) => "unknown",
        }
    }

    pub fn reason(&self) -> Option<&CompatibilityReason> {
        match self {
            Compatibility::Compatible => None,
            Compatibility::Incompatible(reason) | Compatibility::Unknown(reason) => Some(reason),
        }
    }
}

/// What the instance asks of a file. Empty fields mean "not known", which
/// makes compatibility [`Compatibility::Unknown`], never "compatible".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContentTarget {
    pub game_version: String,
    pub loader: String,
}

/// F-099: the identity two files can share. SHA-1 is the byte identity,
/// project identity is the same catalogue project at any version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DuplicateKey {
    Sha1(String),
    Project {
        provider: String,
        project_id: String,
    },
}

impl DuplicateKey {
    /// The legacy spelling, kept because it also names the warning.
    pub fn as_key(&self) -> String {
        match self {
            DuplicateKey::Sha1(hash) => format!("sha1:{hash}"),
            DuplicateKey::Project {
                provider,
                project_id,
            } => format!("provider_project:{provider}:{project_id}"),
        }
    }

    /// `sha1` | `project` — the code the UI translates.
    pub fn kind(&self) -> &'static str {
        match self {
            DuplicateKey::Sha1(_) => "sha1",
            DuplicateKey::Project { .. } => "project",
        }
    }
}

/// A finding about the folder rather than about one axis of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentWarning {
    /// Two or more scanned files share this identity (F-099).
    DuplicateLocalIdentity(DuplicateKey),
    /// The file matches the lock by identity but not by SHA-1: it changed
    /// after it was installed (F-100).
    ManagedHashMismatch { rel_path: String },
}

impl ContentWarning {
    /// The code the UI translates.
    pub fn kind(&self) -> &'static str {
        match self {
            ContentWarning::DuplicateLocalIdentity(_) => "duplicateLocalIdentity",
            ContentWarning::ManagedHashMismatch { .. } => "managedHashMismatch",
        }
    }
}

/// F-097: the five classes the installed-content view shows. Derived from the
/// axes by [`LocalContentFact::class`]; never stored instead of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentClass {
    Managed,
    ExternalUnknown,
    Disabled,
    Duplicate,
    Incompatible,
}

impl ContentClass {
    pub fn as_str(self) -> &'static str {
        match self {
            ContentClass::Managed => "managed",
            ContentClass::ExternalUnknown => "externalUnknown",
            ContentClass::Disabled => "disabled",
            ContentClass::Duplicate => "duplicate",
            ContentClass::Incompatible => "incompatible",
        }
    }
}

/// The scan plus everything the axes proved about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentSnapshot {
    pub facts: Vec<LocalContentFact>,
    pub warnings: Vec<ContentWarning>,
}

impl LocalContentFact {
    /// The one label the screen shows, derived from the axes in the old app's
    /// order. The axes themselves stay on the fact (F-098).
    pub fn class(&self) -> ContentClass {
        if self.disabled {
            ContentClass::Disabled
        } else if !self.duplicate_keys.is_empty() {
            ContentClass::Duplicate
        } else if matches!(self.compatibility, Compatibility::Incompatible(_)) {
            ContentClass::Incompatible
        } else if self.ownership.is_managed() {
            ContentClass::Managed
        } else {
            ContentClass::ExternalUnknown
        }
    }
}

/// The lock record that belongs to a file on disk.
///
/// The lock records the name the file had when it was installed, so a disabled
/// file (`a.jar.disabled`, or a copy inside `disabled_mods/`) must still find
/// its record: without that, everything this app installed and then disabled
/// would look external. Case is folded because the filesystem may not be.
pub fn lock_entry_for_name<'a>(entries: &'a [LockEntry], filename: &str) -> Option<&'a LockEntry> {
    entries
        .iter()
        .find(|entry| same_file(&entry.filename, filename))
}

fn same_file(recorded: &str, disk: &str) -> bool {
    let recorded = recorded.trim().to_lowercase();
    let disk = disk.trim().to_lowercase();
    disk == recorded
        || disk
            .strip_suffix(".disabled")
            .is_some_and(|base| base == recorded)
}

/// Fills every axis on `facts` against the lock and the instance target,
/// and collects the folder-level warnings.
pub fn classify_content(
    mut facts: Vec<LocalContentFact>,
    lock: &LockFile,
    target: &ContentTarget,
) -> ContentSnapshot {
    // Identity first: duplication and hash-mismatch both need to know which
    // project a disk file belongs to, and the lock is the only record of it.
    for fact in &mut facts {
        if let Some(entry) = lock_entry_for_name(&lock.entries, &fact.filename) {
            fact.provider = entry.provider.clone();
            fact.project_id = entry.project_id.clone();
            fact.version_id = entry.version_id.clone();
        }
    }

    let keys: Vec<Vec<DuplicateKey>> = facts.iter().map(duplicate_keys).collect();
    let mut counts: Vec<(DuplicateKey, usize)> = Vec::new();
    for key in keys.iter().flatten() {
        match counts.iter_mut().find(|(seen, _)| seen == key) {
            Some((_, count)) => *count += 1,
            None => counts.push((key.clone(), 1)),
        }
    }
    let shared = |key: &DuplicateKey| counts.iter().any(|(seen, count)| seen == key && *count > 1);

    let mut warnings = Vec::new();
    for (fact, own_keys) in facts.iter_mut().zip(keys) {
        let matched = managed_match(fact, &lock.entries);
        fact.ownership = matched
            .map(|(_, ownership)| ownership)
            .unwrap_or(Ownership::External);
        fact.duplicate_keys = own_keys.into_iter().filter(|key| shared(key)).collect();
        fact.compatibility = compatibility(fact, target);
        fact.hash_mismatch = match matched {
            Some((entry, Ownership::ManagedByVersion)) => hashes_differ(&fact.sha1, &entry.sha1),
            _ => false,
        };
        if fact.hash_mismatch {
            warnings.push(ContentWarning::ManagedHashMismatch {
                rel_path: fact.rel_path.clone(),
            });
        }
    }

    for (key, count) in counts {
        if count > 1 {
            warnings.push(ContentWarning::DuplicateLocalIdentity(key));
        }
    }

    ContentSnapshot { facts, warnings }
}

/// SHA-1 first, then provider + version id: a byte-identical file is ours even
/// when the lock's name for it is gone, and a file that still matches the
/// recorded version is ours even when its bytes changed (F-100).
fn managed_match<'a>(
    fact: &LocalContentFact,
    entries: &'a [LockEntry],
) -> Option<(&'a LockEntry, Ownership)> {
    let sha1 = normalized_sha1(&fact.sha1);
    if !sha1.is_empty() {
        if let Some(entry) = entries
            .iter()
            .find(|entry| normalized_sha1(&entry.sha1) == sha1)
        {
            return Some((entry, Ownership::ManagedBySha1));
        }
    }
    let provider = fact.provider.trim().to_lowercase();
    let version = fact.version_id.trim();
    if provider.is_empty() || version.is_empty() {
        return None;
    }
    entries
        .iter()
        .find(|entry| {
            entry.provider.trim().to_lowercase() == provider && entry.version_id.trim() == version
        })
        .map(|entry| (entry, Ownership::ManagedByVersion))
}

fn normalized_sha1(value: &str) -> String {
    value.trim().to_lowercase()
}

/// Both sides must carry a hash to be compared: a lock written before SHA-1
/// was recorded holds `""`, and "no record" is not "changed".
fn hashes_differ(local: &str, recorded: &str) -> bool {
    let local = normalized_sha1(local);
    let recorded = normalized_sha1(recorded);
    !local.is_empty() && !recorded.is_empty() && local != recorded
}

fn duplicate_keys(fact: &LocalContentFact) -> Vec<DuplicateKey> {
    let mut keys = Vec::new();
    let sha1 = normalized_sha1(&fact.sha1);
    if !sha1.is_empty() {
        keys.push(DuplicateKey::Sha1(sha1));
    }
    let provider = fact.provider.trim().to_lowercase();
    let project_id = fact.project_id.trim();
    if !provider.is_empty() && !project_id.is_empty() {
        keys.push(DuplicateKey::Project {
            provider,
            project_id: project_id.to_string(),
        });
    }
    keys
}

fn compatibility(fact: &LocalContentFact, target: &ContentTarget) -> Compatibility {
    let game = target.game_version.trim();
    let loader_display = target.loader.trim();
    let loader = loader_display.to_lowercase();
    // `any` and `minecraft` are not loaders: they constrain nothing (spec's
    // NON_PROFILE_LOADER_VALUES, kept verbatim).
    let loader_is_constraint = !loader.is_empty() && loader != "any" && loader != "minecraft";
    if game.is_empty() && !loader_is_constraint {
        return Compatibility::Unknown(CompatibilityReason::NoTarget);
    }

    let declared: Vec<&str> = fact
        .game_versions
        .iter()
        .map(|value| value.trim())
        .filter(|value| is_exact_version(value))
        .collect();
    if !game.is_empty() {
        if declared.is_empty() {
            return Compatibility::Unknown(CompatibilityReason::Unproven);
        }
        if !declared.contains(&game) {
            return Compatibility::Incompatible(CompatibilityReason::GameVersion {
                target: game.to_string(),
            });
        }
    }
    if loader_is_constraint {
        if fact.loaders.is_empty() {
            return Compatibility::Unknown(CompatibilityReason::Unproven);
        }
        if !fact
            .loaders
            .iter()
            .any(|declared| declared.as_str() == loader)
        {
            return Compatibility::Incompatible(CompatibilityReason::Loader {
                target: loader_display.to_string(),
            });
        }
    }
    Compatibility::Compatible
}

/// A token that names one release (`1.21.1`, `1.20`) rather than a range
/// (`[1.20.1,1.21.2)`, `>=1.20`, `1.20.x`). Only the former can be tested for
/// membership; reading a range as "not this version" would invent an
/// incompatibility the metadata never asserted.
fn is_exact_version(token: &str) -> bool {
    let token = token.trim();
    let mut chars = token.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_digit()
        && token
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == '-' || c == '_')
}
