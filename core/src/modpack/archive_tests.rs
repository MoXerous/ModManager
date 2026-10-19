//! Archive-reader tests. Every fixture is a real `.mrpack`-shaped zip built
//! here, because the point of these tests is the path from bytes on disk to a
//! plan, not the JSON parser that `mrpack::tests` already covers.

use std::io::{Cursor, Write};

use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use super::archive::{plan_install, read_pack_bounded, InstallOp, PackError, MAX_INDEX_BYTES};
use crate::safety::inspect::ArchiveLimits;

const INDEX: &str = r#"{"formatVersion":1,"game":"minecraft","versionId":"v1","name":"Pack",
 "files":[{"path":"mods/sodium.jar","hashes":{"sha1":"a","sha512":"bb"},
           "downloads":["https://cdn.modrinth.com/x.jar"],"fileSize":10}]}"#;

fn zip_of(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut w = ZipWriter::new(Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default();
    for (name, body) in entries {
        w.start_file(*name, opts).expect("start_file");
        w.write_all(body.as_bytes()).expect("write");
    }
    w.finish().expect("finish").into_inner()
}

fn open(bytes: &[u8]) -> (super::mrpack::Pack, Vec<crate::safety::inspect::MemberInfo>) {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes.to_vec())).expect("zip");
    read_pack_bounded(&mut z, &ArchiveLimits::default(), 1 << 20).expect("read_pack")
}

#[test]
fn mrpack_a_well_formed_pack_plans_one_download_per_index_file() {
    let bytes = zip_of(&[
        ("modrinth.index.json", INDEX),
        ("overrides/config/x.toml", "a = 1"),
    ]);
    let (pack, inventory) = open(&bytes);
    let plan = plan_install(&pack, &inventory).expect("plan");

    assert_eq!(plan.downloads(), 1);
    assert_eq!(plan.overrides(), 1);
    assert!(plan.ops.contains(&InstallOp::Download {
        safe_path: "mods/sodium.jar".to_string(),
        sha512: "bb".to_string(),
        size: Some(10),
    }));
    assert!(plan.ops.contains(&InstallOp::CopyOverride {
        safe_path: "config/x.toml".to_string()
    }));
}

#[test]
fn mrpack_an_archive_without_an_index_is_refused() {
    let bytes = zip_of(&[("overrides/config/x.toml", "a = 1")]);
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
    assert!(matches!(
        read_pack_bounded(&mut z, &ArchiveLimits::default(), 1 << 20),
        Err(PackError::MissingIndex(_))
    ));
}

#[test]
fn mrpack_an_oversized_index_is_refused_before_it_is_read() {
    let bytes = zip_of(&[("modrinth.index.json", INDEX)]);
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
    match read_pack_bounded(&mut z, &ArchiveLimits::default(), 8) {
        Err(PackError::IndexTooLarge { limit: 8, .. }) => {}
        other => panic!("expected IndexTooLarge, got {other:?}"),
    }
    // The production ceiling still admits the same archive.
    let mut z2 =
        zip::ZipArchive::new(Cursor::new(zip_of(&[("modrinth.index.json", INDEX)]))).expect("zip");
    assert!(read_pack_bounded(&mut z2, &ArchiveLimits::default(), MAX_INDEX_BYTES).is_ok());
}

#[test]
fn mrpack_a_traversing_override_member_is_refused_by_the_inspector() {
    // `inspect_zip` runs first and rejects the name outright, so the reader
    // never reaches the index. Two gates, and the outer one is the older.
    let bytes = zip_of(&[
        ("modrinth.index.json", INDEX),
        ("overrides/../../escape.toml", "nope"),
    ]);
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
    assert!(matches!(
        read_pack_bounded(&mut z, &ArchiveLimits::default(), 1 << 20),
        Err(PackError::Inspect(_))
    ));
}

#[test]
fn mrpack_a_later_layer_wins_the_same_path_instead_of_adding_it_twice() {
    let bytes = zip_of(&[
        ("modrinth.index.json", INDEX),
        ("overrides/options.txt", "base"),
        ("server-overrides/options.txt", "server"),
    ]);
    let (pack, inventory) = open(&bytes);
    let plan = plan_install(&pack, &inventory).expect("plan");

    // One file, one op. A plan that listed it twice would promise a second
    // write to the same destination and hide which one actually lands.
    assert_eq!(plan.overrides(), 1, "ops were {:?}", plan.ops);
    assert_eq!(plan.downloads(), 1);
}

#[test]
fn mrpack_members_outside_a_declared_layer_are_not_installed() {
    let bytes = zip_of(&[
        ("modrinth.index.json", INDEX),
        ("random/payload.bin", "junk"),
    ]);
    let (pack, inventory) = open(&bytes);
    let plan = plan_install(&pack, &inventory).expect("plan");
    assert_eq!(plan.overrides(), 0, "ops were {:?}", plan.ops);
}
