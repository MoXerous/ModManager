//! Instance tests: temp dirs only, jars built in-test.

use super::content::{
    classify_content, Compatibility, CompatibilityReason, ContentClass, ContentTarget,
    ContentWarning, DuplicateKey, Ownership,
};
use super::lock::{read_lock, write_lock, LockEntry, LockError, LockFile};
use super::{
    detect_instance_roots, is_disabled_path, iter_local_mod_scan_files, scan_mods, snapshot_tree,
    Instance, LocalContentFact,
};
use crate::jar_reader::tests::build_jar;
use crate::jar_reader::{Loader, ResourceType};

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
fn spec_f067_scan_keeps_relative_path_for_disabled_mods() {
    let d = crate::test_tempdir();
    let mods = mods_with(d.path(), &["a.jar"]);
    let dis = mods.join("disabled_mods");
    std::fs::create_dir_all(&dis).expect("dir");
    build_jar(&dis.join("c.jar"), &[("x.txt", b"v")]);
    let inst = Instance {
        root: d.path().to_path_buf(),
        minecraft_root: d.path().to_path_buf(),
    };
    let facts = scan_mods(&inst);
    let top = facts.iter().find(|f| f.filename == "a.jar").expect("top");
    assert_eq!(top.rel_path, "a.jar");
    let nested = facts
        .iter()
        .find(|f| f.filename == "c.jar")
        .expect("nested");
    assert_eq!(
        nested.rel_path, "disabled_mods/c.jar",
        "the filename alone loses the folder the file lives in"
    );
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
            game_versions: Vec::new(),
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
        game_versions: Vec::new(),
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

// --- F-097..F-100: content classification ------------------------------

/// The neutral fact the scan hands to the classifier: every axis unproven.
/// The filename is the last path segment, exactly as the scan reports it for
/// the one subfolder it reads (`disabled_mods/`).
fn fact(name: &str, sha1: &str) -> LocalContentFact {
    let filename = name.rsplit('/').next().unwrap_or(name).to_string();
    LocalContentFact::new(
        std::path::PathBuf::from(name),
        name.to_string(),
        filename,
        1,
        sha1.to_string(),
        ResourceType::Mod,
        false,
    )
}

fn target(game_version: &str, loader: &str) -> ContentTarget {
    ContentTarget {
        game_version: game_version.to_string(),
        loader: loader.to_string(),
    }
}

fn lock_entry(filename: &str, project: &str, version: &str, sha1: &str) -> LockEntry {
    LockEntry {
        provider: "modrinth".to_string(),
        project_id: project.to_string(),
        slug: String::new(),
        version_id: version.to_string(),
        filename: filename.to_string(),
        sha1: sha1.to_string(),
        sha512: String::new(),
        game_versions: Vec::new(),
    }
}

fn instance_at(root: &std::path::Path) -> Instance {
    Instance {
        root: root.to_path_buf(),
        minecraft_root: root.to_path_buf(),
    }
}

/// F-097: the ownership axis the screen shows. A file the lock records is
/// managed; a file it does not is external/unknown — and the proof is the
/// recorded SHA-1 or version identity, never the filename alone.
#[test]
fn spec_f097_scan_classifies_managed_and_external_files() {
    let d = crate::test_tempdir();
    let mods = d.path().join("mods");
    std::fs::create_dir_all(&mods).expect("mods dir");
    build_jar(&mods.join("managed.jar"), &[("x.txt", b"managed")]);
    build_jar(&mods.join("external.jar"), &[("x.txt", b"external")]);

    let facts = scan_mods(&instance_at(d.path()));
    let managed_sha1 = facts
        .iter()
        .find(|f| f.filename == "managed.jar")
        .expect("managed fact")
        .sha1
        .clone();
    let lock = LockFile {
        entries: vec![lock_entry("managed.jar", "proj", "v1", &managed_sha1)],
    };

    let snapshot = classify_content(facts, &lock, &target("", ""));
    let managed = snapshot
        .facts
        .iter()
        .find(|f| f.filename == "managed.jar")
        .expect("managed fact");
    assert_eq!(managed.ownership, Ownership::ManagedBySha1);
    assert_eq!(managed.project_id, "proj", "the lock names the project");
    assert_eq!(managed.class(), ContentClass::Managed);

    let external = snapshot
        .facts
        .iter()
        .find(|f| f.filename == "external.jar")
        .expect("external fact");
    assert_eq!(external.ownership, Ownership::External);
    assert_eq!(external.project_id, "");
    assert_eq!(external.class(), ContentClass::ExternalUnknown);
}

/// F-097: five classes, one derived label. The axes stay on the fact; only the
/// label collapses, and it follows the old app's ladder: disabled → duplicate
/// → incompatible → managed → external/unknown.
#[test]
fn spec_f097_the_class_follows_the_legacy_precedence_ladder() {
    let incompatible = || {
        Compatibility::Incompatible(CompatibilityReason::Loader {
            target: "fabric".to_string(),
        })
    };

    let mut disabled = fact("disabled.jar", "h");
    disabled.disabled = true;
    disabled.duplicate_keys = vec![DuplicateKey::Sha1("h".to_string())];
    disabled.compatibility = incompatible();
    disabled.ownership = Ownership::ManagedBySha1;
    assert_eq!(disabled.class(), ContentClass::Disabled);

    let mut duplicate = fact("duplicate.jar", "h");
    duplicate.duplicate_keys = vec![DuplicateKey::Project {
        provider: "modrinth".to_string(),
        project_id: "p".to_string(),
    }];
    duplicate.compatibility = incompatible();
    duplicate.ownership = Ownership::ManagedBySha1;
    assert_eq!(duplicate.class(), ContentClass::Duplicate);

    let mut unsupported = fact("incompatible.jar", "h");
    unsupported.compatibility = incompatible();
    unsupported.ownership = Ownership::ManagedBySha1;
    assert_eq!(unsupported.class(), ContentClass::Incompatible);

    let mut managed = fact("managed.jar", "h");
    managed.ownership = Ownership::ManagedBySha1;
    assert_eq!(managed.class(), ContentClass::Managed);

    assert_eq!(
        fact("external.jar", "h").class(),
        ContentClass::ExternalUnknown
    );
}

/// F-097: the compatibility axis needs positive metadata. A missing declaration
/// is "unprovable", not "incompatible", and a declared range is not read as an
/// exclusion: membership in an exact list is the only claim the rule makes.
#[test]
fn spec_f097_compatibility_needs_positive_metadata() {
    struct Case<'a> {
        label: &'a str,
        versions: Vec<&'a str>,
        loaders: Vec<Loader>,
        game: &'a str,
        loader: &'a str,
        expected: Compatibility,
    }

    let cases = vec![
        Case {
            label: "proved",
            versions: vec!["1.21.1"],
            loaders: vec![Loader::Fabric],
            game: "1.21.1",
            loader: "fabric",
            expected: Compatibility::Compatible,
        },
        Case {
            label: "game-mismatch",
            versions: vec!["1.20.1"],
            loaders: vec![Loader::Fabric],
            game: "1.21.1",
            loader: "fabric",
            expected: Compatibility::Incompatible(CompatibilityReason::GameVersion {
                target: "1.21.1".to_string(),
            }),
        },
        Case {
            label: "loader-mismatch",
            versions: vec!["1.21.1"],
            loaders: vec![Loader::Forge],
            game: "1.21.1",
            loader: "fabric",
            expected: Compatibility::Incompatible(CompatibilityReason::Loader {
                target: "fabric".to_string(),
            }),
        },
        Case {
            label: "missing-metadata",
            versions: vec![],
            loaders: vec![],
            game: "1.21.1",
            loader: "fabric",
            expected: Compatibility::Unknown(CompatibilityReason::Unproven),
        },
        Case {
            label: "no-target",
            versions: vec!["1.21.1"],
            loaders: vec![Loader::Fabric],
            game: "",
            loader: "",
            expected: Compatibility::Unknown(CompatibilityReason::NoTarget),
        },
        Case {
            label: "any-loader",
            versions: vec!["1.21.1"],
            loaders: vec![],
            game: "1.21.1",
            loader: "any",
            expected: Compatibility::Compatible,
        },
        Case {
            label: "range-only",
            versions: vec![">=1.20.1"],
            loaders: vec![Loader::Fabric],
            game: "1.21.1",
            loader: "fabric",
            expected: Compatibility::Compatible,
        },
    ];

    for Case {
        label,
        versions,
        loaders,
        game,
        loader,
        expected,
    } in cases
    {
        let mut f = fact(&format!("{label}.jar"), "h");
        f.game_versions = versions.iter().map(|v| v.to_string()).collect();
        f.loaders = loaders;
        let snapshot = classify_content(vec![f], &LockFile::default(), &target(game, loader));
        assert_eq!(
            snapshot.facts[0].compatibility, expected,
            "case {label} produced the wrong compatibility verdict"
        );
    }
}

/// F-097: the provider's declaration, recorded in the lock at install time, is
/// the evidence the scan prefers. A jar that declares `1.21.1` in a `1.21.1`
/// instance is *compatible* by its own metadata — and still incompatible when
/// the version record that installed it said `1.20.1`, because that is what
/// the catalogue promised for this file.
#[test]
fn spec_f097_recorded_provider_versions_are_preferred_over_the_jar() {
    let mut f = fact("recorded.jar", "h");
    f.game_versions = vec!["1.21.1".to_string()];
    let mut entry = lock_entry("recorded.jar", "proj", "v1", "h");
    entry.game_versions = vec!["1.20.1".to_string()];

    let snapshot = classify_content(
        vec![f],
        &LockFile {
            entries: vec![entry],
        },
        &target("1.21.1", ""),
    );

    assert_eq!(
        snapshot.facts[0].compatibility,
        Compatibility::Incompatible(CompatibilityReason::GameVersion {
            target: "1.21.1".to_string(),
        }),
        "the recorded provider declaration is the evidence that settles this"
    );
}

/// F-097: a recorded *range* is judged, not discarded. The old app compared
/// the target with `>=1.21` as a string and called the file incompatible in a
/// `1.20.1` instance; the range excludes `1.20.1`, so the verdict is the same
/// but now it is *derived* — and the reason names the target.
#[test]
fn spec_f097_a_recorded_range_that_excludes_the_target_is_incompatible() {
    let mut f = fact("range.jar", "h");
    // The jar's own metadata would call this file compatible; the record is
    // what the scan must answer from.
    f.game_versions = vec!["1.20.1".to_string()];
    let mut entry = lock_entry("range.jar", "proj", "v1", "h");
    entry.game_versions = vec![">=1.21".to_string()];

    let snapshot = classify_content(
        vec![f],
        &LockFile {
            entries: vec![entry],
        },
        &target("1.20.1", ""),
    );

    assert_eq!(
        snapshot.facts[0].compatibility,
        Compatibility::Incompatible(CompatibilityReason::GameVersion {
            target: "1.20.1".to_string(),
        }),
        "`>=1.21` excludes a 1.20.1 instance, and the exclusion is proven"
    );
    assert_eq!(snapshot.facts[0].class(), ContentClass::Incompatible);
}

/// F-097: the direction that makes the parser worth having. `>=1.20.1` in a
/// `1.21.1` instance includes the target, so the file is compatible — from the
/// provider's record and from the jar's own declaration alike.
#[test]
fn spec_f097_a_range_that_includes_the_instance_target_is_compatible() {
    let mut recorded = fact("recorded-range.jar", "h");
    recorded.game_versions = vec!["1.20.1".to_string()];
    let mut entry = lock_entry("recorded-range.jar", "proj", "v1", "h");
    entry.game_versions = vec![">=1.20.1".to_string()];
    let snapshot = classify_content(
        vec![recorded],
        &LockFile {
            entries: vec![entry],
        },
        &target("1.21.1", ""),
    );
    assert_eq!(
        snapshot.facts[0].compatibility,
        Compatibility::Compatible,
        "the provider's range admits 1.21.1"
    );

    let mut jar = fact("jar-range.jar", "h");
    jar.game_versions = vec!["1.20.x".to_string()];
    let snapshot = classify_content(vec![jar], &LockFile::default(), &target("1.20.4", ""));
    assert_eq!(
        snapshot.facts[0].compatibility,
        Compatibility::Compatible,
        "the jar's wildcard admits 1.20.4"
    );
}

/// F-097: the rule that keeps the incompatible class honest. A declaration
/// this build cannot read settles nothing: it stays `Unknown`, never a false
/// `Incompatible` — and the class stays `Managed`, not `Incompatible`.
#[test]
fn spec_f097_an_unparseable_range_stays_unknown_never_incompatible() {
    let mut f = fact("unreadable-range.jar", "h");
    f.game_versions = vec!["1.20.1".to_string()];
    let mut entry = lock_entry("unreadable-range.jar", "proj", "v1", "h");
    entry.game_versions = vec!["1.20 || 1.21".to_string()];

    let snapshot = classify_content(
        vec![f],
        &LockFile {
            entries: vec![entry],
        },
        &target("1.20.1", ""),
    );

    assert_eq!(
        snapshot.facts[0].compatibility,
        Compatibility::Unknown(CompatibilityReason::Range),
        "an unreadable range is unknown, with the reason"
    );
    assert_eq!(snapshot.facts[0].class(), ContentClass::Managed);
}

/// F-097: a declaration list is a union. One entry that admits the target is
/// enough, even beside one that does not and one that cannot be read.
#[test]
fn spec_f097_one_declaration_that_admits_the_target_wins() {
    let mut f = fact("mixed.jar", "h");
    f.game_versions = vec![
        "1.20 || 1.21".to_string(),
        ">=1.21".to_string(),
        "1.20.1".to_string(),
    ];
    let snapshot = classify_content(vec![f], &LockFile::default(), &target("1.20.1", ""));
    assert_eq!(snapshot.facts[0].compatibility, Compatibility::Compatible);
}

/// F-097: the fallback half. A managed record with no recorded versions — an
/// instance installed by an older build — leaves the archive's own metadata in
/// charge, exactly as before this evidence existed.
#[test]
fn spec_f097_a_managed_file_with_nothing_recorded_falls_back_to_its_jar() {
    let mut f = fact("jar-only.jar", "h");
    f.game_versions = vec!["1.20".to_string()];
    let lock = LockFile {
        entries: vec![lock_entry("jar-only.jar", "proj", "v1", "h")],
    };

    let snapshot = classify_content(vec![f], &lock, &target("1.21.1", ""));

    assert_eq!(snapshot.facts[0].ownership, Ownership::ManagedBySha1);
    assert!(snapshot.facts[0].recorded_game_versions.is_empty());
    assert_eq!(
        snapshot.facts[0].compatibility,
        Compatibility::Incompatible(CompatibilityReason::GameVersion {
            target: "1.21.1".to_string(),
        }),
        "with nothing recorded, the jar's declaration is the evidence"
    );
}

/// F-097: a lock written before the provider's declaration was recorded must
/// still scan. The absent field reads as "nothing recorded" (never a crash),
/// and the file is judged by its jar, exactly like an unrecorded one.
#[test]
fn spec_f097_a_lock_without_recorded_game_versions_still_scans() {
    let d = crate::test_tempdir();
    let p = d.path().join("instance.lock.json");
    std::fs::write(
        &p,
        r#"{"schema_version":1,"entries":[{"provider":"modrinth","project_id":"p",
        "slug":"s","version_id":"v","filename":"legacy.jar","sha1":"h","sha512":"f"}]}"#,
    )
    .expect("write an older lock");

    let lock = read_lock(&p).expect("a lock from an older build must still parse");
    assert_eq!(lock.entries.len(), 1);
    assert!(
        lock.entries[0].game_versions.is_empty(),
        "an absent value is 'nothing recorded', not an error"
    );

    let mut f = fact("legacy.jar", "h");
    f.game_versions = vec!["1.20".to_string()];
    let snapshot = classify_content(vec![f], &lock, &target("1.21.1", ""));

    assert_eq!(snapshot.facts[0].ownership, Ownership::ManagedBySha1);
    assert_eq!(
        snapshot.facts[0].compatibility,
        Compatibility::Incompatible(CompatibilityReason::GameVersion {
            target: "1.21.1".to_string(),
        })
    );
}

/// F-097: the axis is only real if the scan feeds it. The jar's own metadata is
/// the only local source, so a fabric jar in a fabric instance is *provable*,
/// not merely assumed.
#[test]
fn spec_f097_scan_reads_the_declared_loader_and_game_versions() {
    let d = crate::test_tempdir();
    let mods = d.path().join("mods");
    std::fs::create_dir_all(&mods).expect("mods dir");
    build_jar(
        &mods.join("declared.jar"),
        &[(
            "fabric.mod.json",
            br#"{"id":"tmod","depends":{"minecraft":"1.21.1"}}"#,
        )],
    );

    let facts = scan_mods(&instance_at(d.path()));
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].loaders, vec![Loader::Fabric]);
    assert_eq!(facts[0].game_versions, vec!["1.21.1".to_string()]);

    let snapshot = classify_content(facts, &LockFile::default(), &target("1.21.1", "fabric"));
    assert_eq!(snapshot.facts[0].compatibility, Compatibility::Compatible);
}

/// F-098: the axes are independent fields, so one file can be managed *and*
/// disabled *and* a duplicate *and* incompatible at once. Collapsing them into
/// one enum would throw three of the four away.
#[test]
fn spec_f098_managed_disabled_duplicate_and_incompatible_coexist() {
    let mut a = fact("a.jar.disabled", "same-hash");
    a.disabled = true;
    a.game_versions = vec!["1.20.1".to_string()];
    a.loaders = vec![Loader::Forge];

    let mut b = fact("disabled_mods/b.jar", "same-hash");
    b.disabled = true;
    b.game_versions = vec!["1.20.1".to_string()];
    b.loaders = vec![Loader::Forge];

    // The lock records both under their installed names, one of them before the
    // disable rename, so the join has to see through `.jar.disabled`.
    let lock = LockFile {
        entries: vec![
            lock_entry("a.jar", "same-project", "same-version", "same-hash"),
            lock_entry("b.jar", "same-project", "same-version", "same-hash"),
        ],
    };

    let snapshot = classify_content(vec![a, b], &lock, &target("1.21.1", "fabric"));
    assert_eq!(snapshot.facts.len(), 2);
    for f in &snapshot.facts {
        assert_eq!(f.ownership, Ownership::ManagedBySha1);
        assert!(f.disabled, "the activation axis survives the composition");
        assert_eq!(
            f.duplicate_keys,
            vec![
                DuplicateKey::Sha1("same-hash".to_string()),
                DuplicateKey::Project {
                    provider: "modrinth".to_string(),
                    project_id: "same-project".to_string(),
                },
            ]
        );
        assert_eq!(
            f.compatibility,
            Compatibility::Incompatible(CompatibilityReason::GameVersion {
                target: "1.21.1".to_string(),
            })
        );
        assert_eq!(
            f.class(),
            ContentClass::Disabled,
            "the label still prefers disabled"
        );
    }
    assert_eq!(
        snapshot.warnings,
        vec![
            ContentWarning::DuplicateLocalIdentity(DuplicateKey::Sha1("same-hash".to_string())),
            ContentWarning::DuplicateLocalIdentity(DuplicateKey::Project {
                provider: "modrinth".to_string(),
                project_id: "same-project".to_string(),
            }),
        ]
    );
}

/// F-099: the same bytes twice in one folder. The identity key is the recorded
/// SHA-1, and it is reported on both files plus once as a folder warning.
#[test]
fn spec_f099_identical_bytes_are_duplicates_by_sha1() {
    let d = crate::test_tempdir();
    let mods = d.path().join("mods");
    std::fs::create_dir_all(&mods).expect("mods dir");
    build_jar(&mods.join("one.jar"), &[("x.txt", b"same")]);
    build_jar(&mods.join("two.jar"), &[("x.txt", b"same")]);
    build_jar(&mods.join("unique.jar"), &[("x.txt", b"different")]);

    let facts = scan_mods(&instance_at(d.path()));
    let sha1 = facts
        .iter()
        .find(|f| f.filename == "one.jar")
        .expect("one")
        .sha1
        .clone();
    let snapshot = classify_content(facts, &LockFile::default(), &target("", ""));

    for name in ["one.jar", "two.jar"] {
        let f = snapshot
            .facts
            .iter()
            .find(|f| f.filename == name)
            .expect("fact");
        assert_eq!(f.sha1, sha1);
        assert_eq!(f.duplicate_keys, vec![DuplicateKey::Sha1(sha1.clone())]);
        assert_eq!(f.class(), ContentClass::Duplicate);
    }
    let unique = snapshot
        .facts
        .iter()
        .find(|f| f.filename == "unique.jar")
        .expect("unique");
    assert!(
        unique.duplicate_keys.is_empty(),
        "a file nobody shares is not a duplicate: {:?}",
        unique.duplicate_keys
    );
    assert_eq!(
        snapshot.warnings,
        vec![ContentWarning::DuplicateLocalIdentity(DuplicateKey::Sha1(
            sha1
        ))]
    );
}

/// F-099: two *versions* of one project are a duplicate too. Different bytes,
/// different version ids, one (provider, project) identity — and no SHA-1 key,
/// because the hashes really do differ.
#[test]
fn spec_f099_two_versions_of_one_project_are_duplicates_by_identity() {
    let lock = LockFile {
        entries: vec![
            lock_entry("a.jar", "same-project", "v1", "hash-a"),
            lock_entry("b.jar", "same-project", "v2", "hash-b"),
        ],
    };

    let snapshot = classify_content(
        vec![fact("a.jar", "hash-a"), fact("b.jar", "hash-b")],
        &lock,
        &target("", ""),
    );

    let project = DuplicateKey::Project {
        provider: "modrinth".to_string(),
        project_id: "same-project".to_string(),
    };
    for f in &snapshot.facts {
        assert_eq!(f.ownership, Ownership::ManagedBySha1);
        assert_eq!(f.duplicate_keys, vec![project.clone()]);
        assert_eq!(f.class(), ContentClass::Duplicate);
    }
    assert_eq!(
        snapshot.warnings,
        vec![ContentWarning::DuplicateLocalIdentity(project)]
    );
}

/// F-100: the lock is the record of what was installed, so a file whose bytes
/// no longer match it is still *managed* — and now visibly changed instead of
/// indistinguishable from an untouched one.
#[test]
fn spec_f100_a_locked_file_that_changed_reports_a_hash_mismatch() {
    let d = crate::test_tempdir();
    let mods = d.path().join("mods");
    std::fs::create_dir_all(&mods).expect("mods dir");
    build_jar(&mods.join("changed.jar"), &[("x.txt", b"original")]);
    build_jar(&mods.join("intact.jar"), &[("x.txt", b"untouched")]);

    let facts = scan_mods(&instance_at(d.path()));
    let intact_sha1 = facts
        .iter()
        .find(|f| f.filename == "intact.jar")
        .expect("intact fact")
        .sha1
        .clone();
    let lock = LockFile {
        entries: vec![
            lock_entry("changed.jar", "proj-changed", "v-changed", "0000000000"),
            lock_entry("intact.jar", "proj-intact", "v-intact", &intact_sha1),
        ],
    };

    let snapshot = classify_content(facts, &lock, &target("", ""));

    let changed = snapshot
        .facts
        .iter()
        .find(|f| f.filename == "changed.jar")
        .expect("changed fact");
    assert_eq!(
        changed.ownership,
        Ownership::ManagedByVersion,
        "identity still matches the lock, so the file is managed"
    );
    assert!(
        changed.hash_mismatch,
        "the bytes no longer match the record"
    );
    assert_eq!(changed.class(), ContentClass::Managed);

    let intact = snapshot
        .facts
        .iter()
        .find(|f| f.filename == "intact.jar")
        .expect("intact fact");
    assert_eq!(intact.ownership, Ownership::ManagedBySha1);
    assert!(!intact.hash_mismatch, "an untouched file is not a mismatch");

    assert_eq!(
        snapshot.warnings,
        vec![ContentWarning::ManagedHashMismatch {
            rel_path: "changed.jar".to_string(),
        }]
    );
}
