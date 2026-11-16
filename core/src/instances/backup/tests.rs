//! Every F-113/F-114/F-115/F-118 behaviour, one test per rule.
//!
//! Fixtures live in temporary directories: no test touches a real instance.

use super::*;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, UNIX_EPOCH};

/// 2023-11-14T22:13:20Z, fixed so a folder name is an assertion, not a guess.
fn now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

fn subject() -> BackupSubject {
    BackupSubject {
        instance_id: "i1".to_string(),
        name: "Survival".to_string(),
        minecraft_version: Some("1.21.1".to_string()),
        loader: Some("fabric".to_string()),
    }
}

/// An instance with one mod and one config file.
fn instance() -> tempfile::TempDir {
    let dir = crate::test_tempdir();
    std::fs::create_dir_all(dir.path().join("mods")).expect("mods");
    std::fs::write(dir.path().join("mods/sodium.jar"), b"jar").expect("mod");
    std::fs::create_dir_all(dir.path().join("config")).expect("config");
    std::fs::write(dir.path().join("config/options.txt"), b"cfg").expect("cfg");
    dir
}

fn backup(instance: &Path, include_saves: bool) -> BackupRun {
    let plan = build_backup_plan(instance, &subject(), include_saves, now()).expect("plan");
    execute_backup(&plan, &AtomicBool::new(false), &mut |_| {}).expect("run")
}

#[test]
fn spec_f113_backup_copies_mods_and_config_and_writes_a_manifest() {
    let dir = instance();
    let run = backup(dir.path(), false);

    assert!(
        run.backup_dir.join("mods/sodium.jar").is_file(),
        "mods was not mirrored"
    );
    assert!(
        run.backup_dir.join("config/options.txt").is_file(),
        "config was not mirrored"
    );
    assert_eq!(run.copied_files, 2);

    let manifest = manifest::read_manifest(&run.manifest_path).expect("manifest");
    assert_eq!(manifest.schema, manifest::BACKUP_SCHEMA);
    assert_eq!(manifest.instance_id, "i1");
    assert!(!manifest.include_saves);
    let mods_row = manifest
        .entries
        .iter()
        .find(|e| e.path == "mods/sodium.jar")
        .expect("a row per file");
    assert_eq!(mods_row.source, EntrySource::Mods);
    assert_eq!(mods_row.status, EntryStatus::Copied);
    assert_eq!(mods_row.size, 3);
    assert_eq!(mods_row.sha256.len(), 64, "a real SHA-256");
    assert!(
        manifest.digest_matches(),
        "the digest describes its entries"
    );
}

/// Plant a directory link, or report that this host grants neither a symlink
/// nor a junction (the repo's skip convention for privileged fixtures).
fn plant_dir_link(at: &Path, target: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, at).is_ok()
    }
    #[cfg(windows)]
    {
        if std::os::windows::fs::symlink_dir(target, at).is_ok() {
            return true;
        }
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(at)
            .arg(target)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[test]
fn spec_f113_backup_never_writes_into_the_live_instance() {
    let dir = instance();
    let mods_before = crate::instances::snapshot_tree(&dir.path().join("mods"));
    let config_before = crate::instances::snapshot_tree(&dir.path().join("config"));

    let _run = backup(dir.path(), false);

    assert_eq!(
        crate::instances::snapshot_tree(&dir.path().join("mods")),
        mods_before,
        "a backup must not touch mods/"
    );
    assert_eq!(
        crate::instances::snapshot_tree(&dir.path().join("config")),
        config_before,
        "a backup must not touch config/"
    );
}

#[test]
fn spec_f113_backup_excludes_its_own_folders_and_removed_content() {
    let dir = instance();
    std::fs::create_dir_all(
        dir.path()
            .join("mods")
            .join(".modrinth_bulk_downloader_removed_content/2019"),
    )
    .expect("removed");
    std::fs::write(
        dir.path()
            .join("mods/.modrinth_bulk_downloader_removed_content/2019/old.jar"),
        b"old",
    )
    .expect("old");
    std::fs::create_dir_all(
        dir.path()
            .join("config")
            .join(BACKUP_DIRNAME)
            .join("nested"),
    )
    .expect("nested backup");
    std::fs::write(
        dir.path()
            .join("config")
            .join(BACKUP_DIRNAME)
            .join("nested/keep.jar"),
        b"no",
    )
    .expect("no");
    std::fs::create_dir_all(dir.path().join(".mm-staging")).expect("staging");
    std::fs::write(dir.path().join(".mm-staging/x.jar"), b"no").expect("no");

    let run = backup(dir.path(), false);
    assert!(
        !run.backup_dir
            .join("mods")
            .join(".modrinth_bulk_downloader_removed_content")
            .exists(),
        "removed content must not be mirrored"
    );
    assert!(
        !run.backup_dir.join("config").join(BACKUP_DIRNAME).exists(),
        "a backup must not contain a backup"
    );
    let manifest = manifest::read_manifest(&run.manifest_path).expect("manifest");
    for row in &manifest.entries {
        for excluded in EXCLUDED_DIR_NAMES {
            assert!(
                !row.path.split('/').any(|part| part == excluded),
                "excluded name reached the manifest: {}",
                row.path
            );
        }
    }
}

#[test]
fn spec_f113_backup_skips_a_missing_folder_with_a_reason_code() {
    let dir = crate::test_tempdir();
    std::fs::create_dir_all(dir.path().join("mods")).expect("mods");
    std::fs::write(dir.path().join("mods/a.jar"), b"a").expect("a");

    let run = backup(dir.path(), false);
    let manifest = manifest::read_manifest(&run.manifest_path).expect("manifest");
    let row = manifest
        .entries
        .iter()
        .find(|e| e.source == EntrySource::Config)
        .expect("the absent group is still a row");
    assert_eq!(row.status, EntryStatus::Skipped);
    assert_eq!(row.reason_code, "source-missing");
    assert!(run
        .skipped
        .iter()
        .any(|s| s.source == EntrySource::Config && s.reason_code == "source-missing"));
}

#[test]
fn spec_f113_backup_skips_a_symlink_and_never_follows_it() {
    let dir = instance();
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&outside).expect("outside");
    std::fs::write(outside.join("secret.jar"), b"secret").expect("secret");
    let link = dir.path().join("mods/linked");
    if !plant_dir_link(&link, &outside) {
        return; // host allows neither a symlink nor a junction
    }

    let run = backup(dir.path(), false);
    assert!(
        !run.backup_dir.join("mods/linked/secret.jar").exists(),
        "the link was followed: its contents must never be copied"
    );
    let manifest = manifest::read_manifest(&run.manifest_path).expect("manifest");
    let row = manifest
        .entries
        .iter()
        .find(|e| e.path == "mods/linked")
        .expect("the refusal is recorded, not silent");
    assert_eq!(row.status, EntryStatus::Skipped);
    assert_eq!(row.reason_code, "symlink");
}

#[test]
fn spec_f113_manifest_is_written_atomically_and_leaves_no_temp() {
    let dir = instance();
    let run = backup(dir.path(), false);

    let mut leftovers = Vec::new();
    let mut stack = vec![run.backup_dir.clone()];
    while let Some(current) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                stack.push(path);
            } else if name.ends_with(".part") || name.ends_with(".tmp") {
                leftovers.push(name);
            }
        }
    }
    assert!(leftovers.is_empty(), "temporaries left: {leftovers:?}");
    assert!(run.manifest_path.is_file());
    assert_eq!(run.manifest_sha256.len(), 64);
    assert_eq!(
        manifest::manifest_digest(&run.manifest_path).expect("digest"),
        run.manifest_sha256,
        "the digest handed to a restore is the file on disk"
    );
}

fn plan_restore(root: &Path, backup_dir: &Path, include_saves: bool) -> restore::RestorePlan {
    restore::build_restore_plan(root, &subject(), backup_dir, include_saves, now()).expect("plan")
}

#[test]
fn spec_f114_restore_refuses_a_folder_outside_the_backup_root() {
    let dir = instance();
    let outside = crate::test_tempdir();
    // A perfectly good backup, in the wrong place: containment is judged
    // before the manifest is even read.
    let forged = backup(dir.path(), false);
    copy_tree(&forged.backup_dir, &outside.path().join("forged"));

    let err = restore::build_restore_plan(
        dir.path(),
        &subject(),
        &outside.path().join("forged"),
        false,
        now(),
    )
    .expect_err("outside the backup root");
    assert!(
        matches!(err, restore::RestoreError::BackupOutside),
        "got {err:?}"
    );
}

#[test]
fn spec_f114_restore_refuses_a_manifest_of_another_instance() {
    let dir = instance();
    let plan = build_backup_plan(
        dir.path(),
        &BackupSubject {
            instance_id: "i2".to_string(),
            ..subject()
        },
        false,
        now(),
    )
    .expect("plan");
    let run = execute_backup(&plan, &AtomicBool::new(false), &mut |_| {}).expect("run");

    let err = restore::build_restore_plan(dir.path(), &subject(), &run.backup_dir, false, now())
        .expect_err("another instance");
    assert!(
        matches!(err, restore::RestoreError::InstanceMismatch),
        "got {err:?}"
    );
}

/// Byte-for-byte copy of a tree, for building a "backup that is not where it
/// should be" fixture.
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("dir");
    let read = match std::fs::read_dir(from) {
        Ok(read) => read,
        Err(_) => return,
    };
    for entry in read.flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else if let Ok(bytes) = std::fs::read(entry.path()) {
            std::fs::write(target, bytes).expect("file");
        }
    }
}

#[test]
fn spec_f114_restore_requires_the_typed_word() {
    let dir = instance();
    let run = backup(dir.path(), false);
    let plan = plan_restore(dir.path(), &run.backup_dir, false);

    for wrong in ["", "restore", "RESTORE!", " RESTORE 2"] {
        let err = restore::execute_restore(&plan, wrong, &AtomicBool::new(false), &mut |_| {})
            .expect_err("a wrong word must refuse the whole run");
        assert!(
            matches!(err, restore::RestoreError::ConfirmationRequired),
            "got {err:?}"
        );
    }
    assert_eq!(
        std::fs::read(dir.path().join("mods/sodium.jar")).expect("read"),
        b"jar",
        "a refused run writes nothing"
    );
}

#[test]
fn spec_f114_restore_refuses_a_manifest_changed_after_the_preview() {
    let dir = instance();
    let run = backup(dir.path(), false);
    let plan = plan_restore(dir.path(), &run.backup_dir, false);

    // Same document, one byte longer: the entries still parse, so only the
    // freshness token can notice that the plan is about older bytes.
    let manifest_path = plan.backup_dir.join(BACKUP_MANIFEST_NAME);
    let mut bytes = std::fs::read(&manifest_path).expect("bytes");
    bytes.push(b'\n');
    std::fs::write(&manifest_path, bytes).expect("rewrite");

    let err = restore::execute_restore(
        &plan,
        CONFIRMATION_WORD,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .expect_err("a changed manifest must refuse the run");
    assert!(
        matches!(err, restore::RestoreError::ManifestChanged),
        "got {err:?}"
    );
    assert_eq!(
        std::fs::read(dir.path().join("mods/sodium.jar")).expect("read"),
        b"jar",
        "a refused run writes nothing"
    );
}

#[test]
fn spec_f114_restore_moves_the_live_target_aside_then_copies() {
    let dir = instance();
    let run = backup(dir.path(), false);
    // After the backup the live folder moves on: these bytes are the ones the
    // safety copy must keep, not bytes the restore is allowed to lose.
    std::fs::write(dir.path().join("mods/sodium.jar"), b"live-new").expect("live");
    std::fs::write(dir.path().join("mods/extra.jar"), b"extra").expect("extra");

    let plan = plan_restore(dir.path(), &run.backup_dir, false);
    let out = restore::execute_restore(
        &plan,
        CONFIRMATION_WORD,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .expect("run");

    assert_eq!(out.outcome, restore::RestoreOutcome::Complete);
    assert_eq!(
        std::fs::read(dir.path().join("mods/sodium.jar")).expect("read"),
        b"jar",
        "the backup bytes were copied back"
    );
    assert!(
        !dir.path().join("mods/extra.jar").exists(),
        "a file that is not in the backup must not survive the restore"
    );
    let safety = plan.safety_dir.join("current/mods");
    assert_eq!(
        std::fs::read(safety.join("sodium.jar")).expect("safety"),
        b"live-new",
        "the live bytes were moved aside, not overwritten"
    );
    assert_eq!(
        std::fs::read(safety.join("extra.jar")).expect("safety"),
        b"extra"
    );
    assert!(
        run.backup_dir.join("mods/sodium.jar").is_file(),
        "a restore copies: the backup stays restorable"
    );
    assert!(plan.safety_dir.join(RESTORE_MANIFEST_NAME).is_file());
}

#[test]
fn spec_f114_restore_refuses_an_existing_safety_path() {
    let dir = instance();
    let run = backup(dir.path(), false);
    let plan = plan_restore(dir.path(), &run.backup_dir, false);
    // Something already lives where the live target would be moved to. It is
    // not ours to overwrite, so the entry is refused at execution.
    std::fs::create_dir_all(plan.safety_dir.join("current/mods")).expect("safety");
    std::fs::write(plan.safety_dir.join("current/mods/keep.jar"), b"keep").expect("keep");

    let out = restore::execute_restore(
        &plan,
        CONFIRMATION_WORD,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .expect("run");
    let mods = out
        .entries
        .iter()
        .find(|e| e.source == EntrySource::Mods)
        .expect("mods row");
    assert_eq!(mods.state, restore::RestoreState::Failed);
    assert_eq!(mods.reason_code, "safety-exists");
    assert_eq!(
        std::fs::read(plan.safety_dir.join("current/mods/keep.jar")).expect("keep"),
        b"keep",
        "the existing safety path was overwritten"
    );
}

#[test]
fn spec_f114_restore_blocks_an_entry_path_outside_its_own_group() {
    let dir = instance();
    let run = backup(dir.path(), false);
    // A crafted manifest row that claims a file outside mods/. The path is
    // display only, so the plan must block the group rather than obey it.
    let mut manifest = manifest::read_manifest(&run.manifest_path).expect("manifest");
    manifest.entries.push(manifest::BackupEntry {
        source: EntrySource::Mods,
        path: "mods/../../escaped.jar".to_string(),
        kind: manifest::EntryKind::File,
        size: 3,
        sha256: "0".repeat(64),
        modified_at: 0,
        version_id: String::new(),
        status: manifest::EntryStatus::Copied,
        reason_code: String::new(),
    });
    manifest.entries_sha256 = manifest::BackupManifest::digest_of(&manifest.entries);
    manifest::write_manifest(&run.manifest_path, &manifest).expect("rewrite");

    let plan = plan_restore(dir.path(), &run.backup_dir, false);
    let mods = plan
        .entries
        .iter()
        .find(|e| e.source == EntrySource::Mods)
        .expect("mods row");
    assert!(!mods.allowed);
    assert_eq!(mods.reason_code, "entry-outside-group");

    // A hand-made plan is not trusted either: execution re-checks the target.
    let mut forged = plan.clone();
    for entry in &mut forged.entries {
        if entry.source == EntrySource::Mods {
            entry.allowed = true;
            entry.target_path = dir.path().join("elsewhere").join("mods");
        }
    }
    let out = restore::execute_restore(
        &forged,
        CONFIRMATION_WORD,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .expect("run");
    let mods = out
        .entries
        .iter()
        .find(|e| e.source == EntrySource::Mods)
        .expect("mods row");
    assert_eq!(mods.state, restore::RestoreState::Failed);
    assert_eq!(mods.reason_code, "entry-outside-group");
    assert!(
        !dir.path().join("elsewhere/mods").exists(),
        "nothing was written outside the group"
    );
}

#[test]
fn spec_f114_restore_refuses_an_entry_path_that_escapes_the_root() {
    let dir = instance();
    let run = backup(dir.path(), false);
    let pristine = std::fs::read(&run.manifest_path).expect("manifest bytes");
    // Four spellings of "outside the root", each recorded under `mods`. The
    // manifest's path is display and cross-check only, so every one of them
    // must block the group instead of becoming a write target.
    let escapes = [
        "/etc/passwd",
        "C:/Windows/System32/evil.dll",
        "mods/..\\..\\evil.jar",
        "mods//evil.jar",
    ];

    for escape in escapes {
        std::fs::write(&run.manifest_path, &pristine).expect("reset");
        let mut manifest = manifest::read_manifest(&run.manifest_path).expect("manifest");
        manifest.entries.push(manifest::BackupEntry {
            source: EntrySource::Mods,
            path: escape.to_string(),
            kind: manifest::EntryKind::File,
            size: 3,
            sha256: "0".repeat(64),
            modified_at: 0,
            version_id: String::new(),
            status: manifest::EntryStatus::Copied,
            reason_code: String::new(),
        });
        manifest.entries_sha256 = manifest::BackupManifest::digest_of(&manifest.entries);
        manifest::write_manifest(&run.manifest_path, &manifest).expect("rewrite");

        let plan = plan_restore(dir.path(), &run.backup_dir, false);
        let mods = plan
            .entries
            .iter()
            .find(|e| e.source == EntrySource::Mods)
            .expect("mods row");
        assert!(!mods.allowed, "{escape} must not become a write target");
        assert_eq!(mods.reason_code, "entry-outside-group", "{escape}");

        let out = restore::execute_restore(
            &plan,
            CONFIRMATION_WORD,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .expect("run");
        // Whatever ran wrote inside the instance: the escaped spelling never
        // became a path, and the live mods folder kept its bytes.
        for entry in &out.entries {
            assert!(
                entry.target_path.starts_with(dir.path()),
                "{escape} routed a write to {}",
                entry.target_path.display()
            );
        }
        assert!(
            out.entries.iter().all(|e| e.source != EntrySource::Mods),
            "{escape} was executed"
        );
        assert_eq!(
            std::fs::read(dir.path().join("mods/sodium.jar")).expect("read"),
            b"jar",
            "{escape} overwrote the live mods"
        );
        assert!(!dir.path().join("evil.jar").exists(), "{escape}");
        assert!(!dir.path().join("escaped.jar").exists(), "{escape}");
    }
}

#[test]
fn spec_f114_restore_rolls_a_failed_group_back_and_reports_partial() {
    let dir = instance();
    let run = backup(dir.path(), false);
    std::fs::write(dir.path().join("mods/sodium.jar"), b"live-bytes").expect("live");
    // The backup's file no longer matches the manifest: the copy loop must
    // notice mid-file, not declare success over different bytes.
    std::fs::write(run.backup_dir.join("mods/sodium.jar"), b"tampered").expect("tamper");

    let plan = plan_restore(dir.path(), &run.backup_dir, false);
    let out = restore::execute_restore(
        &plan,
        CONFIRMATION_WORD,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .expect("run");

    assert_eq!(out.outcome, restore::RestoreOutcome::Partial);
    let mods = out
        .entries
        .iter()
        .find(|e| e.source == EntrySource::Mods)
        .expect("mods row");
    assert_eq!(mods.state, restore::RestoreState::RolledBack);
    assert_eq!(mods.reason_code, "hash-mismatch");
    assert_eq!(
        std::fs::read(dir.path().join("mods/sodium.jar")).expect("live"),
        b"live-bytes",
        "the failed group was not rolled back to the safety copy"
    );
    assert!(
        dir.path()
            .join(".modrinth_bulk_downloader_restore_safety")
            .is_dir(),
        "the safety copy is the way back and must stay"
    );
}

#[test]
fn spec_f114_restore_writes_the_lock_last_and_skips_it_when_mods_failed() {
    let dir = instance();
    std::fs::write(
        dir.path().join("instance.lock.json"),
        br#"{"schema_version":1,"entries":[]}"#,
    )
    .expect("lock");
    let run = backup(dir.path(), false);
    std::fs::write(
        dir.path().join("instance.lock.json"),
        br#"{"schema_version":1,"entries":[{"project_id":"live"}]}"#,
    )
    .expect("live lock");
    std::fs::write(run.backup_dir.join("mods/sodium.jar"), b"tampered").expect("tamper");

    let plan = plan_restore(dir.path(), &run.backup_dir, false);
    let order: Vec<EntrySource> = plan.allowed_entries().map(|e| e.source).collect();
    let lock_at = order
        .iter()
        .position(|s| *s == EntrySource::Lock)
        .expect("a lock row");
    assert_eq!(
        lock_at,
        order.len() - 1,
        "the lock is restored last: {order:?}"
    );

    let out = restore::execute_restore(
        &plan,
        CONFIRMATION_WORD,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .expect("run");
    let lock = out
        .entries
        .iter()
        .find(|e| e.source == EntrySource::Lock)
        .expect("lock row");
    assert_eq!(lock.state, restore::RestoreState::DependencyFailed);
    assert_eq!(lock.reason_code, "dependency-failed");
    assert_eq!(
        std::fs::read(dir.path().join("instance.lock.json")).expect("lock"),
        br#"{"schema_version":1,"entries":[{"project_id":"live"}]}"#,
        "a lock that would vouch for missing files must not be restored"
    );
}

fn with_world() -> tempfile::TempDir {
    let dir = instance();
    std::fs::create_dir_all(dir.path().join("saves/World/datapacks")).expect("world");
    std::fs::write(dir.path().join("saves/World/level.dat"), b"level").expect("level");
    std::fs::write(dir.path().join("saves/World/datapacks/pack.zip"), b"pack").expect("pack");
    dir
}

#[test]
fn spec_f115_backup_omits_saves_by_default() {
    let dir = with_world();
    let run = backup(dir.path(), false);

    assert!(
        !run.backup_dir.join("saves").exists(),
        "saves is opt-in: nothing under it may be copied"
    );
    let manifest = manifest::read_manifest(&run.manifest_path).expect("manifest");
    assert!(!manifest.include_saves);
    let row = manifest
        .entries
        .iter()
        .find(|e| e.source == EntrySource::Saves)
        .expect("the omission is recorded, not silent");
    assert_eq!(row.status, EntryStatus::Skipped);
    assert_eq!(row.reason_code, "saves-not-included");
}

#[test]
fn spec_f115_backup_copies_saves_only_when_asked() {
    let dir = with_world();
    let run = backup(dir.path(), true);

    assert_eq!(
        std::fs::read(run.backup_dir.join("saves/World/level.dat")).expect("level"),
        b"level"
    );
    let manifest = manifest::read_manifest(&run.manifest_path).expect("manifest");
    assert!(manifest.include_saves);
    assert!(manifest
        .entries
        .iter()
        .any(|e| e.path == "saves/World/level.dat" && e.status == EntryStatus::Copied));
}

#[test]
fn spec_f115_restore_blocks_saves_entries_by_default() {
    let dir = with_world();
    let run = backup(dir.path(), true);
    std::fs::write(dir.path().join("saves/World/level.dat"), b"live-level").expect("live");

    let plan = plan_restore(dir.path(), &run.backup_dir, false);
    let saves = plan
        .entries
        .iter()
        .find(|e| e.source == EntrySource::Saves)
        .expect("saves row");
    assert!(
        !saves.allowed,
        "the manifest's include_saves does not authorise this"
    );
    assert_eq!(saves.reason_code, "saves-blocked-by-default");

    let _out = restore::execute_restore(
        &plan,
        CONFIRMATION_WORD,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .expect("run");
    assert_eq!(
        std::fs::read(dir.path().join("saves/World/level.dat")).expect("live"),
        b"live-level",
        "a blocked saves entry must not be written"
    );
}

#[test]
fn spec_f115_restore_copies_saves_only_when_asked() {
    let dir = with_world();
    let run = backup(dir.path(), true);
    std::fs::write(dir.path().join("saves/World/level.dat"), b"live-level").expect("live");

    let plan = plan_restore(dir.path(), &run.backup_dir, true);
    let out = restore::execute_restore(
        &plan,
        CONFIRMATION_WORD,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .expect("run");

    assert_eq!(out.outcome, restore::RestoreOutcome::Complete);
    assert_eq!(
        std::fs::read(dir.path().join("saves/World/level.dat")).expect("level"),
        b"level",
        "with the explicit flag, saves comes back too"
    );
    let safety = plan.safety_dir.join("current/saves/World/level.dat");
    assert_eq!(
        std::fs::read(safety).expect("safety"),
        b"live-level",
        "the previous world was moved aside, never overwritten"
    );
}

#[test]
fn spec_f118_worlds_preview_writes_nothing() {
    let dir = with_world();
    let saves = dir.path().join("saves");
    let before = crate::instances::snapshot_tree(&saves);

    let preview = worlds::preview_worlds(dir.path());

    assert_eq!(preview.worlds.len(), 1);
    assert_eq!(
        crate::instances::snapshot_tree(&saves),
        before,
        "a preview is a read"
    );
    let root_entries: Vec<String> = std::fs::read_dir(dir.path())
        .expect("ls")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        !root_entries.iter().any(|n| n.contains("restore")),
        "the preview created something: {root_entries:?}"
    );
}

#[test]
fn spec_f118_worlds_preview_lists_a_symlink_without_inspecting_it() {
    let dir = with_world();
    let outside = dir.path().join("outside-world");
    std::fs::create_dir_all(outside.join("datapacks")).expect("outside");
    std::fs::write(outside.join("level.dat"), b"outside").expect("level");
    std::fs::write(outside.join("datapacks/pack.zip"), b"pack").expect("pack");
    let link = dir.path().join("saves/Linked");
    if !plant_dir_link(&link, &outside) {
        return; // host grants neither a symlink nor a junction
    }

    let preview = worlds::preview_worlds(dir.path());
    let row = preview
        .worlds
        .iter()
        .find(|w| w.name == "Linked")
        .expect("a link is listed");
    assert_eq!(row.status, worlds::WorldStatus::Symlink);
    assert_eq!(row.datapack_count, 0, "nothing under a link was counted");
    assert!(
        !row.has_level_dat,
        "a link is not a level.dat, even when it points at one"
    );
}

#[test]
fn spec_f118_worlds_preview_counts_datapacks_and_flags_missing_level_dat() {
    let dir = with_world();
    let world = dir.path().join("saves/World/datapacks");
    std::fs::write(world.join("second.zip"), b"2").expect("second");
    std::fs::write(world.join(".hidden.zip"), b"hidden").expect("hidden");
    std::fs::create_dir_all(dir.path().join("saves/Empty/datapacks")).expect("empty");
    std::fs::create_dir_all(dir.path().join("saves/.hidden-world")).expect("dot");
    std::fs::write(dir.path().join("saves/notes.txt"), b"not a world").expect("notes");

    let preview = worlds::preview_worlds(dir.path());
    let names: Vec<String> = preview.worlds.iter().map(|w| w.name.clone()).collect();
    assert_eq!(
        names,
        vec!["Empty".to_string(), "World".to_string()],
        "dot children are excluded, plain files skipped, and the order ignores case"
    );
    let world_row = preview
        .worlds
        .iter()
        .find(|w| w.name == "World")
        .expect("world");
    assert_eq!(world_row.status, worlds::WorldStatus::Detected);
    assert!(world_row.has_level_dat);
    assert_eq!(
        world_row.datapack_count, 2,
        "the dot-prefixed child is not counted"
    );
    let empty = preview
        .worlds
        .iter()
        .find(|w| w.name == "Empty")
        .expect("empty");
    assert_eq!(empty.status, worlds::WorldStatus::MissingLevelDat);
    assert!(!empty.has_level_dat);
}

#[test]
fn spec_f118_worlds_preview_handles_a_missing_saves_folder() {
    let dir = instance();
    let preview = worlds::preview_worlds(dir.path());

    assert!(!preview.saves_exists);
    assert!(preview.worlds.is_empty());
    assert_eq!(preview.message_code, "worlds.no-saves");
    assert!(
        !dir.path().join("saves").exists(),
        "the preview must not create saves/"
    );
}

#[test]
fn spec_f114_pending_restores_reports_a_safety_folder_without_a_manifest() {
    let dir = instance();
    let stamps = dir.path().join(SAFETY_DIRNAME);
    // A restore that died between moving the target aside and copying it back.
    let died = stamps.join("2023_11_14T22_13_20Z");
    std::fs::create_dir_all(died.join("current/mods")).expect("current");
    std::fs::write(died.join("current/mods/old.jar"), b"old").expect("old");
    // A completed run: its manifest is what says "finished".
    let done = stamps.join("2023_11_14T22_20_00Z");
    std::fs::create_dir_all(done.join("current/mods")).expect("done");
    std::fs::write(done.join(RESTORE_MANIFEST_NAME), b"{}").expect("manifest");

    let pending = restore::pending_restores(dir.path());
    assert_eq!(pending.len(), 1, "a completed run is not pending");
    assert_eq!(pending[0].stamp, "2023_11_14T22_13_20Z");
    assert!(pending[0].has_current);
    assert_eq!(pending[0].groups, vec!["mods".to_string()]);
    assert_eq!(
        std::fs::read(died.join("current/mods/old.jar")).expect("old"),
        b"old",
        "the scan is read-only"
    );
}

#[test]
fn spec_f114_listing_shows_a_legacy_manifest_but_restore_refuses_it() {
    let dir = instance();
    let legacy = dir.path().join(BACKUP_DIRNAME).join("2020_01_01T00_00_00Z");
    std::fs::create_dir_all(legacy.join("mods")).expect("legacy");
    std::fs::write(legacy.join("mods/old.jar"), b"old").expect("old");
    std::fs::write(
        legacy.join(BACKUP_MANIFEST_NAME),
        br#"{"schema":"modrinth_bulk_downloader.profile_update_backup.v1",
             "created_at":"2020-01-01T00:00:00Z","profile_id":"i1","entries":[]}"#,
    )
    .expect("manifest");

    let list = list_backups(dir.path(), "i1");
    assert_eq!(list.len(), 1, "the folder the user can see is listed");
    assert_eq!(list[0].manifest_state.as_str(), "legacy");
    assert!(!list[0].instance_matches);
    assert_eq!(list[0].created_at, "2020-01-01T00:00:00Z");

    let err = restore::build_restore_plan(dir.path(), &subject(), &legacy, false, now())
        .expect_err("the old app's manifest is not an input");
    assert!(
        matches!(err, restore::RestoreError::BackupMissing),
        "got {err:?}"
    );
}
