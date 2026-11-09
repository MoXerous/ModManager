//! `app_paths` and `open_app_folder` through the bridge.
//!
//! The paths card is only useful if the paths it shows are the ones the
//! process actually resolved. These tests pin that: the DTO is built from
//! `AppState::paths()`, the database file is the one this process opened, and
//! the open action refuses anything that is not one of the four known roots
//! before it reaches the file manager.

use super::{app_folder_path, app_paths_with};
use crate::dto::ErrorCode;
use crate::state::AppState;

fn state() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = mm_core::db::open(&dir.path().join("app.sqlite3")).expect("open");
    (dir, AppState::from_db(db))
}

#[test]
fn spec_f151_app_paths_reports_the_resolved_roots() {
    tauri::async_runtime::block_on(async {
        let (_dir, state) = state();
        let paths = state.paths().clone();
        let dto = app_paths_with(&state).await.expect("app paths");

        assert_eq!(dto.data, paths.data_dir().to_string_lossy());
        assert_eq!(dto.config, paths.config_dir().to_string_lossy());
        assert_eq!(dto.cache, paths.cache_dir().to_string_lossy());
        assert_eq!(dto.logs, paths.logs_dir().to_string_lossy());
        assert_eq!(
            dto.db_version,
            mm_core::db::CURRENT_VERSION,
            "a freshly opened file carries the current schema"
        );
        assert!(
            dto.db_bytes > 0,
            "the database file exists, so its size cannot be zero"
        );
        assert_eq!(
            dto.app_version,
            mm_core::app_info().version,
            "the diagnostics card names the build it is running"
        );
    });
}

#[test]
fn spec_f151_app_paths_reports_the_file_the_process_opened() {
    tauri::async_runtime::block_on(async {
        let (dir, state) = state();
        let dto = app_paths_with(&state).await.expect("app paths");
        // The state carries the database at `<dir>/app.sqlite3`, not under
        // `paths.data_dir()`: the reported file must be the open one, because
        // that is the file whose version and size the card claims to show.
        assert_eq!(
            dto.db_file,
            dir.path().join("app.sqlite3").to_string_lossy()
        );
    });
}

#[test]
fn spec_f151_open_app_folder_resolves_only_the_four_roots() {
    let (_dir, state) = state();
    let paths = state.paths();
    let cases = [
        ("data", paths.data_dir().to_path_buf()),
        ("config", paths.config_dir().to_path_buf()),
        ("cache", paths.cache_dir().to_path_buf()),
        ("logs", paths.logs_dir().to_path_buf()),
    ];
    for (kind, expected) in cases {
        let resolved = app_folder_path(paths, kind).expect("known kind");
        assert_eq!(resolved, expected, "{kind} must resolve from Paths");
    }
    // One root feeds all four; a kind cannot escape to a folder of its own.
    let root = paths.data_dir().parent().expect("data has a parent");
    assert_eq!(paths.config_dir().parent(), Some(root));
    assert_eq!(paths.cache_dir().parent(), Some(root));
    assert!(paths.logs_dir().starts_with(paths.data_dir()));
}

#[test]
fn spec_f151_open_app_folder_rejects_unknown_kind() {
    let (_dir, state) = state();
    for kind in ["", "database", "Data", "data/../../etc", "../data"] {
        let error = app_folder_path(state.paths(), kind).expect_err("unknown kind");
        assert_eq!(
            error.code,
            ErrorCode::InvalidInput,
            "{kind:?} is not one of the four roots"
        );
    }
}

#[test]
fn spec_f151_settings_keys_exist_in_both_locales() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/i18n");
    let keys = [
        "settings.paths.title",
        "settings.paths.data",
        "settings.paths.config",
        "settings.paths.cache",
        "settings.paths.logs",
        "settings.paths.database",
        "settings.paths.open",
        "settings.paths.openAria",
        "settings.paths.note",
        "settings.paths.loadFailed",
        "settings.paths.openFailed",
        "settings.diagnostics.title",
        "settings.diagnostics.version",
        "settings.diagnostics.schema",
        "settings.diagnostics.dbSize",
    ];
    for locale in ["ar.json", "en.json"] {
        let text = std::fs::read_to_string(root.join(locale))
            .unwrap_or_else(|e| panic!("cannot read {locale}: {e}"));
        let parsed: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{locale} is not JSON: {e}"));
        for key in keys {
            assert!(
                parsed.get(key).is_some(),
                "{locale} has no {key}; the screen would show the key itself"
            );
        }
    }
}
