//! Journal store tests: atomic write, symlink refusal, read-only discover.

use super::*;
use crate::journal::{ItemState, OperationItem, OperationKind, OperationState};
use std::path::Path;

fn item(id: &str) -> OperationItem {
    OperationItem {
        item_id: id.to_string(),
        project_id: "P1".to_string(),
        title: format!("T{id}"),
        filename: format!("{id}.jar"),
        state: ItemState::Queued,
        partial_path: String::new(),
        final_path: String::new(),
        message: String::new(),
    }
}

fn journal_in(dir: &Path) -> Journal {
    Journal::new(dir.join("operations"))
}

fn snapshot(root: &Path) -> Vec<(String, u64, u128)> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.flatten() {
            let meta = e.metadata().expect("meta");
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            out.push((
                e.file_name().to_string_lossy().into_owned(),
                meta.len(),
                modified,
            ));
        }
    }
    out.sort();
    out
}

#[test]
fn spec_f141_start_writes_canonical_record_and_leaves_no_temp() {
    let d = crate::test_tempdir();
    let j = journal_in(d.path());
    let rec = j
        .start(OperationKind::DownloadBatch, "prof", "/t", vec![item("a")])
        .expect("start");
    let root = j.root().to_path_buf();
    assert!(root.is_dir());
    let files: Vec<String> = std::fs::read_dir(&root)
        .expect("rd")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files.len(), 1, "one record, no temp: {files:?}");
    assert_eq!(files[0], format!("{}.json", rec.operation_id));
    let back = j.read(&rec.operation_id).expect("read");
    assert_eq!(back, rec);
}

#[test]
fn spec_f141_write_is_atomic_over_existing_record() {
    let d = crate::test_tempdir();
    let j = journal_in(d.path());
    let rec = j
        .start(OperationKind::DownloadBatch, "prof", "/t", vec![item("a")])
        .expect("start");
    j.transition_operation(&rec.operation_id, OperationState::Running, None)
        .expect("run");
    let back = j.read(&rec.operation_id).expect("read");
    assert_eq!(back.state, OperationState::Running);
    assert_eq!(back.revision, 2);
    let files: Vec<String> = std::fs::read_dir(j.root())
        .expect("rd")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files.len(), 1, "no temp left behind: {files:?}");
}

#[test]
fn spec_f142_revision_conflict_is_reported() {
    let d = crate::test_tempdir();
    let j = journal_in(d.path());
    let rec = j
        .start(OperationKind::DownloadBatch, "prof", "/t", vec![item("a")])
        .expect("start");
    j.transition_operation(&rec.operation_id, OperationState::Running, Some(1))
        .expect("run");
    let err = j
        .transition_operation(&rec.operation_id, OperationState::Completed, Some(1))
        .expect_err("stale");
    assert!(matches!(err, JournalError::Io(m) if m.contains("revision conflict")));
    // Correct expectation still works.
    j.transition_operation(&rec.operation_id, OperationState::Completed, Some(2))
        .expect("fresh");
}

#[test]
fn spec_f143_discover_never_creates_the_root() {
    let d = crate::test_tempdir();
    let j = journal_in(d.path());
    assert!(!j.root().exists());
    let found = j.discover();
    assert!(found.outcomes.is_empty());
    assert!(!j.root().exists(), "discover must not create the root");
}

#[test]
fn spec_f143_discover_is_read_only_and_casefold_sorted() {
    let d = crate::test_tempdir();
    let j = journal_in(d.path());
    for _ in 0..3 {
        j.start(OperationKind::DownloadBatch, "p", "/t", vec![item("a")])
            .expect("start");
    }
    let before = snapshot(j.root());
    let found = j.discover();
    assert_eq!(found.records().count(), 3);
    assert_eq!(snapshot(j.root()), before, "bytes+mtime unchanged");
    let ids: Vec<String> = found.records().map(|r| r.operation_id.clone()).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted, "casefold-sorted by filename");
}

#[test]
fn spec_f143_bad_record_becomes_an_issue_not_a_panic() {
    let d = crate::test_tempdir();
    let j = journal_in(d.path());
    j.start(OperationKind::DownloadBatch, "p", "/t", vec![item("a")])
        .expect("start");
    // A file whose name does not match any record id.
    let bogus = uuid::Uuid::new_v4().to_string();
    std::fs::write(j.root().join(format!("{bogus}.json")), b"{not json").expect("write");
    let found = j.discover();
    assert_eq!(found.records().count(), 1);
    let issues: Vec<&DiscoveryIssue> = found.issues().collect();
    assert_eq!(issues.len(), 1);
    assert!(issues[0].message.contains("parse"));
    // Privacy: no absolute path leaks into the issue message.
    let leak = d.path().to_string_lossy().into_owned();
    assert!(!issues[0].message.contains(&leak), "{}", issues[0].message);
    assert!(!issues[0].message.contains('\\'), "{}", issues[0].message);
}

#[test]
fn spec_f143_filename_must_match_operation_id() {
    let d = crate::test_tempdir();
    let j = journal_in(d.path());
    let rec = j
        .start(OperationKind::DownloadBatch, "p", "/t", vec![item("a")])
        .expect("start");
    // Rename the file so name ≠ record id.
    let other = uuid::Uuid::new_v4().to_string();
    std::fs::rename(
        j.root().join(format!("{}.json", rec.operation_id)),
        j.root().join(format!("{other}.json")),
    )
    .expect("rename");
    let found = j.discover();
    assert_eq!(found.records().count(), 0);
    assert_eq!(found.issues().count(), 1);
}

#[test]
fn spec_f143_unfinished_filter() {
    let d = crate::test_tempdir();
    let j = journal_in(d.path());
    let running = j
        .start(OperationKind::DownloadBatch, "p", "/t", vec![item("a")])
        .expect("start");
    j.transition_operation(&running.operation_id, OperationState::Running, None)
        .expect("run");
    let done = j
        .start(OperationKind::UpdateInstall, "p", "/t", vec![item("b")])
        .expect("start");
    j.transition_operation(&done.operation_id, OperationState::Running, None)
        .expect("run");
    j.transition_operation(&done.operation_id, OperationState::Completed, None)
        .expect("done");
    let found = j.discover();
    let ids: Vec<&str> = found
        .unfinished()
        .map(|r| r.operation_id.as_str())
        .collect();
    assert_eq!(ids, vec![running.operation_id.as_str()]);
}

#[test]
fn spec_f141_symlink_root_is_refused_for_writes() {
    let d = crate::test_tempdir();
    let real = d.path().join("real-ops");
    std::fs::create_dir_all(&real).expect("dir");
    let link = d.path().join("linked-ops");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real, &link).expect("symlink");
    #[cfg(windows)]
    match std::os::windows::fs::symlink_dir(&real, &link) {
        Ok(()) => {}
        Err(_) => return, // privilege not granted on this host
    }
    let j = Journal::new(&link);
    let err = j
        .start(OperationKind::DownloadBatch, "p", "/t", vec![item("a")])
        .expect_err("refused");
    assert!(matches!(err, JournalError::Io(m) if m.contains("symlink")));
    // Real dir untouched.
    assert!(std::fs::read_dir(&real).expect("rd").next().is_none());
}

#[test]
fn spec_f141_symlink_root_is_not_followed_by_discover() {
    let d = crate::test_tempdir();
    let real = d.path().join("real-ops");
    let j = journal_in(&real);
    j.start(OperationKind::DownloadBatch, "p", "/t", vec![item("a")])
        .expect("start");
    let link = d.path().join("linked-ops");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real, &link).expect("symlink");
    #[cfg(windows)]
    match std::os::windows::fs::symlink_dir(&real, &link) {
        Ok(()) => {}
        Err(_) => return,
    }
    let found = Journal::new(&link).discover();
    assert_eq!(found.records().count(), 0);
    assert_eq!(found.issues().count(), 1);
    assert!(found
        .issues()
        .next()
        .expect("issue")
        .message
        .contains("symlink"));
}

#[test]
fn spec_f141_oversize_record_is_refused_on_read() {
    let d = crate::test_tempdir();
    let j = journal_in(d.path());
    let id = uuid::Uuid::new_v4().to_string();
    let big = vec![b'x'; (MAX_OPERATION_JOURNAL_BYTES + 1) as usize];
    std::fs::create_dir_all(j.root()).expect("root");
    std::fs::write(j.root().join(format!("{id}.json")), big).expect("write");
    let err = j.read(&id).expect_err("too large");
    assert!(matches!(err, JournalError::Io(m) if m.contains("too large")));
    // Discover reports it as an issue, still no panic.
    let found = j.discover();
    assert_eq!(found.issues().count(), 1);
}
