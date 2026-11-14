//! Archive-reader tests. Every fixture is a real `.mrpack`-shaped zip built
//! here, because the point of these tests is the path from bytes on disk to a
//! plan, not the JSON parser that `mrpack::tests` already covers.

use std::io::{Cursor, Write};

use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use super::archive::{plan_install, read_pack_bounded, InstallOp, PackError, MAX_INDEX_BYTES};
use super::mrpack::IndexError;
use crate::safety::inspect::ArchiveLimits;

fn sha128(hex: char) -> String {
    (0..128).map(|_| hex).collect()
}

fn index_json(files: &str, extra: &str) -> String {
    format!(
        r#"{{"formatVersion":1,"game":"minecraft","versionId":"v1","name":"Pack",
         "files":[{files}]{extra}}}"#
    )
}

fn one_download_for(path: &str, sha: &str) -> String {
    format!(
        r#"{{"path":"{path}","hashes":{{"sha1":"a","sha512":"{sha}"}},
          "downloads":["https://cdn.modrinth.com/x.jar"],"fileSize":10}}"#
    )
}

/// An index naming one download; `extra` splices in pack-level fields.
fn index_for(file_path: &str, extra: &str) -> String {
    index_json(&one_download_for(file_path, &sha128('a')), extra)
}

/// The well-formed single-download index the suite shares, now that a real
/// SHA-512 stub is 128 hex characters long and `String`-sized.
fn default_index() -> String {
    index_for("mods/sodium.jar", "")
}

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
fn spec_f069_a_well_formed_pack_plans_one_download_per_index_file() {
    let bytes = zip_of(&[
        ("modrinth.index.json", &default_index()),
        ("overrides/config/x.toml", "a = 1"),
    ]);
    let (pack, inventory) = open(&bytes);
    let plan = plan_install(&pack, &inventory).expect("plan");

    assert_eq!(plan.downloads(), 1);
    assert_eq!(plan.overrides(), 1);
    assert!(plan.ops.contains(&InstallOp::Download {
        safe_path: "mods/sodium.jar".to_string(),
        sha512: sha128('a'),
        size: Some(10),
    }));
    assert!(plan.ops.contains(&InstallOp::CopyOverride {
        safe_path: "config/x.toml".to_string(),
        source_layer: "overrides".to_string(),
    }));
}

#[test]
fn spec_f070_an_archive_without_an_index_is_refused() {
    let bytes = zip_of(&[("overrides/config/x.toml", "a = 1")]);
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
    assert!(matches!(
        read_pack_bounded(&mut z, &ArchiveLimits::default(), 1 << 20),
        Err(PackError::MissingIndex(_))
    ));
}

#[test]
fn spec_f070_an_oversized_index_is_refused_before_it_is_read() {
    let bytes = zip_of(&[("modrinth.index.json", &default_index())]);
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
    match read_pack_bounded(&mut z, &ArchiveLimits::default(), 8) {
        Err(PackError::IndexTooLarge { limit: 8, .. }) => {}
        other => panic!("expected IndexTooLarge, got {other:?}"),
    }
    // The production ceiling still admits the same archive.
    let mut z2 = zip::ZipArchive::new(Cursor::new(zip_of(&[(
        "modrinth.index.json",
        &default_index(),
    )])))
    .expect("zip");
    assert!(read_pack_bounded(&mut z2, &ArchiveLimits::default(), MAX_INDEX_BYTES).is_ok());
}

#[test]
fn spec_f072_a_traversing_override_member_is_refused_by_the_inspector() {
    // `inspect_zip` runs first and rejects the name outright, so the reader
    // never reaches the index. Two gates, and the outer one is the older.
    let bytes = zip_of(&[
        ("modrinth.index.json", &default_index()),
        ("overrides/../../escape.toml", "nope"),
    ]);
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
    assert!(matches!(
        read_pack_bounded(&mut z, &ArchiveLimits::default(), 1 << 20),
        Err(PackError::Inspect(_))
    ));
}

#[test]
fn spec_f073_client_overrides_win_over_overrides_regardless_of_archive_order() {
    // The zip lists the client layer first on purpose. The central directory
    // says nothing about precedence, so a planner that follows archive order
    // hands the win to `overrides` even though the format says otherwise.
    let bytes = zip_of(&[
        ("modrinth.index.json", &default_index()),
        ("client-overrides/options.txt", "client"),
        ("overrides/options.txt", "base"),
    ]);
    let (pack, inventory) = open(&bytes);
    let plan = plan_install(&pack, &inventory).expect("plan");

    // One file, one op. A plan that listed it twice would promise a second
    // write to the same destination and hide which one actually lands.
    assert_eq!(plan.overrides(), 1, "ops were {:?}", plan.ops);
    assert_eq!(plan.downloads(), 1);
    assert!(
        plan.ops.contains(&InstallOp::CopyOverride {
            safe_path: "options.txt".to_string(),
            source_layer: "client-overrides".to_string(),
        }),
        "the client layer must win, not the last archive entry: {:?}",
        plan.ops
    );
    assert_eq!(plan.skips.server_overrides, 0);
}

#[test]
fn spec_f074_server_overrides_are_counted_and_never_installed() {
    let bytes = zip_of(&[
        ("modrinth.index.json", &default_index()),
        ("server-overrides/server.properties", "online-mode=false"),
        ("server-overrides/config/server.toml", "x"),
        ("overrides/options.txt", "base"),
    ]);
    let (pack, inventory) = open(&bytes);
    let plan = plan_install(&pack, &inventory).expect("plan");

    assert_eq!(plan.skips.server_overrides, 2);
    assert_eq!(plan.overrides(), 1);
    assert!(
        plan.ops.iter().all(|op| match op {
            InstallOp::CopyOverride { source_layer, .. } => source_layer != "server-overrides",
            InstallOp::Download { .. } => true,
        }),
        "a client install must never write the server layer: {:?}",
        plan.ops
    );
}

#[test]
fn spec_f074_remote_files_the_client_cannot_use_are_skipped_and_counted() {
    let optional = format!(
        r#"{{"path":"mods/optional.jar","hashes":{{"sha1":"a","sha512":"{}"}},
          "downloads":["https://cdn.modrinth.com/o.jar"],"fileSize":10,
          "env":{{"client":"optional","server":"required"}}}}"#,
        sha128('b')
    );
    let unsupported = format!(
        r#"{{"path":"mods/server-only.jar","hashes":{{"sha1":"a","sha512":"{}"}},
          "downloads":["https://cdn.modrinth.com/s.jar"],"fileSize":10,
          "env":{{"client":"unsupported","server":"required"}}}}"#,
        sha128('c')
    );
    let files = format!(
        "{},{},{}",
        one_download_for("mods/required.jar", &sha128('a')),
        optional,
        unsupported
    );
    let index = index_json(&files, "");
    let bytes = zip_of(&[("modrinth.index.json", &index)]);
    let (pack, inventory) = open(&bytes);
    let plan = plan_install(&pack, &inventory).expect("plan");

    assert_eq!(
        plan.downloads(),
        1,
        "only the required file is fetched: {:?}",
        plan.ops
    );
    assert_eq!(plan.skips.remote_optional, 1);
    assert_eq!(plan.skips.remote_unsupported, 1);
    assert!(plan.ops.contains(&InstallOp::Download {
        safe_path: "mods/required.jar".to_string(),
        sha512: sha128('a'),
        size: Some(10),
    }));
}

#[test]
fn spec_f069_members_outside_a_declared_layer_are_not_installed() {
    let bytes = zip_of(&[
        ("modrinth.index.json", &default_index()),
        ("random/payload.bin", "junk"),
    ]);
    let (pack, inventory) = open(&bytes);
    let plan = plan_install(&pack, &inventory).expect("plan");
    assert_eq!(plan.overrides(), 0, "ops were {:?}", plan.ops);
}

#[test]
fn spec_f069_an_override_wins_a_destination_the_index_also_names() {
    let index = index_for("mods/x.jar", "");
    let bytes = zip_of(&[
        ("modrinth.index.json", &index),
        ("overrides/mods/x.jar", "bundled bytes"),
    ]);
    let (pack, inventory) = open(&bytes);
    let plan = plan_install(&pack, &inventory).expect("plan");

    // One operation for one destination: the override layers are applied
    // after the index files, so the bundled copy is the author's last word
    // and the download would only be fetched to be replaced.
    assert_eq!(plan.downloads(), 0, "ops were {:?}", plan.ops);
    assert_eq!(plan.overrides(), 1, "ops were {:?}", plan.ops);
    assert!(plan.ops.contains(&InstallOp::CopyOverride {
        safe_path: "mods/x.jar".to_string(),
        source_layer: "overrides".to_string(),
    }));
}

#[test]
fn spec_f070_an_index_file_may_not_replace_an_app_managed_file() {
    for name in ["profile.json", "PROFILE.JSON", "instance.lock.json"] {
        let index = index_for(name, "");
        let bytes = zip_of(&[("modrinth.index.json", &index)]);
        let mut z = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
        match read_pack_bounded(&mut z, &ArchiveLimits::default(), 1 << 20) {
            Err(PackError::Index(IndexError::ManagedPath { name: hit, .. })) => {
                assert!(
                    hit.eq_ignore_ascii_case(name),
                    "the refusal must name the file: {hit:?} for {name:?}"
                );
            }
            other => panic!("{name}: expected ManagedPath, got {other:?}"),
        }
    }
}

#[test]
fn spec_f072_an_override_may_not_replace_an_app_managed_file() {
    for member in ["overrides/profile.json", "overrides/instance.lock.json"] {
        let bytes = zip_of(&[("modrinth.index.json", &default_index()), (member, "{}")]);
        let (pack, inventory) = open(&bytes);
        match plan_install(&pack, &inventory) {
            Err(PackError::ManagedPath { .. }) => {}
            other => panic!("{member}: expected ManagedPath, got {other:?}"),
        }
    }
}

#[test]
fn spec_f072_a_pack_may_not_write_into_the_staging_directory() {
    let index = index_for(".mm-staging/evil.jar", "");
    let bytes = zip_of(&[("modrinth.index.json", &index)]);
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
    assert!(matches!(
        read_pack_bounded(&mut z, &ArchiveLimits::default(), 1 << 20),
        Err(PackError::Index(IndexError::StagingPath { .. }))
    ));

    let bytes = zip_of(&[
        ("modrinth.index.json", &default_index()),
        ("overrides/.mm-staging/evil.jar", "x"),
    ]);
    let (pack, inventory) = open(&bytes);
    assert!(matches!(
        plan_install(&pack, &inventory),
        Err(PackError::StagingPath { .. })
    ));
}

#[test]
fn spec_f070_a_file_entry_ending_in_a_slash_is_refused() {
    let index = index_for("mods/", "");
    let bytes = zip_of(&[("modrinth.index.json", &index)]);
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
    assert!(matches!(
        read_pack_bounded(&mut z, &ArchiveLimits::default(), 1 << 20),
        Err(PackError::Index(IndexError::DirectoryPath { .. }))
    ));
}

#[test]
fn spec_f069_a_custom_layer_name_in_the_index_does_not_create_a_layer() {
    // Layer folders are the format's, not the index's: a `clientOverrides`
    // field pointing at `cli/` must not make `cli/` an override layer.
    let index = index_for("mods/sodium.jar", r#","clientOverrides":"cli""#);
    let bytes = zip_of(&[
        ("modrinth.index.json", &index),
        ("cli/options.txt", "hidden"),
        ("client-overrides/options.txt", "client"),
    ]);
    let (pack, inventory) = open(&bytes);
    let plan = plan_install(&pack, &inventory).expect("plan");
    assert_eq!(plan.overrides(), 1, "ops were {:?}", plan.ops);
    assert!(plan.ops.contains(&InstallOp::CopyOverride {
        safe_path: "options.txt".to_string(),
        source_layer: "client-overrides".to_string(),
    }));
}
