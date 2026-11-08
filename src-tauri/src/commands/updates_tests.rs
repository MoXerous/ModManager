//! `check_instance_updates` through the bridge: folder resolution, the core
//! check over a real HTTP response, and the typed error mapping.

use std::path::Path;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::check_updates_with;
use crate::dto::{CheckUpdatesRequest, ErrorCode, UpdateStatusDto, UpdateWarningDto};
use mm_core::instances::{write_lock, LockEntry, LockFile, LOCK_FILE_NAME};
use mm_core::providers::ModrinthClient;

const UPDATE_BODY: &str = r#"{
  "aaa": {
    "id": "v2", "project_id": "p-a", "author_id": "a",
    "featured": false, "name": "2.0", "version_number": "2.0.0",
    "changelog": null, "date_published": "2024-01-01T00:00:00Z",
    "downloads": 5, "version_type": "release", "status": "listed",
    "game_versions": ["1.21.1"], "loaders": ["fabric"],
    "files": [{
      "hashes": {"sha512": "bbb", "sha1": "222"},
      "url": "https://cdn.example/a-2.jar",
      "filename": "a-2.jar", "primary": true, "size": 6, "file_type": null
    }],
    "dependencies": []
  }
}"#;

fn root_with_lock(dir: &Path, entries: Vec<LockEntry>) -> std::path::PathBuf {
    let root = dir.join("instance");
    std::fs::create_dir_all(root.join("mods")).expect("mods");
    write_lock(&root.join(LOCK_FILE_NAME), &LockFile { entries }).expect("lock");
    root
}

fn request(root: &Path, loader: &str, game_version: &str) -> CheckUpdatesRequest {
    CheckUpdatesRequest {
        instance_root: root.to_string_lossy().into_owned(),
        provider: mm_core::providers::modrinth::PROVIDER_ID.to_string(),
        game_version: game_version.to_string(),
        loader: loader.to_string(),
    }
}

fn entry(filename: &str, version_id: &str, sha512: &str, sha1: &str) -> LockEntry {
    LockEntry {
        provider: mm_core::providers::modrinth::PROVIDER_ID.to_string(),
        project_id: "p-a".to_string(),
        slug: String::new(),
        version_id: version_id.to_string(),
        filename: filename.to_string(),
        sha1: sha1.to_string(),
        sha512: sha512.to_string(),
    }
}

/// The unreachable base proves the guard fires before any request: if the
/// bridge asked the network first, the error would be `Timeout`/`Transport`,
/// not the typed input refusal.
fn unreachable() -> ModrinthClient {
    ModrinthClient::with_base("http://127.0.0.1:1").expect("provider")
}

#[test]
fn spec_f120_check_instance_updates_bridge_returns_a_preview() {
    tauri::async_runtime::block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/version_files/update"))
            .respond_with(ResponseTemplate::new(200).set_body_string(UPDATE_BODY))
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().expect("tempdir");
        let root = root_with_lock(dir.path(), vec![entry("a.jar", "v1", "aaa", "111")]);
        let provider = ModrinthClient::with_base(&server.uri()).expect("provider");

        let preview = check_updates_with(&provider, request(&root, "fabric", "1.21.1"))
            .await
            .expect("preview");

        assert_eq!(preview.checked, 1);
        assert!(preview.warnings.is_empty(), "{:?}", preview.warnings);
        let candidate = &preview.candidates[0];
        assert_eq!(candidate.filename, "a.jar");
        assert_eq!(candidate.status, UpdateStatusDto::Available);
        assert_eq!(candidate.latest_version_id.as_deref(), Some("v2"));
        assert_eq!(candidate.latest_version_number.as_deref(), Some("2.0.0"));
        assert_eq!(candidate.latest_filename.as_deref(), Some("a-2.jar"));
        assert_eq!(candidate.latest_sha512.as_deref(), Some("bbb"));
    });
}

#[test]
fn spec_f120_check_instance_updates_bridge_reports_a_missing_lock() {
    tauri::async_runtime::block_on(async {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("instance");
        std::fs::create_dir_all(&root).expect("root");

        let preview = check_updates_with(&unreachable(), request(&root, "fabric", "1.21.1"))
            .await
            .expect("a missing lock is a warning");

        assert_eq!(preview.checked, 0);
        assert!(preview.candidates.is_empty());
        assert_eq!(preview.warnings, vec![UpdateWarningDto::EmptyLock]);
    });
}

#[test]
fn spec_f120_check_instance_updates_bridge_maps_a_non_concrete_target_to_invalid_input() {
    tauri::async_runtime::block_on(async {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = root_with_lock(dir.path(), vec![entry("a.jar", "v1", "aaa", "111")]);

        let err = check_updates_with(&unreachable(), request(&root, "any", "1.21.1"))
            .await
            .expect_err("`any` is not a loader");
        assert_eq!(err.code, ErrorCode::InvalidInput);

        let err = check_updates_with(&unreachable(), request(&root, "fabric", "  "))
            .await
            .expect_err("an empty version is not a target");
        assert_eq!(err.code, ErrorCode::InvalidInput);
    });
}

#[test]
fn spec_f120_check_instance_updates_bridge_refuses_a_newer_lock() {
    tauri::async_runtime::block_on(async {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("instance");
        std::fs::create_dir_all(&root).expect("root");
        std::fs::write(
            root.join(LOCK_FILE_NAME),
            br#"{"schema_version":99,"entries":[]}"#,
        )
        .expect("newer lock");

        let err = check_updates_with(&unreachable(), request(&root, "fabric", "1.21.1"))
            .await
            .expect_err("a newer lock is a typed refusal");
        assert_eq!(err.code, ErrorCode::NewerSchema);
    });
}
