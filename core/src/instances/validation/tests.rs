//! F-095: the report's own tests. The validator reads the filesystem, so every
//! fixture is a temporary directory — never a real instance folder.

use super::*;
use crate::instances::lock::{write_lock, LockEntry, LockFile};
use crate::instances::metadata::{mirror_path, read_mirror, write_mirror};
use std::path::Path;

fn temp() -> tempfile::TempDir {
    crate::test_tempdir()
}

fn row_at(root: &Path, version: Option<&str>, loader: Option<&str>) -> InstanceRow {
    InstanceRow {
        id: "i1".to_string(),
        name: "Survival".to_string(),
        path: root.to_path_buf(),
        minecraft_root: root.to_path_buf(),
        minecraft_version: version.map(str::to_string),
        loader: loader.map(str::to_string),
        source: InstanceSource::Managed,
        created_at: 1,
        scanned_at: None,
    }
}

/// A folder the registry would call healthy: both standard directories and a
/// mirror that agrees with the row.
fn healthy(root: &Path) -> InstanceRow {
    std::fs::create_dir_all(root.join("mods")).expect("mods");
    std::fs::create_dir_all(root.join("config")).expect("config");
    let row = row_at(root, Some("1.21.1"), Some("fabric"));
    write_mirror(root, &crate::instances::metadata::mirror_of(&row)).expect("mirror");
    row
}

fn item_of(report: &ValidationReport, check: ValidationCheck) -> &ValidationItem {
    report
        .items
        .iter()
        .find(|item| item.check == check)
        .unwrap_or_else(|| panic!("{check:?} is missing from the report"))
}

fn lock_entry(filename: &str) -> LockEntry {
    LockEntry {
        provider: "modrinth".to_string(),
        project_id: "proj".to_string(),
        slug: String::new(),
        version_id: "v1".to_string(),
        filename: filename.to_string(),
        sha1: "abc".to_string(),
        sha512: "def".to_string(),
        game_versions: vec!["1.21.1".to_string()],
    }
}

/// F-095: the report is a fixed set of named checks, each emitted once, and a
/// healthy folder produces no error. The count is part of the contract — the
/// old report had thirty rows and the UI groups them by severity.
#[test]
fn spec_f095_the_report_has_thirty_named_checks_and_reports_each_once() {
    let dir = temp();
    let root = dir.path().join("Survival");
    let row = healthy(&root);

    let report = validate_instance(&row);

    assert_eq!(ValidationCheck::ALL.len(), 30, "thirty named checks");
    let mut seen: Vec<ValidationCheck> = Vec::new();
    for item in &report.items {
        assert!(
            !seen.contains(&item.check),
            "{:?} was reported twice",
            item.check
        );
        seen.push(item.check);
    }
    for check in [
        ValidationCheck::InstanceRoot,
        ValidationCheck::MinecraftVersion,
        ValidationCheck::Loader,
        ValidationCheck::ModsDir,
        ValidationCheck::ConfigDir,
        ValidationCheck::ProfileJson,
        ValidationCheck::Lock,
        ValidationCheck::ModsFolderFiles,
        ValidationCheck::LoaderSource,
    ] {
        assert!(seen.contains(&check), "{check:?} is missing");
    }
    assert_ne!(
        report.overall,
        Severity::Error,
        "a healthy folder has no error"
    );
    assert_eq!(
        item_of(&report, ValidationCheck::MinecraftVersion).severity,
        Severity::Ok
    );
    assert_eq!(
        item_of(&report, ValidationCheck::ProfileJson).severity,
        Severity::Ok,
        "the mirror agrees with the row"
    );
}

/// F-095: the ladder, and the check that reads the folder the install depends
/// on. A missing `mods/` is an error; the optional `config/` is not.
#[test]
fn spec_f095_a_missing_mods_folder_is_an_error_and_raises_the_overall_status() {
    let dir = temp();
    let root = dir.path().join("Broken");
    std::fs::create_dir_all(&root).expect("root");
    let row = row_at(&root, Some("1.21.1"), Some("fabric"));

    let report = validate_instance(&row);

    assert_eq!(
        item_of(&report, ValidationCheck::ModsDir).severity,
        Severity::Error
    );
    assert_eq!(report.overall, Severity::Error);
    assert_eq!(
        report.count(Severity::Error),
        1,
        "only the mods folder is an error: {:?}",
        report.items
    );
    assert_eq!(
        item_of(&report, ValidationCheck::ConfigDir).severity,
        Severity::Warning,
        "a launcher creates config on first run"
    );
}

/// F-095: a newer lock is refused by the reader (§5.10), and the report says so
/// instead of quietly showing "nothing recorded".
#[test]
fn spec_f095_a_newer_lock_schema_is_an_error_not_a_silent_empty_list() {
    let dir = temp();
    let root = dir.path().join("Newer");
    let row = healthy(&root);
    std::fs::write(
        root.join(LOCK_FILE_NAME),
        r#"{"schema_version":99,"entries":[]}"#,
    )
    .expect("write lock");

    let report = validate_instance(&row);

    let lock = item_of(&report, ValidationCheck::Lock);
    assert_eq!(lock.severity, Severity::Error);
    assert_eq!(lock.value.as_deref(), Some("99"));
    assert_eq!(report.overall, Severity::Error);
    assert!(
        !report
            .items
            .iter()
            .any(|item| item.check == ValidationCheck::LockMissingFiles),
        "entry checks cannot run on a lock this build refused"
    );
}

/// F-095: validation is read-only. Nothing may be created, renamed, moved or
/// rewritten — not even a `.part` or a temp — so the tree is byte-for-byte the
/// same after a full pass.
#[test]
fn spec_f095_validation_is_read_only() {
    let dir = temp();
    let root = dir.path().join("ReadOnly");
    let row = healthy(&root);
    std::fs::write(root.join("mods").join("a.jar"), b"not a zip").expect("jar");
    write_lock(
        &root.join(LOCK_FILE_NAME),
        &LockFile {
            entries: vec![lock_entry("a.jar")],
        },
    )
    .expect("lock");

    let before = crate::instances::snapshot_tree(&root);
    let report = validate_instance(&row);
    let after = crate::instances::snapshot_tree(&root);

    assert!(!report.items.is_empty());
    assert_eq!(before, after, "validation changed the instance tree");
}

/// F-095: the two file checks the old report drew from the manifest. An
/// unrecorded file is a warning; a record with no file is a different warning,
/// and "no lock at all" is only information.
#[test]
fn spec_f095_an_external_file_is_a_warning_and_a_missing_record_is_a_warning() {
    let dir = temp();
    let root = dir.path().join("Files");
    let row = healthy(&root);
    std::fs::write(root.join("mods").join("external.jar"), b"bytes").expect("jar");

    let report = validate_instance(&row);
    let external = item_of(&report, ValidationCheck::ExternalFiles);
    assert_eq!(external.severity, Severity::Warning);
    assert_eq!(external.count, Some(1));
    assert_eq!(external.value.as_deref(), Some("external.jar"));
    assert_eq!(
        item_of(&report, ValidationCheck::Lock).severity,
        Severity::Info,
        "no lock is a state, not a failure"
    );

    write_lock(
        &root.join(LOCK_FILE_NAME),
        &LockFile {
            entries: vec![lock_entry("gone.jar")],
        },
    )
    .expect("lock");
    let report = validate_instance(&row);
    let missing = item_of(&report, ValidationCheck::LockMissingFiles);
    assert_eq!(missing.severity, Severity::Warning);
    assert_eq!(missing.value.as_deref(), Some("gone.jar"));
}

/// F-095: the portable mirror is only a mirror. When it disagrees with the row
/// the report says which field drifted, and the row stays the truth.
#[test]
fn spec_f095_profile_json_drift_is_a_warning() {
    let dir = temp();
    let root = dir.path().join("Drift");
    let row = healthy(&root);
    let mut meta = read_mirror(&mirror_path(&root)).expect("mirror");
    meta.minecraft_version = Some("1.20.1".to_string());
    meta.loader = Some("forge".to_string());
    write_mirror(&root, &meta).expect("rewrite mirror");

    let report = validate_instance(&row);

    let drift = item_of(&report, ValidationCheck::ProfileJsonTarget);
    assert_eq!(drift.severity, Severity::Warning);
    let value = drift.value.as_deref().unwrap_or_default();
    assert!(value.contains("minecraft=1.20.1"), "{value}");
    assert!(value.contains("loader=forge"), "{value}");
}

/// F-095: the file checks are the F-097 classification, not a second opinion.
/// A jar that declares `1.20.1` in a `1.21.1` instance is incompatible, and the
/// report names it.
#[test]
fn spec_f095_an_incompatible_file_is_reported_from_the_range_judgement() {
    let dir = temp();
    let root = dir.path().join("Incompatible");
    let row = healthy(&root);
    crate::jar_reader::tests::build_jar(
        &root.join("mods").join("old.jar"),
        &[(
            "fabric.mod.json",
            br#"{"id":"tmod","depends":{"minecraft":"1.20.1"}}"#,
        )],
    );

    let report = validate_instance(&row);

    let incompatible = item_of(&report, ValidationCheck::IncompatibleFiles);
    assert_eq!(incompatible.severity, Severity::Warning);
    assert_eq!(incompatible.count, Some(1));
    assert_eq!(incompatible.value.as_deref(), Some("old.jar"));
    assert_eq!(
        item_of(&report, ValidationCheck::LoaderDeclaredByJars).severity,
        Severity::Ok,
        "the jar declares fabric, which is the instance loader"
    );
}

/// F-095: the overall status is the worst item, never an average.
#[test]
fn spec_f095_overall_status_follows_the_severity_ladder() {
    assert_eq!(overall_of(&[]), Severity::Ok);
    assert_eq!(
        overall_of(&[
            item(ValidationCheck::Loader, Severity::Ok),
            item(ValidationCheck::Lock, Severity::Info),
        ]),
        Severity::Info
    );
    assert_eq!(
        overall_of(&[
            item(ValidationCheck::Loader, Severity::Info),
            item(ValidationCheck::Lock, Severity::Warning),
        ]),
        Severity::Warning
    );
    assert_eq!(
        overall_of(&[
            item(ValidationCheck::Lock, Severity::Warning),
            item(ValidationCheck::ModsDir, Severity::Error),
        ]),
        Severity::Error
    );
}
