//! The prune bridge: a confirm is bound to the dry run the user saw.
//!
//! The command rebuilds the plan server-side, and the digest it compares covers
//! the lock's bytes **and** the candidate names. A lock or a folder that moved
//! after the preview produces a different digest, so the write is refused
//! instead of trusting the webview's list.

use std::path::{Path, PathBuf};

use super::target::execute_lock_prune_at;
use crate::dto::{ErrorCode, ExecutePruneRequest};
use mm_core::instances::{read_lock, write_lock, LockEntry, LockFile, LOCK_FILE_NAME};
use mm_core::updates::prune::plan_prune;

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    mods: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("instance");
        let mods = root.join("mods");
        std::fs::create_dir_all(&mods).expect("mods");
        Self {
            _dir: dir,
            root,
            mods,
        }
    }

    fn write(&self, name: &str) {
        std::fs::write(self.mods.join(name), name.as_bytes()).expect("file");
    }

    fn lock(&self, names: &[&str]) {
        let entries = names
            .iter()
            .map(|name| LockEntry {
                provider: "modrinth".to_string(),
                project_id: name.trim_end_matches(".jar").to_string(),
                slug: String::new(),
                version_id: String::new(),
                filename: (*name).to_string(),
                sha1: String::new(),
                sha512: String::new(),
                game_versions: Vec::new(),
            })
            .collect();
        write_lock(&self.root.join(LOCK_FILE_NAME), &LockFile { entries }).expect("lock");
    }

    fn entries(&self) -> usize {
        read_lock(&self.root.join(LOCK_FILE_NAME))
            .expect("lock")
            .entries
            .len()
    }

    fn request(&self, digest: &str) -> ExecutePruneRequest {
        ExecutePruneRequest {
            instance_root: self.root.to_string_lossy().into_owned(),
            expected_digest: digest.to_string(),
        }
    }
}

fn preview_digest(root: &Path) -> String {
    plan_prune(root).expect("preview").digest().to_string()
}

#[test]
fn spec_f130_bridge_refuses_a_digest_that_is_not_the_previewed_plan() {
    let fixture = Fixture::new();
    fixture.write("a.jar");
    fixture.lock(&["a.jar", "b.jar"]);

    let error = execute_lock_prune_at(&fixture.root, &fixture.request("not-the-preview"))
        .expect_err("a foreign token must be refused");

    assert_eq!(error.code, ErrorCode::UpdateContextChanged);
    assert_eq!(fixture.entries(), 2, "a refused confirm removes nothing");
}

#[test]
fn spec_f130_bridge_refuses_a_record_the_preview_never_showed() {
    let fixture = Fixture::new();
    fixture.write("a.jar");
    fixture.write("b.jar");
    fixture.lock(&["a.jar", "b.jar"]);
    let digest = preview_digest(&fixture.root);

    // The file vanishes after the preview: its record is now a candidate the
    // user never reviewed.
    std::fs::remove_file(fixture.mods.join("b.jar")).expect("remove b.jar");
    let error = execute_lock_prune_at(&fixture.root, &fixture.request(&digest))
        .expect_err("an unreviewed candidate must be refused");

    assert_eq!(error.code, ErrorCode::UpdateContextChanged);
    assert_eq!(fixture.entries(), 2, "nothing was removed");
}

#[test]
fn spec_f130_bridge_executes_the_previewed_plan() {
    let fixture = Fixture::new();
    fixture.write("a.jar");
    fixture.lock(&["a.jar", "b.jar"]);
    let digest = preview_digest(&fixture.root);

    let report = execute_lock_prune_at(&fixture.root, &fixture.request(&digest)).expect("execute");

    assert_eq!(report.removed, vec!["b.jar".to_string()]);
    assert_eq!(fixture.entries(), 1);
}
