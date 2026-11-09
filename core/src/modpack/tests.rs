//! Index-reader tests. Every one of these is a refusal the reader owes a
//! hostile pack, checked before anything touches the filesystem.

use super::mrpack::{parse_index, IndexError, Side};

fn index_with(files: &str, extra: &str) -> String {
    format!(
        r#"{{"formatVersion":1,"game":"minecraft","versionId":"abc123",
             "name":"Test Pack","files":[{files}]{extra}}}"#
    )
}

/// Placeholder digest: 128 hexadecimal characters, as SHA-512 demands.
fn sha128(hex: char) -> String {
    (0..128).map(|_| hex).collect()
}

fn one_file() -> String {
    format!(
        r#"{{"path":"mods/sodium.jar",
        "hashes":{{"sha1":"aa","sha512":"{}"}},
        "downloads":["https://cdn.modrinth.com/data/x/y.jar"],
        "fileSize":1234}}"#,
        sha128('a')
    )
}

#[test]
fn mrpack_index_reads_the_documented_shape() {
    let raw = index_with(&one_file(), "");
    let pack = parse_index(raw.as_bytes()).expect("a well formed index");

    assert_eq!(pack.name, "Test Pack");
    assert_eq!(pack.version_id, "abc123");
    assert_eq!(pack.format_version, 1);
    assert_eq!(pack.files.len(), 1);

    let f = &pack.files[0];
    assert_eq!(f.safe_path, "mods/sodium.jar");
    assert_eq!(f.sha512, sha128('a'));
    assert_eq!(f.file_size, Some(1234));
    assert_eq!(f.downloads.len(), 1);
}

#[test]
fn mrpack_a_file_without_env_is_required_on_both_sides() {
    // Guessing "optional" here would let an install skip a dependency and
    // report success.
    let pack = parse_index(index_with(&one_file(), "").as_bytes()).expect("parse");
    assert_eq!(pack.files[0].client, Side::Required);
    assert_eq!(pack.files[0].server, Side::Required);
}

#[test]
fn mrpack_env_is_carried_through_unchanged() {
    let f = format!(
        r#"{{"path":"mods/x.jar","hashes":{{"sha1":"a","sha512":"{}"}},
                 "downloads":["https://cdn.modrinth.com/x.jar"],
                 "env":{{"client":"required","server":"unsupported"}}}}"#,
        sha128('a')
    );
    let pack = parse_index(index_with(&f, "").as_bytes()).expect("parse");
    assert_eq!(pack.files[0].client, Side::Required);
    assert_eq!(pack.files[0].server, Side::Unsupported);
}

#[test]
fn mrpack_a_traversing_path_is_refused() {
    // `path` is resolved against the instance directory. It is exactly as
    // untrusted as a ZIP member name, and goes through the same gate.
    for path in [
        "../escape.jar",
        "mods/../../escape.jar",
        "/etc/passwd",
        "C:\\Windows\\system32.dll",
        "",
        ".",
    ] {
        let f = serde_json::json!({
            "path": path,
            "hashes": { "sha1": "a", "sha512": "b" },
            "downloads": ["https://x/y.jar"],
        })
        .to_string();
        let raw = index_with(&f, "");
        match parse_index(raw.as_bytes()) {
            Err(IndexError::UnsafePath { .. }) => {}
            other => panic!("expected UnsafePath for {path:?}, got {other:?}"),
        }
    }
}

#[test]
fn mrpack_a_traversing_overrides_folder_is_refused() {
    let raw = index_with("", r#","overrides":"../outside""#);
    assert!(matches!(
        parse_index(raw.as_bytes()),
        Err(IndexError::UnsafePath { .. })
    ));
}

#[test]
fn mrpack_a_file_with_no_download_is_refused() {
    let f = r#"{"path":"mods/x.jar","hashes":{"sha1":"a","sha512":"b"},"downloads":[]}"#;
    let raw = index_with(f, "");
    assert!(matches!(
        parse_index(raw.as_bytes()),
        Err(IndexError::NoDownload { .. })
    ));
}

#[test]
fn mrpack_an_empty_sha512_is_refused() {
    let f = r#"{"path":"mods/x.jar","hashes":{"sha1":"a","sha512":"  "},
                "downloads":["https://x/y.jar"]}"#;
    let raw = index_with(f, "");
    assert!(matches!(
        parse_index(raw.as_bytes()),
        Err(IndexError::EmptyHash { .. })
    ));
}

#[test]
fn mrpack_a_sha512_that_is_not_128_hex_characters_is_refused() {
    // The staging file name is derived from this hash before any hash gate
    // can run, so a value that is not a digest has no business reaching the
    // executor: `../`, a stray `/`, or just a wrong length must all go out
    // while the pack is still text.
    for bad in [
        "bb",
        &(0..127).map(|_| 'a').collect::<String>(),
        &(0..128).map(|_| 'g').collect::<String>(),
        "../../../../../../Users/me/pwn",
        "aaaebx.ab_ag-6^",
    ] {
        let f = format!(
            r#"{{"path":"mods/x.jar","hashes":{{"sha1":"a","sha512":"{bad}"}},"downloads":["https://x/y.jar"]}}"#
        );
        let raw = index_with(&f, "");
        match parse_index(raw.as_bytes()) {
            Err(IndexError::InvalidHash { found, .. }) => {
                assert_eq!(found, bad.chars().count(), "for {bad:?}");
            }
            other => panic!("{bad:?}: expected InvalidHash, got {other:?}"),
        }
    }
}

#[test]
fn mrpack_a_128_hex_sha512_is_accepted_verbatim() {
    let good = sha128('b');
    // Single braces: this is a plain string, not a format template. The
    // braces only had to be doubled where a `format!` was doing the
    // unescaping, and doubling them here produced JSON that does not parse.
    let lower = r#"{"path":"mods/x.jar","hashes":{"sha1":"a","sha512":"good"},"downloads":["https://x/y.jar"]}"#
        .replace("good", &good);
    match parse_index(index_with(&lower, "").as_bytes()) {
        Ok(pack) => assert_eq!(pack.files[0].sha512, good, "lower-case hex kept as-is"),
        other => panic!("expected a valid pack, got {other:?}"),
    }
    let upper = r#"{"path":"mods/x.jar","hashes":{"sha1":"a","sha512":"good"},"downloads":["https://x/y.jar"]}"#
        .replace("good", &good.to_uppercase());
    match parse_index(index_with(&upper, "").as_bytes()) {
        Ok(pack) => assert_eq!(
            pack.files[0].sha512,
            good.to_uppercase(),
            "hex case is not rewritten"
        ),
        other => panic!("expected a valid pack, got {other:?}"),
    }
}

#[test]
fn mrpack_an_unknown_format_version_is_refused() {
    let raw = r#"{"formatVersion":99,"game":"minecraft","versionId":"v","name":"n","files":[]}"#;
    assert!(matches!(
        parse_index(raw.as_bytes()),
        Err(IndexError::FormatVersion {
            found: 99,
            supported: 1
        })
    ));
}

#[test]
fn mrpack_a_non_minecraft_game_is_refused() {
    let raw = r#"{"formatVersion":1,"game":"terraria","versionId":"v","name":"n","files":[]}"#;
    assert!(matches!(
        parse_index(raw.as_bytes()),
        Err(IndexError::Game { .. })
    ));
}

#[test]
fn mrpack_override_layers_are_listed_client_first_server_last() {
    let raw = index_with(
        "",
        r#","serverOverrides":"server-overrides","clientOverrides":"client-overrides""#,
    );
    let pack = parse_index(raw.as_bytes()).expect("parse");
    assert_eq!(
        pack.override_layers,
        vec!["client-overrides", "overrides", "server-overrides"]
    );
}

#[test]
fn mrpack_an_empty_pack_is_valid() {
    let raw = r#"{"formatVersion":1,"game":"minecraft","versionId":"v","name":"Empty","files":[]}"#;
    let pack = parse_index(raw.as_bytes()).expect("an empty pack is not an error");
    assert!(pack.files.is_empty());
    // The default layer still exists: a pack may ship no overrides folder.
    assert_eq!(pack.override_layers, vec!["overrides"]);
}

#[test]
fn mrpack_a_file_path_ending_in_a_slash_is_refused() {
    let path = one_file().replace("mods/sodium.jar", "mods/");
    let raw = index_with(&path, "");
    assert!(matches!(
        parse_index(raw.as_bytes()),
        Err(IndexError::DirectoryPath { .. })
    ));
}

#[test]
fn mrpack_a_file_path_that_is_app_managed_is_refused() {
    for name in ["profile.json", "PROFILE.JSON", "instance.lock.json"] {
        let path = one_file().replace("mods/sodium.jar", name);
        let raw = index_with(&path, "");
        match parse_index(raw.as_bytes()) {
            Err(IndexError::ManagedPath { name: hit, .. }) => {
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
fn mrpack_a_managed_file_name_with_a_part_suffix_is_refused() {
    // `<name>.part` is reserved by the downloader's own protocol
    // (`safety::atomic`): a pack that plants `profile.json.part` makes the
    // application's next atomic write of `profile.json` fail forever with
    // a stale-part refusal, so the whole suffix is off-limits at the root.
    for name in [
        "profile.json.part",
        "PROFILE.JSON.part",
        "instance.lock.json.part",
    ] {
        let path = one_file().replace("mods/sodium.jar", name);
        let raw = index_with(&path, "");
        match parse_index(raw.as_bytes()) {
            Err(IndexError::ManagedPath { name: hit, .. }) => {
                assert!(
                    hit.eq_ignore_ascii_case(name.trim_end_matches(".part")),
                    "the refusal must name the managed file: {hit:?} for {name:?}"
                );
            }
            other => panic!("{name}: expected ManagedPath, got {other:?}"),
        }
    }
}

#[test]
fn mrpack_a_file_path_inside_the_staging_directory_is_refused() {
    let path = one_file().replace("mods/sodium.jar", ".mm-staging/sodium.jar");
    let raw = index_with(&path, "");
    assert!(matches!(
        parse_index(raw.as_bytes()),
        Err(IndexError::StagingPath { .. })
    ));
}

#[test]
fn mrpack_a_layer_name_with_a_trailing_slash_is_normalised() {
    let raw = index_with("", r#","clientOverrides":"client-overrides/""#);
    let pack = parse_index(raw.as_bytes()).expect("parse");
    assert_eq!(pack.override_layers, vec!["client-overrides", "overrides"]);
}
