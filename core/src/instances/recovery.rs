//! F-146: what the app may offer after a crashed instance create.
//!
//! Read-only on purpose. `discover()` finds the unfinished record; this module
//! decides which of the safe options applies by *inspecting the disk and the
//! database*, never by trusting the record alone — a record cannot tell
//! "crashed before the first mkdir" from "crashed after the last".
//!
//! The root is never removed. `plan_create` permits a pre-existing empty root,
//! and nothing on disk proves we created it.

use std::path::PathBuf;

use crate::db::instances::{self, InstanceRow};
use crate::instances::metadata;
use crate::instances::registry::RegistryError;
use crate::journal::{Journal, OperationKind, OperationRecord, OperationState};

/// What the user can be offered. Never applied without a deliberate choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryAction {
    /// Nothing to do: the create either finished or left no trace.
    Leave,
    /// The mirror is written and valid, so finishing is safer than deleting:
    /// it completes what the user confirmed in the preview.
    Finish,
    /// Directories exist with no mirror. Offer to remove the empty ones.
    RemoveEmptyChildren,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRecovery {
    pub operation_id: String,
    pub root: PathBuf,
    pub instance_id: String,
    pub action: RecoveryAction,
    /// Why, as data. The UI translates it; core does not write copy.
    pub reason: RecoveryReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryReason {
    /// The record is not a create, or is already terminal.
    NotApplicable,
    /// The row exists: the create succeeded and only the record lagged.
    AlreadyRegistered,
    /// Crashed before the first directory.
    NothingWritten,
    /// Mirror present and readable: finish it.
    MirrorPresent,
    /// Mirror present but unreadable, so it cannot be trusted to finish.
    MirrorUnreadable,
    /// Directories without a mirror.
    DirectoriesWithoutMirror,
}

impl RecoveryReason {
    /// Stable key for the UI to translate; core never ships display strings.
    pub fn key(self) -> &'static str {
        match self {
            Self::NotApplicable => "recovery.instance.notApplicable",
            Self::AlreadyRegistered => "recovery.instance.alreadyRegistered",
            Self::NothingWritten => "recovery.instance.nothingWritten",
            Self::MirrorPresent => "recovery.instance.mirrorPresent",
            Self::MirrorUnreadable => "recovery.instance.mirrorUnreadable",
            Self::DirectoriesWithoutMirror => "recovery.instance.directoriesWithoutMirror",
        }
    }
}

/// Inspects one record. Never mutates anything.
pub fn assess_create(db: &crate::db::Db, record: &OperationRecord) -> CreateRecovery {
    let root = PathBuf::from(&record.target_path);
    let mut out = CreateRecovery {
        operation_id: record.operation_id.clone(),
        root: root.clone(),
        instance_id: record.profile_id.clone(),
        action: RecoveryAction::Leave,
        reason: RecoveryReason::NotApplicable,
    };

    if record.kind != OperationKind::InstanceCreate
        || !matches!(
            record.state,
            OperationState::Planned | OperationState::Running
        )
    {
        return out;
    }

    if instances::get(db, &record.profile_id)
        .ok()
        .flatten()
        .is_some()
    {
        out.reason = RecoveryReason::AlreadyRegistered;
        return out;
    }

    if !root.is_dir() {
        out.reason = RecoveryReason::NothingWritten;
        return out;
    }

    let mirror = metadata::mirror_path(&root);
    match metadata::read_mirror(&mirror) {
        Ok(_) => {
            out.action = RecoveryAction::Finish;
            out.reason = RecoveryReason::MirrorPresent;
        }
        Err(_) => {
            out.action = RecoveryAction::RemoveEmptyChildren;
            out.reason = if mirror.exists() {
                RecoveryReason::MirrorUnreadable
            } else {
                RecoveryReason::DirectoriesWithoutMirror
            };
        }
    }
    out
}

/// Unfinished creates, assessed. Read-only.
pub fn pending_creates(db: &crate::db::Db, journal: &Journal) -> Vec<CreateRecovery> {
    journal
        .discover()
        .unfinished()
        .filter(|r| r.kind == OperationKind::InstanceCreate)
        .map(|r| assess_create(db, r))
        .collect()
}

/// Removes the standard subdirectories that are still empty. Never the root,
/// never `remove_dir_all`.
pub fn remove_empty_children(root: &std::path::Path) -> Result<usize, RegistryError> {
    let mut removed = 0usize;
    for name in crate::instances::registry::STANDARD_DIRS {
        if std::fs::remove_dir(root.join(name)).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Finishes a half-created instance: the mirror is already written and valid,
/// so this re-inserts the row the same way `execute_create` would have.
pub fn finish_create(
    db: &crate::db::Db,
    record: &OperationRecord,
) -> Result<InstanceRow, RegistryError> {
    let root = PathBuf::from(&record.target_path);
    let meta = metadata::read_mirror(&metadata::mirror_path(&root))?;
    // The mirror is a file on disk that the user can edit between the crash
    // and this call. Refuse to finish under an identity the record never
    // asked for, rather than registering a row the user did not confirm.
    if meta.id != record.profile_id {
        return Err(RegistryError::Row(
            instances::InstanceRowError::InvalidSource(format!(
                "profile.json id {:?} does not match the operation {:?}",
                meta.id, record.profile_id
            )),
        ));
    }
    if std::path::Path::new(&meta.instance_root) != root.as_path() {
        return Err(RegistryError::Row(
            instances::InstanceRowError::InvalidSource(
                "profile.json instance_root does not match the operation target".to_string(),
            ),
        ));
    }
    let row = InstanceRow {
        id: record.profile_id.clone(),
        name: meta.name.clone(),
        path: root,
        minecraft_root: PathBuf::from(&meta.minecraft_root),
        minecraft_version: meta.minecraft_version.clone(),
        loader: meta.loader.clone(),
        source: instances::InstanceSource::Managed,
        created_at: meta.created_at,
        scanned_at: None,
    };
    instances::insert(db, &row)?;
    instances::get(db, &row.id)
        .map_err(RegistryError::Row)?
        .ok_or(RegistryError::Row(
            instances::InstanceRowError::DuplicatePath(row.path),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{instances::InstanceSource, open, Db};
    use crate::instances::registry;

    fn db(dir: &std::path::Path) -> Db {
        open(&dir.join("t.db")).expect("open")
    }

    fn journal_in(_dir: &std::path::Path) -> (Journal, tempfile::TempDir) {
        let holder = crate::test_tempdir();
        (Journal::new(holder.path().join("operations")), holder)
    }

    /// A create that reached the folder step, simulated by writing the record
    /// and stopping there — the state a crash leaves behind.
    fn crash_after_dirs(db: &Db, j: &Journal, root: &std::path::Path, id: &str) -> CreateRecovery {
        std::fs::create_dir_all(root).expect("mkdir");
        for d in registry::STANDARD_DIRS {
            std::fs::create_dir(root.join(d)).expect("mkdir child");
        }
        let rec = j
            .start(
                OperationKind::InstanceCreate,
                id,
                &root.to_string_lossy(),
                vec![],
            )
            .expect("start");
        j.transition_operation(&rec.operation_id, OperationState::Running, None)
            .expect("running");
        let record = j.read(&rec.operation_id).expect("read");
        assess_create(db, &record)
    }

    #[test]
    fn spec_f146_mirror_present_offers_finish_not_delete() {
        let dir = crate::test_tempdir();
        let d = db(dir.path());
        let (j, _holder) = journal_in(dir.path());
        let root = dir.path().join("Survival");

        // Reproduce the "mirror written, row not inserted" crash exactly.
        let row = InstanceRow {
            id: "i1".into(),
            name: "Survival".into(),
            path: root.clone(),
            minecraft_root: PathBuf::new(),
            minecraft_version: Some("1.21.1".into()),
            loader: Some("fabric".into()),
            source: InstanceSource::Managed,
            created_at: 5,
            scanned_at: None,
        };
        std::fs::create_dir_all(&root).expect("mkdir");
        metadata::write_mirror(&root, &metadata::mirror_of(&row)).expect("mirror");

        let rec = j
            .start(
                OperationKind::InstanceCreate,
                "i1",
                &root.to_string_lossy(),
                vec![],
            )
            .expect("start");
        j.transition_operation(&rec.operation_id, OperationState::Running, None)
            .expect("running");
        let record = j.read(&rec.operation_id).expect("read");

        let assessment = assess_create(&d, &record);
        assert_eq!(assessment.action, RecoveryAction::Finish);
        assert_eq!(assessment.reason, RecoveryReason::MirrorPresent);

        let finished = finish_create(&d, &record).expect("finish");
        assert_eq!(finished.name, "Survival");
        assert_eq!(instances::list(&d).expect("list").len(), 1);
    }

    #[test]
    fn spec_f146_directories_without_mirror_offer_removal_of_empty_children_only() {
        let dir = crate::test_tempdir();
        let d = db(dir.path());
        let (j, _holder) = journal_in(dir.path());
        let root = dir.path().join("Survival");

        let assessment = crash_after_dirs(&d, &j, &root, "i1");
        assert_eq!(assessment.action, RecoveryAction::RemoveEmptyChildren);
        assert_eq!(assessment.reason, RecoveryReason::DirectoriesWithoutMirror);

        // A child that gained content must survive.
        std::fs::write(root.join("mods").join("Sodium.jar"), b"x").expect("write");
        let removed = remove_empty_children(&root).expect("remove");
        assert_eq!(removed, 1, "only the empty config");
        assert!(
            root.join("mods").is_dir(),
            "the folder with content survives"
        );
        assert!(root.is_dir(), "the root is never removed");
    }

    #[test]
    fn spec_f146_crash_before_the_first_directory_needs_nothing() {
        let dir = crate::test_tempdir();
        let d = db(dir.path());
        let (j, _holder) = journal_in(dir.path());
        let root = dir.path().join("NeverMade");

        let rec = j
            .start(
                OperationKind::InstanceCreate,
                "i1",
                &root.to_string_lossy(),
                vec![],
            )
            .expect("start");
        let record = j.read(&rec.operation_id).expect("read");

        let assessment = assess_create(&d, &record);
        assert_eq!(assessment.action, RecoveryAction::Leave);
        assert_eq!(assessment.reason, RecoveryReason::NothingWritten);
    }

    #[test]
    fn spec_f146_an_unreadable_mirror_is_never_offered_as_finishable() {
        let dir = crate::test_tempdir();
        let d = db(dir.path());
        let (j, _holder) = journal_in(dir.path());
        let root = dir.path().join("Survival");
        std::fs::create_dir_all(&root).expect("mkdir");
        std::fs::write(metadata::mirror_path(&root), b"{ truncated").expect("write");

        let rec = j
            .start(
                OperationKind::InstanceCreate,
                "i1",
                &root.to_string_lossy(),
                vec![],
            )
            .expect("start");
        let record = j.read(&rec.operation_id).expect("read");

        let assessment = assess_create(&d, &record);
        assert_eq!(assessment.reason, RecoveryReason::MirrorUnreadable);
        assert_eq!(assessment.action, RecoveryAction::RemoveEmptyChildren);
        assert!(
            finish_create(&d, &record).is_err(),
            "cannot finish from garbage"
        );
    }

    #[test]
    fn spec_f146_a_finished_create_is_left_alone() {
        let dir = crate::test_tempdir();
        let d = db(dir.path());
        let (j, _holder) = journal_in(dir.path());
        let root = dir.path().join("Survival");
        std::fs::create_dir_all(&root).expect("mkdir");

        // Row exists, record still says Running: the create finished and only
        // the record lagged behind.
        instances::insert(
            &d,
            &InstanceRow {
                id: "i1".into(),
                name: "Survival".into(),
                path: std::fs::canonicalize(&root).expect("canon"),
                minecraft_root: PathBuf::new(),
                minecraft_version: None,
                loader: None,
                source: InstanceSource::Managed,
                created_at: 1,
                scanned_at: None,
            },
        )
        .expect("insert");

        let rec = j
            .start(
                OperationKind::InstanceCreate,
                "i1",
                &root.to_string_lossy(),
                vec![],
            )
            .expect("start");
        let record = j.read(&rec.operation_id).expect("read");

        let assessment = assess_create(&d, &record);
        assert_eq!(assessment.reason, RecoveryReason::AlreadyRegistered);
        assert_eq!(assessment.action, RecoveryAction::Leave);
    }

    #[test]
    fn spec_f146_a_non_create_record_is_never_assessed() {
        let dir = crate::test_tempdir();
        let d = db(dir.path());
        let (j, _holder) = journal_in(dir.path());
        let rec = j
            .start(OperationKind::DownloadBatch, "p1", "target", vec![])
            .expect("start");
        let record = j.read(&rec.operation_id).expect("read");
        let assessment = assess_create(&d, &record);
        assert_eq!(assessment.reason, RecoveryReason::NotApplicable);
        assert!(pending_creates(&d, &j).is_empty(), "not listed as pending");
    }

    #[test]
    fn spec_f141_instance_create_record_has_no_items_and_zero_counters() {
        let dir = crate::test_tempdir();
        let (j, _holder) = journal_in(dir.path());
        let rec = j
            .start(OperationKind::InstanceCreate, "i1", "root", vec![])
            .expect("start");
        assert!(rec.items.is_empty());
        assert_eq!(rec.planned_count, 0);
        assert_eq!(rec.completed_count, 0);
        assert_eq!(rec.staging_path, "");
        assert_eq!(rec.backup_path, "");

        // Round trip through the strict decoder.
        let back = j.read(&rec.operation_id).expect("read");
        assert_eq!(back.kind, OperationKind::InstanceCreate);
        assert!(back.items.is_empty());

        // And discover() sees it, with no issue.
        let d = db(dir.path());
        let pending = j.discover();
        assert_eq!(
            pending.issues().count(),
            0,
            "{:?}",
            pending.issues().collect::<Vec<_>>()
        );
        assert_eq!(pending_creates(&d, &j).len(), 1);
    }
}
