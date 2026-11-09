//! F-181 write path: register legacy profiles as instances in the **new**
//! database.
//!
//! Registration only. No file is written into an imported folder - not
//! `profile.json`, not `instance.lock.json`, not a renamed jar - and the old
//! application's home is never opened for writing. The only writes are the new
//! app database and one page-consistent backup of it, taken before the first
//! row.
//!
//! Plan first, execute second (AGENTS.md §5.4): [`plan_import`] answers what
//! would happen and writes nothing; [`execute_import`] re-plans in Rust so a
//! plan held by the webview is never trusted beyond the selected profile ids.

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::db::instances::{self, InstanceRow, InstanceRowError, InstanceSource};
use crate::db::{Db, DbError};

use super::{LegacyProfileScan, LegacyRootSource, LegacyScan};

/// A selected profile that will be registered.
pub const CODE_READY: &str = "ready";
/// The detected root is already registered in the new database.
pub const CODE_ALREADY_REGISTERED: &str = "already_registered";
/// The profile has no usable root, or the folder it names is gone.
pub const CODE_ROOT_MISSING: &str = "root_missing";
/// Two selected profiles resolve to the same detected instance root.
pub const CODE_DUPLICATE_IN_SELECTION: &str = "duplicate_in_selection";
/// The id the caller sent is not a profile in this scan.
pub const CODE_PROFILE_MISSING: &str = "profile_missing";
/// The row was inserted.
pub const CODE_IMPORTED: &str = "imported";

/// No old application home was found, so nothing can be selected.
pub const BLOCK_DATABASE_MISSING: &str = "database_missing";
/// The old schema is newer than this app understands.
pub const BLOCK_DATABASE_UNSUPPORTED: &str = "database_unsupported";
/// The old database exists but has no `profiles` table.
pub const BLOCK_PROFILES_TABLE_MISSING: &str = "profiles_table_missing";
/// No old application home was found.
pub const BLOCK_ROOT_NOT_FOUND: &str = "root_not_found";

/// What one entry of a plan would do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportAction {
    Register,
    Skip,
    Block,
}

/// One selected profile, resolved to what registration would write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportEntry {
    pub legacy_id: String,
    pub name: String,
    /// Detected instance root; empty when the profile has no usable root.
    pub root: PathBuf,
    /// The Minecraft directory inside [`ImportEntry::root`] (`<root>/.minecraft`
    /// for Prism-style layouts); empty when the profile has no usable root.
    /// `instances::insert` falls back to `root` for an empty value.
    pub minecraft_root: PathBuf,
    pub game_version: Option<String>,
    pub loader: Option<String>,
    pub source: InstanceSource,
    pub action: ImportAction,
    pub code: &'static str,
    /// The row that already owns [`ImportEntry::root`], when there is one.
    pub existing_instance: Option<String>,
}

/// What [`execute_import`] would do. Building it performs no writes, so it is
/// also the dry run the screen shows before asking for confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportPlan {
    pub entries: Vec<ImportEntry>,
    pub ready: usize,
    pub skipped: usize,
    pub blocked: usize,
    /// Set when the scan itself is unusable; entries are then empty.
    pub blocked_reason: Option<&'static str>,
}

/// What one selected profile turned into, after an execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportOutcome {
    pub legacy_id: String,
    pub name: String,
    pub applied: bool,
    pub code: &'static str,
    pub instance_id: Option<String>,
    /// Diagnostic detail (a path, an id). Never a translated sentence.
    pub detail: String,
}

/// What one execute did, including the backup taken before any row was
/// written (`None` when there was nothing to write).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    pub backup: Option<PathBuf>,
    pub imported: usize,
    pub skipped: usize,
    pub blocked: usize,
    pub blocked_reason: Option<&'static str>,
    pub outcomes: Vec<ImportOutcome>,
}

#[derive(Debug, Error)]
pub enum ImportError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error(transparent)]
    Row(#[from] InstanceRowError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Builds the plan. Reads the new database and the scan; writes nothing.
pub fn plan_import(
    db: &Db,
    scan: &LegacyScan,
    selected: &[String],
) -> Result<ImportPlan, ImportError> {
    if let Some(reason) = blocked_reason(scan) {
        return Ok(ImportPlan {
            entries: Vec::new(),
            ready: 0,
            skipped: 0,
            blocked: 0,
            blocked_reason: Some(reason),
        });
    }

    let mut entries = Vec::new();
    let mut seen_ids: Vec<String> = Vec::new();
    let mut seen_paths: Vec<PathBuf> = Vec::new();
    for raw in selected {
        let id = raw.trim();
        if id.is_empty() || seen_ids.iter().any(|seen| seen == id) {
            continue;
        }
        seen_ids.push(id.to_string());
        match scan.profiles.iter().find(|profile| profile.id == id) {
            Some(profile) => entries.push(profile_entry(db, profile, &mut seen_paths)?),
            None => entries.push(missing_profile_entry(id)),
        }
    }

    let count = |action: ImportAction| entries.iter().filter(|e| e.action == action).count();
    Ok(ImportPlan {
        ready: count(ImportAction::Register),
        skipped: count(ImportAction::Skip),
        blocked: count(ImportAction::Block),
        entries,
        blocked_reason: None,
    })
}

/// Applies a fresh plan: backup, one `BEGIN IMMEDIATE` batch, commit.
///
/// Expected per-profile conditions are plan outcomes, never errors. An
/// unexpected insert or database failure rolls the whole batch back and is
/// returned as an error: a half-imported selection is never reported as a
/// success.
pub fn execute_import(
    db: &Db,
    scan: &LegacyScan,
    selected: &[String],
    now: i64,
) -> Result<ImportReport, ImportError> {
    let plan = plan_import(db, scan, selected)?;

    if plan.ready == 0 {
        return Ok(ImportReport {
            backup: None,
            imported: 0,
            skipped: plan.skipped,
            blocked: plan.blocked,
            blocked_reason: plan.blocked_reason,
            outcomes: plan.entries.iter().map(outcome_of).collect(),
        });
    }

    let backup = backup(db)?;

    db.conn()
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(DbError::from)?;

    let batch = (|| -> Result<Vec<ImportOutcome>, ImportError> {
        let mut outcomes = Vec::with_capacity(plan.entries.len());
        for entry in &plan.entries {
            if entry.action != ImportAction::Register {
                outcomes.push(outcome_of(entry));
                continue;
            }
            let row = InstanceRow {
                id: uuid::Uuid::new_v4().to_string(),
                name: entry.name.clone(),
                path: entry.root.clone(),
                minecraft_root: entry.minecraft_root.clone(),
                minecraft_version: entry.game_version.clone(),
                loader: entry.loader.clone(),
                source: entry.source,
                created_at: now,
                scanned_at: None,
            };
            instances::insert(db, &row)?;
            outcomes.push(ImportOutcome {
                legacy_id: entry.legacy_id.clone(),
                name: entry.name.clone(),
                applied: true,
                code: CODE_IMPORTED,
                instance_id: Some(row.id),
                detail: entry.root.display().to_string(),
            });
        }
        Ok(outcomes)
    })();

    let outcomes = match batch {
        Ok(outcomes) => outcomes,
        Err(error) => {
            let _ = db.conn().execute_batch("ROLLBACK");
            return Err(error);
        }
    };
    if let Err(error) = db.conn().execute_batch("COMMIT") {
        let _ = db.conn().execute_batch("ROLLBACK");
        return Err(DbError::from(error).into());
    }

    let imported = outcomes.iter().filter(|outcome| outcome.applied).count();
    Ok(ImportReport {
        backup: Some(backup),
        imported,
        skipped: plan.skipped,
        blocked: plan.blocked,
        blocked_reason: plan.blocked_reason,
        outcomes,
    })
}

/// The first reason the scan cannot produce a plan at all.
fn blocked_reason(scan: &LegacyScan) -> Option<&'static str> {
    if scan.roots.source == LegacyRootSource::None {
        return Some(BLOCK_ROOT_NOT_FOUND);
    }
    let Some(database) = scan.database.as_ref() else {
        return Some(BLOCK_DATABASE_MISSING);
    };
    if !database.supported {
        return Some(BLOCK_DATABASE_UNSUPPORTED);
    }
    if !database.has_profiles_table {
        return Some(BLOCK_PROFILES_TABLE_MISSING);
    }
    None
}

fn missing_profile_entry(id: &str) -> ImportEntry {
    ImportEntry {
        legacy_id: id.to_string(),
        name: id.to_string(),
        root: PathBuf::new(),
        minecraft_root: PathBuf::new(),
        game_version: None,
        loader: None,
        source: InstanceSource::Manual,
        action: ImportAction::Block,
        code: CODE_PROFILE_MISSING,
        existing_instance: None,
    }
}

fn profile_entry(
    db: &Db,
    profile: &LegacyProfileScan,
    seen_paths: &mut Vec<PathBuf>,
) -> Result<ImportEntry, ImportError> {
    let mut entry = ImportEntry {
        legacy_id: profile.id.clone(),
        name: display_name(profile),
        root: PathBuf::new(),
        minecraft_root: PathBuf::new(),
        game_version: normalize_version(&profile.game_version),
        loader: normalize_loader(&profile.loader),
        source: source_for(&profile.kind),
        action: ImportAction::Block,
        code: CODE_ROOT_MISSING,
        existing_instance: None,
    };
    if profile.root.as_os_str().is_empty() || !profile.root_exists {
        return Ok(entry);
    }

    let (instance_root, minecraft_root) = crate::instances::detect_instance_roots(&profile.root);
    entry.root = instance_root.clone();
    entry.minecraft_root = minecraft_root;

    if let Some(owner) = instances::find_by_path(db, &instance_root)? {
        entry.action = ImportAction::Skip;
        entry.code = CODE_ALREADY_REGISTERED;
        entry.existing_instance = Some(owner.id);
        return Ok(entry);
    }

    let key = path_key(&instance_root);
    if seen_paths.contains(&key) {
        entry.action = ImportAction::Block;
        entry.code = CODE_DUPLICATE_IN_SELECTION;
        return Ok(entry);
    }
    seen_paths.push(key);
    entry.action = ImportAction::Register;
    entry.code = CODE_READY;
    Ok(entry)
}

fn outcome_of(entry: &ImportEntry) -> ImportOutcome {
    ImportOutcome {
        legacy_id: entry.legacy_id.clone(),
        name: entry.name.clone(),
        applied: false,
        code: entry.code,
        instance_id: None,
        detail: detail_of(entry),
    }
}

/// The diagnostic fragment for an outcome: a path, or the id that owns it.
///
/// Never prose and never a sentence: this crosses IPC and is shown `dir="ltr"`
/// as evidence, so the UI owns every word around it (the DTO contract in
/// `src-tauri/src/dto.rs` says the same).
fn detail_of(entry: &ImportEntry) -> String {
    if entry.code == CODE_ALREADY_REGISTERED {
        return match entry.existing_instance.as_deref() {
            Some(id) => id.to_string(),
            None => entry.root.display().to_string(),
        };
    }
    if entry.code == CODE_PROFILE_MISSING {
        return entry.legacy_id.clone();
    }
    entry.root.display().to_string()
}

/// The `instances.name` to store: the old name, or the legacy id when it is
/// empty. The fallback is the id alone, never a sentence - an English label
/// would be shown verbatim under Arabic/RTL.
///
/// A control character in a database field would otherwise reach the UI as an
/// invisible instruction, so it becomes a space.
fn display_name(profile: &LegacyProfileScan) -> String {
    let cleaned: String = profile
        .name
        .trim()
        .chars()
        .map(|c| if c.is_ascii_control() { ' ' } else { c })
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        profile.id.clone()
    } else {
        cleaned.to_string()
    }
}

/// F-181 policy: `custom` and `legacy` profiles are ones we take over; every
/// other old kind (including an empty v1 value) was the user's own folder.
fn source_for(kind: &str) -> InstanceSource {
    match kind.trim().to_lowercase().as_str() {
        "custom" | "legacy" => InstanceSource::Adopted,
        _ => InstanceSource::Manual,
    }
}

/// Empty and the "not a mod loader" values the old app accepted mean "no
/// loader". The stored value is always lowercased.
fn normalize_loader(raw: &str) -> Option<String> {
    let value = raw.trim().to_lowercase();
    if value.is_empty()
        || matches!(
            value.as_str(),
            "any" | "minecraft" | "datapack" | "iris" | "optifine" | "canvas"
        )
    {
        return None;
    }
    Some(value)
}

fn normalize_version(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// The same normalisation `instances::insert` stores, so two spellings of one
/// folder cannot both be planned as new rows.
fn path_key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// A filesystem failure is an IO error; everything else is a database error.
fn backup(db: &Db) -> Result<PathBuf, ImportError> {
    db.backup_before_legacy_import()
        .map_err(|error| match error {
            DbError::Io(io) => ImportError::Io(io),
            other => ImportError::Db(other),
        })
}
