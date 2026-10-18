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

impl RecoveryAction {
    /// Stable key for the UI to translate, as [`RecoveryReason::key`].
    pub fn key(self) -> &'static str {
        match self {
            Self::Leave => "recovery.instance.action.leave",
            Self::Finish => "recovery.instance.action.finish",
            Self::RemoveEmptyChildren => "recovery.instance.action.removeEmptyChildren",
        }
    }
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

/// Crashed creates that still need a decision. Settles the rest.
///
/// This is the only production caller of [`settle_create`], and it has to be:
/// a record whose instance is already registered, or whose folder was never
/// created, needs nothing from the user — but without a transition it stays
/// `Planned`/`Running` and `discover().unfinished()` hands it back on every
/// launch, forever. Settling here is what makes recovery converge rather than
/// merely report.
///
/// A failure to settle is deliberately ignored: the record stays pending and is
/// offered again, which is the correct outcome for something we could not
/// record. It is never turned into an error that hides the other items.
///
/// Whatever is left needs the user, so `Leave` is filtered out — every reason
/// `assess_create` reaches it for is now settled above.
pub fn pending_creates(db: &crate::db::Db, journal: &Journal) -> Vec<CreateRecovery> {
    journal
        .discover()
        .unfinished()
        .filter(|r| r.kind == OperationKind::InstanceCreate)
        .filter_map(|r| {
            let assessment = assess_create(db, r);
            let settled = match settle_create(journal, r, assessment.reason) {
                Ok(s) => s,
                Err(e) => {
                    // Logged because a record settled this way leaves no other
                    // trace: no trace is how a bug becomes permanent.
                    tracing::warn!(
                        operation_id = %r.operation_id,
                        error = %e,
                        "recovery: could not settle a create that needed no decision"
                    );
                    false
                }
            };
            // Only a record that both needed nothing and was actually settled
            // may disappear. Failing to settle is not a reason to hide it:
            // offered, it is still there next launch.
            if assessment.action == RecoveryAction::Leave && settled {
                None
            } else {
                Some(assessment)
            }
        })
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

/// Finishes a crashed create and closes its record.
///
/// The caller passes an operation id, never a path: the root comes from the
/// record, so the webview cannot name a folder to act on.
///
/// Closing the record is part of finishing. Without it the assessment stays
/// `MirrorPresent` on every later launch and the finished instance is offered
/// again, this time with a row that already exists.
pub fn finish(
    db: &crate::db::Db,
    journal: &Journal,
    operation_id: &str,
) -> Result<InstanceRow, RegistryError> {
    let record = journal.read(operation_id).map_err(RegistryError::Journal)?;
    require_create(&record)?;
    let row = finish_create(db, &record)?;
    journal
        .transition_operation(operation_id, OperationState::Completed, None)
        .map_err(RegistryError::Journal)?;
    Ok(row)
}

/// `abandon`: removes the standard folders that are still empty, keeps the
/// root, and closes the record so the residue is not offered on every launch.
///
/// `Failed` because that is what the user chose, not because anything broke.
/// The root itself stays — nothing on disk proves we created it, and the user
/// may have put it there.
pub fn abandon(journal: &Journal, operation_id: &str) -> Result<usize, RegistryError> {
    let record = journal.read(operation_id).map_err(RegistryError::Journal)?;
    require_create(&record)?;
    let root = PathBuf::from(&record.target_path);
    let removed = remove_empty_children(root.as_path())?;

    // The mirror goes too, and only when it is *ours*. Leaving it behind is
    // not untidy: `plan_create` refuses any folder holding a profile.json, so
    // the user could never create an instance there again and nothing would
    // ever say why. The id check means a profile.json that is not this
    // operation's — the user's own, or another instance's — is left alone.
    let mirror = metadata::mirror_path(&root);
    if metadata::read_mirror(&mirror).is_ok_and(|m| m.id == record.profile_id) {
        // A mirror we wrote is stale the moment we abandon the create; leaving
        // it would claim an instance that does not exist.
        std::fs::remove_file(&mirror).map_err(RegistryError::Io)?;
    }

    journal
        .transition_operation(operation_id, OperationState::Failed, None)
        .map_err(RegistryError::Journal)?;
    Ok(removed)
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

/// Both mutating entry points are reached from the webview by operation id
/// alone, so each must confirm what kind of record it was handed. Without this,
/// a download batch's id would make `abandon` remove `mods` and `config` under
/// that operation's target path, and delete a `profile.json` whose id happened
/// to match.
fn require_create(record: &OperationRecord) -> Result<(), RegistryError> {
    if record.kind != OperationKind::InstanceCreate {
        return Err(RegistryError::Row(
            instances::InstanceRowError::InvalidSource(format!(
                "operation {:?} is not an instance create",
                record.operation_id
            )),
        ));
    }
    Ok(())
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
    // The mirror is a file the user can edit between the crash and this call,
    // so only `id` and `instance_root` may come from it. `name` is checked
    // with the same rule `plan_create` applies — otherwise recovery is the one
    // path that registers a name the preview would have refused.
    crate::instances::registry::instance_name(&meta.name).map_err(|_| {
        RegistryError::Row(instances::InstanceRowError::InvalidSource(
            "profile.json name is not a safe instance name".to_string(),
        ))
    })?;

    // `minecraft_root` is taken from that same file. `insert` will happily
    // canonicalize any folder on the machine, so without this the row can
    // claim `C:\Windows` as this instance's Minecraft directory.
    let declared = PathBuf::from(&meta.minecraft_root);
    if !declared.starts_with(&root) {
        return Err(RegistryError::Row(
            instances::InstanceRowError::InvalidSource(
                "profile.json minecraft_root is outside the instance".to_string(),
            ),
        ));
    }

    let row = InstanceRow {
        id: record.profile_id.clone(),
        name: meta.name.clone(),
        path: root,
        minecraft_root: declared,
        minecraft_version: meta.minecraft_version.clone(),
        loader: meta.loader.clone(),
        source: instances::InstanceSource::Managed,
        // From the journal, not from the file. The file is editable; the
        // record is ours.
        created_at: record.created_at,
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

    /// Every key core can emit must exist in both locales.
    ///
    /// A missing one does not fail a build or a test anywhere else: the UI
    /// falls back to the key itself and the user reads
    /// `recovery.instance.mirrorUnreadable`. There is no frontend test runner,
    /// so this is the only place the invariant can be checked.
    #[test]
    fn spec_f146_every_emitted_i18n_key_is_translated_in_both_locales() {
        let emitted: Vec<&str> = [
            RecoveryAction::Leave.key(),
            RecoveryAction::Finish.key(),
            RecoveryAction::RemoveEmptyChildren.key(),
        ]
        .into_iter()
        .chain([
            RecoveryReason::NotApplicable.key(),
            RecoveryReason::AlreadyRegistered.key(),
            RecoveryReason::NothingWritten.key(),
            RecoveryReason::MirrorPresent.key(),
            RecoveryReason::MirrorUnreadable.key(),
            RecoveryReason::DirectoriesWithoutMirror.key(),
        ])
        .collect();

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("src")
            .join("i18n");
        for locale in ["ar.json", "en.json"] {
            let text = std::fs::read_to_string(root.join(locale))
                .unwrap_or_else(|e| panic!("cannot read {locale}: {e}"));
            let parsed: serde_json::Value =
                serde_json::from_str(&text).unwrap_or_else(|e| panic!("{locale} is not JSON: {e}"));
            let missing: Vec<&str> = emitted
                .iter()
                .copied()
                .filter(|k| parsed.get(*k).is_none())
                .collect();
            assert!(missing.is_empty(), "{locale} is missing {missing:?}");
        }
    }

    #[test]
    fn spec_f146_finish_refuses_a_mirror_whose_fields_the_user_edited() {
        // The mirror is a file on disk between the crash and the recovery, so
        // anything in it may have been edited. Only `id` and `instance_root`
        // may come from it.
        let dir = crate::test_tempdir();
        let d = db(dir.path());
        let (j, _holder) = journal_in(dir.path());

        for (what, tamper) in [
            (
                "a name the preview would have refused",
                Box::new(|m: &mut metadata::ProfileMetadata| m.name = "CON".into())
                    as Box<dyn Fn(&mut metadata::ProfileMetadata)>,
            ),
            (
                "a minecraft_root outside the instance",
                Box::new(|m: &mut metadata::ProfileMetadata| {
                    m.minecraft_root = dir.path().to_string_lossy().into_owned()
                }),
            ),
        ] {
            let root = dir.path().join(format!("T{}", what.len()));
            std::fs::create_dir_all(&root).expect("mkdir");
            let row = InstanceRow {
                id: "i1".into(),
                name: "Survival".into(),
                path: root.clone(),
                minecraft_root: root.clone(),
                minecraft_version: None,
                loader: None,
                source: InstanceSource::Managed,
                created_at: 5,
                scanned_at: None,
            };
            let mut meta = metadata::mirror_of(&row);
            tamper(&mut meta);
            metadata::write_mirror(&root, &meta).expect("mirror");
            let rec = j
                .start(
                    OperationKind::InstanceCreate,
                    "i1",
                    &root.to_string_lossy(),
                    vec![],
                )
                .expect("start");

            assert!(
                finish_create(&d, &j.read(&rec.operation_id).expect("read")).is_err(),
                "{what} was accepted"
            );
            assert!(
                instances::list(&d).expect("list").is_empty(),
                "{what} registered a row"
            );
        }
    }

    #[test]
    fn spec_f146_abandoning_removes_our_own_mirror_so_the_folder_stays_usable() {
        // Leaving the mirror behind is not untidy: `plan_create` refuses any
        // folder holding a profile.json, so the user could never create an
        // instance there again and nothing would ever say why.
        let dir = crate::test_tempdir();
        let d = db(dir.path());
        let (j, _holder) = journal_in(dir.path());
        let root = dir.path().join("Survival");

        let c = crash_after_dirs(&d, &j, &root, "i1");
        let row = InstanceRow {
            id: "i1".into(),
            name: "Survival".into(),
            path: root.clone(),
            minecraft_root: PathBuf::new(),
            minecraft_version: None,
            loader: None,
            source: InstanceSource::Managed,
            created_at: 5,
            scanned_at: None,
        };
        metadata::write_mirror(&root, &metadata::mirror_of(&row)).expect("mirror");

        abandon(&j, &c.operation_id).expect("abandon");
        assert!(
            !metadata::mirror_path(&root).exists(),
            "our mirror survives and bricks the folder"
        );
        // And the folder is genuinely reusable again, which is the whole point.
        assert!(
            registry::plan_create(&root, "Again", None, None, 9).is_ok(),
            "the folder is still refused after an abandon"
        );
    }

    #[test]
    fn spec_f146_abandoning_leaves_a_mirror_that_is_not_ours_alone() {
        // The id check is what makes deleting the mirror safe: a profile.json
        // belonging to something else is the user's file, not our residue.
        let dir = crate::test_tempdir();
        let d = db(dir.path());
        let (j, _holder) = journal_in(dir.path());
        let root = dir.path().join("SomeoneElses");

        let c = crash_after_dirs(&d, &j, &root, "i1");
        let other = InstanceRow {
            id: "not-our-operation".into(),
            name: "SomeoneElses".into(),
            path: root.clone(),
            minecraft_root: PathBuf::new(),
            minecraft_version: None,
            loader: None,
            source: InstanceSource::Managed,
            created_at: 5,
            scanned_at: None,
        };
        metadata::write_mirror(&root, &metadata::mirror_of(&other)).expect("mirror");

        abandon(&j, &c.operation_id).expect("abandon");
        assert!(
            metadata::mirror_path(&root).is_file(),
            "deleted a mirror that is not ours"
        );
    }

    #[test]
    fn spec_f146_finish_and_abandon_refuse_a_record_that_is_not_a_create() {
        // Both take an operation id from the webview. A download batch's id must
        // not be able to make `abandon` delete folders under its target path.
        let dir = crate::test_tempdir();
        let d = db(dir.path());
        let (j, _holder) = journal_in(dir.path());
        let root = dir.path().join("SomeDownloadsTarget");
        std::fs::create_dir_all(root.join("mods")).expect("mkdir");
        std::fs::write(root.join("config"), b"not a folder").expect("write");

        let rec = j
            .start(
                OperationKind::DownloadBatch,
                "p1",
                &root.to_string_lossy(),
                vec![],
            )
            .expect("start");
        j.transition_operation(&rec.operation_id, OperationState::Running, None)
            .expect("running");

        assert!(
            finish(&d, &j, &rec.operation_id).is_err(),
            "finish accepted it"
        );
        assert!(
            abandon(&j, &rec.operation_id).is_err(),
            "abandon accepted it"
        );
        // Nothing touched.
        assert!(root.join("mods").is_dir(), "mods was removed");
        assert!(root.join("config").is_file(), "a file was removed");
    }

    #[test]
    fn spec_f146_a_create_that_cannot_be_settled_stays_offered() {
        // The failure this pins: a record that needed no decision was dropped
        // whether or not settling it worked, so a failed settle lost it from both
        // ends - not terminal, and never shown again.
        let dir = crate::test_tempdir();
        let d = db(dir.path());
        let (j, _holder) = journal_in(dir.path());
        let root = dir.path().join("AlreadyThere");
        std::fs::create_dir_all(&root).expect("mkdir");

        // The create finished on disk but its record never left `Planned`.
        instances::insert(
            &d,
            &InstanceRow {
                id: "i1".into(),
                name: "AlreadyThere".into(),
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

        // `AlreadyRegistered` wants `Completed`, and `Planned` may not go
        // straight there. This is the real shape of a failed settle.
        assert_eq!(
            assess_create(&d, &record).reason,
            RecoveryReason::AlreadyRegistered
        );
        assert!(settle_create(&j, &record, RecoveryReason::AlreadyRegistered).is_err());

        // Dropping it would lose it from both ends: not terminal, never shown.
        let offered = pending_creates(&d, &j);
        assert_eq!(
            offered.len(),
            1,
            "a create that could not be settled was dropped from both ends"
        );
        assert_eq!(offered[0].operation_id, rec.operation_id);
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

        // One call. Both crashes need no decision, so `pending_creates`
        // settles them and returns nothing. That is the whole claim, and unlike
        // the version that settled by hand, it is what production does.
        assert!(
            pending_creates(&d, &j).is_empty(),
            "a create needing no decision was offered to the user"
        );
        assert!(
            pending_creates(&d, &j).is_empty(),
            "a settled record came back"
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
    fn spec_f146_finishing_registers_the_instance_and_stops_offering_it() {
        let dir = crate::test_tempdir();
        let d = db(dir.path());
        let (j, _holder) = journal_in(dir.path());
        let root = dir.path().join("Survival");

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
        for sd in registry::STANDARD_DIRS {
            std::fs::create_dir(root.join(sd)).expect("mkdir child");
        }
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

        // The whole point: after finishing, the record is terminal. If it were
        // left Running it would be offered again — and now with a row that
        // already exists, which reads to the user as a second instance.
        let finished = finish(&d, &j, &rec.operation_id).expect("finish");
        assert_eq!(finished.id, "i1");
        assert_eq!(instances::list(&d).expect("list").len(), 1);
        assert_eq!(
            j.read(&rec.operation_id).expect("read").state,
            OperationState::Completed
        );
        assert!(
            pending_creates(&d, &j).is_empty(),
            "a finished create is still being offered"
        );
    }

    #[test]
    fn spec_f146_abandoning_removes_the_empty_children_and_closes_the_record() {
        let dir = crate::test_tempdir();
        let d = db(dir.path());
        let (j, _holder) = journal_in(dir.path());
        let root = dir.path().join("Survival");

        let c = crash_after_dirs(&d, &j, &root, "i1");
        assert_eq!(c.action, RecoveryAction::RemoveEmptyChildren);
        // `mods` gained content, so only `config` may go.
        std::fs::write(root.join("mods").join("Sodium.jar"), b"x").expect("write");

        let removed = abandon(&j, &c.operation_id).expect("abandon");
        assert_eq!(removed, 1, "only the empty directory");
        assert!(!root.join("config").exists(), "the empty one is gone");
        assert!(root.join("mods").is_dir(), "content survives");
        assert!(root.is_dir(), "the root is never removed");

        assert_eq!(
            j.read(&c.operation_id).expect("read").state,
            OperationState::Failed
        );
        assert!(
            pending_creates(&d, &j).is_empty(),
            "an abandoned create is still being offered"
        );
    }

    #[test]
    fn spec_f146_recovery_takes_an_operation_id_and_never_a_path() {
        // The webview is untrusted input (commands.rs header). `finish` and
        // `abandon` therefore derive the root from the record; if they ever
        // accepted a path, a caller could name any folder on the machine.
        let dir = crate::test_tempdir();
        let d = db(dir.path());
        let (j, _holder) = journal_in(dir.path());

        // An id that names no record: refused by the journal, never a path.
        assert!(matches!(
            finish(&d, &j, "not-an-operation").unwrap_err(),
            RegistryError::Journal(_)
        ));
        assert!(matches!(
            abandon(&j, "not-an-operation").unwrap_err(),
            RegistryError::Journal(_)
        ));
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
        // A bare record with no folder is settled as `Failed` by
        // `pending_creates` — it needs no decision — so the user is offered
        // nothing, while the strict decoder still round-tripped it intact.
        assert!(
            pending_creates(&d, &j).is_empty(),
            "a record with nothing written should not be offered"
        );
        assert_eq!(
            j.read(&rec.operation_id).expect("read").state,
            OperationState::Failed
        );
    }
}
