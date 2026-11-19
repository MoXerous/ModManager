//! F-145: the read-only operations view over IPC.
//!
//! The commands are thin — the records, the assessment and the reason keys are
//! core's — so what is tested here is the bridge's own promises: every kind is
//! listed with the word the UI branches on, an arbitrary id is assessed with
//! the action of the surface that already owns its kind, and a stale row is a
//! typed `notFound` instead of an `io` failure the user cannot act on.

use std::path::Path;

use mm_core::journal::{Journal, OperationKind, OperationState, PackApplyMeta};

use crate::dto::ErrorCode;
use crate::state::AppState;

use super::operations::{assess_operation_with, list_operations_with};

/// The journal store the state itself reads, addressed without holding its
/// mutex — the same alias `pack.rs` builds for long-running work.
fn journal_of(state: &AppState) -> Journal {
    let handle = state.journal_handle();
    let root = handle.lock().expect("journal lock").root().to_path_buf();
    Journal::new(root)
}

fn state_over(dir: &Path) -> AppState {
    AppState::from_db(mm_core::db::open(&dir.join("registry.db")).expect("open db"))
}

fn pack_meta(dir: &Path) -> PackApplyMeta {
    PackApplyMeta {
        pack_name: "The Pack".to_string(),
        pack_version_id: "v1".to_string(),
        archive_sha512: "a".repeat(128),
        archive_path: dir.join("the-pack.mrpack").to_string_lossy().into_owned(),
    }
}

/// F-145: one command lists the journal's records ACROSS kinds. A download
/// batch — the kind no recovery surface was ever built for — stands in the
/// same list as a create and a pack apply, each carrying the typed word the UI
/// groups by and, when unfinished, the reason the surface that owns it would
/// give.
#[tokio::test]
async fn spec_f145_lists_operations_across_kinds_over_ipc() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = state_over(dir.path());
    let journal = journal_of(&state);
    let downloads = dir.path().join("downloads");
    let instance = dir.path().join("instance");
    std::fs::create_dir_all(&downloads).expect("mkdir downloads");

    let batch = journal
        .start(
            OperationKind::DownloadBatch,
            "prof-1",
            &downloads.to_string_lossy(),
            vec![],
        )
        .expect("start batch")
        .operation_id;
    let create = journal
        .start(
            OperationKind::InstanceCreate,
            "i9",
            &instance.to_string_lossy(),
            vec![],
        )
        .expect("start create")
        .operation_id;
    let pack = journal
        .start_pack(
            "i1",
            &instance.to_string_lossy(),
            vec![],
            pack_meta(dir.path()),
        )
        .expect("start pack")
        .operation_id;

    let listing = list_operations_with(&state).await.expect("list");
    assert!(
        listing.issues.is_empty(),
        "a healthy journal reports no issues: {listing:?}"
    );
    let rows = listing.rows;

    assert_eq!(
        rows.len(),
        3,
        "every record is listed, not only recoverable ones"
    );
    let kinds: Vec<&str> = rows.iter().map(|r| r.kind.as_str()).collect();
    for kind in ["download_batch", "instance_create", "mrpack_install"] {
        assert!(kinds.contains(&kind), "{kind} is missing from {kinds:?}");
    }
    assert!(
        rows.iter().all(|r| r.state == "planned"),
        "a fresh record reads as `planned`: {:?}",
        rows.iter().map(|r| r.state.as_str()).collect::<Vec<_>>()
    );

    let batch_row = rows
        .iter()
        .find(|r| r.operation_id == batch)
        .expect("the batch is in the list");
    assert_eq!(batch_row.kind, "download_batch");
    assert_eq!(batch_row.profile_id, "prof-1");
    assert_eq!(batch_row.target_path, downloads.to_string_lossy());
    assert_eq!(batch_row.planned_count, 0);
    assert_eq!(batch_row.pack_name, "", "a batch carries no pack name");
    assert_eq!(
        batch_row.reason, "operations.reason.interrupted",
        "a kind with no recovery surface says it was interrupted: {batch_row:?}"
    );

    let create_row = rows
        .iter()
        .find(|r| r.operation_id == create)
        .expect("the create is in the list");
    assert_eq!(
        create_row.reason, "operations.reason.interrupted",
        "the list states only that the record never finished, not the create's exact reason"
    );

    let pack_row = rows
        .iter()
        .find(|r| r.operation_id == pack)
        .expect("the pack apply is in the list");
    assert_eq!(pack_row.pack_name, "The Pack");
    assert_eq!(
        pack_row.reason, "operations.reason.interrupted",
        "the list did not hash the pack archive to say it"
    );
}

/// F-145: the assessor takes an arbitrary operation id and answers with the
/// action the owning surface already offers — `recovery.instance.action.*` for
/// a create, nothing at all for a batch — plus what the disk holds. Nothing is
/// written: the record is still `planned` afterwards.
#[tokio::test]
async fn spec_f145_assess_answers_with_the_surface_that_already_owns_the_kind() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = state_over(dir.path());
    let journal = journal_of(&state);
    let downloads = dir.path().join("downloads");
    let instance = dir.path().join("instance");
    std::fs::create_dir_all(&downloads).expect("mkdir downloads");
    // A folder with no mirror: the Instances banner's own offering.
    std::fs::create_dir_all(instance.join("mods")).expect("mkdir instance/mods");
    std::fs::write(downloads.join("landed.jar"), b"complete").expect("write final");

    let create = journal
        .start(
            OperationKind::InstanceCreate,
            "i9",
            &instance.to_string_lossy(),
            vec![],
        )
        .expect("start create")
        .operation_id;
    let batch = journal
        .start(
            OperationKind::DownloadBatch,
            "prof-1",
            &downloads.to_string_lossy(),
            vec![],
        )
        .expect("start batch")
        .operation_id;
    journal
        .replace_items(
            &batch,
            vec![mm_core::journal::OperationItem {
                item_id: "landed.jar".to_string(),
                project_id: String::new(),
                title: "Landed".to_string(),
                filename: "landed.jar".to_string(),
                state: mm_core::journal::ItemState::Queued,
                partial_path: downloads
                    .join("landed.jar.part")
                    .to_string_lossy()
                    .into_owned(),
                final_path: downloads.join("landed.jar").to_string_lossy().into_owned(),
                message: String::new(),
            }],
            None,
        )
        .expect("items");

    let assessed = assess_operation_with(&state, create.clone())
        .await
        .expect("assess create");
    assert_eq!(assessed.kind, "instance_create");
    assert_eq!(assessed.state, "planned");
    assert_eq!(
        assessed.action, "recovery.instance.action.removeEmptyChildren",
        "the action is the one the Instances banner already renders: {assessed:?}"
    );
    assert_eq!(
        assessed.reason,
        "recovery.instance.directoriesWithoutMirror"
    );
    assert_eq!(assessed.target_path, instance.to_string_lossy());
    assert!(!assessed.staging_present);
    assert!(!assessed.backup_present);

    let assessed = assess_operation_with(&state, batch)
        .await
        .expect("assess batch");
    assert_eq!(assessed.kind, "download_batch");
    assert_eq!(assessed.item_count, 1, "the count is the record's items");
    assert_eq!(
        assessed.final_present, 1,
        "the disk, not the counters, is the evidence: {assessed:?}"
    );
    assert_eq!(assessed.part_present, 0);
    assert_eq!(
        assessed.action, "",
        "a batch has no surface, so the view invents no mutation: {assessed:?}"
    );
    assert_eq!(assessed.reason, "operations.reason.interrupted");

    // Read-only: the create record is exactly where it was.
    let record = journal.read(&create).expect("read back");
    assert_eq!(record.state, OperationState::Planned);
}

/// F-145: an assess request is answered by code, not by a raw string. A row
/// that vanished between the read and the click is `notFound` (refresh the
/// table), and an id shaped like a path is `invalidInput` — refused before it
/// could name a file outside the journal.
#[tokio::test]
async fn spec_f145_assess_refuses_a_stale_row_and_a_path_shaped_id_by_code() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = state_over(dir.path());

    let stale = assess_operation_with(&state, "00000000-0000-4000-8000-000000000000".to_string())
        .await
        .expect_err("no such record");
    assert_eq!(stale.code, ErrorCode::NotFound, "{stale:?}");

    for id in [r"..\..\outside", "../outside", "not-an-operation"] {
        let err = assess_operation_with(&state, id.to_string())
            .await
            .expect_err("not an operation id");
        assert_eq!(err.code, ErrorCode::InvalidInput, "{id:?} gave {err:?}");
    }
}

/// F-145: a journal that genuinely cannot be read is `io`, not `notFound` and
/// not a generic internal error. A directory where the record's `.json` should
/// be is the case the user can do nothing about by refreshing, so the bridge
/// must say so with its own typed code.
#[tokio::test]
async fn spec_f145_a_journal_failure_keeps_its_own_code_over_ipc() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = state_over(dir.path());
    let journal = journal_of(&state);
    let id = "22222222-2222-4222-8222-222222222222";
    std::fs::create_dir_all(journal.root().join(format!("{id}.json"))).expect("mkdir record path");

    let err = assess_operation_with(&state, id.to_string())
        .await
        .expect_err("a directory is not a record");

    assert_eq!(err.code, ErrorCode::Io, "{err:?}");
}

/// F-145: a record the scan cannot read is reported as an issue, not dropped:
/// the old centre surfaced unreadable records, and the DTO carries them so the
/// screen can too. Nothing about the file is changed.
#[tokio::test]
async fn spec_f145_a_corrupt_record_is_reported_as_an_issue_over_ipc() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = state_over(dir.path());
    let journal = journal_of(&state);
    journal
        .start(
            OperationKind::DownloadBatch,
            "prof-1",
            &dir.path().to_string_lossy(),
            vec![],
        )
        .expect("start batch");
    let corrupt_id = "11111111-1111-4111-8111-111111111111".to_string();
    let corrupt = journal.root().join(format!("{corrupt_id}.json"));
    std::fs::write(&corrupt, b"{ not a record").expect("write corrupt");
    let before = std::fs::read(&corrupt).expect("read corrupt");

    let listing = list_operations_with(&state).await.expect("list");

    assert_eq!(listing.rows.len(), 1, "{listing:?}");
    assert_eq!(listing.issues.len(), 1, "{listing:?}");
    assert_eq!(
        listing.issues[0].filename,
        format!("{corrupt_id}.json"),
        "the issue names the file, basename only: {listing:?}"
    );
    assert!(
        !listing.issues[0].message.is_empty(),
        "the issue says why: {listing:?}"
    );
    assert_eq!(
        std::fs::read(&corrupt).expect("read corrupt"),
        before,
        "the listing rewrote the unreadable record"
    );
}
