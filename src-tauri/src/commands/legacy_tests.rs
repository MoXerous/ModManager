//! F-181 at the bridge: the one input is validated, the absolute path reaches
//! core, and the report the screen renders keeps every fact core found.

use std::fs;
use std::path::{Path, PathBuf};

use super::legacy::{explicit_home, scan_with};
use crate::dto::ErrorCode;

const OLD_SCHEMA: &str = "
CREATE TABLE profiles (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    game_version TEXT NOT NULL,
    loader TEXT NOT NULL,
    download_dir TEXT NOT NULL,
    instance_root TEXT DEFAULT '',
    mods_dir TEXT DEFAULT '',
    profile_root TEXT DEFAULT ''
);
";

/// A small old home: one profile whose root holds both JSON documents.
fn seed_old_home(root: &Path) -> (PathBuf, PathBuf) {
    let home = root.join("old-home");
    let instance = home.join("instance");
    fs::create_dir_all(&instance).expect("instance root");
    let data = home.join("data");
    fs::create_dir_all(&data).expect("data dir");

    let connection = rusqlite::Connection::open(data.join("app.sqlite3")).expect("old db");
    connection.execute_batch(OLD_SCHEMA).expect("schema");
    connection
        .pragma_update(None, "user_version", 2)
        .expect("user_version");
    connection
        .execute(
            "INSERT INTO profiles (name, game_version, loader, download_dir, profile_root)
             VALUES ('Old one', '1.20.1', 'fabric', ?1, ?2)",
            rusqlite::params![
                instance.display().to_string(),
                instance.display().to_string()
            ],
        )
        .expect("profile row");
    drop(connection);

    fs::write(
        instance.join("profile.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema": "modrinth_bulk_downloader.profile_metadata.v1",
            "profile": {
                "name": "Old one",
                "paths": { "profile_root": instance.display().to_string() }
            }
        }))
        .expect("profile json"),
    )
    .expect("write profile.json");
    fs::write(
        instance.join("installed_mods.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema": "modrinth_bulk_downloader.installed_mods.v1",
            "managed_mods": [
                { "project_id": "P1", "version_id": "V1", "managed_status": "managed" },
                { "project_id": "P2", "version_id": "V2", "managed_status": "managed" },
                { "project_id": "", "version_id": "V3", "managed_status": "managed" }
            ]
        }))
        .expect("manifest json"),
    )
    .expect("write installed_mods.json");

    (home, instance)
}

#[test]
fn spec_f181_a_relative_home_is_refused_before_any_scan() {
    let err = explicit_home(Some("old-home".to_string())).expect_err("a relative path is refused");
    assert_eq!(err.code, ErrorCode::InvalidInput);

    // Nothing typed means "discover", not "the process directory".
    assert_eq!(explicit_home(None).expect("absent is discovery"), None);
    assert_eq!(
        explicit_home(Some("   ".to_string())).expect("blank is discovery"),
        None
    );
}

#[test]
fn spec_f181_an_absolute_home_is_scanned_and_reported_as_data() {
    let dir = tempfile::tempdir().expect("temp");
    let (home, instance) = seed_old_home(dir.path());
    let scratch = dir.path().join("scratch");

    let report = scan_with(Some(home.clone()), &scratch).expect("scan");

    assert_eq!(report.roots.source, "explicit");
    assert_eq!(
        report.roots.db_file,
        home.join("data").join("app.sqlite3").display().to_string()
    );

    let database = report.database.expect("database reported");
    assert_eq!(database.user_version, 2);
    assert!(database.supported);
    assert!(database.has_profiles_table);
    assert!(database.columns.iter().any(|c| c == "profile_root"));

    assert_eq!(report.profiles.len(), 1);
    let profile = &report.profiles[0];
    assert_eq!(profile.id, "1");
    assert_eq!(profile.name, "Old one");
    assert_eq!(profile.game_version, "1.20.1");
    assert_eq!(profile.loader, "fabric");
    assert_eq!(profile.root, instance.display().to_string());
    assert!(profile.root_exists);
    assert!(profile.profile_json.present);
    assert!(profile.profile_json.ok);
    assert_eq!(
        profile.profile_json.schema,
        "modrinth_bulk_downloader.profile_metadata.v1"
    );
    assert!(profile.installed_mods.present);
    assert!(profile.installed_mods.ok);
    assert_eq!(profile.installed_mods.managed, 2);
    assert_eq!(profile.installed_mods.skipped, 1);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);

    // The snapshot copy never outlives the report.
    let leftovers: Vec<_> = fs::read_dir(&scratch)
        .expect("scratch base was created")
        .flatten()
        .map(|entry| entry.file_name())
        .collect();
    assert!(
        leftovers.is_empty(),
        "leftover scratch entries: {leftovers:?}"
    );
}

/// An explicit home that does not exist is still an explicit answer: the scan
/// reports a missing database instead of silently falling back to the
/// machine's own old home.
#[test]
fn spec_f181_a_missing_explicit_home_is_not_replaced_by_discovery() {
    let dir = tempfile::tempdir().expect("temp");
    let missing = dir.path().join("not-there");
    let scratch = dir.path().join("scratch");

    let report = scan_with(Some(missing.clone()), &scratch).expect("scan");

    assert_eq!(report.roots.source, "explicit");
    assert!(report.database.is_none());
    assert!(report.profiles.is_empty());
    assert!(report.warnings.iter().any(|w| w.code == "database_missing"));
}

/// A file that is not a database is a warning the screen can show, not an IPC
/// failure: the rest of the old home may still be readable.
#[test]
fn spec_f181_an_unreadable_database_is_a_warning_and_still_cleans_scratch() {
    let dir = tempfile::tempdir().expect("temp");
    let home = dir.path().join("old-home");
    let data = home.join("data");
    fs::create_dir_all(&data).expect("data dir");
    fs::write(data.join("app.sqlite3"), b"not a database").expect("garbage db");
    let scratch = dir.path().join("scratch");

    let report = scan_with(Some(home), &scratch).expect("scan");

    assert!(report.database.is_none());
    assert!(report
        .warnings
        .iter()
        .any(|w| w.code == "database_unreadable"));
    let leftovers: Vec<_> = fs::read_dir(&scratch)
        .expect("scratch base was created")
        .flatten()
        .map(|entry| entry.file_name())
        .collect();
    assert!(
        leftovers.is_empty(),
        "leftover scratch entries: {leftovers:?}"
    );
}
