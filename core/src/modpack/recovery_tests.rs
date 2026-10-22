//! F-076: the recovery of a crashed or failed pack apply.
//!
//! Every test runs in `crate::test_tempdir()` and against wiremock on
//! loopback, exactly like the executor's own tests. No real instance, no
//! real network, and nothing outside the temporary tree is touched.
//!
//! The crash is simulated the way the executor's own tests simulate it: the
//! journal callback fails on the Nth item write, and the error propagates
//! exactly as a kill would produce it.

use std::io::{Cursor, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::archive::{plan_install, read_pack};
use super::exec::{sha512_of_reader, ApplyReport, ExecError, PackIdentity, PlanExecutor};
use super::mrpack::STAGING_DIR;
use super::recovery::{
    dismiss, pending_pack_applies, recover_with, PackRecoveryAction, PackRecoveryReason,
    RecoveryError,
};
use crate::downloader::client::DownloadError;
use crate::downloader::Downloader;
use crate::journal::{
    pack_item, ItemState, Journal, JournalError, JournalOp, OperationItem, OperationKind,
    OperationRecord, OperationState, PackApplyMeta,
};
use crate::providers::{ModrinthClient, Provider};
use crate::safety::atomic::part_path;
use crate::safety::extract::extract_safely;
use crate::safety::inspect::ArchiveLimits;

/// The pack every test applies: two override files and nothing else.
///
/// Overrides are the cheapest way to write a file: the bytes are already in
/// the archive, so a test needs no server to prove that a file landed.
fn pack_bytes(overrides: &[(&str, &[u8])]) -> Vec<u8> {
    let index = serde_json::json!({
        "formatVersion": 1,
        "game": "minecraft",
        "versionId": "v1",
        "name": "Recovery Pack",
        "files": [],
        "overrides": "overrides",
    })
    .to_string();

    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    w.start_file(
        "modrinth.index.json",
        zip::write::SimpleFileOptions::default(),
    )
    .expect("index member");
    w.write_all(index.as_bytes()).expect("index bytes");
    for (name, body) in overrides {
        let member = format!("overrides/{name}");
        w.start_file(member.as_str(), zip::write::SimpleFileOptions::default())
            .expect("override member");
        w.write_all(body).expect("override bytes");
    }
    w.finish().expect("finish").into_inner()
}

/// A pack whose index names one download, served by `url`.
///
/// The hash is the one of `body`, so a server that serves anything else
/// fails the download at the hash gate.
fn pack_with_download(url: &str, body: &[u8]) -> Vec<u8> {
    let index = serde_json::json!({
        "formatVersion": 1,
        "game": "minecraft",
        "versionId": "v1",
        "name": "Recovery Pack",
        "files": [{
            "path": "mods/mod.bin",
            "hashes": {"sha1": sha1_hex(body), "sha512": sha512_hex(body)},
            "downloads": [url],
            "fileSize": body.len(),
        }],
        "overrides": "overrides",
    })
    .to_string();

    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    w.start_file(
        "modrinth.index.json",
        zip::write::SimpleFileOptions::default(),
    )
    .expect("index member");
    w.write_all(index.as_bytes()).expect("index bytes");
    w.finish().expect("finish").into_inner()
}

fn sha512_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    sha2::Sha512::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn sha1_hex(bytes: &[u8]) -> String {
    use sha1::Digest as _;
    sha1::Sha1::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The digest of a file on disk, the way the apply computes it.
fn digest_of(path: &Path) -> String {
    let mut file = std::fs::File::open(path).expect("the pack");
    sha512_of_reader(&mut file).expect("the digest")
}

/// Every file below `dir`, sorted, so "nothing was written" is a claim
/// about the whole tree and not just about one destination.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// The one record the journal holds.
fn the_record(journal: &Journal) -> OperationRecord {
    journal
        .discover()
        .records()
        .next()
        .cloned()
        .expect("the apply must be journalled")
}

/// The record's item for one destination.
fn item_of<'a>(record: &'a OperationRecord, item_id: &str) -> &'a OperationItem {
    record
        .items
        .iter()
        .find(|item| item.item_id == item_id)
        .expect("the item")
}

/// The callback the ordinary apply wants: it starts the record and writes
/// every later transition into it.
fn starting_writer(
    journal: &Journal,
) -> impl FnMut(JournalOp) -> Result<OperationRecord, JournalError> + '_ {
    let mut id: Option<String> = None;
    move |op| match op {
        JournalOp::PackStart {
            target,
            items,
            pack,
        } => {
            let record = journal.start_pack("", &target, items, pack)?;
            id = Some(record.operation_id.clone());
            Ok(record)
        }
        JournalOp::Operation(state) => {
            let id = current(&id)?;
            journal.transition_operation(&id, state, None)
        }
        JournalOp::Item { item_id, state } => {
            let id = current(&id)?;
            journal.transition_item(&id, &item_id, state, None)
        }
        JournalOp::Items { transitions } => {
            let id = current(&id)?;
            journal.transition_items(&id, &transitions, None)
        }
        JournalOp::Start { .. } => Err(JournalError::ItemMissing("not a batch".into())),
    }
}

fn current(id: &Option<String>) -> Result<String, JournalError> {
    id.clone()
        .ok_or_else(|| JournalError::ItemMissing("the apply wrote no operation yet".into()))
}

/// The callback the recovery wants: every write lands in the record the
/// given id names.
fn writer_for<'a>(
    journal: &'a Journal,
    operation_id: &str,
) -> impl FnMut(JournalOp) -> Result<OperationRecord, JournalError> + 'a {
    let id = operation_id.to_string();
    move |op| match op {
        JournalOp::Operation(state) => journal.transition_operation(&id, state, None),
        JournalOp::Item { item_id, state } => journal.transition_item(&id, &item_id, state, None),
        JournalOp::Items { transitions } => journal.transition_items(&id, &transitions, None),
        JournalOp::Start { .. } | JournalOp::PackStart { .. } => Err(JournalError::ItemMissing(
            "a recovery must not start a record".into(),
        )),
    }
}

/// The crash simulation: the journal callback fails on the Nth item
/// transition, exactly as the executor's own tests do it.
fn crashing_writer(
    journal: &Journal,
    budget: usize,
) -> impl FnMut(JournalOp) -> Result<OperationRecord, JournalError> + '_ {
    let mut inner = starting_writer(journal);
    let mut spent = 0usize;
    move |op| {
        if let JournalOp::Items { transitions } = &op {
            spent += transitions.len();
            if spent > budget {
                return Err(JournalError::Io("simulated crash".into()));
            }
        }
        inner(op)
    }
}

/// The crash that lands after the last file and before the record is
/// closed: every item is terminal, the operation is still running.
fn closing_crash_writer(
    journal: &Journal,
) -> impl FnMut(JournalOp) -> Result<OperationRecord, JournalError> + '_ {
    let mut inner = starting_writer(journal);
    move |op| match op {
        JournalOp::Operation(OperationState::Completed) => {
            Err(JournalError::Io("simulated crash".into()))
        }
        other => inner(other),
    }
}

/// The crash inside a recovery: the journal callback fails on the Nth item
/// transition, so the recovery itself dies half way.
fn crashing_recovery_writer<'a>(
    journal: &'a Journal,
    operation_id: &str,
    budget: usize,
) -> impl FnMut(JournalOp) -> Result<OperationRecord, JournalError> + 'a {
    let mut inner = writer_for(journal, operation_id);
    let mut spent = 0usize;
    move |op| {
        if let JournalOp::Items { transitions } = &op {
            spent += transitions.len();
            if spent > budget {
                return Err(JournalError::Io("simulated crash".into()));
            }
        }
        inner(op)
    }
}

/// Applies the pack the way the IPC layer does: read, plan, extract, apply.
///
/// The recovery starts from the same three steps, so a test that crashes an
/// apply has to crash the real one.
async fn apply_pack(
    provider: &dyn Provider,
    downloader: &Downloader,
    archive_path: &Path,
    instance_root: &Path,
    write: &mut (dyn FnMut(JournalOp) -> Result<OperationRecord, JournalError> + Send),
    allow_insecure_loopback: bool,
) -> Result<ApplyReport, ExecError> {
    let limits = ArchiveLimits::default();
    let mut file = std::fs::File::open(archive_path).expect("the pack");
    let digest = sha512_of_reader(&mut file).expect("the digest");
    file.seek(SeekFrom::Start(0)).expect("rewind");
    let mut archive = zip::ZipArchive::new(file).expect("zip");
    let (pack, inventory) = read_pack(&mut archive, &limits).expect("the index");
    let plan = plan_install(&pack, &inventory).expect("the plan");
    let staging = instance_root.join(STAGING_DIR);
    extract_safely(&mut archive, &staging, &limits, &mut |_| {
        std::ops::ControlFlow::Continue(())
    })
    .expect("extract");
    let identity = PackIdentity {
        name: pack.name.clone(),
        version_id: pack.version_id.clone(),
        archive_sha512: digest,
        archive_path: archive_path.to_string_lossy().into_owned(),
    };
    let executor = PlanExecutor::new(
        provider,
        downloader,
        &pack,
        &plan,
        &staging,
        instance_root,
        identity,
    )
    .allow_insecure_loopback(allow_insecure_loopback);
    executor.apply(write, &mut |_| {}).await
}

/// A provider that knows nothing: the pack's own URLs are the candidates.
fn offline_provider() -> ModrinthClient {
    ModrinthClient::with_base("http://127.0.0.1:9").expect("provider")
}

/// A record of a pack apply, with the given destinations and states.
///
/// The recovery's inputs, built by hand: the assessment and the recovery
/// both start from a record, and a hand-made one lets a test put the record
/// in a state a real crash cannot produce on demand.
fn record_for(
    journal: &Journal,
    archive: &Path,
    instance: &Path,
    destinations: &[(&str, ItemState)],
) -> OperationRecord {
    let items: Vec<OperationItem> = destinations
        .iter()
        .map(|(name, state)| {
            let dest = instance.join(name);
            let part = part_path(&dest);
            let mut item = pack_item(
                name,
                "Recovery Pack",
                &part.to_string_lossy(),
                &dest.to_string_lossy(),
            );
            item.state = *state;
            item
        })
        .collect();
    let meta = PackApplyMeta {
        pack_name: "Recovery Pack".to_string(),
        pack_version_id: "v1".to_string(),
        archive_sha512: digest_of(archive),
        archive_path: archive.to_string_lossy().into_owned(),
    };
    let record = journal
        .start_pack("", &instance.to_string_lossy(), items, meta)
        .expect("start");
    journal
        .transition_operation(&record.operation_id, OperationState::Running, None)
        .expect("running");
    the_record(journal)
}

/// The crash the recovery is for: two files, the first landed, the second
/// never started.
async fn crash_between_two_files(
    dir: &Path,
) -> (Journal, PathBuf, PathBuf, Downloader, ModrinthClient) {
    let instance = dir.join("instance");
    std::fs::create_dir_all(&instance).expect("instance");
    let archive = dir.join("pack.mrpack");
    std::fs::write(&archive, pack_bytes(&[("a.txt", b"a"), ("b.txt", b"b")])).expect("pack");

    let journal = Journal::new(dir.join("operations"));
    let downloader = Downloader::new(1).expect("downloader");
    let provider = offline_provider();

    // The first file's chain is three transitions; the crash lands on the
    // second file's write.
    {
        let mut write = crashing_writer(&journal, 3);
        let error = apply_pack(
            &provider,
            &downloader,
            &archive,
            &instance,
            &mut write,
            false,
        )
        .await
        .expect_err("the crash must propagate");
        assert!(matches!(error, ExecError::Journal(_)), "got {error}");
    }

    (journal, archive, instance, downloader, provider)
}

#[tokio::test]
async fn spec_f076_a_crash_between_two_files_leaves_the_first_recorded_and_the_second_queued() {
    let dir = crate::test_tempdir();
    let (journal, _archive, instance, _downloader, _provider) =
        crash_between_two_files(dir.path()).await;

    let record = the_record(&journal);
    assert_eq!(record.kind, OperationKind::MrpackInstall);
    assert_eq!(record.state, OperationState::Running);
    assert_eq!(record.items.len(), 2);
    // The ending of a file rides in the next file's write, so the record
    // still calls the first file `committing` — the crash caught it there.
    assert_eq!(item_of(&record, "a.txt").state, ItemState::Committing);
    assert_eq!(item_of(&record, "b.txt").state, ItemState::Queued);

    // The disk agrees with the record about what landed: the first file is
    // there, the second is not.
    assert_eq!(std::fs::read(instance.join("a.txt")).expect("read"), b"a");
    assert!(!instance.join("b.txt").exists());
}

#[tokio::test]
async fn spec_f076_recovery_resumes_the_files_that_were_not_written() {
    let dir = crate::test_tempdir();
    let (journal, _archive, instance, downloader, provider) =
        crash_between_two_files(dir.path()).await;
    let operation_id = the_record(&journal).operation_id;

    let mut write = writer_for(&journal, &operation_id);
    let report = recover_with(
        &provider,
        &downloader,
        &journal,
        &operation_id,
        &mut write,
        &mut |_| {},
        false,
    )
    .await
    .expect("recovers");

    // The file that had landed is left exactly as it was.
    assert_eq!(std::fs::read(instance.join("a.txt")).expect("read"), b"a");
    // The file that was missing is written.
    assert_eq!(std::fs::read(instance.join("b.txt")).expect("read"), b"b");
    assert_eq!(report.written, 1, "only the missing file was written");
    assert_eq!(report.skipped, 1, "the landed file was left alone");
    assert_eq!(report.failed, 0);
    assert_eq!(report.state, OperationState::Completed);

    // The record is closed, and it says what happened.
    let record = the_record(&journal);
    assert_eq!(record.state, OperationState::Completed);
    assert_eq!(item_of(&record, "a.txt").state, ItemState::Completed);
    assert_eq!(item_of(&record, "b.txt").state, ItemState::Completed);
    assert!(
        files_under(&instance)
            .iter()
            .all(|p| p.extension().is_none_or(|e| e != "part")),
        "a scratch file survived the recovery"
    );
}

#[tokio::test]
async fn spec_f076_a_crash_after_the_last_file_but_before_the_commit_only_closes_the_record() {
    let dir = crate::test_tempdir();
    let instance = dir.path().join("instance");
    std::fs::create_dir_all(&instance).expect("instance");
    let archive = dir.path().join("pack.mrpack");
    std::fs::write(&archive, pack_bytes(&[("a.txt", b"a"), ("b.txt", b"b")])).expect("pack");

    let journal = Journal::new(dir.path().join("operations"));
    let downloader = Downloader::new(1).expect("downloader");
    let provider = offline_provider();

    // The crash lands on the record's closing write: every file is in
    // place, every item is terminal, and the operation is still running.
    let mut write = closing_crash_writer(&journal);
    let error = apply_pack(
        &provider,
        &downloader,
        &archive,
        &instance,
        &mut write,
        false,
    )
    .await
    .expect_err("the crash must propagate");
    assert!(matches!(error, ExecError::Journal(_)), "got {error}");

    let record = the_record(&journal);
    assert_eq!(record.state, OperationState::Running);
    assert_eq!(item_of(&record, "a.txt").state, ItemState::Completed);
    assert_eq!(item_of(&record, "b.txt").state, ItemState::Completed);

    let before = files_under(&instance);
    let operation_id = record.operation_id.clone();
    let mut write = writer_for(&journal, &operation_id);
    let report = recover_with(
        &provider,
        &downloader,
        &journal,
        &operation_id,
        &mut write,
        &mut |_| {},
        false,
    )
    .await
    .expect("recovers");

    // The recovery writes NOTHING and only closes the record.
    assert_eq!(report.written, 0);
    assert_eq!(report.skipped, 2);
    assert_eq!(report.state, OperationState::Completed);
    assert_eq!(
        files_under(&instance),
        before,
        "the recovery wrote to the instance"
    );
    assert_eq!(the_record(&journal).state, OperationState::Completed);
}

#[tokio::test]
async fn spec_f076_a_crash_during_the_commit_is_resolved_by_the_destination_hash() {
    // One rule, two ways: a `committing` item says the part was verified
    // and the rename was pending, so only the disk knows what happened.
    for (what, body, expect_written) in [
        (
            "the destination holds the expected bytes",
            b"a".to_vec(),
            0usize,
        ),
        (
            "the destination holds something else",
            b"tampered".to_vec(),
            1usize,
        ),
    ] {
        let dir = crate::test_tempdir();
        let instance = dir.path().join("instance");
        std::fs::create_dir_all(&instance).expect("instance");
        let archive = dir.path().join("pack.mrpack");
        std::fs::write(&archive, pack_bytes(&[("a.txt", b"a")])).expect("pack");

        let journal = Journal::new(dir.path().join("operations"));
        let downloader = Downloader::new(1).expect("downloader");
        let provider = offline_provider();

        // The record of an apply that died in the commit window.
        let dest = instance.join("a.txt");
        let part = part_path(&dest);
        std::fs::write(&dest, &body).expect("the destination");
        let record = record_for(
            &journal,
            &archive,
            &instance,
            &[("a.txt", ItemState::Committing)],
        );
        assert_eq!(
            item_of(&record, "a.txt").partial_path,
            part.to_string_lossy()
        );

        let mut write = writer_for(&journal, &record.operation_id);
        let report = recover_with(
            &provider,
            &downloader,
            &journal,
            &record.operation_id,
            &mut write,
            &mut |_| {},
            false,
        )
        .await
        .expect("recovers");

        assert_eq!(report.written, expect_written, "{what}");
        // Either way the destination ends up holding the bytes the plan
        // pinned.
        assert_eq!(std::fs::read(&dest).expect("read"), b"a", "{what}");
        assert_eq!(
            the_record(&journal).state,
            OperationState::Completed,
            "{what}"
        );
    }
}

#[tokio::test]
async fn spec_f076_recovery_run_twice_is_a_no_op_the_second_time() {
    let dir = crate::test_tempdir();
    let (journal, _archive, instance, downloader, provider) =
        crash_between_two_files(dir.path()).await;
    let operation_id = the_record(&journal).operation_id;

    let mut write = writer_for(&journal, &operation_id);
    recover_with(
        &provider,
        &downloader,
        &journal,
        &operation_id,
        &mut write,
        &mut |_| {},
        false,
    )
    .await
    .expect("recovers");

    let revision = the_record(&journal).revision;
    let files = files_under(&instance);

    // The second run finds a closed case: it writes nothing, and it does
    // not start a second record.
    let mut write = writer_for(&journal, &operation_id);
    let report = recover_with(
        &provider,
        &downloader,
        &journal,
        &operation_id,
        &mut write,
        &mut |_| {},
        false,
    )
    .await
    .expect("the second run does nothing");
    assert_eq!(report.written, 0);
    assert_eq!(report.state, OperationState::Completed);
    assert_eq!(
        the_record(&journal).revision,
        revision,
        "the second run wrote to the record"
    );
    assert_eq!(files_under(&instance), files);
    assert_eq!(
        journal.discover().records().count(),
        1,
        "a second record was created"
    );
}

#[tokio::test]
async fn spec_f076_a_crash_inside_the_recovery_resumes_from_where_it_stopped() {
    // A crash inside the recovery leaves the record open, with the file
    // that landed in place and the item still saying `committing`. The next
    // run asks the disk, finds the file there, and closes the record
    // without rewriting anything.
    let dir = crate::test_tempdir();
    let (journal, _archive, instance, downloader, provider) =
        crash_between_two_files(dir.path()).await;
    let operation_id = the_record(&journal).operation_id;

    // The recovery dies after the file landed but before the record says
    // so: the chain write goes through, the ending does not.
    let mut write = crashing_recovery_writer(&journal, &operation_id, 3);
    let error = recover_with(
        &provider,
        &downloader,
        &journal,
        &operation_id,
        &mut write,
        &mut |_| {},
        false,
    )
    .await
    .expect_err("the crash must propagate");
    assert!(
        matches!(error, RecoveryError::Exec(ExecError::Journal(_))),
        "got {error:?}"
    );

    // The case is still open, and the file that landed is in place.
    let record = the_record(&journal);
    assert_eq!(record.state, OperationState::Running);
    assert_eq!(
        item_of(&record, "b.txt").state,
        ItemState::Committing,
        "the ending of the file did not reach the record"
    );
    assert_eq!(std::fs::read(instance.join("b.txt")).expect("read"), b"b");

    // The next run finishes it: the disk says the file landed, so nothing
    // is rewritten and the record is closed.
    let mut write = writer_for(&journal, &operation_id);
    let report = recover_with(
        &provider,
        &downloader,
        &journal,
        &operation_id,
        &mut write,
        &mut |_| {},
        false,
    )
    .await
    .expect("recovers");
    assert_eq!(report.written, 0, "the file was written twice");
    assert_eq!(report.state, OperationState::Completed);
    assert_eq!(the_record(&journal).state, OperationState::Completed);
    assert_eq!(std::fs::read(instance.join("a.txt")).expect("read"), b"a");
    assert_eq!(std::fs::read(instance.join("b.txt")).expect("read"), b"b");
}

#[tokio::test]
async fn spec_f076_recovery_on_a_clean_instance_writes_nothing() {
    let dir = crate::test_tempdir();
    let journal = Journal::new(dir.path().join("operations"));
    let downloader = Downloader::new(1).expect("downloader");
    let provider = offline_provider();

    // Nothing is offered: there is no record at all.
    assert!(pending_pack_applies(&journal).is_empty());

    // And nothing can be recovered.
    let mut write = writer_for(&journal, "not-an-operation");
    let error = recover_with(
        &provider,
        &downloader,
        &journal,
        "not-an-operation",
        &mut write,
        &mut |_| {},
        false,
    )
    .await
    .expect_err("must refuse");
    assert!(matches!(error, RecoveryError::Journal(_)), "got {error:?}");
    assert_eq!(journal.discover().records().count(), 0);
    assert!(files_under(dir.path()).is_empty());
}

#[tokio::test]
async fn spec_f080_recovery_refuses_a_pack_that_changed_since_the_plan() {
    // F-080: the pack changed after the preview, inside the F-076 recovery.
    // The plan-digest binding lives in the record, so the recovery needs no
    // token from the webview: the archive it is handed has to be the
    // archive the plan was built from.
    let dir = crate::test_tempdir();
    let (journal, archive, instance, downloader, provider) =
        crash_between_two_files(dir.path()).await;
    let operation_id = the_record(&journal).operation_id;

    // The pack on disk is no longer the archive the plan was built from.
    std::fs::write(
        &archive,
        pack_bytes(&[("a.txt", b"other"), ("b.txt", b"other")]),
    )
    .expect("the other pack");

    let mut write = writer_for(&journal, &operation_id);
    let error = recover_with(
        &provider,
        &downloader,
        &journal,
        &operation_id,
        &mut write,
        &mut |_| {},
        false,
    )
    .await
    .expect_err("must refuse");
    assert!(matches!(error, RecoveryError::PackChanged), "got {error:?}");

    // Nothing was written: the instance still holds what the crashed apply
    // put there, and nothing else.
    assert_eq!(std::fs::read(instance.join("a.txt")).expect("read"), b"a");
    assert!(!instance.join("b.txt").exists());
    assert_eq!(the_record(&journal).state, OperationState::Running);
}

#[tokio::test]
async fn spec_f076_recovery_refuses_an_instance_folder_that_was_deleted_underneath() {
    let dir = crate::test_tempdir();
    let (journal, _archive, instance, downloader, provider) =
        crash_between_two_files(dir.path()).await;
    let operation_id = the_record(&journal).operation_id;

    // The user deleted the folder. The recovery does not recreate it: a
    // folder the user deleted is not ours to bring back, and nothing on
    // the disk proves we should.
    std::fs::remove_dir_all(&instance).expect("remove");

    let mut write = writer_for(&journal, &operation_id);
    let error = recover_with(
        &provider,
        &downloader,
        &journal,
        &operation_id,
        &mut write,
        &mut |_| {},
        false,
    )
    .await
    .expect_err("must refuse");
    assert!(
        matches!(error, RecoveryError::InstanceMissing(_)),
        "got {error:?}"
    );
    assert!(!instance.exists(), "the folder was recreated");
    // The case is settled: there is nothing left to resume.
    assert_eq!(the_record(&journal).state, OperationState::Failed);
    assert!(
        pending_pack_applies(&journal).is_empty(),
        "a settled case is offered again"
    );
}

#[tokio::test]
async fn spec_f076_recovery_clears_the_part_an_interrupted_commit_left_behind() {
    // The wedge: `write_part` refuses a `.part` it cannot attribute, so a
    // `.part` left behind by an interrupted commit makes that destination
    // unwritable — the pack would be permanently uninstallable. The journal
    // is the only proof that the part is ours, so the record is what lets
    // the recovery clear it.
    let dir = crate::test_tempdir();
    let (journal, _archive, instance, downloader, provider) =
        crash_between_two_files(dir.path()).await;
    let operation_id = the_record(&journal).operation_id;

    // The interrupted write of the second file left its verified part
    // behind.
    let part = part_path(&instance.join("b.txt"));
    assert!(part.is_file(), "the crash left no part to clear");

    let mut write = writer_for(&journal, &operation_id);
    recover_with(
        &provider,
        &downloader,
        &journal,
        &operation_id,
        &mut write,
        &mut |_| {},
        false,
    )
    .await
    .expect("recovers");

    assert!(!part.exists(), "the part was not cleared");
    assert_eq!(std::fs::read(instance.join("b.txt")).expect("read"), b"b");
}

#[tokio::test]
async fn spec_f076_recovery_never_touches_a_part_the_journal_does_not_claim() {
    // Only the part the record names is ours. A part at a path no record
    // mentions is somebody else's file: it stays, and the write refuses
    // the destination with the honest `StalePart` error instead of
    // destroying it.
    let dir = crate::test_tempdir();
    let instance = dir.path().join("instance");
    std::fs::create_dir_all(&instance).expect("instance");
    let archive = dir.path().join("pack.mrpack");
    std::fs::write(&archive, pack_bytes(&[("a.txt", b"a"), ("b.txt", b"b")])).expect("pack");

    let journal = Journal::new(dir.path().join("operations"));
    let downloader = Downloader::new(1).expect("downloader");
    let provider = offline_provider();

    // A record that names the first destination only.
    let record = record_for(
        &journal,
        &archive,
        &instance,
        &[("a.txt", ItemState::Queued)],
    );
    // And a foreign part at the second destination.
    let foreign = part_path(&instance.join("b.txt"));
    std::fs::write(&foreign, b"not ours").expect("foreign");

    let mut write = writer_for(&journal, &record.operation_id);
    let error = recover_with(
        &provider,
        &downloader,
        &journal,
        &record.operation_id,
        &mut write,
        &mut |_| {},
        false,
    )
    .await
    .expect_err("must refuse the foreign part");
    assert!(
        matches!(
            error,
            RecoveryError::Exec(ExecError::Atomic(
                crate::safety::atomic::AtomicError::StalePart(_)
            ))
        ),
        "got {error:?}"
    );

    // The file the record does claim is written.
    assert_eq!(std::fs::read(instance.join("a.txt")).expect("read"), b"a");
    // And the foreign part is untouched.
    assert_eq!(std::fs::read(&foreign).expect("read"), b"not ours");
}

#[tokio::test]
async fn spec_f076_a_failed_pack_apply_leaves_the_operation_running() {
    // A failed apply leaves the instance half applied, exactly like a
    // crash, so the operation stays RUNNING: the record is what the
    // recovery reads, and a terminal record is never offered again.
    let dir = crate::test_tempdir();
    let instance = dir.path().join("instance");
    std::fs::create_dir_all(&instance).expect("instance");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/version_files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;
    // The server serves bytes the pack did not pin.
    Mock::given(method("GET"))
        .and(path("/bad.bin"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"evil-bytes".to_vec()))
        .mount(&server)
        .await;

    let archive = dir.path().join("pack.mrpack");
    std::fs::write(
        &archive,
        pack_with_download(&format!("{}/bad.bin", server.uri()), b"good-bytes"),
    )
    .expect("pack");

    let journal = Journal::new(dir.path().join("operations"));
    let downloader = Downloader::new(1).expect("downloader");
    let provider = ModrinthClient::with_base(&server.uri()).expect("provider");

    let mut write = starting_writer(&journal);
    let error = apply_pack(
        &provider,
        &downloader,
        &archive,
        &instance,
        &mut write,
        true,
    )
    .await
    .expect_err("must refuse the wrong bytes");
    assert!(
        matches!(error, ExecError::Download(DownloadError::HashMismatch)),
        "got {error}"
    );

    let record = the_record(&journal);
    assert_eq!(
        record.state,
        OperationState::Running,
        "a failed apply must leave the operation running"
    );
    assert_eq!(
        item_of(&record, "mods/mod.bin").state,
        ItemState::Failed,
        "the item that failed is failed"
    );
    // And the case is offered, because it can still be finished.
    let offered = pending_pack_applies(&journal);
    assert_eq!(offered.len(), 1, "the failed apply is not offered");
    assert_eq!(offered[0].action, PackRecoveryAction::Resume);
}

#[tokio::test]
async fn spec_f146_the_pack_recovery_assessment_reports_evidence_not_guesses() {
    let dir = crate::test_tempdir();
    let instance = dir.path().join("instance");
    std::fs::create_dir_all(&instance).expect("instance");
    let archive = dir.path().join("pack.mrpack");
    std::fs::write(&archive, pack_bytes(&[("a.txt", b"a"), ("b.txt", b"b")])).expect("pack");

    // Nothing written yet.
    let journal = Journal::new(dir.path().join("operations"));
    let record = record_for(
        &journal,
        &archive,
        &instance,
        &[("a.txt", ItemState::Queued), ("b.txt", ItemState::Queued)],
    );
    let offered = pending_pack_applies(&journal);
    assert_eq!(offered.len(), 1);
    assert_eq!(offered[0].reason, PackRecoveryReason::NothingWritten);
    assert_eq!(offered[0].action, PackRecoveryAction::Resume);
    assert_eq!(offered[0].planned_count, 2);
    assert_eq!(offered[0].done_count, 0);
    assert_eq!(offered[0].operation_id, record.operation_id);

    // One file in place: the evidence is the disk, not the record.
    std::fs::write(instance.join("a.txt"), b"a").expect("the file");
    let offered = pending_pack_applies(&journal);
    assert_eq!(offered[0].reason, PackRecoveryReason::PartiallyApplied);
    assert_eq!(offered[0].done_count, 1);

    // Every file in place: only the commit is missing.
    std::fs::write(instance.join("b.txt"), b"b").expect("the file");
    let offered = pending_pack_applies(&journal);
    assert_eq!(offered[0].reason, PackRecoveryReason::FullyApplied);
    assert_eq!(offered[0].done_count, 2);

    // The instance folder is gone.
    std::fs::remove_dir_all(&instance).expect("remove");
    let offered = pending_pack_applies(&journal);
    assert_eq!(offered[0].reason, PackRecoveryReason::InstanceMissing);
    assert_eq!(offered[0].action, PackRecoveryAction::Dismiss);
    std::fs::create_dir_all(&instance).expect("instance");

    // The archive is gone.
    std::fs::remove_file(&archive).expect("remove");
    let offered = pending_pack_applies(&journal);
    assert_eq!(offered[0].reason, PackRecoveryReason::PackMissing);
    assert_eq!(offered[0].action, PackRecoveryAction::Dismiss);

    // The archive is there but it is not the one the plan was built from.
    std::fs::write(&archive, pack_bytes(&[("a.txt", b"other")])).expect("the other pack");
    let offered = pending_pack_applies(&journal);
    assert_eq!(offered[0].reason, PackRecoveryReason::PackChanged);
    assert_eq!(offered[0].action, PackRecoveryAction::Dismiss);

    // A record that is not a pack apply is never offered.
    let other = journal
        .start(OperationKind::DownloadBatch, "p1", "target", vec![])
        .expect("start");
    journal
        .transition_operation(&other.operation_id, OperationState::Running, None)
        .expect("running");
    let offered = pending_pack_applies(&journal);
    assert_eq!(offered.len(), 1, "a batch was offered as a pack apply");
    assert_eq!(offered[0].operation_id, record.operation_id);
}

#[tokio::test]
async fn spec_f141_discover_reports_an_unfinished_pack_apply() {
    let dir = crate::test_tempdir();
    let instance = dir.path().join("instance");
    std::fs::create_dir_all(&instance).expect("instance");
    let archive = dir.path().join("pack.mrpack");
    std::fs::write(&archive, pack_bytes(&[("a.txt", b"a")])).expect("pack");

    let journal = Journal::new(dir.path().join("operations"));
    let record = record_for(
        &journal,
        &archive,
        &instance,
        &[("a.txt", ItemState::Queued)],
    );

    // The record is unfinished, and `discover()` is what hands it back on
    // the next launch.
    let found = journal.discover();
    assert_eq!(found.issues().count(), 0);
    let unfinished: Vec<&OperationRecord> = found.unfinished().collect();
    assert_eq!(unfinished.len(), 1);
    assert_eq!(unfinished[0].operation_id, record.operation_id);
    assert_eq!(unfinished[0].kind, OperationKind::MrpackInstall);
    assert!(unfinished[0].pack.is_some(), "the record names the pack");
}

#[tokio::test]
async fn spec_f076_dismissing_the_case_closes_the_record_without_touching_the_instance() {
    let dir = crate::test_tempdir();
    let (journal, _archive, instance, _downloader, _provider) =
        crash_between_two_files(dir.path()).await;
    let operation_id = the_record(&journal).operation_id;
    let before = files_under(&instance);

    dismiss(&journal, &operation_id).expect("dismiss");

    // The record is closed: the user chose this.
    assert_eq!(the_record(&journal).state, OperationState::Failed);
    // And nothing on the disk was touched: a rollback would delete the
    // user's own files.
    assert_eq!(files_under(&instance), before);
    assert!(
        pending_pack_applies(&journal).is_empty(),
        "a dismissed case is offered again"
    );
}

#[tokio::test]
async fn spec_f076_the_recovery_refuses_a_record_that_is_not_a_pack_apply() {
    // Both mutating entry points are reached by operation id, so each must
    // confirm what kind of record it was handed.
    let dir = crate::test_tempdir();
    let journal = Journal::new(dir.path().join("operations"));
    let downloader = Downloader::new(1).expect("downloader");
    let provider = offline_provider();

    let record = journal
        .start(OperationKind::DownloadBatch, "p1", "target", vec![])
        .expect("start");
    journal
        .transition_operation(&record.operation_id, OperationState::Running, None)
        .expect("running");

    let mut write = writer_for(&journal, &record.operation_id);
    let error = recover_with(
        &provider,
        &downloader,
        &journal,
        &record.operation_id,
        &mut write,
        &mut |_| {},
        false,
    )
    .await
    .expect_err("must refuse");
    assert!(
        matches!(error, RecoveryError::NotAPackApply(_)),
        "got {error:?}"
    );
    assert!(matches!(
        dismiss(&journal, &record.operation_id),
        Err(RecoveryError::NotAPackApply(_))
    ));
}
