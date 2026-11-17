//! Archive-reader tests. Every fixture is a real `.mrpack`-shaped zip built
//! here, because the point of these tests is the path from bytes on disk to a
//! plan, not the JSON parser that `mrpack::tests` already covers.

use std::io::{Cursor, Write};

use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use super::archive::{
    plan_install, preview_export_archive_bounded, read_pack_bounded, InstallOp, PackError,
    MAX_INDEX_BYTES,
};
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

// --- F-158: importing this app's own export archive -------------------------

use super::archive::{preview_export_archive, ExportArchivePreview, ExportImportError};
use crate::modpack::export::{
    manifest_json, Exclusion, ExportMod, ExportPlan, ExportSkip, OverrideFile, SkipReason,
};

fn sha512_bytes(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha512::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A manifest shaped exactly like the exporter writes it, and naming the same
/// pack the shared `default_index()` names, with one file, one override, one
/// skip and one exclusion. The index digest is the digest of the index bytes
/// the fixture's archive actually carries, so the manifest passes the gate a
/// real export passes.
fn export_manifest_json() -> String {
    let plan = ExportPlan {
        name: "Pack".to_string(),
        version_id: "v1".to_string(),
        summary: Some("Summary".to_string()),
        game_version: "1.21.1".to_string(),
        loader: "fabric".to_string(),
        files: vec![ExportMod {
            project_id: "p1".to_string(),
            version_id: "v1".to_string(),
            title: "Sodium".to_string(),
            project_type: "mod".to_string(),
            folder: "mods",
            file_name: "sodium.jar".to_string(),
            url: "https://cdn.modrinth.com/s.jar".to_string(),
            size: 10,
            sha1: "a".repeat(40),
            sha512: sha128('a'),
        }],
        overrides: vec![OverrideFile {
            archive_path: "overrides/config/x.toml".to_string(),
            source: std::path::PathBuf::new(),
            size: 5,
        }],
        skipped: vec![ExportSkip {
            project_id: "c1".to_string(),
            version_id: "cv1".to_string(),
            reason: SkipReason::UnsupportedProvider,
        }],
        excluded: vec![crate::modpack::export::ExcludedEntry {
            path: "config/debug.log".to_string(),
            reason: Exclusion::Secret,
        }],
        lock_sha512: sha128('a'),
    };
    serde_json::to_string(&manifest_json(
        &plan,
        &sha512_bytes(default_index().as_bytes()),
    ))
    .expect("manifest json")
}

/// The fixture manifest rewritten at one JSON pointer: the manifest is
/// attacker-controlled text, and every identity check below is a refusal the
/// tampering must meet.
fn manifest_with(mutate: impl FnOnce(&mut serde_json::Value)) -> String {
    let mut v: serde_json::Value =
        serde_json::from_str(&export_manifest_json()).expect("fixture manifest");
    mutate(&mut v);
    v.to_string()
}

/// An export archive: the index the format wants, the manifest this app adds,
/// and one override the executor would copy.
fn export_bytes(manifest: Option<&str>) -> Vec<u8> {
    let mut entries: Vec<(&str, String)> = vec![("modrinth.index.json", default_index())];
    if let Some(m) = manifest {
        entries.push(("export_manifest.json", m.to_string()));
    }
    entries.push(("overrides/config/x.toml", "a = 1".to_string()));
    let refs: Vec<(&str, &str)> = entries.iter().map(|(n, b)| (*n, b.as_str())).collect();
    zip_of(&refs)
}

fn preview(bytes: &[u8]) -> Result<ExportArchivePreview, ExportImportError> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes.to_vec())).expect("zip");
    preview_export_archive(&mut z, &ArchiveLimits::default())
}

#[test]
fn spec_f158_preview_reads_the_export_manifest_without_extracting() {
    let bytes = export_bytes(Some(&export_manifest_json()));
    let p = preview(&bytes).expect("our own export is previewable");
    assert_eq!(p.name, "Pack");
    assert_eq!(p.version_id, "v1");
    assert_eq!(p.game_version, "1.21.1");
    assert_eq!(p.loader, "fabric");
    assert_eq!(p.lock_sha512, sha128('a'));
    assert_eq!(p.file_count, 1);
    assert_eq!(p.override_count, 1);
    assert_eq!(p.skipped_count, 1);
    assert_eq!(
        p.excluded,
        vec![("config/debug.log".to_string(), "secret".to_string())]
    );
}

#[test]
fn spec_f158_preview_refuses_a_foreign_archive_without_the_manifest() {
    // A plain .mrpack is not this app's export: no manifest, no import.
    let bytes = zip_of(&[("modrinth.index.json", &default_index())]);
    assert!(matches!(
        preview(&bytes),
        Err(ExportImportError::NotOurExport(_))
    ));
}

#[test]
fn spec_f158_preview_refuses_an_unsupported_manifest_schema() {
    let foreign = r#"{"schemaVersion":99,"name":"x","versionId":"v","gameVersion":"1","loader":"fabric","lockSha512":"","included":{"files":[],"overrides":[]},"skipped":[],"excluded":[]}"#;
    let bytes = export_bytes(Some(foreign));
    match preview(&bytes) {
        Err(ExportImportError::SchemaVersion { found: 99, .. }) => {}
        other => panic!("expected a schema refusal, got {other:?}"),
    }
}

#[test]
fn spec_f158_preview_refuses_a_malformed_manifest() {
    let bytes = export_bytes(Some("{not json"));
    assert!(matches!(
        preview(&bytes),
        Err(ExportImportError::ManifestJson(_))
    ));
}

#[test]
fn spec_f158_import_refuses_a_symlink_member_instead_of_skipping_it() {
    // The manifest never gets read: the shared inspector refuses the archive
    // outright, which is also what a `..` member meets (F-072 pins that one).
    // The mutation check below is the point: the ONLY difference between the
    // refused archive and the accepted control is the member's mode bits, so
    // the refusal is the link rule alone — a reader that skips links instead
    // of refusing them fails this test.
    let link = archive_with_link_member(false);
    assert!(matches!(
        preview(&link),
        Err(ExportImportError::Pack(PackError::Inspect(_)))
    ));

    // The control: the same archive with the member as a regular file. The
    // manifest is consistent with it (one file, one override member), so the
    // preview succeeds — proving the refusal came from the link, not from the
    // manifest, the counts or the index.
    let regular = archive_with_link_member(true);
    let p = preview(&regular).expect("a regular member is not a link");
    assert_eq!(p.name, "Pack");
}

/// The archive the symlink test builds: the index, a manifest consistent with
/// one file and one override member, and `overrides/link.toml` — a symlink
/// when `regular` is false, a plain file when it is true.
fn archive_with_link_member(regular: bool) -> Vec<u8> {
    let plan = ExportPlan {
        name: "Pack".to_string(),
        version_id: "v1".to_string(),
        game_version: "1.21.1".to_string(),
        loader: "fabric".to_string(),
        files: vec![ExportMod {
            project_id: "p1".to_string(),
            version_id: "v1".to_string(),
            title: "Sodium".to_string(),
            project_type: "mod".to_string(),
            folder: "mods",
            file_name: "sodium.jar".to_string(),
            url: "https://cdn.modrinth.com/s.jar".to_string(),
            size: 10,
            sha1: "a".repeat(40),
            sha512: sha128('a'),
        }],
        overrides: vec![OverrideFile {
            archive_path: "overrides/link.toml".to_string(),
            source: std::path::PathBuf::new(),
            size: 9,
        }],
        lock_sha512: sha128('a'),
        ..ExportPlan::default()
    };
    let manifest = serde_json::to_string(&manifest_json(
        &plan,
        &sha512_bytes(default_index().as_bytes()),
    ))
    .expect("manifest json");

    let mut w = ZipWriter::new(Cursor::new(Vec::new()));
    if regular {
        w.start_file("overrides/link.toml", SimpleFileOptions::default())
            .expect("regular member");
    } else {
        w.start_file(
            "overrides/link.toml",
            SimpleFileOptions::default().external_attributes(0o120777 << 16),
        )
        .expect("link member");
    }
    w.write_all(b"../escape").expect("write");
    w.start_file("modrinth.index.json", SimpleFileOptions::default())
        .expect("index");
    w.write_all(default_index().as_bytes()).expect("write");
    w.start_file("export_manifest.json", SimpleFileOptions::default())
        .expect("manifest");
    w.write_all(manifest.as_bytes()).expect("write");
    w.finish().expect("finish").into_inner()
}

// --- identity: the manifest must name THIS archive ---------------------------

#[test]
fn spec_f158_preview_refuses_a_manifest_naming_another_pack() {
    let foreign = manifest_with(|v| {
        v["name"] = "Other Pack".into();
        v["versionId"] = "v9".into();
    });
    assert!(matches!(
        preview(&export_bytes(Some(&foreign))),
        Err(ExportImportError::ManifestJson(_))
    ));
}

/// The transplant attack: a hand-written manifest claiming another archive's
/// identity. The index digest it records is not the digest of the index in
/// this archive, and that is a refusal of its own, not a malformed-JSON note.
#[test]
fn spec_f158_preview_refuses_a_manifest_whose_index_digest_disagrees() {
    let foreign = manifest_with(|v| {
        v["indexSha512"] = sha128('e').into();
    });
    match preview(&export_bytes(Some(&foreign))) {
        Err(ExportImportError::IndexDigest { .. }) => {}
        other => panic!("expected an index-digest refusal, got {other:?}"),
    }
}

#[test]
fn spec_f158_preview_refuses_manifest_counts_that_disagree_with_the_archive() {
    // One override too few: the manifest cannot claim less than the archive
    // carries, or the preview under-reports what the import will write.
    let fewer = manifest_with(|v| {
        v["included"]["overrides"] = serde_json::json!([]);
    });
    match preview(&export_bytes(Some(&fewer))) {
        Err(ExportImportError::CountMismatch {
            field,
            manifest,
            archive,
        }) => {
            assert_eq!(field, "override files");
            assert_eq!(manifest, 0);
            assert_eq!(archive, 1);
        }
        other => panic!("expected a count refusal, got {other:?}"),
    }

    // And one indexed file too many.
    let more = manifest_with(|v| {
        v["included"]["files"] = serde_json::json!([
            { "path": "mods/sodium.jar", "size": 10 },
            { "path": "mods/ghost.jar", "size": 1 },
        ]);
    });
    match preview(&export_bytes(Some(&more))) {
        Err(ExportImportError::CountMismatch {
            field,
            manifest,
            archive,
        }) => {
            assert_eq!(field, "indexed files");
            assert_eq!(manifest, 2);
            assert_eq!(archive, 1);
        }
        other => panic!("expected a count refusal, got {other:?}"),
    }
}

/// A loader the format's loader-key rule does not know must never reach the
/// instance registry row the import creates.
#[test]
fn spec_f158_preview_refuses_a_loader_the_format_does_not_carry() {
    let foreign = manifest_with(|v| {
        v["loader"] = "rift".into();
    });
    match preview(&export_bytes(Some(&foreign))) {
        Err(ExportImportError::InvalidLoader { loader }) => assert_eq!(loader, "rift"),
        other => panic!("expected a loader refusal, got {other:?}"),
    }
}

#[test]
fn spec_f158_preview_refuses_an_oversized_manifest_before_reading_it() {
    // The manifest ceiling is a parameter so it can be tested without an
    // eight-megabyte fixture; `preview_export_archive` is what production
    // calls and uses the index ceiling.
    let bytes = export_bytes(Some(&export_manifest_json()));
    let mut z = zip::ZipArchive::new(Cursor::new(bytes.clone())).expect("zip");
    match preview_export_archive_bounded(&mut z, &ArchiveLimits::default(), 16) {
        Err(ExportImportError::Pack(PackError::IndexTooLarge { limit: 16, .. })) => {}
        other => panic!("expected an oversized-manifest refusal, got {other:?}"),
    }
    // The production ceiling still admits the same archive.
    let mut z2 = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
    assert!(
        preview_export_archive_bounded(&mut z2, &ArchiveLimits::default(), MAX_INDEX_BYTES).is_ok()
    );
}
