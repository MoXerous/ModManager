//! Index-reader tests. Every one of these is a refusal the reader owes a
//! hostile pack, checked before anything touches the filesystem.

use super::mrpack::{parse_index, IndexError, Side};

fn index_with(files: &str, extra: &str) -> String {
    format!(
        r#"{{"formatVersion":1,"game":"minecraft","versionId":"abc123",
             "name":"Test Pack","files":[{files}]{extra}}}"#
    )
}

fn one_file() -> String {
    r#"{"path":"mods/sodium.jar",
        "hashes":{"sha1":"aa","sha512":"bb"},
        "downloads":["https://cdn.modrinth.com/data/x/y.jar"],
        "fileSize":1234}"#
        .to_string()
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
    assert_eq!(f.sha512, "bb");
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
    let f = r#"{"path":"mods/x.jar","hashes":{"sha1":"a","sha512":"b"},
                 "downloads":["https://cdn.modrinth.com/x.jar"],
                 "env":{"client":"required","server":"unsupported"}}"#;
    let pack = parse_index(index_with(f, "").as_bytes()).expect("parse");
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
