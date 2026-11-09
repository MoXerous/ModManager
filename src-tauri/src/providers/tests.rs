//! F-014 bridge tests: key selection, reporting, and the search mapping.
//!
//! None of these touches the network unless it mounts wiremock itself, and
//! none mutates the process environment: every key path takes the environment
//! value as a parameter (the same rule core's secret tests follow).

use serde_json::Value;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{build_with, key_status_with, save_key};
use crate::dto::{ErrorCode, KeySourceDto};
use crate::state::AppState;

/// A state whose database, config directory and journal are all private to
/// the test. The `TempDir` is returned so the caller keeps it alive.
fn state() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = mm_core::db::open(&dir.path().join("t.db")).expect("db");
    (dir, AppState::from_db(db))
}

/// ADR 0004 §2-1-4: no key, no client. The refusal must happen before any
/// request, so a wiremock-free state is enough.
#[test]
fn spec_f014_missing_key_blocks_curseforge_before_any_request() {
    let (_dir, state) = state();
    let Err(err) = build_with(&state, mm_core::providers::curseforge::PROVIDER_ID, None) else {
        panic!("a missing key must not build a CurseForge client");
    };
    assert_eq!(err.code, ErrorCode::ProviderUnauthorized);
}

/// ADR 0004 §2-1-4/§2-1-5: the write succeeds, the source is `File`, and the
/// value never appears in what crosses IPC.
#[test]
fn spec_f014_saved_key_is_written_and_reported_without_the_value() {
    let (_dir, state) = state();
    let dto = save_key(&state, "cf-secret-sentinel").expect("save");

    assert_eq!(dto.source, KeySourceDto::File);
    assert!(dto.has_key);
    assert!(
        dto.path.ends_with("curseforge.secret.json"),
        "the status named a different file: {}",
        dto.path
    );

    // The file is real and parses as the secret schema.
    let text = std::fs::read_to_string(&dto.path).expect("read the secret file");
    let parsed: Value = serde_json::from_str(&text).expect("the secret file is JSON");
    assert_eq!(parsed["api_key"], "cf-secret-sentinel");

    // The DTO is what crosses IPC; the key must not be in it.
    let json = serde_json::to_string(&dto).expect("serialize");
    assert!(
        !json.contains("cf-secret-sentinel"),
        "the key leaked into the status DTO: {json}"
    );

    // File wins over the environment (ADR 0004 §2-1-4).
    let status = key_status_with(&state, Some("env-sentinel")).expect("status");
    assert_eq!(status.source, KeySourceDto::File);
}

/// The environment is a fallback, not a second source of truth: it is
/// reported only when no file exists.
#[test]
fn spec_f014_environment_key_is_reported_when_the_file_is_absent() {
    let (_dir, state) = state();
    let dto = key_status_with(&state, Some("env-sentinel")).expect("status");
    assert_eq!(dto.source, KeySourceDto::Environment);
    assert!(dto.has_key);

    let json = serde_json::to_string(&dto).expect("serialize");
    assert!(
        !json.contains("env-sentinel"),
        "the key leaked into the status DTO: {json}"
    );
}

/// A file that exists but is corrupt is a typed error, never a silent fall
/// back to the environment (ADR 0004 §2-1-4).
#[test]
fn spec_f014_corrupt_key_file_is_an_error_not_an_environment_fallback() {
    let (_dir, state) = state();
    let secret = state.config_dir().join("curseforge.secret.json");
    std::fs::write(&secret, r#"{"schema_version":1,"api_key":"#).expect("fixture");

    let err = key_status_with(&state, Some("env-sentinel")).expect_err("a corrupt file must fail");
    assert_eq!(err.code, ErrorCode::Internal);
    assert!(
        err.message.contains("not valid JSON"),
        "the failure was not the typed parse error: {}",
        err.message
    );
    assert!(
        !err.message.contains("env-sentinel"),
        "the environment key leaked into the error: {}",
        err.message
    );
}

/// An id nothing recognises is the caller's mistake, not a missing key.
#[test]
fn spec_f014_unknown_provider_is_invalid_input() {
    let (_dir, state) = state();
    let Err(err) = build_with(&state, "nope", None) else {
        panic!("an unknown id must not select a client");
    };
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

/// The search mapping keeps the older contract (hits + total) and stamps each
/// hit with the identity of the client that answered, not with a literal.
#[test]
fn spec_f014_search_maps_hits_with_the_selected_provider() {
    tauri::async_runtime::block_on(async {
        // Never retype the fixture: read the captured response and the total
        // it declares, so the assertion follows the file.
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../core/tests/fixtures/json/curseforge/mods_search.json");
        let body = std::fs::read_to_string(&fixture_path).expect("read the fixture");
        let fixture: Value = serde_json::from_str(&body).expect("fixture json");
        let expected_total = fixture["pagination"]["totalCount"]
            .as_u64()
            .expect("pagination.totalCount");

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/mods/search"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;

        let provider = mm_core::providers::CurseForgeClient::with_base(&server.uri(), "test-key")
            .expect("client");
        let query = crate::dto::SearchQueryDto {
            query: "jei".to_string(),
            provider: mm_core::providers::curseforge::PROVIDER_ID.to_string(),
            game_version: None,
            loader: None,
            project_type: "mod".to_string(),
            limit: 20,
            offset: 0,
        };

        let page = crate::commands::search_with(&provider, query)
            .await
            .expect("search failed");
        assert!(!page.hits.is_empty(), "the fixture produced no hits");
        assert!(
            page.hits
                .iter()
                .all(|h| h.provider_id == mm_core::providers::curseforge::PROVIDER_ID),
            "a hit carried another provider: {:?}",
            page.hits.iter().map(|h| &h.provider_id).collect::<Vec<_>>()
        );
        assert_eq!(page.total, expected_total);
    });
}

/// The path in the status DTO is the documented file under the state's own
/// config directory — not a hard-coded user profile.
#[test]
fn spec_f014_key_status_path_is_the_config_dir_file() {
    let (_dir, state) = state();
    let dto = key_status_with(&state, None).expect("status");
    assert_eq!(
        std::path::PathBuf::from(&dto.path),
        state.config_dir().join("curseforge.secret.json")
    );
    // The config dir itself is the private one this state owns, never the
    // machine's real profile.
    assert!(
        state.config_dir().starts_with(state.config_root()),
        "the config dir is not under the private test root: {}",
        state.config_dir().display()
    );
}
