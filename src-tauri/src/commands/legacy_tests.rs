//! F-181 at the bridge: the one input is validated, the absolute path reaches
//! core, and the report the screen renders keeps every fact core found.

use std::fs;
use std::path::{Path, PathBuf};

use super::legacy::{
    execute_import_with, explicit_home, normalize_selection, plan_import_with, scan_with,
};
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

/// The whole write path through the bridge: a real scan, a plan, a backup and
/// a registered row. The old home is read, never written.
#[test]
fn spec_f181_bridge_plans_and_executes_with_a_real_scan() {
    let dir = tempfile::tempdir().expect("temp");
    let (home, instance) = seed_old_home(dir.path());
    let scratch = dir.path().join("scratch");
    let db = mm_core::db::open(&dir.path().join("new.db")).expect("new db");

    let plan =
        plan_import_with(&db, Some(home.clone()), &scratch, vec!["1".to_string()]).expect("plan");
    assert_eq!(plan.blocked_reason, None);
    assert_eq!((plan.ready, plan.skipped, plan.blocked), (1, 0, 0));
    assert_eq!(plan.entries.len(), 1);
    let entry = &plan.entries[0];
    assert_eq!(entry.legacy_id, "1");
    assert_eq!(entry.name, "Old one");
    assert_eq!(entry.root, instance.display().to_string());
    assert_eq!(entry.action, "register");
    assert_eq!(entry.code, "ready");
    assert_eq!(entry.source, "manual", "an empty v1 kind is not adopted");
    assert_eq!(entry.game_version.as_deref(), Some("1.20.1"));
    assert_eq!(entry.loader.as_deref(), Some("fabric"));
    assert_eq!(
        mm_core::db::instances::list(&db).expect("list").len(),
        0,
        "a plan writes nothing"
    );

    let report = execute_import_with(&db, Some(home.clone()), &scratch, vec!["1".to_string()])
        .expect("execute");
    assert_eq!(report.imported, 1);
    assert_eq!((report.skipped, report.blocked), (0, 0));
    let backup = report.backup.expect("a backup was taken");
    assert!(Path::new(&backup).is_file(), "backup missing at {backup}");
    assert_eq!(report.entries[0].code, "imported");
    assert!(report.entries[0].applied);
    assert!(report.entries[0].instance_id.is_some());

    let rows = mm_core::db::instances::list(&db).expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "Old one");
    assert_eq!(
        rows[0].source,
        mm_core::db::instances::InstanceSource::Manual
    );

    // The same selection again is a skip, not a second row and not a backup.
    let again = execute_import_with(&db, Some(home), &scratch, vec!["1".to_string()])
        .expect("second execute");
    assert_eq!(again.imported, 0);
    assert_eq!(again.backup, None);
    assert_eq!(again.entries[0].code, "already_registered");
    assert_eq!(mm_core::db::instances::list(&db).expect("list").len(), 1);

    // The snapshot copies never outlive their command.
    let leftovers: Vec<_> = fs::read_dir(&scratch)
        .expect("scratch base")
        .flatten()
        .map(|entry| entry.file_name())
        .collect();
    assert!(
        leftovers.is_empty(),
        "leftover scratch entries: {leftovers:?}"
    );
}

#[test]
fn spec_f181_bridge_refuses_an_empty_selection() {
    for empty in [vec![], vec![String::new()], vec!["   ".to_string()]] {
        let err = normalize_selection(empty).expect_err("an empty selection is malformed");
        assert_eq!(err.code, ErrorCode::InvalidInput);
    }

    // Order is kept and repeats are dropped.
    let got = normalize_selection(vec![
        " 2 ".to_string(),
        "1".to_string(),
        "2".to_string(),
        String::new(),
    ])
    .expect("selection");
    assert_eq!(got, vec!["2".to_string(), "1".to_string()]);
}

/// A newer old schema is a plan reason, not an IPC failure: the screen has a
/// sentence for it and nothing was written.
#[test]
fn spec_f181_bridge_reports_an_unsupported_database_as_a_plan_reason() {
    let dir = tempfile::tempdir().expect("temp");
    let (home, _instance) = seed_old_home(dir.path());
    {
        let connection =
            rusqlite::Connection::open(home.join("data").join("app.sqlite3")).expect("old db");
        connection
            .pragma_update(None, "user_version", 3)
            .expect("stamp a newer schema");
    }
    let scratch = dir.path().join("scratch");
    let db = mm_core::db::open(&dir.path().join("new.db")).expect("new db");

    let plan = plan_import_with(&db, Some(home), &scratch, vec!["1".to_string()]).expect("plan");

    assert_eq!(plan.blocked_reason.as_deref(), Some("database_unsupported"));
    assert!(plan.entries.is_empty());
    assert_eq!((plan.ready, plan.skipped, plan.blocked), (0, 0, 0));
}
