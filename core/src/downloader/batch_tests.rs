//! F-063: the batch drives the journal and the downloader together, so it is
//! tested against both rather than against a mock of either.

use super::*;
use crate::downloader::BatchProgress;
use crate::downloader::Downloader;
use crate::downloader::JournalOp;
use crate::journal::{ItemState, Journal, OperationKind, OperationRecord, OperationState};
use std::sync::atomic::{AtomicBool, Ordering};
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
            &mut |_: BatchProgress| {},
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
            &mut |_: BatchProgress| {},
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
        &mut |_: BatchProgress| {},
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

/// F-020: the batch reports its own totals beside the per-item events.
///
/// The old app showed ONE bar for the whole batch; per-file bytes alone cannot
/// reproduce it. The total is the sum of every item's size (`None` when one is
/// unknown), and `done` folds each settled item in — a skipped file was
/// already on disk, so its bytes count as done too.
#[tokio::test]
async fn spec_f020_batch_progress_totals_cover_all_items() {
    let dir = crate::test_tempdir();
    let (j, _holder) = jrnl(dir.path());
    let mods = dir.path().join("mods");
    std::fs::create_dir_all(&mods).expect("mkdir");
    let d = Downloader::new(2).expect("downloader");
    let server = wiremock::MockServer::start().await;
    let body_a = zip_bytes("META-INF/MANIFEST.MF", &[b'a'; 64]);
    let body_b = zip_bytes("META-INF/MANIFEST.MF", &[b'b'; 128]);
    for (name, body) in [("/a.jar", &body_a), ("/b.jar", &body_b)] {
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(name))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_bytes(body.clone())
                    .insert_header("content-type", "application/java-archive"),
            )
            .mount(&server)
            .await;
    }
    let mut a = item(dir.path(), "a.jar", &body_a);
    a.url = format!("{}/a.jar", server.uri());
    let mut b = item(dir.path(), "b.jar", &body_b);
    b.url = format!("{}/b.jar", server.uri());
    let expected_total = body_a.len() as u64 + body_b.len() as u64;

    let mut batch: Vec<BatchProgress> = Vec::new();
    let r = execute_batch(
        &d,
        &mut writer(&j),
        &mods,
        &[a, b],
        no_cancel(),
        &mut |_, _| {},
        &mut |p| batch.push(p),
    )
    .await;
    assert!(r.is_ok(), "batch failed: {r:?}");
    assert!(mods.join("a.jar").is_file());
    assert!(mods.join("b.jar").is_file());

    // The totals name the whole batch, not one file.
    assert!(
        !batch.is_empty(),
        "a batch with no batch-level event cannot show an overall bar"
    );
    for p in &batch {
        assert_eq!(p.total, Some(expected_total), "total must cover both items");
    }
    assert_eq!(batch[0].done, 0, "the batch starts with nothing done");
    assert_eq!(
        batch.last().expect("events").done,
        expected_total,
        "the last event must count every byte of both files"
    );
    // Monotone: the bar never rewinds.
    for pair in batch.windows(2) {
        assert!(pair[1].done >= pair[0].done, "batch done went backwards");
    }
}

/// F-036: a cancel that lands mid-download is its own terminal state.
///
/// The flag is flipped from inside the progress callback, so the download is
/// genuinely in flight. The batch must answer `Cancelled` — not a download
/// failure — journal the operation as cancelled, and record the in-flight
/// item as cancelled, because the history must tell a cancellation apart from
/// a failure.
#[tokio::test]
async fn spec_f036_cancelling_mid_download_is_cancelled_not_failed() {
    let dir = crate::test_tempdir();
    let (j, _holder) = jrnl(dir.path());
    let mods = dir.path().join("mods");
    std::fs::create_dir_all(&mods).expect("mkdir");
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

    let cancel = Arc::new(AtomicBool::new(false));
    let flip = cancel.clone();
    let mut flipped = false;
    let mut saw_cancelled_event = false;
    let err = execute_batch(
        &d,
        &mut writer(&j),
        &mods,
        std::slice::from_ref(&it),
        cancel,
        &mut |_id, p| {
            if let ItemProgress::Cancelled = p {
                saw_cancelled_event = true;
            }
            // Flip on the first progress event: the request is in flight.
            if !flipped {
                flipped = true;
                flip.store(true, Ordering::Relaxed);
            }
        },
        &mut |_| {},
    )
    .await
    .expect_err("a cancelled batch must stop");
    assert!(
        matches!(err, BatchError::Cancelled),
        "a mid-flight cancel must be Cancelled, got {err:?}"
    );
    assert!(
        saw_cancelled_event,
        "the in-flight item must report its own cancelled event"
    );
    assert!(
        !mods.join("a.jar").is_file(),
        "a cancelled download must not commit"
    );

    let rec = j
        .discover()
        .records()
        .next()
        .cloned()
        .expect("a cancelled batch is still recorded");
    assert_eq!(rec.state, OperationState::Cancelled);
    assert_eq!(
        rec.items[0].state,
        ItemState::Cancelled,
        "the in-flight item is cancelled, not failed"
    );
}

/// A raw loopback server with two personalities, because wiremock cannot
/// truncate a body mid-response: attempt 1 promises the whole file
/// (`content-length`) and delivers only three bytes before closing — the
/// client sees a truncated body and retries. Attempt 2 answers 200 with the
/// whole body regardless of the `Range` header — a resume the server ignores,
/// which is the restart the test exists to cover.
fn truncating_then_full_server(
    body: Vec<u8>,
) -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = std::thread::spawn(move || {
        let total = body.len();
        for attempt in 0..2 {
            let (mut stream, _) = match listener.accept() {
                Ok(pair) => pair,
                Err(_) => return,
            };
            use std::io::{Read as _, Write as _};
            // Drain the request head; the URL is irrelevant to the answer.
            let mut buf = [0u8; 8192];
            let mut got = 0;
            loop {
                match stream.read(&mut buf[got..]) {
                    Ok(0) => break,
                    Ok(n) => {
                        got += n;
                        if buf[..got].windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let head =
                format!("HTTP/1.1 200 OK\r\ncontent-length: {total}\r\nconnection: close\r\n\r\n");
            let _ = stream.write_all(head.as_bytes());
            match attempt {
                // Attempt 1: promise everything, deliver three bytes, close.
                // The client reports a truncated body and retries.
                0 => {
                    let _ = stream.write_all(&body[..3]);
                }
                // Attempt 2: the whole body, `Range` or not. A resume that
                // the server ignores is a restart from zero.
                _ => {
                    let _ = stream.write_all(&body);
                }
            }
            // Dropping the stream closes the connection either way.
        }
    });
    (addr, handle)
}

/// F-020: the batch bar must never rewind.
///
/// A cancelled run leaves a `.part`; the retry sends `Range` and a server
/// that ignores it answers 200 with the whole body — the downloader restarts
/// from zero. The old code reported `settled_bytes + p.done`, so the bar ran
/// BACKWARDS the moment the restart reset `done`. The reported `done` is a
/// high-water mark now: it may stall while the restart re-fetches, never go
/// down.
#[tokio::test]
async fn spec_f020_the_batch_bar_never_rewinds_when_a_resume_restarts() {
    let dir = crate::test_tempdir();
    let (j, _holder) = jrnl(dir.path());
    let mods = dir.path().join("mods");
    std::fs::create_dir_all(&mods).expect("mkdir");
    // A fast retry policy: the truncation forces one retry, and waiting a
    // full second of backoff for it would be noise.
    let d = crate::downloader::Downloader::with_policy(
        2,
        crate::downloader::retry::RetryPolicy {
            max_attempts: 2,
            base_secs: 0.05,
            max_secs: 0.2,
        },
    )
    .expect("downloader");

    let body = zip_bytes("META-INF/MANIFEST.MF", b"0123456789");
    let mut it = item(dir.path(), "a.jar", &body);
    let (addr, server) = truncating_then_full_server(body.clone());
    it.url = format!("http://{addr}/a.jar");

    let mut batch: Vec<BatchProgress> = Vec::new();
    let r = execute_batch(
        &d,
        &mut writer(&j),
        &mods,
        std::slice::from_ref(&it),
        no_cancel(),
        &mut |_, _| {},
        &mut |p| batch.push(p),
    )
    .await;
    let _ = server.join();
    assert!(r.is_ok(), "the restart must still land the file: {r:?}");
    assert_eq!(
        std::fs::read(mods.join("a.jar")).expect("read"),
        body,
        "the second attempt delivered the whole file"
    );

    // The bar itself: every event after the first must be >= its predecessor,
    // restart or no restart — and the batch ends on the terminal event.
    assert!(batch.len() > 2, "the restart produced no events: {batch:?}");
    for pair in batch.windows(2) {
        assert!(
            pair[1].done >= pair[0].done,
            "the batch bar rewound: {} -> {}",
            pair[0].done,
            pair[1].done
        );
    }
    let last = batch.last().expect("events");
    assert!(last.terminal, "a settled batch ends on the terminal event");
    assert_eq!(
        last.done,
        body.len() as u64,
        "the bar ends where the batch ended"
    );
    assert!(
        !batch.iter().take(batch.len() - 1).any(|p| p.terminal),
        "only the final event is terminal"
    );
}

/// F-020: the terminal batch event fires on the FAILURE path too.
///
/// A batch that stops early must still tell the bar it stopped: without a
/// terminal event the throttle can hold the last progress event back and the
/// bar hangs where it last crossed.
#[tokio::test]
async fn spec_f020_a_failed_batch_still_emits_a_terminal_event() {
    let dir = crate::test_tempdir();
    let (j, _holder) = jrnl(dir.path());
    let mods = dir.path().join("mods");
    std::fs::create_dir_all(&mods).expect("mkdir");
    let d = Downloader::new(1).expect("downloader");
    // The URL is refused by DNS: the download fails without any server in
    // the way, which is the failure path this test needs.
    let mut it = item(dir.path(), "a.jar", b"x");
    it.url = "https://downloads.invalid/a.jar".to_string();

    let mut batch: Vec<BatchProgress> = Vec::new();
    let err = execute_batch(
        &d,
        &mut writer(&j),
        &mods,
        std::slice::from_ref(&it),
        no_cancel(),
        &mut |_, _| {},
        &mut |p| batch.push(p),
    )
    .await
    .expect_err("the batch must fail");
    assert!(matches!(
        err,
        BatchError::Download { .. } | BatchError::Journal(_)
    ));

    let last = batch.last().expect("the batch reported something");
    assert!(
        last.terminal,
        "a failed batch must end on a terminal event: {batch:?}"
    );
}

/// F-020: the terminal event fires on the CANCEL path too — the bar must not
/// hang on a batch the user stopped.
#[tokio::test]
async fn spec_f020_a_cancelled_batch_still_emits_a_terminal_event() {
    let dir = crate::test_tempdir();
    let (j, _holder) = jrnl(dir.path());
    let mods = dir.path().join("mods");
    std::fs::create_dir_all(&mods).expect("mkdir");
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

    let cancel = Arc::new(AtomicBool::new(false));
    let flip = cancel.clone();
    let mut batch: Vec<BatchProgress> = Vec::new();
    let mut flipped = false;
    let err = execute_batch(
        &d,
        &mut writer(&j),
        &mods,
        std::slice::from_ref(&it),
        cancel,
        &mut |_, _| {
            // Flip on the first progress event: the request is in flight.
            if !flipped {
                flipped = true;
                flip.store(true, Ordering::Relaxed);
            }
        },
        &mut |p| batch.push(p),
    )
    .await
    .expect_err("a cancelled batch must stop");
    assert!(matches!(err, BatchError::Cancelled));

    let last = batch.last().expect("the batch reported something");
    assert!(
        last.terminal,
        "a cancelled batch must end on a terminal event: {batch:?}"
    );
}
