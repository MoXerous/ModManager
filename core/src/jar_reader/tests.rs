//! Reader tests: zips built in-test, temp dirs only, no binaries.

use super::cache::ScanCache;
use super::loader::{detect_loaders, scan_jars_for_loader, Loader};
use super::*;
use std::io::Write;

pub(crate) fn build_jar(path: &std::path::Path, entries: &[(&str, &[u8])]) {
    let f = std::fs::File::create(path).expect("jar file");
    let mut w = zip::ZipWriter::new(f);
    for (name, data) in entries {
        w.start_file(*name, zip::write::SimpleFileOptions::default())
            .expect("entry");
        w.write_all(data).expect("write");
    }
    w.finish().expect("finish");
}

const FABRIC: &[u8] = br#"{
    "id": "tmod", "name": "T Mod", "version": "1.0.0",
    "description": "d", "depends": {"minecraft": "1.21"}
}"#;

const QUILT_LIST: &[u8] = br#"{
    "quilt_loader": {"id": "qmod",
        "metadata": {"name": "Q Mod"},
        "version": "2.0.0",
        "depends": [{"id": "minecraft", "versions": ["1.21"]}]}
}"#;

const QUILT_MAP: &[u8] = br#"{
    "quilt_loader": {"id": "qmod",
        "metadata": {"name": "Q Mod"},
        "version": "2.0.0",
        "depends": {"minecraft": {"versions": "1.20"}}}
}"#;

const FORGE: &[u8] = br#"
modLoader="javafml"
loaderVersion="[47,)"
[[mods]]
modId="fmod"
displayName="F Mod"
version="3.0.0"
description="d"
[[dependencies.fmod]]
modId="minecraft"
versionRange="[1.20.1,1.21.2)"
"#;

const NEOFORGE: &[u8] = br#"
modLoader="javafml"
loaderVersion="[4,)"
[[mods]]
modId="nmod"
displayName="N Mod"
version="4.0.0"
description="d"
[[dependencies.nmod]]
modId="minecraft"
versionRange="[1.21,)"
"#;

fn jar_in(dir: &std::path::Path, name: &str, entries: &[(&str, &[u8])]) -> std::path::PathBuf {
    let p = dir.join(name);
    build_jar(&p, entries);
    p
}

#[test]
fn spec_f085_reads_fabric_metadata() {
    let d = crate::test_tempdir();
    let p = jar_in(d.path(), "a.jar", &[("fabric.mod.json", FABRIC)]);
    let m = read_local_mod_metadata(&p).expect("metadata");
    assert_eq!(m.mod_id, "tmod");
    assert_eq!(m.display_name(), "T Mod");
    assert_eq!(m.version, "1.0.0");
    assert_eq!(m.loaders, vec![Loader::Fabric]);
    assert_eq!(m.game_versions, vec!["1.21".to_string()]);
    assert_eq!(m.source_file, p);
}

#[test]
fn spec_f085_reads_quilt_list_depends() {
    let d = crate::test_tempdir();
    let p = jar_in(d.path(), "q.jar", &[("quilt.mod.json", QUILT_LIST)]);
    let m = read_local_mod_metadata(&p).expect("metadata");
    assert_eq!(m.mod_id, "qmod");
    assert_eq!(m.loaders, vec![Loader::Quilt]);
    assert_eq!(m.game_versions, vec!["1.21".to_string()]);
}

#[test]
fn spec_f085_reads_quilt_map_depends() {
    let d = crate::test_tempdir();
    let p = jar_in(d.path(), "q.jar", &[("quilt.mod.json", QUILT_MAP)]);
    let m = read_local_mod_metadata(&p).expect("metadata");
    assert_eq!(m.game_versions, vec!["1.20".to_string()]);
}

#[test]
fn spec_f086_forge_keeps_raw_version_range() {
    let d = crate::test_tempdir();
    let p = jar_in(d.path(), "f.jar", &[("META-INF/mods.toml", FORGE)]);
    let m = read_local_mod_metadata(&p).expect("metadata");
    assert_eq!(m.mod_id, "fmod");
    assert_eq!(m.loaders, vec![Loader::Forge]);
    assert_eq!(m.game_versions, vec!["[1.20.1,1.21.2)".to_string()]);
}

#[test]
fn spec_f088_reads_neoforge_metadata() {
    let d = crate::test_tempdir();
    let p = jar_in(
        d.path(),
        "n.jar",
        &[("META-INF/neoforge.mods.toml", NEOFORGE)],
    );
    let m = read_local_mod_metadata(&p).expect("metadata");
    assert_eq!(m.mod_id, "nmod");
    assert_eq!(m.loaders, vec![Loader::NeoForge]);
    assert_eq!(m.game_versions, vec!["[1.21,)".to_string()]);
}

#[test]
fn spec_f089_first_wins_fabric_over_forge() {
    let d = crate::test_tempdir();
    let p = jar_in(
        d.path(),
        "both.jar",
        &[("fabric.mod.json", FABRIC), ("META-INF/mods.toml", FORGE)],
    );
    let m = read_local_mod_metadata(&p).expect("metadata");
    assert_eq!(m.loaders, vec![Loader::Fabric]);
    assert_eq!(m.mod_id, "tmod");
}

#[test]
fn spec_f089_no_metadata_returns_none() {
    let d = crate::test_tempdir();
    let p = jar_in(d.path(), "plain.jar", &[("random.txt", b"hi")]);
    assert_eq!(read_local_mod_metadata(&p), None);
}

#[test]
fn spec_f085_corrupt_jar_returns_none() {
    let d = crate::test_tempdir();
    let p = d.path().join("bad.jar");
    std::fs::write(&p, b"not a zip at all").expect("write");
    assert_eq!(read_local_mod_metadata(&p), None);
    // Missing file is also None, never Err (OSError ⇒ None).
    assert_eq!(read_local_mod_metadata(&d.path().join("missing.jar")), None);
}

#[test]
fn spec_f085_oversize_entry_returns_none() {
    let d = crate::test_tempdir();
    let big = vec![b'x'; (MAX_METADATA_BYTES + 16) as usize];
    let p = jar_in(d.path(), "big.jar", &[("fabric.mod.json", &big)]);
    assert_eq!(read_local_mod_metadata(&p), None);
}

#[test]
fn spec_f085_cleans_placeholders_and_truncates_160() {
    let d = crate::test_tempdir();
    let long = "y".repeat(200);
    let body = format!(
        r#"{{"id": "m", "name": "  Hello ${{name}}   World  ", "version": "{long}", "description": "d"}}"#
    );
    let p = jar_in(d.path(), "c.jar", &[("fabric.mod.json", body.as_bytes())]);
    let m = read_local_mod_metadata(&p).expect("metadata");
    assert_eq!(m.name, "Hello World");
    assert_eq!(m.version.len(), 160);
}

#[test]
fn spec_f085_versions_dedupe_and_cap_8() {
    let d = crate::test_tempdir();
    let body = r#"{"id": "m", "version": "1",
        "depends": {"minecraft": ["", "*", "1.21", "1.21", "2", "3", "4", "5", "6", "7", "8", "9"]}}"#;
    let p = jar_in(d.path(), "v.jar", &[("fabric.mod.json", body.as_bytes())]);
    let m = read_local_mod_metadata(&p).expect("metadata");
    assert_eq!(m.game_versions.len(), 8);
    assert_eq!(m.game_versions[0], "1.21");
    assert!(!m.game_versions.contains(&"*".to_string()));
}

#[test]
fn spec_f085_strips_utf8_bom() {
    let d = crate::test_tempdir();
    let mut body = vec![0xEF, 0xBB, 0xBF];
    body.extend_from_slice(FABRIC);
    let p = jar_in(d.path(), "bom.jar", &[("fabric.mod.json", &body)]);
    let m = read_local_mod_metadata(&p).expect("metadata");
    assert_eq!(m.mod_id, "tmod");
}

#[test]
fn spec_f090_classifies_five_types() {
    type Case<'a> = (&'a str, &'a [(&'a str, &'a [u8])], ResourceType);
    let d = crate::test_tempdir();
    let cases: &[Case] = &[
        ("m.jar", &[("fabric.mod.json", FABRIC)], ResourceType::Mod),
        (
            "s.zip",
            &[("shaders/x.fsh", b"v")],
            ResourceType::Shaderpack,
        ),
        (
            "d.zip",
            &[("pack.mcmeta", b"{}"), ("data/x", b"v")],
            ResourceType::Datapack,
        ),
        (
            "r.zip",
            &[("My Pack/pack.mcmeta", b"{}"), ("assets/y", b"v")],
            ResourceType::Resourcepack,
        ),
        ("u.zip", &[("notes.txt", b"hi")], ResourceType::Unknown),
    ];
    for (name, entries, want) in cases {
        let p = jar_in(d.path(), name, entries);
        assert_eq!(classify_local_resource_type(&p), *want, "{name}");
    }
}

#[test]
fn spec_f093_scan_jars_sorted_and_capped_at_30() {
    let d = crate::test_tempdir();
    for i in 0..35 {
        let name = format!("m{i:02}.jar");
        jar_in(d.path(), &name, &[("x.txt", b"v")]);
    }
    let jars = scan_jars_for_loader(d.path());
    assert_eq!(jars.len(), 30);
    let first = jars[0]
        .file_name()
        .expect("name")
        .to_string_lossy()
        .into_owned();
    let last = jars[29]
        .file_name()
        .expect("name")
        .to_string_lossy()
        .into_owned();
    assert_eq!(first, "m00.jar");
    assert_eq!(last, "m29.jar");
}

#[test]
fn spec_f093_detect_guard_skips_scan_when_authoritative() {
    let d = crate::test_tempdir();
    // Corrupt jar present: authoritative loaders must win without scanning.
    std::fs::write(d.path().join("bad.jar"), b"garbage").expect("write");
    let mut cache = ScanCache::new();
    let out = detect_loaders(&[Loader::Fabric], d.path(), &mut cache);
    assert_eq!(out, vec![Loader::Fabric]);
}

#[test]
fn spec_f085_scan_cache_skips_unchanged_jar() {
    let d = crate::test_tempdir();
    let p = jar_in(d.path(), "a.jar", &[("fabric.mod.json", FABRIC)]);
    let mut cache = ScanCache::new();
    let m1 = cache.get(&p);
    let m2 = cache.get(&p);
    assert!(m1.is_some() && m2.is_some());
    assert_eq!(cache.reads, 1);
    // Rewrite with different metadata: mtime changes ⇒ rescan.
    std::fs::remove_file(&p).expect("rm");
    jar_in(d.path(), "a.jar", &[("quilt.mod.json", QUILT_LIST)]);
    let m3 = cache.get(&p);
    assert_eq!(cache.reads, 2);
    assert_eq!(m3.expect("meta").loaders, vec![Loader::Quilt]);
}
