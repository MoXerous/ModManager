//! F-095: the read-only profile validation report.
//!
//! One instance, one pass, thirty named checks over the things a broken
//! instance actually breaks on: the registered roots, the target
//! (`minecraft_version` + loader), the portable `profile.json` mirror, the
//! standard folders, `instance.lock.json`, and the files on disk as the
//! classifier already sees them (F-097..F-100).
//!
//! Reading only. Nothing here creates, renames, moves or writes a file: the
//! checks are `is_dir`, `read_dir`, `read_lock`, `read_mirror` and the scan,
//! and the report is the whole output. The old app had a repair action beside
//! this report; that stays out of scope until a spec asks for it, and the UI
//! comment that called validation "dropped on purpose" was stale (features.md
//! keeps F-095 at M2).
//!
//! Every check names the concrete guard it reads. A check that cannot read its
//! input says so instead of passing.

use std::path::Path;

use super::content::{classify_content, ContentTarget};
use super::lock::{read_lock, LockError, LOCK_FILE_NAME};
use super::metadata::{mirror_path, read_mirror, MetadataError};
use super::{scan_mods, Instance};
use crate::db::instances::{InstanceRow, InstanceSource};
use crate::jar_reader::{detect_loaders, ScanCache};

/// The four loaders this build can plan and install for.
const KNOWN_LOADERS: [&str; 4] = ["fabric", "quilt", "forge", "neoforge"];

/// The other value the old policy called a profile loader: a vanilla instance
/// has no mod loader, but it is a recognised target, not a broken one.
const VANILLA_LOADER: &str = "vanilla";

/// Values that name a Discover content platform, never a profile mod loader.
/// The old `profile_loader_policy` graded every one of them an error: a
/// persisted platform makes compatibility checks and scans misleading.
const NON_PROFILE_LOADERS: [&str; 6] =
    ["any", "minecraft", "datapack", "iris", "optifine", "canvas"];

/// The standard folder a Minecraft instance always has.
const MODS_DIR: &str = "mods";

/// The folder the launcher creates on first run; the reference grades a
/// missing one as a warning, never an error.
const CONFIG_DIR: &str = "config";

/// Where the toggle puts a disabled jar.
const DISABLED_DIR: &str = "disabled_mods";

/// How many names one item quotes before it says "(+N more)".
const SAMPLE_LIMIT: usize = 6;

/// The ladder the old report used, in the order it escalates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Ok,
    Info,
    Warning,
    Error,
}

impl Severity {
    /// The stable code the UI translates.
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Ok => "ok",
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

/// The thirty checks. The code is the i18n key suffix and is stable across
/// releases; the list is exhaustive on purpose, so a removed check cannot
/// silently stay in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationCheck {
    InstanceRoot,
    MinecraftRoot,
    MinecraftVersion,
    Loader,
    LoaderRecognized,
    ProfileJson,
    ProfileJsonRoots,
    ProfileJsonTarget,
    ModsDir,
    ConfigDir,
    ModsSubfolders,
    Lock,
    LockDuplicateFilenames,
    LockUnsafeFilename,
    LockMissingFiles,
    LockWithoutSha1,
    LockWithoutSha512,
    LockWithoutProject,
    LockWithoutGameVersions,
    ModsFolderFiles,
    ManagedFiles,
    ExternalFiles,
    DuplicateFiles,
    IncompatibleFiles,
    DisabledFiles,
    HashMismatch,
    UnreadableFiles,
    UnknownResourceType,
    LoaderSource,
    LoaderDeclaredByJars,
}

impl ValidationCheck {
    pub const ALL: [ValidationCheck; 30] = [
        ValidationCheck::InstanceRoot,
        ValidationCheck::MinecraftRoot,
        ValidationCheck::MinecraftVersion,
        ValidationCheck::Loader,
        ValidationCheck::LoaderRecognized,
        ValidationCheck::ProfileJson,
        ValidationCheck::ProfileJsonRoots,
        ValidationCheck::ProfileJsonTarget,
        ValidationCheck::ModsDir,
        ValidationCheck::ConfigDir,
        ValidationCheck::ModsSubfolders,
        ValidationCheck::Lock,
        ValidationCheck::LockDuplicateFilenames,
        ValidationCheck::LockUnsafeFilename,
        ValidationCheck::LockMissingFiles,
        ValidationCheck::LockWithoutSha1,
        ValidationCheck::LockWithoutSha512,
        ValidationCheck::LockWithoutProject,
        ValidationCheck::LockWithoutGameVersions,
        ValidationCheck::ModsFolderFiles,
        ValidationCheck::ManagedFiles,
        ValidationCheck::ExternalFiles,
        ValidationCheck::DuplicateFiles,
        ValidationCheck::IncompatibleFiles,
        ValidationCheck::DisabledFiles,
        ValidationCheck::HashMismatch,
        ValidationCheck::UnreadableFiles,
        ValidationCheck::UnknownResourceType,
        ValidationCheck::LoaderSource,
        ValidationCheck::LoaderDeclaredByJars,
    ];

    /// The stable code the UI translates.
    pub fn as_str(self) -> &'static str {
        match self {
            ValidationCheck::InstanceRoot => "instanceRoot",
            ValidationCheck::MinecraftRoot => "minecraftRoot",
            ValidationCheck::MinecraftVersion => "minecraftVersion",
            ValidationCheck::Loader => "loader",
            ValidationCheck::LoaderRecognized => "loaderRecognized",
            ValidationCheck::ProfileJson => "profileJson",
            ValidationCheck::ProfileJsonRoots => "profileJsonRoots",
            ValidationCheck::ProfileJsonTarget => "profileJsonTarget",
            ValidationCheck::ModsDir => "modsDir",
            ValidationCheck::ConfigDir => "configDir",
            ValidationCheck::ModsSubfolders => "modsSubfolders",
            ValidationCheck::Lock => "lock",
            ValidationCheck::LockDuplicateFilenames => "lockDuplicateFilenames",
            ValidationCheck::LockUnsafeFilename => "lockUnsafeFilename",
            ValidationCheck::LockMissingFiles => "lockMissingFiles",
            ValidationCheck::LockWithoutSha1 => "lockWithoutSha1",
            ValidationCheck::LockWithoutSha512 => "lockWithoutSha512",
            ValidationCheck::LockWithoutProject => "lockWithoutProject",
            ValidationCheck::LockWithoutGameVersions => "lockWithoutGameVersions",
            ValidationCheck::ModsFolderFiles => "modsFolderFiles",
            ValidationCheck::ManagedFiles => "managedFiles",
            ValidationCheck::ExternalFiles => "externalFiles",
            ValidationCheck::DuplicateFiles => "duplicateFiles",
            ValidationCheck::IncompatibleFiles => "incompatibleFiles",
            ValidationCheck::DisabledFiles => "disabledFiles",
            ValidationCheck::HashMismatch => "hashMismatch",
            ValidationCheck::UnreadableFiles => "unreadableFiles",
            ValidationCheck::UnknownResourceType => "unknownResourceType",
            ValidationCheck::LoaderSource => "loaderSource",
            ValidationCheck::LoaderDeclaredByJars => "loaderDeclaredByJars",
        }
    }
}

/// One row of the report: the check, its severity, and the datum it read.
///
/// The values are data, not sentences: the UI owns the wording for both
/// languages, and `count`/`total` let it say "3 of 7" without parsing text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationItem {
    pub check: ValidationCheck,
    pub severity: Severity,
    /// The concrete datum: a version, a loader, a filename, a sample of names.
    pub value: Option<String>,
    /// The path the check read, when it has one.
    pub path: Option<String>,
    pub count: Option<usize>,
    pub total: Option<usize>,
}

/// The report. `overall` is derived from the items, never passed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationReport {
    pub overall: Severity,
    pub checked_at: i64,
    pub items: Vec<ValidationItem>,
}

impl ValidationReport {
    pub fn count(&self, severity: Severity) -> usize {
        self.items
            .iter()
            .filter(|item| item.severity == severity)
            .count()
    }
}

fn item(check: ValidationCheck, severity: Severity) -> ValidationItem {
    ValidationItem {
        check,
        severity,
        value: None,
        path: None,
        count: None,
        total: None,
    }
}

fn with_value(mut item: ValidationItem, value: impl Into<String>) -> ValidationItem {
    item.value = Some(value.into());
    item
}

fn with_path(mut item: ValidationItem, path: impl Into<String>) -> ValidationItem {
    item.path = Some(path.into());
    item
}

fn with_count(mut item: ValidationItem, count: usize) -> ValidationItem {
    item.count = Some(count);
    item
}

fn with_total(mut item: ValidationItem, total: usize) -> ValidationItem {
    item.total = Some(total);
    item
}

/// The ladder: the worst item wins.
fn overall_of(items: &[ValidationItem]) -> Severity {
    items
        .iter()
        .map(|item| item.severity)
        .max()
        .unwrap_or(Severity::Ok)
}

/// Validates one registered instance. Read-only.
pub fn validate_instance(row: &InstanceRow) -> ValidationReport {
    let mut items = Vec::new();

    let root = row.path.as_path();
    let minecraft_root = if row.minecraft_root.as_os_str().is_empty() {
        root
    } else {
        row.minecraft_root.as_path()
    };
    check_roots(root, minecraft_root, &mut items);
    check_target(row, &mut items);
    check_profile_json(row, root, &mut items);

    let mods_dir = minecraft_root.join(MODS_DIR);
    check_directories(minecraft_root, &mods_dir, &mut items);

    let lock = check_lock(root, &mut items);
    check_mods_content(row, &mods_dir, lock.as_ref(), &mut items);

    ValidationReport {
        overall: overall_of(&items),
        checked_at: epoch_seconds(),
        items,
    }
}

fn check_roots(root: &Path, minecraft_root: &Path, items: &mut Vec<ValidationItem>) {
    let display = root.to_string_lossy().into_owned();
    if root.is_dir() {
        items.push(with_path(
            item(ValidationCheck::InstanceRoot, Severity::Ok),
            display,
        ));
    } else {
        items.push(with_path(
            item(ValidationCheck::InstanceRoot, Severity::Error),
            display,
        ));
    }

    let display = minecraft_root.to_string_lossy().into_owned();
    if !minecraft_root.is_dir() {
        items.push(with_path(
            item(ValidationCheck::MinecraftRoot, Severity::Error),
            display,
        ));
    } else if minecraft_root == root {
        items.push(item(ValidationCheck::MinecraftRoot, Severity::Ok));
    } else {
        // Prism/MultiMC layout: the row's `path` is the wrapper, this is the
        // game folder inside it (F-094/F-101).
        items.push(with_path(
            item(ValidationCheck::MinecraftRoot, Severity::Info),
            display,
        ));
    }
}

fn check_target(row: &InstanceRow, items: &mut Vec<ValidationItem>) {
    match concrete(&row.minecraft_version) {
        Some(version) => items.push(with_value(
            item(ValidationCheck::MinecraftVersion, Severity::Ok),
            version,
        )),
        None => items.push(item(ValidationCheck::MinecraftVersion, Severity::Error)),
    }

    match concrete(&row.loader) {
        Some(loader) => {
            items.push(with_value(
                item(ValidationCheck::Loader, Severity::Ok),
                loader.clone(),
            ));
            let normalized = loader.to_lowercase();
            if NON_PROFILE_LOADERS.contains(&normalized.as_str()) {
                // A content platform is not a mod loader (old
                // `profile_loader_policy::profile_loader_issue`).
                items.push(with_value(
                    item(ValidationCheck::LoaderRecognized, Severity::Error),
                    loader,
                ));
            } else if KNOWN_LOADERS.contains(&normalized.as_str()) || normalized == VANILLA_LOADER {
                items.push(item(ValidationCheck::LoaderRecognized, Severity::Ok));
            } else {
                items.push(with_value(
                    item(ValidationCheck::LoaderRecognized, Severity::Warning),
                    loader,
                ));
            }
        }
        None => items.push(item(ValidationCheck::Loader, Severity::Error)),
    }
}

fn check_profile_json(row: &InstanceRow, root: &Path, items: &mut Vec<ValidationItem>) {
    let path = mirror_path(root);
    let display = path.to_string_lossy().into_owned();
    if !path.exists() {
        // Only a managed folder is expected to carry the mirror (F-103: a
        // linked folder is never written to).
        let severity = match row.source {
            InstanceSource::Managed => Severity::Warning,
            InstanceSource::Manual | InstanceSource::Adopted => Severity::Info,
        };
        items.push(with_path(
            item(ValidationCheck::ProfileJson, severity),
            display,
        ));
        return;
    }

    match read_mirror(&path) {
        Ok(meta) => {
            items.push(with_path(
                item(ValidationCheck::ProfileJson, Severity::Ok),
                display,
            ));
            let mut drift = Vec::new();
            if !same_path(&meta.instance_root, root) {
                drift.push(meta.instance_root.clone());
            }
            let minecraft_root = if row.minecraft_root.as_os_str().is_empty() {
                root
            } else {
                row.minecraft_root.as_path()
            };
            if !same_path(&meta.minecraft_root, minecraft_root) {
                drift.push(meta.minecraft_root.clone());
            }
            if drift.is_empty() {
                items.push(item(ValidationCheck::ProfileJsonRoots, Severity::Ok));
            } else {
                items.push(with_value(
                    item(ValidationCheck::ProfileJsonRoots, Severity::Warning),
                    drift.join(", "),
                ));
            }

            let mut target_drift = Vec::new();
            if concrete(&meta.minecraft_version) != concrete(&row.minecraft_version) {
                target_drift.push(format!(
                    "minecraft={}",
                    meta.minecraft_version.as_deref().unwrap_or("")
                ));
            }
            if concrete(&meta.loader) != concrete(&row.loader) {
                target_drift.push(format!("loader={}", meta.loader.as_deref().unwrap_or("")));
            }
            if target_drift.is_empty() {
                items.push(item(ValidationCheck::ProfileJsonTarget, Severity::Ok));
            } else {
                items.push(with_value(
                    item(ValidationCheck::ProfileJsonTarget, Severity::Warning),
                    target_drift.join(", "),
                ));
            }
        }
        Err(MetadataError::NewerSchema { found, .. }) => items.push(with_value(
            with_path(item(ValidationCheck::ProfileJson, Severity::Error), display),
            found,
        )),
        Err(_) => items.push(with_path(
            item(ValidationCheck::ProfileJson, Severity::Error),
            display,
        )),
    }
}

fn check_directories(minecraft_root: &Path, mods_dir: &Path, items: &mut Vec<ValidationItem>) {
    let display = mods_dir.to_string_lossy().into_owned();
    let mods_present = mods_dir.is_dir();
    items.push(if mods_present {
        with_path(item(ValidationCheck::ModsDir, Severity::Ok), display)
    } else {
        with_path(item(ValidationCheck::ModsDir, Severity::Error), display)
    });

    let config = minecraft_root.join(CONFIG_DIR);
    let display = config.to_string_lossy().into_owned();
    items.push(if config.is_dir() {
        with_path(item(ValidationCheck::ConfigDir, Severity::Ok), display)
    } else {
        // Launchers create it on first run; its absence is not a broken folder.
        with_path(item(ValidationCheck::ConfigDir, Severity::Warning), display)
    });

    if !mods_present {
        return;
    }
    // The content listing is non-recursive (F-101): a jar filed in a subfolder
    // is not listed, which is worth saying once rather than per file.
    let mut subfolders: Vec<String> = std::fs::read_dir(mods_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.path().is_dir())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| !name.eq_ignore_ascii_case(DISABLED_DIR))
                .collect()
        })
        .unwrap_or_default();
    subfolders.sort();
    items.push(if subfolders.is_empty() {
        item(ValidationCheck::ModsSubfolders, Severity::Ok)
    } else {
        with_count(
            with_value(
                item(ValidationCheck::ModsSubfolders, Severity::Info),
                sample(&subfolders),
            ),
            subfolders.len(),
        )
    });
}

/// Reads the lock and reports its own state, then the entry checks. Returns
/// the lock when it was readable so the content checks can use it.
fn check_lock(root: &Path, items: &mut Vec<ValidationItem>) -> Option<super::lock::LockFile> {
    let path = root.join(LOCK_FILE_NAME);
    let display = path.to_string_lossy().into_owned();
    if !path.exists() {
        // Nothing recorded is a state, not a failure: every file stays
        // external/unknown until something is installed.
        items.push(with_path(
            item(ValidationCheck::Lock, Severity::Info),
            display,
        ));
        return None;
    }
    if !path.is_file() {
        items.push(with_path(
            item(ValidationCheck::Lock, Severity::Error),
            display,
        ));
        return None;
    }
    let lock = match read_lock(&path) {
        Ok(lock) => lock,
        Err(LockError::NewerSchema { found, .. }) => {
            // §5.10: a newer lock is refused, never read as an empty list.
            items.push(with_value(
                with_path(item(ValidationCheck::Lock, Severity::Error), display),
                found.to_string(),
            ));
            return None;
        }
        Err(_) => {
            items.push(with_path(
                item(ValidationCheck::Lock, Severity::Error),
                display,
            ));
            return None;
        }
    };

    items.push(with_count(
        with_path(item(ValidationCheck::Lock, Severity::Ok), display),
        lock.entries.len(),
    ));

    let mut duplicates: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut unsafe_names: Vec<String> = Vec::new();
    let mut without_sha1 = 0usize;
    let mut without_sha512 = 0usize;
    let mut without_project = 0usize;
    let mut without_versions = 0usize;
    for entry in &lock.entries {
        let folded = entry.filename.trim().to_lowercase();
        if seen.contains(&folded) {
            duplicates.push(entry.filename.clone());
        } else {
            seen.push(folded);
        }
        if !is_safe_filename(&entry.filename) {
            unsafe_names.push(entry.filename.clone());
        }
        if entry.sha1.trim().is_empty() {
            without_sha1 += 1;
        }
        if entry.sha512.trim().is_empty() {
            without_sha512 += 1;
        }
        if entry.project_id.trim().is_empty() {
            without_project += 1;
        }
        if !entry
            .game_versions
            .iter()
            .any(|version| !version.trim().is_empty())
        {
            without_versions += 1;
        }
    }

    items.push(if duplicates.is_empty() {
        item(ValidationCheck::LockDuplicateFilenames, Severity::Ok)
    } else {
        with_count(
            with_value(
                item(ValidationCheck::LockDuplicateFilenames, Severity::Warning),
                sample(&duplicates),
            ),
            duplicates.len(),
        )
    });
    items.push(if unsafe_names.is_empty() {
        item(ValidationCheck::LockUnsafeFilename, Severity::Ok)
    } else {
        with_count(
            with_value(
                item(ValidationCheck::LockUnsafeFilename, Severity::Warning),
                sample(&unsafe_names),
            ),
            unsafe_names.len(),
        )
    });
    items.push(counted(
        ValidationCheck::LockWithoutSha1,
        without_sha1,
        Severity::Info,
    ));
    items.push(counted(
        ValidationCheck::LockWithoutSha512,
        without_sha512,
        Severity::Info,
    ));
    items.push(counted(
        ValidationCheck::LockWithoutProject,
        without_project,
        Severity::Info,
    ));
    items.push(counted(
        ValidationCheck::LockWithoutGameVersions,
        without_versions,
        Severity::Info,
    ));

    Some(lock)
}

/// The on-disk half: the scan, the classification, and the checks that need
/// both. Skipped entirely when the mods folder is not there — the `modsDir`
/// error already says why.
fn check_mods_content(
    row: &InstanceRow,
    mods_dir: &Path,
    lock: Option<&super::lock::LockFile>,
    items: &mut Vec<ValidationItem>,
) {
    if !mods_dir.is_dir() {
        return;
    }
    let minecraft_root = if row.minecraft_root.as_os_str().is_empty() {
        row.path.clone()
    } else {
        row.minecraft_root.clone()
    };
    let instance = Instance {
        root: row.path.clone(),
        minecraft_root,
    };
    let facts = scan_mods(&instance);
    let lock_entries = lock.cloned().unwrap_or_default();
    let target = ContentTarget {
        game_version: concrete(&row.minecraft_version).unwrap_or_default(),
        loader: concrete(&row.loader).unwrap_or_default(),
    };
    let snapshot = classify_content(facts, &lock_entries, &target);

    items.push(if snapshot.facts.is_empty() {
        with_path(
            item(ValidationCheck::ModsFolderFiles, Severity::Info),
            mods_dir.to_string_lossy().into_owned(),
        )
    } else {
        with_count(
            item(ValidationCheck::ModsFolderFiles, Severity::Ok),
            snapshot.facts.len(),
        )
    });

    let managed = snapshot
        .facts
        .iter()
        .filter(|fact| fact.ownership.is_managed())
        .count();
    let total = snapshot.facts.len();
    items.push(if managed == 0 {
        with_total(item(ValidationCheck::ManagedFiles, Severity::Info), total)
    } else {
        with_total(
            with_count(item(ValidationCheck::ManagedFiles, Severity::Ok), managed),
            total,
        )
    });

    let names = |filter: &dyn Fn(&&super::LocalContentFact) -> bool| -> Vec<String> {
        snapshot
            .facts
            .iter()
            .filter(filter)
            .map(|fact| fact.rel_path.clone())
            .collect()
    };

    let external = names(&|fact| !fact.ownership.is_managed());
    items.push(flagged(
        ValidationCheck::ExternalFiles,
        Severity::Warning,
        &external,
    ));
    let duplicates = names(&|fact| !fact.duplicate_keys.is_empty());
    items.push(flagged(
        ValidationCheck::DuplicateFiles,
        Severity::Warning,
        &duplicates,
    ));
    let incompatible =
        names(&|fact| matches!(fact.compatibility, super::Compatibility::Incompatible(_)));
    items.push(flagged(
        ValidationCheck::IncompatibleFiles,
        Severity::Warning,
        &incompatible,
    ));
    let disabled = names(&|fact| fact.disabled);
    items.push(flagged(
        ValidationCheck::DisabledFiles,
        Severity::Info,
        &disabled,
    ));
    let mismatched = names(&|fact| fact.hash_mismatch);
    items.push(flagged(
        ValidationCheck::HashMismatch,
        Severity::Warning,
        &mismatched,
    ));
    let unreadable = names(&|fact| fact.sha1.trim().is_empty());
    items.push(flagged(
        ValidationCheck::UnreadableFiles,
        Severity::Warning,
        &unreadable,
    ));
    let unknown_type =
        names(&|fact| matches!(fact.resource_type, crate::jar_reader::ResourceType::Unknown));
    items.push(flagged(
        ValidationCheck::UnknownResourceType,
        Severity::Info,
        &unknown_type,
    ));

    // A record whose file is gone is a stale record, not a missing install: the
    // user's answer is the lock cleanup, and this row is what points at it.
    //
    // Presence is a recursive question (F-131): moving a jar into `mods/<sub>/`
    // does not make the recorded file absent, even though the content listing
    // above stays non-recursive (F-101). The scan and the helper are the ones
    // the lock prune already uses, so the two paths cannot disagree about what
    // "present" means.
    if let Some(lock) = lock {
        let presence = super::scan_mods_recursive(&instance);
        let mut missing: Vec<String> = lock
            .entries
            .iter()
            .filter(|entry| !crate::updates::target::entry_is_present(entry, &presence))
            .map(|entry| entry.filename.clone())
            .collect();
        missing.sort();
        items.push(flagged(
            ValidationCheck::LockMissingFiles,
            Severity::Warning,
            &missing,
        ));
    }

    let mut cache = ScanCache::new();
    let detected = detect_loaders(&[], mods_dir, &mut cache);
    if detected.is_empty() {
        items.push(item(ValidationCheck::LoaderSource, Severity::Info));
    } else {
        items.push(with_value(
            item(ValidationCheck::LoaderSource, Severity::Ok),
            detected
                .iter()
                .map(|loader| loader.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }

    let target_loader = concrete(&row.loader).map(|loader| loader.to_lowercase());
    match target_loader {
        Some(loader) if KNOWN_LOADERS.contains(&loader.as_str()) => {
            let mismatched: Vec<String> = detected
                .iter()
                .filter(|declared| declared.as_str() != loader)
                .map(|declared| declared.as_str().to_string())
                .collect();
            if mismatched.is_empty() {
                items.push(item(ValidationCheck::LoaderDeclaredByJars, Severity::Ok));
            } else {
                items.push(with_value(
                    item(ValidationCheck::LoaderDeclaredByJars, Severity::Warning),
                    mismatched.join(", "),
                ));
            }
        }
        // No concrete target loader: the check has nothing to compare against
        // and says so instead of passing.
        _ => items.push(item(ValidationCheck::LoaderDeclaredByJars, Severity::Info)),
    }
}

/// `Ok` for zero, `severity` (with the count) above it.
fn counted(check: ValidationCheck, count: usize, severity: Severity) -> ValidationItem {
    if count == 0 {
        item(check, Severity::Ok)
    } else {
        with_count(item(check, severity), count)
    }
}

/// `Ok` for an empty list, `severity` with the names above it.
fn flagged(check: ValidationCheck, severity: Severity, names: &[String]) -> ValidationItem {
    if names.is_empty() {
        item(check, Severity::Ok)
    } else {
        with_count(
            with_value(item(check, severity), sample(names)),
            names.len(),
        )
    }
}

/// Trims a stored value and treats the sentinel/blank as absent, so the check
/// never says "configured: unknown".
fn concrete(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("unknown"))
        .map(str::to_string)
}

/// The old report's sample shape: first few names, then how many were hidden.
fn sample(names: &[String]) -> String {
    let shown: Vec<&str> = names
        .iter()
        .take(SAMPLE_LIMIT)
        .map(String::as_str)
        .collect();
    if names.len() > SAMPLE_LIMIT {
        format!(
            "{} (+{} more)",
            shown.join(", "),
            names.len() - SAMPLE_LIMIT
        )
    } else {
        shown.join(", ")
    }
}

/// A lock filename must be one path component: the lock is durable state that
/// a hand edit can reach, and a separator or `..` there would address a file
/// outside the instance.
fn is_safe_filename(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && !name.contains(['/', '\\', ':', '\0']) && name != "." && name != ".."
}

/// Whether a path recorded in the portable mirror still names the folder the
/// row holds. Canonicalize first (the mirror may spell the same folder
/// differently); fall back to a case-insensitive compare, which is what NTFS
/// itself does.
fn same_path(recorded: &str, actual: &Path) -> bool {
    let recorded = Path::new(recorded);
    if recorded == actual {
        return true;
    }
    match (
        std::fs::canonicalize(recorded),
        std::fs::canonicalize(actual),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => recorded
            .to_string_lossy()
            .eq_ignore_ascii_case(&actual.to_string_lossy()),
    }
}

/// Unix seconds, the unit the instance row's timestamps use.
fn epoch_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
