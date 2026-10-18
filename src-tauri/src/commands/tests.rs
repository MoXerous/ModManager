//! IPC-layer tests: mapping + guards. No WebView needed.

use super::*;
use crate::dto::ErrorCode;

#[test]
fn spec_f104_registry_errors_map_to_codes() {
    use mm_core::db::instances::InstanceRowError;
    use mm_core::db::DbError;
    use mm_core::instances::metadata::MetadataError;
    use mm_core::instances::registry::RegistryError;
    use std::path::PathBuf;

    let cases: Vec<(RegistryError, ErrorCode)> = vec![
        // The request cannot be satisfied as stated.
        (
            RegistryError::RootNotEmpty(PathBuf::from("a")),
            ErrorCode::InvalidInput,
        ),
        (
            RegistryError::AlreadyAnInstance(PathBuf::from("a")),
            ErrorCode::InvalidInput,
        ),
        (
            RegistryError::RootIsFile(PathBuf::from("a")),
            ErrorCode::InvalidInput,
        ),
        (
            RegistryError::UnsafeName("CON".into()),
            ErrorCode::InvalidInput,
        ),
        (
            RegistryError::Row(InstanceRowError::DuplicatePath(PathBuf::from("a"))),
            ErrorCode::InvalidInput,
        ),
        // Folder the user pointed at is not there vs is not a folder.
        (
            RegistryError::Row(InstanceRowError::UnresolvablePath {
                path: "a".into(),
                source: std::io::Error::from(std::io::ErrorKind::NotFound),
            }),
            ErrorCode::NotFound,
        ),
        (
            RegistryError::Row(InstanceRowError::UnresolvablePath {
                path: "a".into(),
                source: std::io::Error::other("not a folder"),
            }),
            ErrorCode::InvalidInput,
        ),
        // Retryable: disk.
        (
            RegistryError::Io(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
            ErrorCode::Io,
        ),
        (
            RegistryError::Row(InstanceRowError::Db(DbError::Io(std::io::Error::from(
                std::io::ErrorKind::PermissionDenied,
            )))),
            ErrorCode::Io,
        ),
        // Needs a newer app, not a retry.
        (
            RegistryError::Row(InstanceRowError::Db(DbError::NewerSchema {
                found: 9,
                supported: 3,
            })),
            ErrorCode::NewerSchema,
        ),
        // A corrupt file we wrote is our bug.
        (
            RegistryError::Metadata(MetadataError::Parse("{".into())),
            ErrorCode::Internal,
        ),
        (
            RegistryError::Metadata(MetadataError::NewerSchema {
                found: "v9".into(),
                supported: "v1".into(),
            }),
            ErrorCode::NewerSchema,
        ),
        (
            RegistryError::Metadata(MetadataError::TooLarge { size: 9, max: 1 }),
            ErrorCode::InvalidInput,
        ),
        (
            RegistryError::RolledBack(2, "boom".into()),
            ErrorCode::Internal,
        ),
    ];
    for (err, want) in cases {
        assert_eq!(registry_error(&err).code, want, "{err}");
    }
}

/// The screen's "never scanned" wording depends on this staying a null that
/// serialises as null, not an optional field that may be absent.
#[test]
fn spec_f101_summary_marks_a_deleted_folder_without_hiding_the_row() {
    let dir = tempfile::tempdir().expect("temp");
    let root = dir.path().join("Gone");
    std::fs::create_dir_all(&root).expect("mkdir");
    let row = mm_core::db::instances::InstanceRow {
        id: "i1".into(),
        name: "Gone".into(),
        path: std::fs::canonicalize(&root).expect("canon"),
        minecraft_root: PathBuf::new(),
        minecraft_version: None,
        loader: None,
        source: mm_core::db::instances::InstanceSource::Manual,
        created_at: 1,
        scanned_at: None,
    };
    assert!(!summary(&row).path_missing, "present on disk");

    std::fs::remove_dir_all(&root).expect("remove");
    let gone = summary(&row);
    assert!(gone.path_missing, "reported as needing attention");
    assert_eq!(gone.id, "i1", "the row is still described, not dropped");
    assert_eq!(gone.scanned_at, None);
    assert_eq!(gone.minecraft_version, None);
    assert_eq!(gone.source, crate::dto::InstanceSourceDto::Manual);
}

/// Exercises the real handle: `AppState` -> `spawn_blocking` -> the `Mutex` ->
/// SQLite. No Tauri app needed, which is the point of keeping the state a
/// plain struct.
#[test]
fn spec_f101_list_instances_runs_through_the_shared_db_handle() {
    let dir = tempfile::tempdir().expect("temp");
    let db = mm_core::db::open(&dir.path().join("t.db")).expect("open");
    let root = dir.path().join("Survival");
    std::fs::create_dir_all(&root).expect("mkdir");
    let canonical = std::fs::canonicalize(&root).expect("canon");

    mm_core::db::instances::insert(
        &db,
        &mm_core::db::instances::InstanceRow {
            id: "i1".into(),
            name: "Survival".into(),
            path: canonical.clone(),
            minecraft_root: PathBuf::new(),
            minecraft_version: Some("1.21.1".into()),
            loader: Some("fabric".into()),
            source: mm_core::db::instances::InstanceSource::Managed,
            created_at: 1,
            scanned_at: None,
        },
    )
    .expect("seed");

    let state = AppState::from_db(db);
    let rows = tauri::async_runtime::block_on(
        state.with_db(|db| mm_core::db::instances::list(db).map_err(|e| e.to_string())),
    )
    .expect("list");

    assert_eq!(rows.len(), 1);
    let dto = summary(&rows[0]);
    assert_eq!(dto.name, "Survival");
    assert_eq!(dto.path, canonical.to_string_lossy());
    assert!(!dto.path_missing);
    assert_eq!(dto.minecraft_version.as_deref(), Some("1.21.1"));
}

#[test]
fn spec_f019_provider_errors_map_to_codes() {
    use mm_core::providers::ProviderError;
    let cases: Vec<(ProviderError, ErrorCode)> = vec![
        (ProviderError::Api { status: 404 }, ErrorCode::NotFound),
        (ProviderError::Api { status: 503 }, ErrorCode::ProviderApi),
        (ProviderError::Timeout, ErrorCode::Timeout),
        (ProviderError::Transport, ErrorCode::Io),
        (ProviderError::InvalidJson, ErrorCode::Io),
    ];
    for (err, want) in cases {
        assert_eq!(IpcError::from_provider(&err).code, want, "{err}");
    }
    let limited = IpcError::from_provider(&ProviderError::RateLimited { reset_seconds: 90 });
    assert_eq!(limited.code, ErrorCode::RateLimited);
    // The reset window crosses as data, never parsed from a message.
    assert_eq!(limited.retry_after_seconds, Some(90));
}

#[test]
fn spec_f134_newer_lock_schema_surfaces_as_code() {
    use mm_core::instances::LockError;
    let err = IpcError::from_lock(&LockError::NewerSchema {
        found: 9,
        supported: 1,
    });
    assert_eq!(err.code, ErrorCode::NewerSchema);
}

#[test]
fn spec_f067_toggle_action_maps_to_core() {
    use crate::dto::ToggleActionDto;
    use mm_core::instances::toggle::ToggleAction;
    assert_eq!(
        ToggleAction::from(ToggleActionDto::Disable),
        ToggleAction::Disable
    );
    assert_eq!(
        ToggleAction::from(ToggleActionDto::Enable),
        ToggleAction::Enable
    );
}

#[test]
fn untrusted_paths_are_filtered_before_core() {
    let raw = vec![
        "ok.jar".to_string(),
        "../escape.jar".to_string(),
        "sub/dir.jar".to_string(),
        "..".to_string(),
        "  ".to_string(),
    ];
    let kept = filenames(&raw);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].file_name().expect("n").to_string_lossy(), "ok.jar");
}

#[test]
fn resolve_dir_rejects_missing_and_non_dir() {
    assert!(matches!(
        resolve_dir("   "),
        Err(e) if e.code == ErrorCode::InvalidInput
    ));
    assert!(matches!(
        resolve_dir(r"C:\definitely\not\here"),
        Err(e) if e.code == ErrorCode::Io
    ));
    let d = tempfile::tempdir().expect("temp");
    let file = d.path().join("f.txt");
    std::fs::write(&file, b"x").expect("write");
    assert!(matches!(
        resolve_dir(&file.to_string_lossy()),
        Err(e) if e.code == ErrorCode::InvalidInput
    ));
    assert!(resolve_dir(&d.path().to_string_lossy()).is_ok());
}

#[test]
fn search_limit_is_clamped() {
    use crate::dto::SearchQueryDto;
    let too_many = SearchQueryDto {
        query: "x".to_string(),
        game_version: None,
        loader: None,
        project_type: String::new(),
        limit: 9999,
        offset: 0,
    };
    let core = too_many.to_core();
    assert_eq!(core.limit, 50);
    // Empty project type defaults to "mod".
    assert_eq!(core.project_type, "mod");
}

#[test]
fn dto_camel_case_shape_is_stable() {
    let dto = crate::dto::PlanEntryDto {
        source: "selected".to_string(),
        project_id: "p".to_string(),
        title: "t".to_string(),
        project_type: "mod".to_string(),
        version_id: None,
        version_number: Some("1.0".to_string()),
        filename: Some("a.jar".to_string()),
        status: "downloadable".to_string(),
        reason: "Ready for download.".to_string(),
        dependency_type: String::new(),
    };
    let json = serde_json::to_value(dto).expect("json");
    assert!(json.get("projectId").is_some());
    assert!(json.get("versionNumber").is_some());
    assert!(json.get("dependencyType").is_some());
    assert!(json.get("project_id").is_none());
}
