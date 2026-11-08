//! Instance tests: temp dirs only, jars built in-test.

use super::lock::{read_lock, write_lock, LockEntry, LockError, LockFile};
use super::{
    detect_instance_roots, is_disabled_path, iter_local_mod_scan_files, scan_mods, snapshot_tree,
    Instance,
};
use crate::jar_reader::tests::build_jar;

fn mods_with(dir: &std::path::Path, names: &[&str]) -> std::path::PathBuf {
    let mods = dir.join("mods");
    std::fs::create_dir_all(&mods).expect("mods dir");
    for n in names {
        let p = mods.join(n);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).expect("parent");
        }
        if *n != "sub/nested.jar" {
            build_jar(&p, &[("x.txt", b"v")]);
        }
    }
    mods
}

#[test]
fn spec_f067_detects_disabled_suffix_and_segment() {
    let d = crate::test_tempdir();
    let mods = mods_with(d.path(), &["a.jar", "b.jar.disabled"]);
    let dis = mods.join("disabled_mods");
    std::fs::create_dir_all(&dis).expect("dir");
    build_jar(&dis.join("c.jar"), &[("x.txt", b"v")]);
    assert!(!is_disabled_path(&mods.join("a.jar")));
    assert!(is_disabled_path(&mods.join("b.jar.disabled")));
    assert!(is_disabled_path(&dis.join("c.jar")));
}

#[test]
fn spec_f101_scan_files_top_level_plus_disabled_sorted() {
    let d = crate::test_tempdir();
    let mods = mods_with(d.path(), &["example.jar", "active.jar", "sub/nested.jar"]);
    build_jar(&mods.join("disabled.jar"), &[("x.txt", b"v")]);
    let dis = mods.join("disabled_mods");
    std::fs::create_dir_all(&dis).expect("dir");
    build_jar(&dis.join("old.jar"), &[("x.txt", b"v")]);
    let files = iter_local_mod_scan_files(&mods);
    let names: Vec<String> = files
        .iter()
        .map(|p| p.file_name().expect("name").to_string_lossy().into_owned())
        .collect();
    // Non-recursive: nested excluded; sorted by full path casefolded
    // (spec-faithful: disabled_mods/ sorts with its directory prefix).
    assert_eq!(
        names,
        vec!["active.jar", "disabled.jar", "old.jar", "example.jar"]
    );
    // Missing dir ⇒ [].
    assert!(iter_local_mod_scan_files(&d.path().join("nope")).is_empty());
}

#[test]
fn spec_f101_scan_is_read_only() {
    use crate::jar_reader::ResourceType;
    let d = crate::test_tempdir();
    let mods = mods_with(d.path(), &["a.jar"]);
    let before = snapshot_tree(&mods);
    let inst = Instance {
        root: d.path().to_path_buf(),
        minecraft_root: d.path().to_path_buf(),
    };
    let facts = scan_mods(&inst);
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].resource_type, ResourceType::Unknown);
    assert_eq!(snapshot_tree(&mods), before);
}

#[test]
fn spec_f101_prism_child_uses_parent_root() {
    let d = crate::test_tempdir();
    let parent = d.path().join("instance");
    let child = parent.join(".minecraft");
    std::fs::create_dir_all(child.join("mods")).expect("dirs");
    std::fs::write(parent.join("mmc-pack.json"), b"{}").expect("write");
    let (root, mc) = detect_instance_roots(&child);
    assert_eq!(root, parent);
    assert_eq!(mc, child);
}

#[test]
fn spec_f101_a_lock_written_before_sha512_existed_still_reads() {
    // The field was added after locks were already on disk. Every field here is
    // `serde(default)` for that reason, and this is the check: an old lock must
    // not become unreadable, or upgrading the app would strand its instances.
    let d = crate::test_tempdir();
    let p = d.path().join("instance.lock.json");
    std::fs::write(
        &p,
        r#"{"schema_version":1,"entries":[{"provider":"modrinth","project_id":"a",
        "slug":"a-mod","version_id":"v","filename":"a.jar","sha1":"00"}]}"#,
    )
    .expect("write old lock");

    let back = read_lock(&p).expect("an old lock must still parse");
    assert_eq!(back.entries.len(), 1);
    assert_eq!(back.entries[0].sha1, "00");
    assert_eq!(
        back.entries[0].sha512, "",
        "absent sha512 reads as empty, not as an error"
    );
}

#[test]
fn spec_f101_schema_one_lock_empty_provider_reads_as_modrinth() {
    // Before CurseForge, Modrinth was the only provider. An empty provider in
    // a schema-1 lock therefore has exactly one meaning; a filled provider is
    // left untouched (ADR 0004 §2-3 item 5).
    let d = crate::test_tempdir();
    let p = d.path().join("instance.lock.json");
    std::fs::write(
        &p,
        r#"{"schema_version":1,"entries":[
        {"project_id":"old","slug":"old","version_id":"v1","filename":"old.jar","sha1":"00"},
        {"provider":"curseforge","project_id":"cf","slug":"cf","version_id":"v2","filename":"cf.jar","sha1":"11"}]}"#,
    )
    .expect("write lock");

    let back = read_lock(&p).expect("read");
    assert_eq!(back.entries[0].provider, "modrinth");
    assert_eq!(back.entries[1].provider, "curseforge");
}

#[test]
fn spec_f101_lock_roundtrip_and_rejects_newer_schema() {
    let d = crate::test_tempdir();
    let p = d.path().join("instance.lock.json");
    let lock = LockFile {
        entries: vec![LockEntry {
            provider: "modrinth".to_string(),
            project_id: "a".to_string(),
            slug: "a-mod".to_string(),
            version_id: "v".to_string(),
            filename: "a.jar".to_string(),
            sha1: "00".to_string(),
            sha512: "ff".to_string(),
        }],
    };
    write_lock(&p, &lock).expect("write");
    assert_eq!(read_lock(&p).expect("read"), lock);
    std::fs::write(&p, r#"{"schema_version":99,"entries":[]}"#).expect("write");
    assert!(matches!(read_lock(&p), Err(LockError::NewerSchema { .. })));
}

/// F-128: the lock is rewritten in place on every install, so its writer must
/// never depend on the `.part` namespace (a transfer that resumes owns it) and
/// must never leave a temporary behind. A stale `.part` is somebody else's
/// file: it neither blocks the write nor gets deleted.
#[test]
fn spec_f128_write_lock_is_atomic_and_survives_stale_temp() {
    let d = crate::test_tempdir();
    let p = d.path().join("instance.lock.json");
    let entry = |name: &str| LockEntry {
        provider: "modrinth".to_string(),
        project_id: "proj".to_string(),
        slug: "a-mod".to_string(),
        version_id: "v1".to_string(),
        filename: name.to_string(),
        sha1: "00".to_string(),
        sha512: "ff".to_string(),
    };

    write_lock(
        &p,
        &LockFile {
            entries: vec![entry("a.jar")],
        },
    )
    .expect("first write");
    assert_eq!(read_lock(&p).expect("read").entries[0].filename, "a.jar");

    let part = d.path().join("instance.lock.json.part");
    std::fs::write(&part, b"foreign part").expect("fixture");

    write_lock(
        &p,
        &LockFile {
            entries: vec![entry("b.jar")],
        },
    )
    .expect("overwrite");
    assert_eq!(
        read_lock(&p).expect("read").entries[0].filename,
        "b.jar",
        "the second write must replace the first, not append to it"
    );
    assert_eq!(
        std::fs::read(&part).expect("read"),
        b"foreign part",
        "a `.part` this writer never created must survive untouched"
    );

    let leftovers: Vec<String> = std::fs::read_dir(d.path())
        .expect("ls")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "temporaries left: {leftovers:?}");
}
