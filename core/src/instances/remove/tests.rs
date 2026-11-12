//! Safe removal tests: guards, typed confirmation, backup + manifest.
//!
//! Every fixture lives in a temp directory; no test touches a real mod folder.

use super::*;
use std::time::Duration;

fn fixture() -> tempfile::TempDir {
    let d = crate::test_tempdir();
    std::fs::create_dir_all(d.path().join("mods")).expect("mods");
    d
}

fn touch(dir: &Path, rel: &str) -> PathBuf {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).expect("parent");
    }
    std::fs::write(&p, b"jar-bytes").expect("write");
    p
}

/// 2023-11-14T22:13:20Z, a fixed instant so folder names are deterministic.
fn now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

fn plan_of(mods: &Path, paths: &[PathBuf], external: bool) -> RemovePlan {
    let candidates: Vec<RemoveCandidate> = paths
        .iter()
        .map(|p| RemoveCandidate {
            path: p.clone(),
            external,
        })
        .collect();
    build_remove_preview(mods, &candidates, now())
}

#[test]
fn spec_f111_preview_refuses_unsafe_candidates() {
    let d = fixture();
    let mods = d.path().join("mods");
    let ok = touch(&mods, "a.jar");
    let disabled = touch(&mods, "b.jar.disabled");
    let nested = touch(&mods, "disabled_mods/c.jar");
    let txt = touch(&mods, "notes.txt");
    let outside = touch(d.path(), "outside.jar");
    let backup_file = touch(
        &mods,
        ".modrinth_bulk_downloader_removed_content/2020/files/old.jar",
    );
    let missing = mods.join("missing.jar");

    let plan = plan_of(
        &mods,
        &[ok, disabled, nested, txt, outside, missing, backup_file],
        false,
    );
    let by_name = |name: &str| {
        plan.entries
            .iter()
            .find(|e| e.source.file_name().and_then(|n| n.to_str()) == Some(name))
            .expect("entry")
    };

    assert!(by_name("a.jar").allowed);
    assert!(by_name("b.jar.disabled").allowed);
    // `disabled_mods/` is part of the scan, so removal must keep its relative
    // path inside the backup instead of collapsing to a bare filename.
    assert!(by_name("c.jar").allowed);
    assert_eq!(
        by_name("c.jar")
            .backup_path
            .strip_prefix(&plan.backup_dir)
            .expect("inside backup"),
        Path::new("files").join("disabled_mods").join("c.jar")
    );
    assert!(!by_name("notes.txt").allowed);
    assert_eq!(by_name("notes.txt").reason_code, "unsupported");
    assert_eq!(by_name("outside.jar").reason_code, "outside");
    assert_eq!(by_name("missing.jar").reason_code, "source-missing");
    assert_eq!(by_name("old.jar").reason_code, "backup-folder");
    assert!(!plan.requires_confirmation());
}

#[test]
fn spec_f111_missing_root_refuses_everything() {
    let d = fixture();
    let missing_mods = d.path().join("mods-not-there");
    let plan = plan_of(&missing_mods, &[missing_mods.join("a.jar")], false);
    assert_eq!(plan.entries.len(), 1);
    assert!(!plan.entries[0].allowed);
    assert_eq!(plan.entries[0].reason_code, "root-missing");
    let run = execute_remove(&plan, "").expect("no confirmation needed");
    assert!(run.results.is_empty());
    assert!(run.manifest_error.is_empty());
    assert!(!plan.backup_dir.exists(), "nothing to record, no folder");
}

#[cfg(unix)]
#[test]
fn spec_f111_symlink_source_is_refused() {
    let d = fixture();
    let mods = d.path().join("mods");
    let outside = touch(d.path(), "outside.jar");
    let link = mods.join("link.jar");
    std::os::unix::fs::symlink(&outside, &link).expect("symlink");
    let plan = plan_of(&mods, &[link], false);
    assert!(!plan.entries[0].allowed);
    assert_eq!(plan.entries[0].reason_code, "symlink");
    assert!(outside.exists(), "nothing was touched");
}

#[test]
fn spec_f112_execute_moves_to_backup_and_writes_manifest() {
    let d = fixture();
    let mods = d.path().join("mods");
    let a = touch(&mods, "a.jar");
    let b = touch(&mods, "disabled_mods/b.jar.disabled");
    let plan = plan_of(&mods, &[a.clone(), b.clone()], false);

    let run = execute_remove(&plan, "").expect("local files need no word");
    assert!(run.manifest_error.is_empty(), "{}", run.manifest_error);
    assert_eq!(run.results.len(), 2);
    assert!(run
        .results
        .iter()
        .all(|r| r.state == RemoveState::MovedToBackup));

    assert!(!a.exists(), "source left the active folder");
    assert!(!b.exists(), "nested source left the active folder");
    // The dated folder is inside the canonical mods folder (Windows prefixes
    // canonical paths with `\\?\`, hence the canonical comparison).
    assert!(plan
        .backup_dir
        .starts_with(std::fs::canonicalize(&mods).expect("canonical mods")));
    assert!(plan
        .backup_dir
        .ends_with(Path::new(REMOVE_BACKUP_DIRNAME).join("2023_11_14T22_13_20Z")));
    assert_eq!(
        std::fs::read(plan.backup_dir.join("files").join("a.jar")).expect("backup bytes"),
        b"jar-bytes"
    );
    assert_eq!(
        std::fs::read(
            plan.backup_dir
                .join("files")
                .join("disabled_mods")
                .join("b.jar.disabled")
        )
        .expect("nested backup bytes"),
        b"jar-bytes"
    );

    let manifest = std::fs::read_to_string(&run.manifest_path).expect("manifest");
    let doc: serde_json::Value = serde_json::from_str(&manifest).expect("json");
    assert_eq!(doc["schema"], REMOVE_SCHEMA);
    assert_eq!(doc["created_at"], "2023-11-14T22:13:20Z");
    assert_eq!(doc["entries"].as_array().expect("entries").len(), 2);
    assert_eq!(doc["entries"][0]["status"], "moved-to-backup");

    // The backup folder is not content: a rescans sees no files.
    assert!(crate::instances::iter_local_mod_scan_files(&mods).is_empty());
}

#[test]
fn spec_f112_external_content_requires_the_typed_word() {
    let d = fixture();
    let mods = d.path().join("mods");
    let a = touch(&mods, "unknown.jar");
    let plan = plan_of(&mods, std::slice::from_ref(&a), true);
    assert!(plan.requires_confirmation());
    assert!(plan.entries[0].requires_confirmation);

    assert_eq!(
        execute_remove(&plan, ""),
        Err(RemoveError::ConfirmationRequired)
    );
    assert_eq!(
        execute_remove(&plan, "remove"),
        Err(RemoveError::ConfirmationRequired)
    );
    assert!(a.exists(), "a refused run moves nothing");

    let run = execute_remove(&plan, " REMOVE ").expect("typed word accepted");
    assert_eq!(run.results.len(), 1);
    assert!(!a.exists());
    assert!(run.results[0].backup_path.exists());
}

#[test]
fn spec_f112_existing_backup_destination_is_not_overwritten() {
    let d = fixture();
    let mods = d.path().join("mods");
    let a = touch(&mods, "a.jar");
    let plan = plan_of(&mods, std::slice::from_ref(&a), false);
    let occupied = plan.entries[0].backup_path.clone();
    std::fs::create_dir_all(occupied.parent().expect("parent")).expect("dir");
    std::fs::write(&occupied, b"someone-else").expect("fixture");

    let run = execute_remove(&plan, "").expect("run");
    assert_eq!(run.results[0].state, RemoveState::Failed);
    assert_eq!(run.results[0].reason_code, "backup-exists");
    assert_eq!(std::fs::read(&a).expect("source intact"), b"jar-bytes");
    assert_eq!(
        std::fs::read(&occupied).expect("destination intact"),
        b"someone-else"
    );
}

#[test]
fn spec_f111_a_second_run_in_the_same_second_gets_a_new_folder() {
    let d = fixture();
    let mods = d.path().join("mods");
    let first = touch(&mods, "first.jar");
    let one = plan_of(&mods, &[first], false);
    execute_remove(&one, "").expect("first run");

    let second = touch(&mods, "second.jar");
    let two = plan_of(&mods, &[second], false);
    assert_ne!(one.backup_dir, two.backup_dir, "no shared dated folder");
    let run = execute_remove(&two, "").expect("second run");
    assert!(run.results[0].backup_path.exists());
}

#[test]
fn spec_f111_timestamp_is_utc_iso_seconds() {
    assert_eq!(utc_timestamp(UNIX_EPOCH), "1970-01-01T00:00:00Z");
    assert_eq!(utc_timestamp(now()), "2023-11-14T22:13:20Z");
    assert_eq!(backup_stamp("2023-11-14T22:13:20Z"), "2023_11_14T22_13_20Z");
}
