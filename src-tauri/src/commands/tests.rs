//! IPC-layer tests: mapping + guards. No WebView needed.

use super::*;
use crate::dto::ErrorCode;

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
