//! F-113..F-115/F-118 driven through the IPC layer for real.
//!
//! Same harness as `pack_tests.rs`: `AppState::from_db` over a temp database,
//! `Channel::new` with a plain closure, temp directories for instances.
//!
//! What these tests add over `core`'s own suite, and why they exist at all:
//!
//! - the registry gate is not a formality — an unregistered folder must be
//!   refused **before** a backup folder exists;
//! - the restore token (`manifestSha256`) is the only thing tying a confirm to
//!   the preview, so a mismatched token must stop before the first write;
//! - every channel ends on a terminal event, because the screen follows the
//!   channel and a silent refusal looks like a hang;
//! - `manifestState` crosses as a word the UI translates, never as a sentence.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use super::backup::{
    execute_instance_backup_with, execute_instance_restore_with, list_instance_backups_with,
    pending_instance_restores_with, plan_instance_restore_with, preview_instance_worlds_with,
    restore_error,
};
use crate::dto::{
    BackupEventDto, BackupRequest, ErrorCode, RestoreEventDto, RestoreExecuteRequest,
    RestorePreviewRequest,
};
use crate::state::AppState;

/// A registry that owns `instance_roots`, one row each.
fn app_state_with_roots(dir: &Path, instance_roots: &[&Path]) -> AppState {
    let db = mm_core::db::open(&dir.join("registry.db")).expect("open db");
    for (index, instance_root) in instance_roots.iter().enumerate() {
        let id = format!("i{}", index + 1);
        mm_core::db::instances::insert(
            &db,
            &mm_core::db::instances::InstanceRow {
                id: id.clone(),
                name: format!("Test Instance {id}"),
                path: std::fs::canonicalize(instance_root).expect("canonical instance"),
                minecraft_root: PathBuf::new(),
                minecraft_version: Some("1.21.1".to_string()),
                loader: Some("fabric".to_string()),
                source: mm_core::db::instances::InstanceSource::Managed,
                created_at: 1,
                scanned_at: None,
            },
        )
        .expect("seed instance row");
    }
    AppState::from_db(db)
}

/// An instance with content a backup can actually carry.
fn instance_with_content(dir: &Path) -> PathBuf {
    let root = dir.join("instance");
    std::fs::create_dir_all(root.join("mods")).expect("mods");
    std::fs::create_dir_all(root.join("config")).expect("config");
    std::fs::write(root.join("mods/one.jar"), b"jar one").expect("jar");
    std::fs::write(root.join("config/one.toml"), b"key = 1\n").expect("config file");
    root
}

fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temp dir")
}

type Sink = Arc<Mutex<Vec<Value>>>;

fn backup_channel(sink: &Sink) -> tauri::ipc::Channel<BackupEventDto> {
    let sink = sink.clone();
    tauri::ipc::Channel::new(move |body| {
        if let tauri::ipc::InvokeResponseBody::Json(text) = body {
            let value: Value = serde_json::from_str(&text).expect("event json");
            sink.lock().expect("sink").push(value);
        }
        Ok(())
    })
}

fn restore_channel(sink: &Sink) -> tauri::ipc::Channel<RestoreEventDto> {
    let sink = sink.clone();
    tauri::ipc::Channel::new(move |body| {
        if let tauri::ipc::InvokeResponseBody::Json(text) = body {
            let value: Value = serde_json::from_str(&text).expect("event json");
            sink.lock().expect("sink").push(value);
        }
        Ok(())
    })
}

async fn make_backup(state: &AppState, root: &Path) -> PathBuf {
    let sink: Sink = Arc::new(Mutex::new(Vec::new()));
    let result = execute_instance_backup_with(
        state,
        BackupRequest {
            instance_root: root.to_string_lossy().into_owned(),
            include_saves: None,
        },
        backup_channel(&sink),
    )
    .await
    .expect("a backup of a plain instance");
    PathBuf::from(result.backup_dir)
}

/// F-113: an unregistered folder is refused, and nothing is created in it.
///
/// The gate is the point: a backup copies out of whatever folder it is handed,
/// so "a path the webview sent" must not be authority for reading it.
#[tokio::test]
async fn spec_f113_backup_refuses_a_folder_the_registry_does_not_own() {
    let dir = tempdir();
    let stranger = instance_with_content(dir.path());
    let state = app_state_with_roots(dir.path(), &[]);
    let sink: Sink = Arc::new(Mutex::new(Vec::new()));

    let err = execute_instance_backup_with(
        &state,
        BackupRequest {
            instance_root: stranger.to_string_lossy().into_owned(),
            include_saves: Some(false),
        },
        backup_channel(&sink),
    )
    .await
    .expect_err("an unregistered folder must be refused");

    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(
        !stranger
            .join(".modrinth_bulk_downloader_profile_backups")
            .exists(),
        "a refused request must not create a backup root"
    );
}

/// F-113: the channel is how the screen learns a run ended, on success and on
/// refusal alike. Every phase is an i18n key, and the last one is terminal.
#[tokio::test]
async fn spec_f113_backup_events_are_keys_and_end_on_a_terminal_event() {
    let dir = tempdir();
    let root = instance_with_content(dir.path());
    let state = app_state_with_roots(dir.path(), &[&root]);
    let sink: Sink = Arc::new(Mutex::new(Vec::new()));

    execute_instance_backup_with(
        &state,
        BackupRequest {
            instance_root: root.to_string_lossy().into_owned(),
            include_saves: None,
        },
        backup_channel(&sink),
    )
    .await
    .expect("a backup");

    let events = sink.lock().expect("sink");
    assert!(!events.is_empty(), "a run must narrate itself");
    for event in events.iter() {
        let phase = event["phase"].as_str().expect("phase is a string");
        assert!(phase.starts_with("backup.phase."), "not a key: {phase}");
    }
    let last = events.last().expect("an event");
    assert_eq!(last["phase"], "backup.phase.done");
    assert_eq!(last["terminal"], true);
}

/// F-113/F-114: the listing says, per folder, whether its manifest is usable.
/// A legacy folder is listed (the user can see it) and says `legacy`.
#[tokio::test]
async fn spec_f113_listing_maps_every_manifest_state_to_a_word() {
    let dir = tempdir();
    let root = instance_with_content(dir.path());
    let state = app_state_with_roots(dir.path(), &[&root]);
    let present = make_backup(&state, &root).await;

    let backups = present.parent().expect("backup root");
    // Missing: a folder with no manifest at all.
    std::fs::create_dir_all(backups.join("2020_01_01T00_00_00Z")).expect("missing");
    // Legacy: the old application's schema, which is listed and refused.
    let legacy = backups.join("2020_01_02T00_00_00Z");
    std::fs::create_dir_all(&legacy).expect("legacy");
    std::fs::write(
        legacy.join("backup_manifest.json"),
        br#"{"schema":"modrinth_bulk_downloader.profile_update_backup.v1",
             "created_at":"2020-01-02T00:00:00Z","profile_id":"i1","entries":[]}"#,
    )
    .expect("legacy manifest");
    // Invalid: JSON, but not a manifest this app wrote.
    let invalid = backups.join("2020_01_03T00_00_00Z");
    std::fs::create_dir_all(&invalid).expect("invalid");
    std::fs::write(invalid.join("backup_manifest.json"), b"{}").expect("invalid manifest");

    let listing = list_instance_backups_with(&state, root.to_string_lossy().into_owned())
        .await
        .expect("a listing");

    let by_stamp = |stamp: &str| {
        listing
            .backups
            .iter()
            .find(|b| b.stamp == stamp)
            .unwrap_or_else(|| panic!("{stamp} must be listed"))
    };
    assert_eq!(by_stamp("2020_01_01T00_00_00Z").manifest_state, "missing");
    assert_eq!(by_stamp("2020_01_02T00_00_00Z").manifest_state, "legacy");
    assert_eq!(by_stamp("2020_01_03T00_00_00Z").manifest_state, "invalid");
    // The real one is present, matches this instance, and counted.
    let ours = listing
        .backups
        .iter()
        .find(|b| b.manifest_state == "present")
        .expect("the backup we just wrote");
    assert!(ours.instance_matches);
    assert!(ours.entry_count > 0);
    // The root is named, so the screen can offer to open it.
    assert!(listing.backup_root.ends_with("profile_backups"));
}

/// F-114: the confirm must carry the manifest digest of the plan it read. A
/// different token stops before the first write — this is the only link between
/// what the user was shown and what would be written.
#[tokio::test]
async fn spec_f114_execute_restore_refuses_a_different_manifest_token() {
    let dir = tempdir();
    let root = instance_with_content(dir.path());
    let state = app_state_with_roots(dir.path(), &[&root]);
    let backup = make_backup(&state, &root).await;

    // A real preview, whose token we then ignore on purpose.
    let preview = plan_instance_restore_with(
        &state,
        RestorePreviewRequest {
            instance_root: root.to_string_lossy().into_owned(),
            backup_dir: backup.to_string_lossy().into_owned(),
            include_saves: None,
        },
    )
    .await
    .expect("a plan");
    assert!(!preview.manifest_sha256.is_empty());

    let before = std::fs::read(root.join("mods/one.jar")).expect("live jar");
    std::fs::write(root.join("mods/one.jar"), b"changed after the preview").expect("edit");

    let sink: Sink = Arc::new(Mutex::new(Vec::new()));
    let err = execute_instance_restore_with(
        &state,
        RestoreExecuteRequest {
            instance_root: root.to_string_lossy().into_owned(),
            backup_dir: backup.to_string_lossy().into_owned(),
            include_saves: false,
            expected_manifest_sha256: "0".repeat(64),
            confirmation: "RESTORE".to_string(),
        },
        restore_channel(&sink),
    )
    .await
    .expect_err("a different token is not this plan");

    assert_eq!(err.code, ErrorCode::RestoreMismatch);
    assert_eq!(
        std::fs::read(root.join("mods/one.jar")).expect("live jar"),
        b"changed after the preview",
        "a refused confirm must not restore anything"
    );
    assert_ne!(
        std::fs::read(root.join("mods/one.jar")).expect("live jar"),
        before,
        "the file was edited, so the refusal is visible"
    );
}

/// F-114: the typed word is checked in Rust, not only in the dialog.
#[tokio::test]
async fn spec_f114_execute_restore_requires_the_typed_word() {
    let dir = tempdir();
    let root = instance_with_content(dir.path());
    let state = app_state_with_roots(dir.path(), &[&root]);
    let backup = make_backup(&state, &root).await;
    let preview = plan_instance_restore_with(
        &state,
        RestorePreviewRequest {
            instance_root: root.to_string_lossy().into_owned(),
            backup_dir: backup.to_string_lossy().into_owned(),
            include_saves: None,
        },
    )
    .await
    .expect("a plan");

    let sink: Sink = Arc::new(Mutex::new(Vec::new()));
    let err = execute_instance_restore_with(
        &state,
        RestoreExecuteRequest {
            instance_root: root.to_string_lossy().into_owned(),
            backup_dir: backup.to_string_lossy().into_owned(),
            include_saves: false,
            expected_manifest_sha256: preview.manifest_sha256.clone(),
            confirmation: "restore".to_string(),
        },
        restore_channel(&sink),
    )
    .await
    .expect_err("the lowercase word is not the word");

    assert_eq!(err.code, ErrorCode::InvalidInput);
    // The refusal narrates its own ending: the screen follows the channel, and
    // a rejection alone would leave the progress view on its last event.
    let events = sink.lock().expect("sink");
    let last = events.last().expect("the failure event");
    assert_eq!(last["phase"], "restore.phase.failed");
    assert_eq!(last["terminal"], true);
}

/// F-114: a completed restore narrates itself and ends on `done`.
#[tokio::test]
async fn spec_f114_restore_events_end_on_a_terminal_event() {
    let dir = tempdir();
    let root = instance_with_content(dir.path());
    let state = app_state_with_roots(dir.path(), &[&root]);
    let backup = make_backup(&state, &root).await;
    std::fs::write(root.join("mods/one.jar"), b"live bytes the backup replaces").expect("edit");
    let preview = plan_instance_restore_with(
        &state,
        RestorePreviewRequest {
            instance_root: root.to_string_lossy().into_owned(),
            backup_dir: backup.to_string_lossy().into_owned(),
            include_saves: None,
        },
    )
    .await
    .expect("a plan");

    let sink: Sink = Arc::new(Mutex::new(Vec::new()));
    let run = execute_instance_restore_with(
        &state,
        RestoreExecuteRequest {
            instance_root: root.to_string_lossy().into_owned(),
            backup_dir: backup.to_string_lossy().into_owned(),
            include_saves: false,
            expected_manifest_sha256: preview.manifest_sha256,
            confirmation: "RESTORE".to_string(),
        },
        restore_channel(&sink),
    )
    .await
    .expect("a restore");

    assert_eq!(run.outcome, "complete");
    assert_eq!(
        std::fs::read(root.join("mods/one.jar")).expect("restored jar"),
        b"jar one"
    );
    let events = sink.lock().expect("sink");
    let last = events.last().expect("an event");
    assert_eq!(last["phase"], "restore.phase.done");
    assert_eq!(last["terminal"], true);
    for event in events.iter() {
        assert!(
            event["phase"]
                .as_str()
                .expect("phase")
                .starts_with("restore.phase."),
            "every restore phase is a key"
        );
    }
}

/// F-114: the two restore codes that are not "invalid input", mapped where both
/// commands share the mapping.
#[test]
fn spec_f114_restore_errors_map_to_the_two_new_codes() {
    use mm_core::instances::backup::restore::RestoreError;
    assert_eq!(
        restore_error(&RestoreError::ManifestChanged).code,
        ErrorCode::BackupChanged
    );
    assert_eq!(
        restore_error(&RestoreError::InstanceMismatch).code,
        ErrorCode::RestoreMismatch
    );
    assert_eq!(
        restore_error(&RestoreError::BackupMissing).code,
        ErrorCode::NotFound
    );
    assert_eq!(
        restore_error(&RestoreError::Manifest(
            mm_core::instances::backup::manifest::ManifestError::NewerSchema {
                found: "mm.instance_backup.v9".to_string(),
                supported: "mm.instance_backup.v1".to_string(),
            }
        ))
        .code,
        ErrorCode::NewerSchema
    );
}

/// F-114: the crash surface the design names — a safety folder with no manifest
/// is reported, read-only, and a completed one is not.
#[tokio::test]
async fn spec_f114_pending_restores_are_reported_through_ipc() {
    let dir = tempdir();
    let root = instance_with_content(dir.path());
    let state = app_state_with_roots(dir.path(), &[&root]);
    let safety = root
        .join(".modrinth_bulk_downloader_restore_safety")
        .join("2023_11_14T22_13_20Z");
    std::fs::create_dir_all(safety.join("current/mods")).expect("safety");
    std::fs::write(safety.join("current/mods/old.jar"), b"old").expect("old");

    let pending = pending_instance_restores_with(&state, root.to_string_lossy().into_owned())
        .await
        .expect("a scan");

    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].stamp, "2023_11_14T22_13_20Z");
    assert_eq!(pending[0].groups, vec!["mods".to_string()]);
    assert_eq!(pending[0].message_code, "restore.pending-incomplete");
}

/// F-118: the preview reads, and an instance without worlds says so with a key.
#[tokio::test]
async fn spec_f118_worlds_preview_through_ipc_reports_a_missing_saves_folder() {
    let dir = tempdir();
    let root = instance_with_content(dir.path());
    let state = app_state_with_roots(dir.path(), &[&root]);

    let preview = preview_instance_worlds_with(&state, root.to_string_lossy().into_owned())
        .await
        .expect("a preview");

    assert!(!preview.saves_exists);
    assert!(preview.worlds.is_empty());
    assert_eq!(preview.message_code, "worlds.no-saves");
    assert!(
        !root.join("saves").exists(),
        "a preview never creates saves/"
    );
}
