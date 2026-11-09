//! F-063: the batch drives the journal and the downloader together, so it is
//! tested against both rather than against a mock of either.

use super::*;
use crate::downloader::Downloader;
use crate::downloader::JournalOp;
use crate::journal::{ItemState, Journal, OperationKind, OperationRecord, OperationState};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// A real zip, because the downloader refuses to commit anything that is not an
/// archive (`DownloadError::NotArchive`) — the same builder its own tests use,
/// rather than a second one that could drift.
fn zip_bytes(name: &str, data: &[u8]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    w.start_file(name, zip::write::SimpleFileOptions::default())
        .expect("fixture");
    use std::io::Write;
    w.write_all(data).expect("fixture");
    w.finish().expect("fixture").into_inner()
}
/// Turns a real journal into the callback `execute_batch` wants. Production
/// wraps a mutex around exactly this; the batch never sees either.
fn writer(
    j: &Journal,
) -> impl FnMut(JournalOp) -> Result<OperationRecord, crate::journal::JournalError> + '_ {
    move |op| match op {
        JournalOp::Start { target, items } => {
            j.start(OperationKind::DownloadBatch, "", &target, items)
        }
        JournalOp::Operation(state) => {
            let id = current_id(j)?;
            j.transition_operation(&id, state, None)
        }
        JournalOp::Item { item_id, state } => {
            let id = current_id(j)?;
            j.transition_item(&id, &item_id, state, None)
        }
        // The pack variants are not the downloader's business: a batch never
        // starts a pack apply and never batches its transitions.
        JournalOp::PackStart { .. } | JournalOp::Items { .. } => Err(
            crate::journal::JournalError::ItemMissing("not a download batch op".into()),
        ),
    }
}

fn current_id(j: &Journal) -> Result<String, crate::journal::JournalError> {
    j.discover()
        .records()
        .next()
        .map(|r| r.operation_id.clone())
        .ok_or(crate::journal::JournalError::ItemMissing(
            "no operation".into(),
        ))
}
fn jrnl(_dir: &std::path::Path) -> (Journal, tempfile::TempDir) {
    let holder = crate::test_tempdir();
    (Journal::new(holder.path().join("operations")), holder)
}

fn item(_dir: &std::path::Path, name: &str, body: &[u8]) -> BatchItem {
    let sha1 = {
        use sha1::Digest as _;
        let mut h = sha1::Sha1::new();
        h.update(body);
        let out = h.finalize();
        out.iter().map(|b| format!("{b:02x}")).collect::<String>()
    };
    BatchItem {
        item_id: name.to_string(),
        project_id: "p".to_string(),
        title: name.to_string(),
        filename: name.to_string(),
        // Replaced by the test's server; kept here so a failing field is obvious.
        url: String::new(),
        size: Some(body.len() as u64),
        sha1_hex: Some(sha1),
        sha512_hex: None,
        allow_insecure_loopback: true,
    }
}

fn no_cancel() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

/// The whole point: files land, and the journal records the batch as completed
/// with every item accounted for. If the journal were not written, a crash
/// would leave files no record claims.
#[tokio::test]
async fn spec_f063_batch_lands_files_and_completes_the_journal_operation() {
    let dir = crate::test_tempdir();
    let (j, _holder) = jrnl(dir.path());
    let mods = dir.path().join("mods");
    std::fs::create_dir_all(&mods).expect("mkdir");
    let d = Downloader::new(2).expect("downloader");
    let server = wiremock::MockServer::start().await;
    let body = zip_bytes("META-INF/MANIFEST.MF", b"Manifest-Version: 1.0\n");
    let sha1 = item(dir.path(), "a.jar", &body).sha1_hex.clone().unwrap();
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/a.jar"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_bytes(body.clone())
                .insert_header("content-type", "application/java-archive"),
        )
        .mount(&server)
        .await;

    let mut it = item(dir.path(), "a.jar", &body);
    it.url = format!("{}/a.jar", server.uri());
    // wiremock is loopback http, which production forbids.
    let landed = {
        let mut seen: Vec<(String, ItemProgress)> = Vec::new();
        let r = execute_batch(
            &d,
            &mut writer(&j),
            &mods,
            std::slice::from_ref(&it),
            no_cancel(),
            &mut |id, p| seen.push((id.to_string(), p)),
        )
        .await;
        let _ = &sha1;
        assert!(r.is_ok(), "batch failed: {r:?}");
        assert!(mods.join("a.jar").is_file(), "the file did not land");
        // Progress is reported, and it ends on a landing.
        assert!(seen
            .iter()
            .any(|(_, p)| matches!(p, ItemProgress::Landed { .. })));
        r.expect("landed")
    };

    assert_eq!(landed.len(), 1);
    let rec = j
        .discover()
        .records()
        .next()
        .cloned()
        .expect("the batch must be journalled");
    assert_eq!(rec.kind, OperationKind::DownloadBatch);
    assert_eq!(rec.state, OperationState::Completed);
    assert_eq!(rec.items.len(), 1);
    assert_eq!(rec.items[0].state, ItemState::Completed);
    assert_eq!(rec.planned_count, 1);
    assert_eq!(rec.completed_count, 1);
}

/// The reason `execute_batch` takes a callback and not a `&Journal`.
///
/// The batch holds the journal for the length of a download. Production wraps
/// a mutex around it, so a three-minute download would hold that mutex for
/// three minutes and every other command touching the journal — the recovery
/// reads on the Instances page among them — would wait for the last byte.
///
/// This asserts the lock is free *during* a transfer, by taking it from inside
/// the progress callback. If someone ever "simplifies" the callback back into a
/// long-lived guard, this stops the suite rather than shipping the stall.
#[tokio::test]
async fn spec_f063_the_journal_is_not_locked_while_a_download_runs() {
    let dir = crate::test_tempdir();
    let (j, _holder) = jrnl(dir.path());
    let mods = dir.path().join("mods");
    std::fs::create_dir_all(&mods).expect("mkdir");

    // What production does: a mutex, opened and closed per write.
    let handle = std::sync::Arc::new(std::sync::Mutex::new(j));
    let d = Downloader::new(1).expect("downloader");
    let server = wiremock::MockServer::start().await;
    let body = zip_bytes("META-INF/MANIFEST.MF", b"Manifest-Version: 1.0\n");
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/a.jar"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_bytes(body.clone())
                .insert_header("content-type", "application/java-archive"),
        )
        .mount(&server)
        .await;
    let mut it = item(dir.path(), "a.jar", &body);
    it.url = format!("{}/a.jar", server.uri());

    let mut contended = false;
    {
        let h = handle.clone();
        let mut write = move |op: JournalOp| {
            // Short: lock, write, drop. Never held across a download.
            let guard = h.lock().expect("journal lock");
            match op {
                JournalOp::Start { target, items } => {
                    guard.start(OperationKind::DownloadBatch, "", &target, items)
                }
                JournalOp::Operation(state) => {
                    let id = guard
                        .discover()
                        .records()
                        .next()
                        .map(|r| r.operation_id.clone());
                    match id {
                        Some(id) => guard.transition_operation(&id, state, None),
                        None => Err(crate::journal::JournalError::ItemMissing("none".into())),
                    }
                }
                JournalOp::Item { item_id, state } => {
                    let id = guard
                        .discover()
                        .records()
                        .next()
                        .map(|r| r.operation_id.clone());
                    match id {
                        Some(id) => guard.transition_item(&id, &item_id, state, None),
                        None => Err(crate::journal::JournalError::ItemMissing("none".into())),
                    }
                }
                JournalOp::PackStart { .. } | JournalOp::Items { .. } => Err(
                    crate::journal::JournalError::ItemMissing("not a download batch op".into()),
                ),
            }
        };
        let probe = handle.clone();
        let _hold = probe.clone();
        let r = execute_batch(
            &d,
            &mut write,
            &mods,
            std::slice::from_ref(&it),
            no_cancel(),
            &mut |_, _| {
                // The whole point: mid-transfer, the lock is available.
                if probe.try_lock().is_ok() {
                    contended = true;
                }
            },
        )
        .await;
        assert!(r.is_ok(), "batch failed: {r:?}");
    }
    assert!(
        contended,
        "the journal lock was held for the whole download, not per write"
    );
}
/// the batch was abandoned, and saying otherwise would be a lie in the history.
#[tokio::test]
async fn spec_f063_cancelling_before_the_first_byte_is_terminal_and_cancelled() {
    let dir = crate::test_tempdir();
    let (j, _holder) = jrnl(dir.path());
    let mods = dir.path().join("mods");
    std::fs::create_dir_all(&mods).expect("mkdir");
    let d = Downloader::new(1).expect("downloader");
    let it = item(dir.path(), "a.jar", b"x");

    let cancelled = Arc::new(AtomicBool::new(true));
    let err = execute_batch(
        &d,
        &mut writer(&j),
        &mods,
        std::slice::from_ref(&it),
        cancelled,
        &mut |_, _| {},
    )
    .await
    .expect_err("must stop");
    assert!(matches!(err, BatchError::Cancelled));
    assert!(
        !mods.join("a.jar").exists(),
        "nothing should have downloaded"
    );

    let rec = j
        .discover()
        .records()
        .next()
        .cloned()
        .expect("a cancelled batch is still recorded");
    assert_eq!(rec.state, OperationState::Cancelled);
    assert!(
        j.discover().unfinished().next().is_none(),
        "a cancelled record must not be offered as unfinished"
    );
}
