//! F-181: the reader is explicit, read-only and tolerant of both old schemas.

use std::fs;
use std::path::{Path, PathBuf};

use super::discover::roots_from;
use super::json::{INSTALLED_MODS_JSON, PROFILE_JSON};
use super::*;

const V1_DDL: &str = "
CREATE TABLE profiles (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    game_version TEXT NOT NULL,
    loader TEXT NOT NULL,
    download_dir TEXT NOT NULL,
    notes TEXT DEFAULT '',
    last_download_at TEXT,
    instance_mode TEXT DEFAULT 'classic',
    instance_root TEXT DEFAULT '',
    minecraft_root TEXT DEFAULT '',
    mods_dir TEXT DEFAULT '',
    shaderpacks_dir TEXT DEFAULT '',
    resourcepacks_dir TEXT DEFAULT '',
    saves_dir TEXT DEFAULT '',
    servers_file TEXT DEFAULT '',
    config_dir TEXT DEFAULT '',
    screenshots_dir TEXT DEFAULT '',
    detected_game_version TEXT DEFAULT '',
    detected_loader TEXT DEFAULT '',
    instance_validation_status TEXT DEFAULT 'unknown',
    instance_validation_message TEXT DEFAULT '',
    allow_file_deletion_on_remove INTEGER DEFAULT 0,
    instance_last_validated_at TEXT
);
";

const V2_DDL: &str = "
CREATE TABLE profiles (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    game_version TEXT NOT NULL,
    loader TEXT NOT NULL,
    download_dir TEXT NOT NULL,
    notes TEXT DEFAULT '',
    last_download_at TEXT,
    instance_mode TEXT DEFAULT 'classic',
    instance_root TEXT DEFAULT '',
    minecraft_root TEXT DEFAULT '',
    mods_dir TEXT DEFAULT '',
    shaderpacks_dir TEXT DEFAULT '',
    resourcepacks_dir TEXT DEFAULT '',
    saves_dir TEXT DEFAULT '',
    servers_file TEXT DEFAULT '',
    config_dir TEXT DEFAULT '',
    screenshots_dir TEXT DEFAULT '',
    detected_game_version TEXT DEFAULT '',
    detected_loader TEXT DEFAULT '',
    instance_validation_status TEXT DEFAULT 'unknown',
    instance_validation_message TEXT DEFAULT '',
    allow_file_deletion_on_remove INTEGER DEFAULT 0,
    instance_last_validated_at TEXT,
    profile_kind TEXT NOT NULL DEFAULT 'legacy',
    ownership_policy TEXT NOT NULL DEFAULT 'legacy_unknown',
    profile_root TEXT NOT NULL DEFAULT '',
    loader_version TEXT NOT NULL DEFAULT '',
    source_provider TEXT NOT NULL DEFAULT '',
    source_project_id TEXT NOT NULL DEFAULT '',
    source_version_id TEXT NOT NULL DEFAULT '',
    source_manifest_sha256 TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT '',
    last_used_at TEXT NOT NULL DEFAULT ''
);
";

fn seed_db(home: &Path, ddl: &str, user_version: u32, inserts: &[&str]) -> PathBuf {
    let data = home.join("data");
    fs::create_dir_all(&data).expect("data dir");
    let db_file = data.join(DB_FILE_NAME);
    let connection = rusqlite::Connection::open(&db_file).expect("open legacy db");
    connection.execute_batch(ddl).expect("schema");
    connection
        .pragma_update(None, "user_version", user_version)
        .expect("user_version");
    for sql in inserts {
        connection.execute_batch(sql).expect("seed row");
    }
    db_file
}

fn roots(home: &Path) -> LegacyRoots {
    roots_from(Some(home), None, None, None)
}

fn scan_home(home: &Path, scratch: &Path) -> LegacyScan {
    scan(&roots(home), scratch).expect("scan")
}

fn tree(root: &Path) -> Vec<(String, u64)> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, u64)>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let rel = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if meta.is_dir() {
                out.push((format!("{rel}/"), 0));
                walk(&path, base, out);
            } else {
                out.push((rel, meta.len()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

fn write_json(path: &Path, value: &serde_json::Value) {
    fs::create_dir_all(path.parent().expect("parent")).expect("json parent");
    let bytes = serde_json::to_vec_pretty(value).expect("serialize");
    fs::write(path, bytes).expect("write json");
}

#[test]
fn spec_f181_missing_root_reports_none_and_writes_nothing() {
    let dir = crate::test_tempdir();
    let scratch = dir.path().join("scratch");

    let report = scan(&LegacyRoots::none(), &scratch).expect("scan");

    assert_eq!(report.roots.source, LegacyRootSource::None);
    assert!(report.database.is_none());
    assert!(report.profiles.is_empty());
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(report.warnings[0].code, warning::ROOT_NOT_FOUND);
    assert!(
        !scratch.exists(),
        "a missing root must not create the scratch tree"
    );
}

#[test]
fn spec_f181_snapshot_is_a_copy_and_legacy_root_is_untouched() {
    let dir = crate::test_tempdir();
    let home = dir.path().join("old-home");
    let instance = home.join("instance");
    fs::create_dir_all(&instance).expect("instance");
    seed_db(
        &home,
        V2_DDL,
        2,
        &[&format!(
            "INSERT INTO profiles (name, game_version, loader, download_dir, profile_root, profile_kind)
             VALUES ('Legacy', '1.21.1', 'fabric', '{}', '{}', 'linked_instance')",
            instance.display(),
            instance.display()
        )],
    );
    write_json(
        &instance.join(PROFILE_JSON),
        &serde_json::json!({
            "schema": json::PROFILE_SCHEMA_V1,
            "profile": { "name": "Legacy", "paths": { "profile_root": instance } }
        }),
    );
    write_json(
        &instance.join(INSTALLED_MODS_JSON),
        &serde_json::json!({
            "schema": json::INSTALLED_MODS_SCHEMA_V1,
            "managed_mods": [
                { "project_id": "P1", "version_id": "V1", "managed_status": "managed" }
            ]
        }),
    );
    let before = tree(&home);
    let scratch = dir.path().join("scratch");

    let report = scan_home(&home, &scratch);

    assert!(report.database.is_some());
    assert_eq!(report.profiles.len(), 1);
    assert_eq!(tree(&home), before, "the old home must not change");
    assert!(
        tree(&scratch).is_empty(),
        "the snapshot must be deleted before returning"
    );
}

#[test]
fn spec_f181_reads_v2_profiles_and_columns() {
    let dir = crate::test_tempdir();
    let home = dir.path().join("old-home");
    let instance = home.join("instance");
    fs::create_dir_all(&instance).expect("instance");
    seed_db(
        &home,
        V2_DDL,
        2,
        &[&format!(
            "INSERT INTO profiles
             (id, name, game_version, loader, download_dir, profile_root, profile_kind, ownership_policy)
             VALUES (7, 'Fabric one', '1.20.1', 'fabric', '{}', '{}', 'linked_instance', 'user_linked')",
            instance.display(),
            instance.display()
        )],
    );

    let report = scan_home(&home, &dir.path().join("scratch"));
    let database = report.database.as_ref().expect("database");

    assert_eq!(database.user_version, 2);
    assert!(database.supported);
    assert!(database.has_profiles_table);
    assert_eq!(database.profile_count, 1);
    assert!(database.columns.iter().any(|c| c == "profile_kind"));
    assert_eq!(report.profiles.len(), 1);
    let profile = &report.profiles[0];
    assert_eq!(profile.id, "7");
    assert_eq!(profile.name, "Fabric one");
    assert_eq!(profile.game_version, "1.20.1");
    assert_eq!(profile.loader, "fabric");
    assert_eq!(profile.kind, "linked_instance");
    assert_eq!(profile.ownership, "user_linked");
    assert_eq!(profile.root, instance);
    assert!(profile.root_exists);
}

#[test]
fn spec_f181_reads_v1_schema_without_assuming_columns() {
    let dir = crate::test_tempdir();
    let home = dir.path().join("old-home");
    let instance = home.join("instance");
    fs::create_dir_all(&instance).expect("instance");
    seed_db(
        &home,
        V1_DDL,
        1,
        &[&format!(
            "INSERT INTO profiles (name, game_version, loader, download_dir, mods_dir)
             VALUES ('Old', '1.19.2', 'forge', '{}', '{}')",
            home.display(),
            instance.join("mods").display()
        )],
    );

    let report = scan_home(&home, &dir.path().join("scratch"));
    let database = report.database.as_ref().expect("database");

    assert_eq!(database.user_version, 1);
    assert!(database.supported);
    assert!(database.columns.iter().all(|c| c != "profile_kind"));
    assert_eq!(report.profiles.len(), 1);
    let profile = &report.profiles[0];
    assert_eq!(profile.kind, "");
    assert_eq!(profile.ownership, "");
    assert_eq!(
        profile.root, instance,
        "a `mods` directory resolves to its parent"
    );
    assert!(profile.root_exists);
}

#[test]
fn spec_f181_newer_database_is_reported_unsupported_without_rows() {
    let dir = crate::test_tempdir();
    let home = dir.path().join("old-home");
    seed_db(
        &home,
        V2_DDL,
        3,
        &[
            "INSERT INTO profiles (name, game_version, loader, download_dir)
           VALUES ('Future', '1.21.1', 'fabric', 'D:/somewhere')",
        ],
    );

    let report = scan_home(&home, &dir.path().join("scratch"));
    let database = report.database.as_ref().expect("database");

    assert_eq!(database.user_version, 3);
    assert!(!database.supported);
    assert_eq!(database.profile_count, 0);
    assert!(report.profiles.is_empty());
    assert!(report
        .warnings
        .iter()
        .any(|w| { w.code == warning::DB_VERSION_UNSUPPORTED && w.detail.contains("found=3") }));
}

#[test]
fn spec_f181_reads_profile_json_and_installed_mods() {
    let dir = crate::test_tempdir();
    let home = dir.path().join("old-home");
    let instance = home.join("instance");
    fs::create_dir_all(&instance).expect("instance");
    seed_db(
        &home,
        V2_DDL,
        2,
        &[&format!(
            "INSERT INTO profiles (name, game_version, loader, download_dir, profile_root)
             VALUES ('Legacy', '1.21.1', 'fabric', '{}', '{}')",
            instance.display(),
            instance.display()
        )],
    );
    write_json(
        &instance.join(PROFILE_JSON),
        &serde_json::json!({
            "schema": json::PROFILE_SCHEMA_V1,
            "profile": {
                "name": "Legacy",
                "paths": {
                    "profile_root": instance,
                    "minecraft_root": instance,
                    "mods": instance.join("mods")
                }
            }
        }),
    );
    write_json(
        &instance.join(INSTALLED_MODS_JSON),
        &serde_json::json!({
            "schema": json::INSTALLED_MODS_SCHEMA_V1,
            "managed_mods": [
                { "project_id": "P1", "version_id": "V1", "managed_status": "managed" },
                { "project_id": "P2", "version_id": "V2" }
            ]
        }),
    );

    let report = scan_home(&home, &dir.path().join("scratch"));
    let profile = &report.profiles[0];

    assert!(profile.profile_json.present);
    assert!(profile.profile_json.ok, "{:?}", profile.profile_json);
    assert_eq!(profile.profile_json.schema, json::PROFILE_SCHEMA_V1);
    assert!(profile.installed_mods.present);
    assert!(profile.installed_mods.ok);
    assert_eq!(profile.installed_mods.managed, 2);
    assert_eq!(profile.installed_mods.skipped, 0);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

#[test]
fn spec_f181_skips_external_and_unknown_manifest_records() {
    let dir = crate::test_tempdir();
    let home = dir.path().join("old-home");
    let instance = home.join("instance");
    fs::create_dir_all(&instance).expect("instance");
    seed_db(
        &home,
        V2_DDL,
        2,
        &[&format!(
            "INSERT INTO profiles (name, game_version, loader, download_dir, profile_root)
             VALUES ('Legacy', '1.21.1', 'fabric', '{}', '{}')",
            instance.display(),
            instance.display()
        )],
    );
    write_json(
        &instance.join(INSTALLED_MODS_JSON),
        &serde_json::json!({
            "schema": json::INSTALLED_MODS_SCHEMA_V1,
            "managed_mods": [
                { "project_id": "P1", "version_id": "V1", "managed_status": "managed" },
                { "project_id": "P2", "version_id": "V2", "managed_status": "external" },
                { "project_id": "P3", "version_id": "V3", "managed_status": "unknown" },
                { "project_id": "", "version_id": "V4", "managed_status": "managed" },
                { "project_id": "P5", "version_id": "V5", "managed_status": "external / unknown" },
                { "project_id": "P6", "version_id": "V6" }
            ]
        }),
    );

    let report = scan_home(&home, &dir.path().join("scratch"));
    let fact = &report.profiles[0].installed_mods;

    assert!(fact.ok);
    assert_eq!(fact.managed, 2);
    assert_eq!(fact.skipped, 4);
}

#[test]
fn spec_f181_corrupt_json_is_a_warning_not_a_failure() {
    let dir = crate::test_tempdir();
    let home = dir.path().join("old-home");
    let instance = home.join("instance");
    fs::create_dir_all(&instance).expect("instance");
    seed_db(
        &home,
        V2_DDL,
        2,
        &[&format!(
            "INSERT INTO profiles (name, game_version, loader, download_dir, profile_root)
             VALUES ('Legacy', '1.21.1', 'fabric', '{}', '{}')",
            instance.display(),
            instance.display()
        )],
    );
    fs::write(instance.join(PROFILE_JSON), b"{").expect("corrupt json");
    fs::write(instance.join(INSTALLED_MODS_JSON), b"[]").expect("wrong shape");

    let report = scan_home(&home, &dir.path().join("scratch"));

    assert_eq!(report.profiles.len(), 1);
    let profile = &report.profiles[0];
    assert!(profile.profile_json.present);
    assert!(!profile.profile_json.ok);
    assert_eq!(profile.profile_json.detail, "invalid_json");
    assert!(profile.installed_mods.present);
    assert!(!profile.installed_mods.ok);
    assert_eq!(profile.installed_mods.detail, "not_an_object");
    assert!(report
        .warnings
        .iter()
        .any(|w| w.code == warning::PROFILE_JSON_UNREADABLE));
    assert!(report
        .warnings
        .iter()
        .any(|w| w.code == warning::INSTALLED_MODS_UNREADABLE));
}

#[test]
fn spec_f181_snapshot_is_removed_after_scan() {
    let dir = crate::test_tempdir();
    let home = dir.path().join("old-home");
    seed_db(
        &home,
        V2_DDL,
        2,
        &[
            "INSERT INTO profiles (name, game_version, loader, download_dir)
           VALUES ('Legacy', '1.21.1', 'fabric', 'D:/somewhere')",
        ],
    );
    let scratch = dir.path().join("scratch");

    let report = scan_home(&home, &scratch);

    assert!(report.database.is_some());
    let leftovers: Vec<_> = fs::read_dir(&scratch)
        .expect("scratch exists")
        .flatten()
        .map(|entry| entry.file_name())
        .collect();
    assert!(
        leftovers.is_empty(),
        "leftover snapshot files: {leftovers:?}"
    );
}

#[test]
fn spec_f181_profile_root_ladder_follows_the_old_application() {
    let row = LegacyProfileRow {
        profile_root: "  ".to_string(),
        instance_root: "C:/instances/one".to_string(),
        minecraft_root: "C:/minecraft".to_string(),
        mods_dir: "C:/minecraft/mods".to_string(),
        download_dir: "C:/downloads".to_string(),
        ..LegacyProfileRow::default()
    };
    assert_eq!(row.root(), Some(PathBuf::from("C:/instances/one")));

    let row = LegacyProfileRow {
        mods_dir: "C:/minecraft/mods".to_string(),
        download_dir: "C:/downloads".to_string(),
        ..LegacyProfileRow::default()
    };
    assert_eq!(row.root(), Some(PathBuf::from("C:/minecraft")));

    let row = LegacyProfileRow {
        download_dir: "C:/downloads/mods".to_string(),
        ..LegacyProfileRow::default()
    };
    assert_eq!(row.root(), Some(PathBuf::from("C:/downloads")));

    let row = LegacyProfileRow::default();
    assert_eq!(row.root(), None);
}
