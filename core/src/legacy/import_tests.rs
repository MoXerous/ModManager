//! F-181 write path: registration only, plan first, and the old home stays
//! byte-for-byte unchanged.

use std::fs;
use std::path::{Path, PathBuf};

use super::discover::roots_from;
use super::import::*;
use super::*;

use crate::db::instances::{self, InstanceRowError, InstanceSource};
use crate::db::Db;

const V2_DDL: &str = "
CREATE TABLE profiles (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    game_version TEXT NOT NULL,
    loader TEXT NOT NULL,
    download_dir TEXT NOT NULL,
    instance_root TEXT DEFAULT '',
    minecraft_root TEXT DEFAULT '',
    mods_dir TEXT DEFAULT '',
    profile_root TEXT NOT NULL DEFAULT '',
    profile_kind TEXT NOT NULL DEFAULT 'legacy',
    ownership_policy TEXT NOT NULL DEFAULT 'legacy_unknown'
);
";

/// A v1-shaped table: no `profile_kind`, so every row reads as an empty kind.
const V1_DDL: &str = "
CREATE TABLE profiles (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    game_version TEXT NOT NULL,
    loader TEXT NOT NULL,
    download_dir TEXT NOT NULL,
    instance_root TEXT DEFAULT '',
    minecraft_root TEXT DEFAULT '',
    mods_dir TEXT DEFAULT ''
);
";

fn seed_db(home: &Path, ddl: &str, user_version: u32, inserts: &[String]) -> PathBuf {
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

fn scan_home(home: &Path, scratch: &Path) -> LegacyScan {
    let roots = roots_from(Some(home), None, None, None);
    scan(&roots, scratch).expect("scan")
}

fn new_db(dir: &Path) -> Db {
    crate::db::open(&dir.join("new.db")).expect("new db")
}

fn v2_insert(name: &str, version: &str, loader: &str, root: &Path, kind: &str) -> String {
    format!(
        "INSERT INTO profiles (name, game_version, loader, download_dir, profile_root, profile_kind)
         VALUES ('{name}', '{version}', '{loader}', '{}', '{}', '{kind}')",
        root.display(),
        root.display()
    )
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

fn ids(selected: &[&str]) -> Vec<String> {
    selected.iter().map(|s| (*s).to_string()).collect()
}

fn ready_code(entry: &ImportEntry) -> (&'static str, ImportAction) {
    (entry.code, entry.action)
}

#[test]
fn spec_f181_plan_registers_ready_and_skips_already_registered() {
    let dir = crate::test_tempdir();
    let home = dir.path().join("old-home");
    let cold = home.join("Cold");
    let taken = home.join("Taken");
    fs::create_dir_all(&cold).expect("cold root");
    fs::create_dir_all(&taken).expect("taken root");
    seed_db(
        &home,
        V2_DDL,
        2,
        &[
            v2_insert("Cold one", "1.21.1", "fabric", &cold, "custom"),
            v2_insert("Taken one", "1.20.1", "forge", &taken, "linked_instance"),
        ],
    );
    let report = scan_home(&home, &dir.path().join("scratch"));
    let db = new_db(dir.path());

    // Someone already registered the second folder in the new database.
    instances::insert(
        &db,
        &instances::InstanceRow {
            id: "already".to_string(),
            name: "Already".to_string(),
            path: taken.clone(),
            minecraft_root: PathBuf::new(),
            minecraft_version: None,
            loader: None,
            source: InstanceSource::Manual,
            created_at: 1,
            scanned_at: None,
        },
    )
    .expect("seed registered row");

    let plan = plan_import(&db, &report, &ids(&["1", "2"])).expect("plan");

    assert_eq!(plan.blocked_reason, None);
    assert_eq!((plan.ready, plan.skipped, plan.blocked), (1, 1, 0));
    assert_eq!(
        ready_code(&plan.entries[0]),
        (CODE_READY, ImportAction::Register)
    );
    assert_eq!(plan.entries[0].name, "Cold one");
    assert_eq!(plan.entries[0].root, cold);

    assert_eq!(
        ready_code(&plan.entries[1]),
        (CODE_ALREADY_REGISTERED, ImportAction::Skip)
    );
    assert_eq!(
        plan.entries[1].existing_instance.as_deref(),
        Some("already")
    );
}

#[test]
fn spec_f181_plan_blocks_missing_root_and_duplicate_selection() {
    let dir = crate::test_tempdir();
    let home = dir.path().join("old-home");
    let shared = home.join("Shared");
    let gone = home.join("Gone");
    fs::create_dir_all(&shared).expect("shared root");
    seed_db(
        &home,
        V2_DDL,
        2,
        &[
            v2_insert("Shared one", "1.21.1", "fabric", &shared, "custom"),
            v2_insert("Shared two", "1.21.1", "fabric", &shared, "custom"),
            v2_insert("Gone one", "1.21.1", "fabric", &gone, "custom"),
        ],
    );
    let report = scan_home(&home, &dir.path().join("scratch"));
    let db = new_db(dir.path());

    let plan = plan_import(&db, &report, &ids(&["1", "2", "3", "999"])).expect("plan");

    assert_eq!(plan.blocked_reason, None);
    assert_eq!((plan.ready, plan.skipped, plan.blocked), (1, 0, 3));
    assert_eq!(
        ready_code(&plan.entries[0]),
        (CODE_READY, ImportAction::Register)
    );
    assert_eq!(
        ready_code(&plan.entries[1]),
        (CODE_DUPLICATE_IN_SELECTION, ImportAction::Block)
    );
    assert_eq!(
        ready_code(&plan.entries[2]),
        (CODE_ROOT_MISSING, ImportAction::Block)
    );
    assert_eq!(
        ready_code(&plan.entries[3]),
        (CODE_PROFILE_MISSING, ImportAction::Block)
    );
    assert_eq!(plan.entries[3].name, "999", "an unknown id names itself");
    assert!(plan.entries[3].root.as_os_str().is_empty());

    // A repeated id is one entry, not a duplicate-in-selection blocker.
    let plan = plan_import(&db, &report, &ids(&["1", "1"])).expect("plan");
    assert_eq!(plan.entries.len(), 1);
    assert_eq!(plan.ready, 1);
}

#[test]
fn spec_f181_plan_maps_old_kinds_to_instance_source() {
    let dir = crate::test_tempdir();
    let home = dir.path().join("old-home");
    let mut inserts = Vec::new();
    let mut expected = Vec::new();
    for (index, kind) in ["custom", "legacy", "linked_instance", "linked_modpack"]
        .iter()
        .enumerate()
    {
        let root = home.join(format!("kind-{index}"));
        fs::create_dir_all(&root).expect("root");
        inserts.push(v2_insert(
            &format!("kind {index}"),
            "1.21.1",
            "fabric",
            &root,
            kind,
        ));
        expected.push(match *kind {
            "custom" | "legacy" => InstanceSource::Adopted,
            _ => InstanceSource::Manual,
        });
    }
    seed_db(&home, V2_DDL, 2, &inserts);
    let report = scan_home(&home, &dir.path().join("scratch"));
    let db = new_db(dir.path());

    let plan = plan_import(&db, &report, &ids(&["1", "2", "3", "4"])).expect("plan");
    let got: Vec<InstanceSource> = plan.entries.iter().map(|e| e.source).collect();
    assert_eq!(got, expected);

    // A v1 database has no `profile_kind` column at all: empty kind => manual.
    let v1_home = dir.path().join("v1-home");
    let v1_root = v1_home.join("old");
    fs::create_dir_all(&v1_root).expect("v1 root");
    seed_db(
        &v1_home,
        V1_DDL,
        1,
        &[format!(
            "INSERT INTO profiles (name, game_version, loader, download_dir, instance_root)
             VALUES ('Old', '1.19.2', 'forge', '{}', '{}')",
            v1_root.display(),
            v1_root.display()
        )],
    );
    let v1_report = scan_home(&v1_home, &dir.path().join("scratch-v1"));
    assert_eq!(v1_report.profiles[0].kind, "");
    let plan = plan_import(&db, &v1_report, &ids(&["1"])).expect("v1 plan");
    assert_eq!(plan.entries[0].source, InstanceSource::Manual);
}

#[test]
fn spec_f181_plan_normalizes_loader_and_version() {
    let dir = crate::test_tempdir();
    let home = dir.path().join("old-home");
    let fabric = home.join("fabric");
    let iris = home.join("iris");
    let blank = home.join("blank");
    for root in [&fabric, &iris, &blank] {
        fs::create_dir_all(root).expect("root");
    }
    seed_db(
        &home,
        V2_DDL,
        2,
        &[
            v2_insert("Fabric", " 1.21.1 ", "FABRIC", &fabric, "custom"),
            v2_insert("Iris", "1.20.1", "iris", &iris, "custom"),
            v2_insert("Blank", "   ", "  ", &blank, "custom"),
        ],
    );
    let report = scan_home(&home, &dir.path().join("scratch"));
    let db = new_db(dir.path());

    let plan = plan_import(&db, &report, &ids(&["1", "2", "3"])).expect("plan");

    assert_eq!(plan.entries[0].loader.as_deref(), Some("fabric"));
    assert_eq!(plan.entries[0].game_version.as_deref(), Some("1.21.1"));
    assert_eq!(plan.entries[1].loader, None, "iris is not a loader here");
    assert_eq!(plan.entries[2].loader, None);
    assert_eq!(plan.entries[2].game_version, None);
}

#[test]
fn spec_f181_plan_refuses_an_unsupported_database_with_a_reason() {
    let dir = crate::test_tempdir();
    let home = dir.path().join("old-home");
    let root = home.join("future");
    fs::create_dir_all(&root).expect("root");
    seed_db(
        &home,
        V2_DDL,
        3,
        &[v2_insert("Future", "1.21.1", "fabric", &root, "custom")],
    );
    let report = scan_home(&home, &dir.path().join("scratch"));
    let db = new_db(dir.path());

    let plan = plan_import(&db, &report, &ids(&["1"])).expect("plan");

    assert_eq!(plan.blocked_reason, Some(BLOCK_DATABASE_UNSUPPORTED));
    assert!(plan.entries.is_empty());
    assert_eq!((plan.ready, plan.skipped, plan.blocked), (0, 0, 0));

    // A missing old root is the first reason, before the database ones.
    let empty = scan(&LegacyRoots::none(), &dir.path().join("scratch-none")).expect("scan");
    let plan = plan_import(&db, &empty, &ids(&["1"])).expect("plan");
    assert_eq!(plan.blocked_reason, Some(BLOCK_ROOT_NOT_FOUND));
    assert!(plan.entries.is_empty());
}

#[test]
fn spec_f181_execute_backs_up_the_new_database_before_writing() {
    let dir = crate::test_tempdir();
    let home = dir.path().join("old-home");
    let root = home.join("instance");
    fs::create_dir_all(&root).expect("root");
    seed_db(
        &home,
        V2_DDL,
        2,
        &[v2_insert("Old one", "1.21.1", "fabric", &root, "custom")],
    );
    let report = scan_home(&home, &dir.path().join("scratch"));
    let db = new_db(dir.path());

    // A row that predates the import must be in the backup too.
    let seeded = dir.path().join("seeded");
    fs::create_dir_all(&seeded).expect("seeded root");
    instances::insert(
        &db,
        &instances::InstanceRow {
            id: "seeded".to_string(),
            name: "Seeded".to_string(),
            path: seeded,
            minecraft_root: PathBuf::new(),
            minecraft_version: None,
            loader: None,
            source: InstanceSource::Managed,
            created_at: 1,
            scanned_at: None,
        },
    )
    .expect("seed row");

    let outcome = execute_import(&db, &report, &ids(&["1"]), 42).expect("execute");

    let backup = outcome.backup.clone().expect("a backup was taken");
    assert_eq!(outcome.imported, 1);
    let name = backup
        .file_name()
        .and_then(|n| n.to_str())
        .expect("backup name");
    assert!(
        name.contains("pre-legacy-import.") && name.ends_with(".bak"),
        "unexpected backup name {name}"
    );

    let old = crate::db::open(&backup).expect("the backup opens");
    assert_eq!(
        instances::list(&old).expect("list").len(),
        1,
        "the backup holds the pre-import state only"
    );
    assert_eq!(
        instances::list(&db).expect("list").len(),
        2,
        "live db has both"
    );

    let imported = instances::list(&db)
        .expect("list")
        .into_iter()
        .find(|r| r.id != "seeded")
        .expect("imported row");
    assert_eq!(imported.name, "Old one");
    assert_eq!(imported.source, InstanceSource::Adopted);
    assert_eq!(imported.created_at, 42);
    assert_eq!(imported.minecraft_version.as_deref(), Some("1.21.1"));
    assert_eq!(imported.loader.as_deref(), Some("fabric"));
}

#[test]
fn spec_f181_execute_writes_nothing_when_no_profile_is_ready() {
    let dir = crate::test_tempdir();
    let home = dir.path().join("old-home");
    let gone = home.join("gone");
    seed_db(
        &home,
        V2_DDL,
        2,
        &[v2_insert("Gone", "1.21.1", "fabric", &gone, "custom")],
    );
    let report = scan_home(&home, &dir.path().join("scratch"));
    let db = new_db(dir.path());
    let before = instances::list(&db).expect("list").len();

    let outcome = execute_import(&db, &report, &ids(&["1", "999"]), 7).expect("execute");

    assert_eq!(outcome.backup, None, "nothing to write, nothing to back up");
    assert_eq!(outcome.imported, 0);
    assert_eq!(outcome.blocked, 2);
    assert_eq!(instances::list(&db).expect("list").len(), before);
    let backups: Vec<_> = fs::read_dir(dir.path())
        .expect("ls")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".bak"))
        .collect();
    assert!(backups.is_empty(), "no backup without writes: {backups:?}");
}

#[test]
fn spec_f181_execute_leaves_the_old_home_unchanged() {
    let dir = crate::test_tempdir();
    let home = dir.path().join("old-home");
    let root = home.join("instance");
    fs::create_dir_all(root.join("mods")).expect("mods");
    fs::write(root.join("mods").join("a.jar"), b"not really a jar").expect("mod");
    fs::write(root.join(PROFILE_JSON_MARKER), b"{\"schema\":\"x\"}").expect("profile json");
    seed_db(
        &home,
        V2_DDL,
        2,
        &[v2_insert("Old one", "1.21.1", "fabric", &root, "custom")],
    );
    let before = tree(&home);
    let report = scan_home(&home, &dir.path().join("scratch"));
    let db = new_db(dir.path());

    let outcome = execute_import(&db, &report, &ids(&["1"]), 1).expect("execute");

    assert_eq!(outcome.imported, 1);
    assert_eq!(tree(&home), before, "the old home must not change at all");
}

#[test]
fn spec_f181_execute_is_idempotent_for_the_same_selection() {
    let dir = crate::test_tempdir();
    let home = dir.path().join("old-home");
    let root = home.join("instance");
    fs::create_dir_all(&root).expect("root");
    seed_db(
        &home,
        V2_DDL,
        2,
        &[v2_insert("Old one", "1.21.1", "fabric", &root, "custom")],
    );
    let report = scan_home(&home, &dir.path().join("scratch"));
    let db = new_db(dir.path());

    let first = execute_import(&db, &report, &ids(&["1"]), 1).expect("first");
    assert_eq!(first.imported, 1);
    assert!(first.backup.is_some());

    let second = execute_import(&db, &report, &ids(&["1"]), 2).expect("second");
    assert_eq!(second.imported, 0);
    assert_eq!(second.backup, None, "no second row, no second backup");
    assert_eq!(second.skipped, 1);
    assert_eq!(second.outcomes[0].code, CODE_ALREADY_REGISTERED);
    assert_eq!(instances::list(&db).expect("list").len(), 1);
}

#[test]
fn spec_f181_import_detects_a_prism_style_root() {
    let dir = crate::test_tempdir();
    let home = dir.path().join("old-home");
    let wrapper = home.join("instance");
    let mc = wrapper.join(".minecraft");
    fs::create_dir_all(&mc).expect("minecraft root");
    fs::write(wrapper.join("mmc-pack.json"), b"{}").expect("launcher metadata");
    seed_db(
        &home,
        V2_DDL,
        2,
        &[v2_insert("Prism", "1.21.1", "fabric", &mc, "custom")],
    );
    let report = scan_home(&home, &dir.path().join("scratch"));
    let db = new_db(dir.path());

    let plan = plan_import(&db, &report, &ids(&["1"])).expect("plan");
    assert_eq!(
        plan.entries[0].root, wrapper,
        "the parent is the instance root"
    );

    let outcome = execute_import(&db, &report, &ids(&["1"]), 5).expect("execute");
    assert_eq!(outcome.imported, 1);
    let row = instances::list(&db).expect("list").pop().expect("row");
    assert_eq!(row.path, std::fs::canonicalize(&wrapper).expect("canon"));
    assert_eq!(
        row.minecraft_root,
        std::fs::canonicalize(&mc).expect("canon")
    );
}

#[test]
fn spec_f181_plan_does_not_write() {
    let dir = crate::test_tempdir();
    let home = dir.path().join("old-home");
    let root = home.join("instance");
    fs::create_dir_all(&root).expect("root");
    seed_db(
        &home,
        V2_DDL,
        2,
        &[v2_insert("Old one", "1.21.1", "fabric", &root, "custom")],
    );
    let scratch = dir.path().join("scratch");
    let report = scan_home(&home, &scratch);
    let db = new_db(dir.path());
    let before = instances::list(&db).expect("list").len();

    let plan = plan_import(&db, &report, &ids(&["1"])).expect("plan");

    assert_eq!(plan.ready, 1);
    assert_eq!(instances::list(&db).expect("list").len(), before);
    let backups: Vec<_> = fs::read_dir(dir.path())
        .expect("ls")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".bak"))
        .collect();
    assert!(
        backups.is_empty(),
        "a preview writes no backup: {backups:?}"
    );
    assert!(
        tree(&scratch).is_empty(),
        "the preview must leave no scratch files"
    );
}

#[test]
fn spec_f181_execute_rolls_back_the_whole_batch_when_an_insert_fails() {
    let dir = crate::test_tempdir();
    let home = dir.path().join("old-home");
    let first = home.join("first");
    let second = home.join("second");
    fs::create_dir_all(&first).expect("first root");
    fs::create_dir_all(&second).expect("second root");
    seed_db(
        &home,
        V2_DDL,
        2,
        &[
            v2_insert("First", "1.21.1", "fabric", &first, "custom"),
            v2_insert("Second", "1.21.1", "fabric", &second, "custom"),
        ],
    );
    let report = scan_home(&home, &dir.path().join("scratch"));
    assert!(
        report.profiles.iter().all(|profile| profile.root_exists),
        "the scan must see both roots before one is deleted"
    );

    // Deleted *after* the scan on purpose. `execute_import` re-plans from this
    // snapshot, so both entries are still `ready`; only `instances::insert`
    // looks at the disk again, and it refuses the second folder. That makes the
    // failure deterministic without reaching into the database.
    fs::remove_dir_all(&second).expect("delete the second root");

    let db = new_db(dir.path());
    let error =
        execute_import(&db, &report, &ids(&["1", "2"]), 9).expect_err("the batch must fail");
    assert!(
        matches!(
            error,
            ImportError::Row(InstanceRowError::UnresolvablePath { .. })
        ),
        "got {error:?}"
    );

    assert!(
        instances::list(&db).expect("list").is_empty(),
        "the first insert must be rolled back with the failed second one"
    );

    // The backup is taken before the first insert by design, so a failed batch
    // still leaves one: it holds the pre-import state the rollback returns the
    // live database to.
    let backups: Vec<_> = fs::read_dir(dir.path())
        .expect("ls")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".bak"))
        .collect();
    assert_eq!(
        backups.len(),
        1,
        "one backup for the attempted batch: {backups:?}"
    );
}

/// A marker file, not the real `profile.json` reader: the test only needs a
/// file that would show up in a tree comparison.
const PROFILE_JSON_MARKER: &str = "profile.json";
