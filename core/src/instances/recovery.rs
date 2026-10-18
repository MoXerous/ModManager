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

/// Moves a record that needs no user choice into the terminal state its
/// assessment already implies. Returns whether anything was settled.
///
/// `Leave` means "do nothing", and for two of its three reasons nothing does
/// need doing — yet the record stays `Planned`/`Running`, and
/// `discover().unfinished()` is exactly what re-offers it. So without this,
/// every crashed create is presented again on every launch, forever.
///
/// `AlreadyRegistered` settles as `Completed` and `NothingWritten` as
/// `Failed`. Both are only reachable from `Running`, which is the sole state
/// the insert path can be in, so the state machine accepts the move; if that
/// ever stops holding, `transition_operation` rejects it rather than silently
/// settling a record wrongly.
///
/// shortcut: this trusts the on-disk and database inspection rather than
/// holding an exclusive lock on the journal, so a second running instance
/// could in principle settle a record that its own process is mid-create.
/// Guard with a journal lock when concurrent app instances are possible.
pub fn settle_create(
    journal: &Journal,
    record: &OperationRecord,
    reason: RecoveryReason,
) -> Result<bool, crate::journal::JournalError> {
    let to = match reason {
        // The instance is registered: the create did finish, and only the
        // record lagged behind. Failed would be a lie the user would read.
        RecoveryReason::AlreadyRegistered => OperationState::Completed,
        // The folder was never created, so there is nothing to finish, to
        // clean up, or to choose between.
        RecoveryReason::NothingWritten => OperationState::Failed,
        // Already terminal, not a create, or a genuine choice for the user.
        _ => return Ok(false),
    };
    journal.transition_operation(&record.operation_id, to, None)?;
    Ok(true)
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
    fn spec_f146_recovery_converges_so_a_crash_is_not_offered_forever() {
        // The defect this pins: nothing ever terminated a create record, and
        // `discover().unfinished()` re-offers exactly those. So every crashed
        // create came back on every launch, for as long as the record lived.
        let dir = crate::test_tempdir();
        let d = db(dir.path());
        let (j, _holder) = journal_in(dir.path());

        // Two crashes that need no choice: one whose instance is already
        // registered, one that died before creating anything.
        let done_root = dir.path().join("Done");
        let lost_root = dir.path().join("Lost");

        let finished = j
            .start(
                OperationKind::InstanceCreate,
                "done",
                &done_root.to_string_lossy(),
                vec![],
            )
            .expect("start")
            .operation_id;
        j.transition_operation(&finished, OperationState::Running, None)
            .expect("running");
        std::fs::create_dir_all(&done_root).expect("mkdir");
        instances::insert(
            &d,
            &InstanceRow {
                id: "done".into(),
                name: "Done".into(),
                path: std::fs::canonicalize(dir.path())
                    .expect("canon")
                    .join("Done"),
                minecraft_root: PathBuf::new(),
                minecraft_version: None,
                loader: None,
                source: InstanceSource::Managed,
                created_at: 1,
                scanned_at: None,
            },
        )
        .expect("insert");

        let lost = j
            .start(
                OperationKind::InstanceCreate,
                "lost",
                &lost_root.to_string_lossy(),
                vec![],
            )
            .expect("start")
            .operation_id;
        j.transition_operation(&lost, OperationState::Running, None)
            .expect("running");

        // Launch one: both are assessed, and both are settled.
        let first = pending_creates(&d, &j);
        assert_eq!(first.len(), 2, "both crashes are found");
        for c in &first {
            assert_eq!(c.action, RecoveryAction::Leave, "nothing to choose");
            assert!(
                settle_create(&j, &j.read(&c.operation_id).expect("read"), c.reason)
                    .expect("settle"),
                "{:?} is settleable",
                c.reason
            );
        }

        // Launch two, and every launch after it: nothing is offered again.
        // This is the whole claim — one pass is not convergence, the second
        // empty pass is.
        assert!(
            pending_creates(&d, &j).is_empty(),
            "settled records are still being offered"
        );
        assert_eq!(
            j.read(&finished).expect("read").state,
            OperationState::Completed,
            "a registered instance is not a failure"
        );
        assert_eq!(
            j.read(&lost).expect("read").state,
            OperationState::Failed,
            "nothing was created, so nothing succeeded"
        );
    }

    #[test]
    fn spec_f146_settling_never_touches_a_record_that_needs_a_choice() {
        // The mirror is valid, so the user must be offered Finish. Settling
        // must leave it pending, or recovery would swallow a real decision.
        let dir = crate::test_tempdir();
        let d = db(dir.path());
        let (j, _holder) = journal_in(dir.path());
        let root = dir.path().join("Survival");

        let assessment = crash_after_dirs(&d, &j, &root, "i1");
        assert_eq!(assessment.action, RecoveryAction::RemoveEmptyChildren);

        let record = j.read(&assessment.operation_id).expect("read");
        assert!(!settle_create(&j, &record, assessment.reason).expect("settle"));
        assert_eq!(
            j.read(&assessment.operation_id).expect("read").state,
            OperationState::Running,
            "still pending"
        );
        assert_eq!(pending_creates(&d, &j).len(), 1, "still offered");
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
