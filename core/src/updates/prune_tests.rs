//! Metadata-prune tests (F-130, F-131). Real lock file, real scan, real
//! bytes read back: the only promise that matters is that no entry whose file
//! exists is ever removed.

use std::path::{Path, PathBuf};

use super::prune::{execute_prune, plan_prune, PruneError};
use crate::instances::lock::{read_lock, write_lock, LockEntry, LockFile, LOCK_FILE_NAME};

fn temp_root() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/test-tmp");
    std::fs::create_dir_all(&base).expect("test root");
    tempfile::Builder::new()
        .prefix("mm-prune-")
        .tempdir_in(&base)
        .expect("temp dir")
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    mods: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = temp_root();
        let root = dir.path().join("instance");
        let mods = root.join("mods");
        std::fs::create_dir_all(&mods).expect("mods");
        Self {
            _dir: dir,
            root,
            mods,
        }
    }

    fn write(&self, name: &str, bytes: &[u8]) {
        let path = self.mods.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent dir");
        }
        std::fs::write(path, bytes).expect("file");
    }

    fn lock(&self, entries: Vec<LockEntry>) {
        write_lock(&self.root.join(LOCK_FILE_NAME), &LockFile { entries }).expect("lock");
    }

    fn entries(&self) -> Vec<LockEntry> {
        read_lock(&self.root.join(LOCK_FILE_NAME))
            .expect("lock")
            .entries
    }

    fn lock_bytes(&self) -> Vec<u8> {
        std::fs::read(self.root.join(LOCK_FILE_NAME)).expect("lock bytes")
    }
}

fn entry(filename: &str, project_id: &str) -> LockEntry {
    LockEntry {
        provider: "modrinth".to_string(),
        project_id: project_id.to_string(),
        slug: project_id.to_string(),
        version_id: format!("{project_id}-v1"),
        filename: filename.to_string(),
        sha1: "00".to_string(),
        sha512: String::new(),
        game_versions: Vec::new(),
    }
}

#[test]
fn spec_f130_dry_run_lists_only_lock_entries_with_no_file() {
    let fixture = Fixture::new();
    fixture.write("a.jar", b"a");
    fixture.lock(vec![entry("a.jar", "alpha"), entry("b.jar", "beta")]);
    let before = fixture.lock_bytes();

    let plan = plan_prune(&fixture.root).expect("plan");
    assert!(plan.dry_run);
    assert_eq!(
        plan.candidates
            .iter()
            .map(|c| c.filename.as_str())
            .collect::<Vec<_>>(),
        vec!["b.jar"]
    );
    assert_eq!(plan.lock_entries, 2);
    assert_eq!(plan.present_files, 1);
    // The dry run is a read: the lock's bytes did not move.
    assert_eq!(fixture.lock_bytes(), before);
}

#[test]
fn spec_f130_confirmed_prune_removes_metadata_only() {
    let fixture = Fixture::new();
    fixture.write("a.jar", b"a");
    fixture.lock(vec![entry("a.jar", "alpha"), entry("b.jar", "beta")]);

    let plan = plan_prune(&fixture.root).expect("plan");
    let report = execute_prune(&fixture.root, plan.confirm()).expect("execute");

    assert_eq!(report.removed, vec!["b.jar".to_string()]);
    assert!(report.kept_present.is_empty());
    assert_eq!(report.lock_entries_before, 2);
    assert_eq!(report.lock_entries_after, 1);
    assert_eq!(
        fixture
            .entries()
            .iter()
            .map(|e| e.filename.as_str())
            .collect::<Vec<_>>(),
        vec!["a.jar"]
    );
    // Metadata only: the surviving jar was not renamed, moved or rewritten.
    assert_eq!(
        std::fs::read(fixture.mods.join("a.jar")).expect("a.jar"),
        b"a"
    );
}

#[test]
fn spec_f130_prune_keeps_an_entry_whose_file_reappeared() {
    let fixture = Fixture::new();
    fixture.write("a.jar", b"a");
    fixture.lock(vec![entry("a.jar", "alpha"), entry("b.jar", "beta")]);

    let plan = plan_prune(&fixture.root).expect("plan");
    assert_eq!(plan.candidates.len(), 1);

    // Between the dry run and the confirm the user restored the file. Presence
    // is a veto: the entry stays, and the report says why.
    fixture.write("b.jar", b"b");
    let report = execute_prune(&fixture.root, plan.confirm()).expect("execute");

    assert!(report.removed.is_empty());
    assert_eq!(report.kept_present, vec!["b.jar".to_string()]);
    assert_eq!(fixture.entries().len(), 2);
}

#[test]
fn spec_f130_prune_with_no_candidates_writes_nothing() {
    let fixture = Fixture::new();
    fixture.write("a.jar", b"a");
    fixture.lock(vec![entry("a.jar", "alpha")]);
    let before = fixture.lock_bytes();

    let plan = plan_prune(&fixture.root).expect("plan");
    assert!(plan.is_empty());
    let report = execute_prune(&fixture.root, plan.confirm()).expect("execute");

    assert!(report.removed.is_empty());
    assert_eq!(
        fixture.lock_bytes(),
        before,
        "an empty prune must not rewrite the lock"
    );
}

#[test]
fn spec_f130_execute_refuses_a_record_the_dry_run_never_showed() {
    // Between the preview and the confirm a file vanished. The fresh plan now
    // has a candidate the user never saw; removing it would delete a record
    // outside the reviewed set.
    let fixture = Fixture::new();
    fixture.write("a.jar", b"a");
    fixture.write("b.jar", b"b");
    fixture.lock(vec![entry("a.jar", "alpha"), entry("b.jar", "beta")]);
    let plan = plan_prune(&fixture.root).expect("plan");
    assert!(plan.candidates.is_empty(), "both files are still present");

    std::fs::remove_file(fixture.mods.join("b.jar")).expect("remove b.jar");
    let error = execute_prune(&fixture.root, plan.confirm()).expect_err("must refuse");
    assert!(matches!(error, PruneError::LockChanged));
    assert_eq!(fixture.entries().len(), 2, "nothing was removed");
}

#[test]
fn spec_f130_prune_refuses_a_lock_that_moved_since_the_dry_run() {
    let fixture = Fixture::new();
    fixture.write("a.jar", b"a");
    fixture.lock(vec![entry("a.jar", "alpha"), entry("b.jar", "beta")]);
    let plan = plan_prune(&fixture.root).expect("plan");

    // Another writer added a record after the user saw the plan.
    fixture.lock(vec![
        entry("a.jar", "alpha"),
        entry("b.jar", "beta"),
        entry("c.jar", "gamma"),
    ]);

    let error = execute_prune(&fixture.root, plan.confirm()).expect_err("must refuse");
    assert!(matches!(error, PruneError::LockChanged));
    assert_eq!(fixture.entries().len(), 3, "nothing was removed");
}

#[test]
fn spec_f131_prune_never_names_an_unknown_present_file() {
    let fixture = Fixture::new();
    fixture.write("a.jar", b"a");
    fixture.write("hand-installed.jar", b"hand made");
    fixture.lock(vec![entry("a.jar", "alpha")]);

    let plan = plan_prune(&fixture.root).expect("plan");
    assert!(
        plan.candidates.is_empty(),
        "unknown files are not lock entries and cannot be pruned"
    );
    let report = execute_prune(&fixture.root, plan.confirm()).expect("execute");
    assert!(report.removed.is_empty());
    assert!(fixture.mods.join("hand-installed.jar").is_file());
}

#[test]
fn spec_f131_prune_keeps_a_record_whose_jar_moved_into_a_subfolder() {
    let fixture = Fixture::new();
    fixture.write("sub/b.jar", b"b");
    fixture.lock(vec![entry("b.jar", "beta")]);

    let plan = plan_prune(&fixture.root).expect("plan");
    assert!(
        plan.candidates.is_empty(),
        "a jar under mods/<sub>/ is still present"
    );
}

#[test]
fn spec_f131_a_disabled_file_is_present_for_prune() {
    let fixture = Fixture::new();
    fixture.write("b.jar.disabled", b"b");
    fixture.lock(vec![entry("b.jar", "beta")]);

    let plan = plan_prune(&fixture.root).expect("plan");
    assert!(
        plan.candidates.is_empty(),
        "a disabled jar is the recorded file, not a missing one"
    );
}
