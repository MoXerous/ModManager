//! F-168: the watcher command's wiring, against a real folder and a real
//! watcher. The emit is a recording closure, so no webview is needed.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::dto::ContentStaleDto;
use crate::state::AppState;

/// One instance at a time: a new watch replaces the old one, so the watcher
/// of a previous instance is stopped, not left running.
#[test]
fn spec_f168_a_new_watch_replaces_the_previous_instance_watch() {
    let dir = tempfile::tempdir().expect("tempdir");
    let first = tempfile::tempdir().expect("tempdir");
    let second = tempfile::tempdir().expect("tempdir");
    let state = AppState::from_db(mm_core::db::open(&dir.path().join("t.db")).expect("db"));

    let (tx1, rx1) = std::sync::mpsc::channel();
    let root1 = first.path().to_path_buf();
    super::start_content_watch(&state, root1.clone(), move |_| {
        let _ = tx1.send(());
    })
    .expect("first watch");
    let (tx2, rx2) = std::sync::mpsc::channel();
    let root2 = second.path().to_path_buf();
    super::start_content_watch(&state, root2.clone(), move |_| {
        let _ = tx2.send(());
    })
    .expect("second watch");

    // The first watcher is gone: writing into the first instance stays
    // silent, because its watcher was stopped by the replacement.
    std::thread::sleep(Duration::from_millis(100));
    std::fs::write(first.path().join("late.jar"), b"jar").expect("write");
    let quiet = rx1.recv_timeout(Duration::from_millis(700));
    assert!(
        matches!(
            quiet,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected | _)
        ),
        "the replaced watcher must not answer"
    );
    assert!(
        matches!(quiet, Err(std::sync::mpsc::RecvTimeoutError::Disconnected)),
        "a replaced watcher is stopped and its channel closes"
    );

    // The second is the live one, and it signals with the SECOND root.
    std::fs::write(second.path().join("new.jar"), b"jar").expect("write");
    rx2.recv_timeout(Duration::from_secs(5)).expect("signal");
}

/// The event the bridge emits carries the root, and only the root.
#[test]
fn spec_f168_the_stale_signal_names_the_instance_root_only() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::from_db(mm_core::db::open(&dir.path().join("t.db")).expect("db"));
    let instance = tempfile::tempdir().expect("tempdir");
    let seen: Arc<Mutex<Vec<ContentStaleDto>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let root = instance.path().to_path_buf();
    super::start_content_watch(&state, root.clone(), move |dto| {
        if let Ok(mut guard) = sink.lock() {
            guard.push(dto);
        }
    })
    .expect("watch");
    std::thread::sleep(Duration::from_millis(100));
    std::fs::write(instance.path().join("moved.jar"), b"jar").expect("write");
    // Wait for at least one emission.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "no signal arrived in time"
        );
        let count = seen.lock().expect("seen").len();
        if count > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    for dto in seen.lock().expect("seen").iter() {
        assert_eq!(
            dto.instance_root,
            root.to_string_lossy(),
            "the event names the watched root"
        );
    }
}

/// Explicit teardown: `unwatch` stops the watcher and clears the slot.
#[test]
fn spec_f168_unwatch_stops_and_clears_the_watch() {
    let dir = tempfile::tempdir().expect("tempdir");
    let instance = tempfile::tempdir().expect("tempdir");
    let state = AppState::from_db(mm_core::db::open(&dir.path().join("t.db")).expect("db"));
    let (tx, rx) = std::sync::mpsc::channel();
    let root = instance.path().to_path_buf();
    super::start_content_watch(&state, root.clone(), move |_| {
        let _ = tx.send(());
    })
    .expect("watch");
    state.stop_content_watch();
    let closed = rx.recv_timeout(Duration::from_secs(2));
    assert!(
        matches!(closed, Err(std::sync::mpsc::RecvTimeoutError::Disconnected)),
        "teardown closes the watcher's channel, got {closed:?}"
    );
}
