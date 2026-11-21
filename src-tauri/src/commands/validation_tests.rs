//! F-095 through the bridge: the row is the input, and the report crosses IPC
//! as stable codes plus the datums each check read. Nothing here needs a
//! webview; the folder is a temporary directory.

use super::validate_instance_at;
use crate::dto::ErrorCode;
use crate::state::AppState;
use mm_core::db::instances::{insert, InstanceRow, InstanceSource};
use std::path::{Path, PathBuf};

fn state_with(
    root: &Path,
    version: Option<&str>,
    loader: Option<&str>,
) -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = mm_core::db::open(&dir.path().join("app.sqlite3")).expect("open db");
    insert(
        &db,
        &InstanceRow {
            id: "i1".to_string(),
            name: "Survival".to_string(),
            path: std::fs::canonicalize(root).expect("canonical"),
            minecraft_root: PathBuf::new(),
            minecraft_version: version.map(str::to_string),
            loader: loader.map(str::to_string),
            source: InstanceSource::Manual,
            created_at: 1,
            scanned_at: None,
        },
    )
    .expect("seed row");
    (dir, AppState::from_db(db))
}

/// F-095: the report crosses as data. Every check is a stable code, the
/// overall status is the worst row, and a linked folder with both standard
/// directories produces no error.
#[test]
fn spec_f095_validate_instance_over_ipc_returns_the_coded_report() {
    tauri::async_runtime::block_on(async {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("Survival");
        std::fs::create_dir_all(root.join("mods")).expect("mods");
        std::fs::create_dir_all(root.join("config")).expect("config");
        let (_db_dir, state) = state_with(&root, Some("1.21.1"), Some("fabric"));

        let report = validate_instance_at(&state, "i1").await.expect("report");

        // A report is a subset of the named checks: entry checks only run when
        // a lock was readable, so the contract is "known codes, once each",
        // not a fixed row count.
        let known: Vec<&str> = mm_core::instances::validation::ValidationCheck::ALL
            .iter()
            .map(|check| check.as_str())
            .collect();
        let mut seen: Vec<&str> = Vec::new();
        for item in &report.items {
            assert!(
                known.contains(&item.check.as_str()),
                "unknown check code {:?}",
                item.check
            );
            assert!(
                !seen.contains(&item.check.as_str()),
                "{} was reported twice",
                item.check
            );
            seen.push(item.check.as_str());
        }
        assert!(
            report
                .items
                .iter()
                .any(|item| item.check == "modsDir" && item.severity == "ok"),
            "the mods folder exists"
        );
        assert!(
            report
                .items
                .iter()
                .any(|item| item.check == "minecraftVersion"
                    && item.value.as_deref() == Some("1.21.1")),
            "the check names the version it read"
        );
        assert!(
            !report.items.iter().any(|item| item.severity == "error"),
            "a linked folder with both standard directories has no error: {:?}",
            report.items
        );
        assert_ne!(report.overall, "error");
        assert!(report.checked_at > 0, "the report says when it ran");
    });
}

/// F-095: an id the registry does not hold is `NotFound`, not an empty report
/// that reads as "everything is fine".
#[test]
fn spec_f095_an_unknown_instance_id_is_not_found() {
    tauri::async_runtime::block_on(async {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("Instance");
        std::fs::create_dir_all(&root).expect("root");
        let (_db_dir, state) = state_with(&root, Some("1.21.1"), Some("fabric"));

        let error = validate_instance_at(&state, "missing")
            .await
            .expect_err("must refuse");
        assert_eq!(error.code, ErrorCode::NotFound);
    });
}
