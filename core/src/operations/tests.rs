//! F-145: the unified read-only operations view, driven for real.
//!
//! Every test drives a real journal over a temp directory and a real
//! database; the disk evidence is real files. The one rule above all: the
//! listing and the assessment write NOTHING.

use std::path::Path;

use super::*;
use crate::db;
use crate::instances::metadata::{self, ProfileMetadata, PROFILE_SCHEMA};
use crate::journal::{
    ItemState, Journal, OperationItem, OperationKind, OperationRecord, OperationState,
    PackApplyMeta,
};

fn item(final_path: &Path, part_path: &Path, state: ItemState) -> OperationItem {
    OperationItem {
        item_id: final_path.to_string_lossy().into_owned(),
        project_id: String::new(),
        title: "a mod".to_string(),
        filename: final_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        state,
        partial_path: part_path.to_string_lossy().into_owned(),
        final_path: final_path.to_string_lossy().into_owned(),
        message: String::new(),
    }
}

fn db_at(dir: &Path) -> db::Db {
    db::open(&dir.join("registry.db")).expect("open db")
}

/// A batch record with the item states the record carries, real on disk.
fn started_batch(journal: &Journal, root: &Path, items: Vec<OperationItem>) -> String {
    journal
        .start(
            OperationKind::DownloadBatch,
            "prof-1",
            &root.to_string_lossy(),
            items,
        )
        .expect("start batch")
        .operation_id
}

fn pack_meta(archive: &Path) -> PackApplyMeta {
    PackApplyMeta {
        pack_name: "The Pack".to_string(),
        pack_version_id: "v1".to_string(),
        archive_sha512: "a".repeat(128),
        archive_path: archive.to_string_lossy().into_owned(),
    }
}

/// Every file below `dir`, relative, sorted — the write-nothing proof.
fn snapshot(dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, out: &mut Vec<String>) {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let rel = e
                    .path()
                    .strip_prefix(dir)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| e.path().to_string_lossy().into_owned());
                out.push(rel);
                if e.path().is_dir() {
                    walk(&e.path(), out);
                }
            }
        }
        out.sort();
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out
}

/// F-145: one command lists the journal's records ACROSS kinds — a download
/// batch, an instance create and a pack apply stand side by side in one
/// table, each with kind, state, times, counters, owner and, when unfinished,
/// a reason key. Today no IPC surface says anything about a batch record; a
/// list that keeps them out is the gap this closes.
#[test]
fn spec_f145_lists_operations_across_kinds() {
    let dir = crate::test_tempdir();
    let journal = Journal::new(dir.path().join("operations"));
    let db = db_at(dir.path());

    // A finished batch: one item completed, then the operation completed.
    let downloads = dir.path().join("downloads");
    let done_id = {
        let id = started_batch(
            &journal,
            &downloads,
            vec![item(
                &downloads.join("mod-a.jar"),
                &downloads.join("mod-a.jar.part"),
                ItemState::Queued,
            )],
        );
        journal
            .transition_items(
                &id,
                &[
                    (
                        downloads.join("mod-a.jar").to_string_lossy().into_owned(),
                        ItemState::Downloading,
                    ),
                    (
                        downloads.join("mod-a.jar").to_string_lossy().into_owned(),
                        ItemState::Verifying,
                    ),
                    (
                        downloads.join("mod-a.jar").to_string_lossy().into_owned(),
                        ItemState::Committing,
                    ),
                    (
                        downloads.join("mod-a.jar").to_string_lossy().into_owned(),
                        ItemState::Completed,
                    ),
                ],
                None,
            )
            .expect("item completes");
        journal
            .transition_operation(&id, OperationState::Running, None)
            .expect("run");
        journal
            .transition_operation(&id, OperationState::Completed, None)
            .expect("complete");
        id
    };

    // A running batch with an older clock, written by hand the way the store
    // itself writes records: full record, valid shape.
    let running_id = {
        let mut r = OperationRecord::new(OperationKind::DownloadBatch, "prof-1", "D:\\dl", vec![]);
        r.updated_at = 1;
        r.created_at = 1;
        std::fs::write(
            journal.root().join(format!("{}.json", r.operation_id)),
            serde_json::to_string_pretty(&r).expect("serialize") + "\n",
        )
        .expect("write record");
        r.operation_id
    };

    // A crashed create and a crashed pack apply, the kinds the per-kind
    // surfaces already know.
    let create_id = journal
        .start(
            OperationKind::InstanceCreate,
            "i9",
            &dir.path().join("new-instance").to_string_lossy(),
            vec![],
        )
        .expect("start create")
        .operation_id;
    let pack_id = journal
        .start_pack(
            "i1",
            &dir.path().join("instance").to_string_lossy(),
            vec![],
            pack_meta(&dir.path().join("the-pack.mrpack")),
        )
        .expect("start pack")
        .operation_id;

    let rows = list_operations(&db, &journal);

    assert_eq!(rows.len(), 4, "every record the journal holds is listed");
    let kinds: Vec<OperationKind> = rows.iter().map(|r| r.kind).collect();
    assert!(
        kinds.contains(&OperationKind::DownloadBatch)
            && kinds.contains(&OperationKind::InstanceCreate)
            && kinds.contains(&OperationKind::MrpackInstall),
        "the three kinds are all present: {kinds:?}"
    );

    // The finished batch: terminal, so no reason key, and the counters it
    // recorded.
    let done = rows
        .iter()
        .find(|r| r.operation_id == done_id)
        .expect("done row");
    assert_eq!(done.state, OperationState::Completed);
    assert_eq!(done.completed_count, 1, "{done:?}");
    assert_eq!(done.planned_count, 1, "{done:?}");
    assert_eq!(done.reason, "", "a terminal record needs no reason");

    // The unfinished ones carry a reason key, not a sentence.
    let running = rows
        .iter()
        .find(|r| r.operation_id == running_id)
        .expect("running row");
    assert_eq!(running.state, OperationState::Planned);
    assert_eq!(
        running.reason, "operations.reason.interrupted",
        "a batch with no recovery surface gets the generic reason: {running:?}"
    );
    let create = rows
        .iter()
        .find(|r| r.operation_id == create_id)
        .expect("create row");
    assert_eq!(
        create.reason, "recovery.instance.nothingWritten",
        "the create's reason comes from the create assessment itself: {create:?}"
    );
    let pack = rows
        .iter()
        .find(|r| r.operation_id == pack_id)
        .expect("pack row");
    assert_eq!(pack.pack_name, "The Pack", "{pack:?}");
    assert_eq!(
        pack.reason, "recovery.pack.instanceMissing",
        "the pack's reason comes from the pack assessment itself: {pack:?}"
    );

    // Newest first: the hand-written record from epoch 1 sinks to the end.
    assert_eq!(
        rows.last().map(|r| r.operation_id.as_str()),
        Some(running_id.as_str())
    );
}

/// F-145: the assessor for an arbitrary record reuses the create assessment —
/// a crashed create with a readable mirror is offered `finish`, exactly as
/// the Instances banner would offer it, not a second opinion.
#[test]
fn spec_f145_assesses_a_create_record_through_the_existing_create_logic() {
    let dir = crate::test_tempdir();
    let journal = Journal::new(dir.path().join("operations"));
    let db = db_at(dir.path());
    let root = dir.path().join("instance");
    std::fs::create_dir_all(&root).expect("mkdir");

    metadata::write_mirror(
        &root,
        &ProfileMetadata {
            schema: PROFILE_SCHEMA.to_string(),
            id: "i9".to_string(),
            name: "Nine".to_string(),
            minecraft_version: None,
            loader: None,
            source: "managed".to_string(),
            instance_root: root.to_string_lossy().into_owned(),
            minecraft_root: String::new(),
            created_at: 1,
        },
    )
    .expect("write mirror");

    let id = journal
        .start(
            OperationKind::InstanceCreate,
            "i9",
            &root.to_string_lossy(),
            vec![],
        )
        .expect("start create")
        .operation_id;

    let before = snapshot(dir.path());
    let assessed = assess_operation(&db, &journal, &id).expect("assess");
    let after = snapshot(dir.path());

    assert_eq!(assessed.kind, OperationKind::InstanceCreate);
    assert_eq!(
        assessed.action, "recovery.instance.action.finish",
        "the action is the one the per-kind surface already offers: {assessed:?}"
    );
    assert_eq!(assessed.reason, "recovery.instance.mirrorPresent");
    assert_eq!(before, after, "the assessment wrote something");
}

/// F-145: the assessor reuses the pack assessment the same way — a pack apply
/// whose instance is gone is reported `instanceMissing`, not reassessed by a
/// second implementation.
#[test]
fn spec_f145_assesses_a_pack_apply_record_through_the_existing_pack_logic() {
    let dir = crate::test_tempdir();
    let journal = Journal::new(dir.path().join("operations"));
    let db = db_at(dir.path());

    let id = journal
        .start_pack(
            "i1",
            &dir.path().join("gone-instance").to_string_lossy(),
            vec![],
            pack_meta(&dir.path().join("the-pack.mrpack")),
        )
        .expect("start pack")
        .operation_id;

    let before = snapshot(dir.path());
    let assessed = assess_operation(&db, &journal, &id).expect("assess");
    let after = snapshot(dir.path());

    assert_eq!(assessed.kind, OperationKind::MrpackInstall);
    assert_eq!(
        assessed.reason, "recovery.pack.instanceMissing",
        "the pack assessment's own answer: {assessed:?}"
    );
    assert_eq!(
        assessed.action, "recovery.pack.action.dismiss",
        "the action key the Modpack banner already renders for a missing instance: {assessed:?}"
    );
    assert_eq!(before, after, "the assessment wrote something");
}

/// F-145: a download batch — the kind no recovery surface exists for — is
/// answered from the disk alone: which final destinations landed, which
/// `.part` files remain, and no action, because the view may not invent a
/// mutation no existing surface supports.
#[test]
fn spec_f145_assesses_a_download_batch_from_the_disk_without_writing() {
    let dir = crate::test_tempdir();
    let journal = Journal::new(dir.path().join("operations"));
    let db = db_at(dir.path());
    let target = dir.path().join("downloads");
    std::fs::create_dir_all(&target).expect("mkdir");

    let landed = target.join("landed.jar");
    std::fs::write(&landed, b"complete").expect("write final");
    let partial = target.join("partial.jar.part");
    std::fs::write(&partial, b"partial").expect("write part");

    let items = vec![
        item(
            &landed,
            &target.join("landed.jar.part"),
            ItemState::Committing,
        ),
        item(
            &target.join("partial.jar"),
            &partial,
            ItemState::Downloading,
        ),
        item(
            &target.join("missing.jar"),
            &target.join("missing.jar.part"),
            ItemState::Queued,
        ),
    ];
    let id = started_batch(&journal, &target, items);
    journal
        .transition_operation(&id, OperationState::Running, None)
        .expect("run");

    let before = snapshot(dir.path());
    let assessed = assess_operation(&db, &journal, &id).expect("assess");
    let after = snapshot(dir.path());

    assert_eq!(assessed.kind, OperationKind::DownloadBatch);
    assert_eq!(assessed.item_count, 3);
    assert_eq!(
        assessed.final_present, 1,
        "the disk, not the record's counters, is the evidence: {assessed:?}"
    );
    assert_eq!(assessed.part_present, 1, "{assessed:?}");
    assert!(!assessed.staging_present, "no staging was recorded");
    assert!(!assessed.backup_present, "no backup was recorded");
    assert_eq!(assessed.reason, "operations.reason.interrupted");
    assert_eq!(
        assessed.action, "",
        "a batch has no recovery surface, so the view invents none: {assessed:?}"
    );
    assert_eq!(before, after, "the assessment wrote something");
}

/// F-145: the one key this module emits for kinds with no recovery surface
/// must exist in both locales. There is no frontend test runner, so a missing
/// translation otherwise reaches the user as the raw key, not as a failure.
#[test]
fn spec_f145_every_emitted_i18n_key_is_translated_in_both_locales() {
    let emitted: Vec<&str> = vec![REASON_INTERRUPTED];

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

/// F-145: an id the journal does not hold is an answer about the REQUEST, not
/// a failing disk. A table row can vanish between the read and the click, and
/// the bridge must be able to say "not found" instead of "io error" — the
/// difference between "refresh the table" and "your disk is broken".
#[test]
fn spec_f145_an_unknown_operation_id_is_unknown_not_an_io_failure() {
    let dir = crate::test_tempdir();
    let journal = Journal::new(dir.path().join("operations"));
    let db = db_at(dir.path());
    let missing = uuid::Uuid::new_v4().to_string();

    let err = assess_operation(&db, &journal, &missing).expect_err("no such record");

    assert!(
        matches!(&err, AssessError::Unknown(id) if id == &missing),
        "an absent record is `Unknown`, not a disk failure: {err:?}"
    );
}

/// F-145: the assessor is reached from the webview with an arbitrary string,
/// so an id that is not the canonical uuid the journal writes is refused
/// BEFORE it becomes a path — `..\..\anything` must never be stat-ed.
#[test]
fn spec_f145_a_non_canonical_operation_id_never_becomes_a_path() {
    let dir = crate::test_tempdir();
    let journal = Journal::new(dir.path().join("operations"));
    let db = db_at(dir.path());
    // A record-shaped JSON just outside the journal, which a traversal id
    // would find if the id were joined into a path unchecked.
    std::fs::write(dir.path().join("outside.json"), b"{}").expect("write");

    for id in [r"..\outside", "../outside", "not-an-operation", ""] {
        let err = assess_operation(&db, &journal, id).expect_err("not a record id");
        assert!(
            matches!(err, AssessError::MalformedId(_)),
            "{id:?} was not refused as malformed: {err:?}"
        );
    }
}

/// F-145: a terminal record needs nothing, so its assessment carries no
/// reason and no action — the state column is all the view says.
#[test]
fn spec_f145_a_terminal_record_is_assessed_with_nothing_to_do() {
    let dir = crate::test_tempdir();
    let journal = Journal::new(dir.path().join("operations"));
    let db = db_at(dir.path());
    let target = dir.path().join("downloads");
    std::fs::create_dir_all(&target).expect("mkdir");

    let id = started_batch(&journal, &target, vec![]);
    journal
        .transition_operation(&id, OperationState::Running, None)
        .expect("run");
    journal
        .transition_operation(&id, OperationState::Completed, None)
        .expect("complete");

    let assessed = assess_operation(&db, &journal, &id).expect("assess");
    assert_eq!(assessed.state, OperationState::Completed);
    assert_eq!(assessed.reason, "", "{assessed:?}");
    assert_eq!(assessed.action, "", "{assessed:?}");
}
