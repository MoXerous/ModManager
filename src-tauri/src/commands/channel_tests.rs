//! `execute_install` driven through a real `tauri::ipc::Channel`.
//!
//! This is the test that closes a gap nothing else could. The install command
//! and its Channel had been covered by reasoning and by unit tests on the
//! pieces; no Tauri IPC command in this project had ever run inside a Tauri
//! app. That was left as "someone should press the button", which is not a
//! verification.
//!
//! `Channel::new` takes a plain closure and `Channel` itself needs no window
//! and no runtime, so the whole path - resolve, journal, download, hash
//! verification, atomic rename, and every event that crosses to the webview -
//! runs here with nobody's hand on anything.
//!
//! What this does not cover, stated rather than hidden: the
//! `#[tauri::command]` attribute's own argument extraction. Reaching that
//! needs `tauri::test::mock_builder`, and on Windows the resulting test
//! binary fails to load (STATUS_ENTRYPOINT_NOT_FOUND) before any test runs.
//! Tauri's argument extraction is the framework's code; the parts of this
//! command that are ours are all exercised below.
//!
//! One honest limitation: `InstallEventDto` derives `Serialize`, not
//! `Deserialize`, because nothing in the app ever decodes it - the webview has
//! its own generated type. So the events are asserted as JSON rather than as a
//! round-trip through the Rust struct. Asserting on a decoded struct here
//! would have required adding a derive that production does not need.

use std::io::{Cursor, Write};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::execute_install_with;
use crate::dto::InstallRequest;
use crate::state::AppState;

/// A jar that is a real zip, because the verifier checks ZIP magic on `.jar`.
fn jar_bytes() -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    w.start_file(
        "META-INF/MANIFEST.MF",
        zip::write::SimpleFileOptions::default(),
    )
    .expect("start");
    w.write_all(b"Manifest-Version: 1.0\n").expect("write");
    w.finish().expect("finish").into_inner()
}

fn sha512_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha512::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn sha1_hex(bytes: &[u8]) -> String {
    use sha1::Digest;
    sha1::Sha1::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Mounts the three Modrinth v2 calls `resolve_and_plan` makes, with the file
/// URL pointed back at this server and hashes computed from the bytes. A
/// guessed hash would make the download fail verification correctly and the
/// test would prove nothing.
async fn mount_modrinth(server: &MockServer, jar: &[u8]) {
    mount_modrinth_serving(server, jar, jar).await;
}

/// The same fixture, with the bytes the server actually returns chosen
/// separately from the bytes the version metadata promises. The failure test
/// needs a body that cannot match its declared hash.
async fn mount_modrinth_serving(server: &MockServer, jar: &[u8], body: &[u8]) {
    let project_body = r#"{
        "id": "AANobbMI", "slug": "sodium", "title": "Sodium",
        "description": "d", "body": "b", "client_side": "required",
        "server_side": "required", "game_versions": ["1.21.1"],
        "project_type": "mod", "downloads": 10, "followers": 1,
        "categories": ["fabric"], "loaders": ["fabric"], "versions": ["vvvv"],
        "icon_url": null, "team": "t", "status": "approved",
        "license": {"id": "LGPL-3.0"}, "gallery": []
    }"#;

    // The resolver resolves the slug to an id and then asks by *id*, not by
    // slug. Mounting only the slug path gives a 404 that degrades to "no
    // versions", which looks like a compatibility problem and is not one.
    for p in ["/project/sodium", "/project/AANobbMI"] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_string(project_body))
            .mount(server)
            .await;
    }

    let versions = format!(
        r#"[{{
            "id": "vvvv", "project_id": "AANobbMI", "author_id": "a",
            "featured": false, "name": "0.5.8", "version_number": "0.5.8",
            "changelog": null, "date_published": "2024-01-01T00:00:00Z",
            "downloads": 5, "version_type": "release", "status": "listed",
            "game_versions": ["1.21.1"], "loaders": ["fabric"],
            "files": [{{
                "hashes": {{"sha512": "{}", "sha1": "{}"}},
                "url": "{}/data/sodium.jar",
                "filename": "sodium-0.5.8.jar", "primary": true,
                "size": {}, "file_type": null
            }}],
            "dependencies": []
        }}]"#,
        sha512_hex(jar),
        sha1_hex(jar),
        server.uri(),
        jar.len()
    );
    for p in ["/project/sodium/version", "/project/AANobbMI/version"] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_string(versions.clone()))
            .mount(server)
            .await;
    }

    Mock::given(method("GET"))
        .and(path("/data/sodium.jar"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(body.to_vec())
                .insert_header("content-type", "application/java-archive"),
        )
        .mount(server)
        .await;
}

type Sink = Arc<Mutex<Vec<Value>>>;

/// The command records what it installed in the registry's table, so the
/// folder must be a registered instance before an install can target it.
fn register_instance(db: &mm_core::db::Db, root: &std::path::Path) {
    use mm_core::db::instances::{InstanceRow, InstanceSource};
    mm_core::db::instances::insert(
        db,
        &InstanceRow {
            id: "inst-1".to_string(),
            name: "Test instance".to_string(),
            path: root.to_path_buf(),
            minecraft_root: root.to_path_buf(),
            minecraft_version: Some("1.21.1".to_string()),
            loader: Some("fabric".to_string()),
            source: InstanceSource::Managed,
            created_at: 0,
            scanned_at: None,
        },
    )
    .expect("instance row");
}

fn recording_channel(sink: &Sink) -> tauri::ipc::Channel<InstallEvent> {
    let sink = sink.clone();
    tauri::ipc::Channel::new(move |body| {
        if let tauri::ipc::InvokeResponseBody::Json(text) = body {
            match serde_json::from_str::<Value>(&text) {
                Ok(value) => sink.lock().expect("sink").push(value),
                Err(e) => panic!("the webview could not decode this event as JSON: {e}\n{text}"),
            }
        }
        Ok(())
    })
}

// The channel type the command declares; naming it here keeps the two in sync
// without pulling a DTO that does not need to exist.
type InstallEvent = crate::dto::InstallEventDto;

fn phase(e: &Value) -> &str {
    e["phase"].as_str().unwrap_or("<not a string>")
}

#[test]
fn spec_f037_install_emits_a_well_formed_event_sequence_over_a_real_channel() {
    tauri::async_runtime::block_on(async {
        let server = MockServer::start().await;
        let jar = jar_bytes();
        mount_modrinth(&server, &jar).await;

        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("instance");
        std::fs::create_dir_all(root.join("mods")).expect("mkdir");
        let db = mm_core::db::open(&dir.path().join("t.db")).expect("db");
        register_instance(&db, &root);
        let state = AppState::from_db(db);

        let sink: Sink = Arc::new(Mutex::new(vec![]));
        let provider =
            mm_core::providers::ModrinthClient::with_base(&server.uri()).expect("provider");

        let req = InstallRequest {
            instance_root: root.to_string_lossy().into_owned(),
            projects: vec!["sodium".to_string()],
            provider: "modrinth".to_string(),
            game_version: "1.21.1".to_string(),
            loader: "fabric".to_string(),
        };
        let result =
            execute_install_with(&state, &provider, req, recording_channel(&sink), true).await;

        let result = result.expect("install failed");
        assert_eq!(result.installed.len(), 1, "nothing installed: {result:?}");

        // The bytes landed, verified, under their real name.
        let landed = root.join("mods").join("sodium-0.5.8.jar");
        assert!(landed.is_file(), "the jar did not land");
        assert_eq!(std::fs::read(&landed).expect("read"), jar);

        // The install left a record: the identity and both hashes in the
        // instance lock. Without this, "check for updates" would have nothing
        // to ask about.
        let lock = mm_core::instances::read_lock(&root.join("instance.lock.json"))
            .expect("the install left no lock");
        assert_eq!(
            lock.entries.len(),
            1,
            "one file must produce one entry: {:?}",
            lock.entries
        );
        let entry = &lock.entries[0];
        assert_eq!(entry.provider, "modrinth");
        assert_eq!(entry.project_id, "AANobbMI");
        assert_eq!(entry.version_id, "vvvv");
        assert_eq!(entry.filename, "sodium-0.5.8.jar");
        assert_eq!(entry.sha512, sha512_hex(&jar));
        assert_eq!(entry.sha1, sha1_hex(&jar));

        let events = sink.lock().expect("events").clone();
        assert!(!events.is_empty(), "the channel carried nothing at all");

        assert!(
            events
                .iter()
                .all(|e| e["itemId"].as_str().is_some_and(|s| !s.is_empty())),
            "an event had no itemId: {events:?}"
        );
        assert!(
            events
                .iter()
                .all(|e| phase(e).starts_with("install.phase.")),
            "a phase was not an i18n key: {events:?}"
        );
        // Every event is for the one item, keyed by its version id - not by the
        // filename. That was this test's wrong assumption first time.
        assert!(
            events.iter().all(|e| e["itemId"] == "vvvv"),
            "an event named a different item: {events:?}"
        );

        // A progress event carries real numbers. The throttle can coalesce
        // the first event away: 162 bytes finish inside the 100 ms window, so
        // `started` may legitimately never be sent. Asserting it did arrive
        // would be asserting a coincidence.
        for e in events
            .iter()
            .filter(|e| phase(e) == "install.phase.downloading")
        {
            assert!(e["done"].is_number() && e["total"].is_number(), "{e}");
        }

        // The last thing said is that it finished, and it names the real file.
        // A queue that goes quiet, or that ends on progress instead of on a
        // landing, is the bug this asserts against.
        let last = events.last().expect("last");
        assert_eq!(
            phase(last),
            "install.phase.landed",
            "the last event was not terminal: {events:?}"
        );
        assert_eq!(
            last["path"].as_str().expect("path"),
            landed.to_string_lossy(),
            "the landing event named a different file"
        );
    });
}

/// Isolates the resolver from the rest of the command. When an install says
/// "nothing to install" this is where the answer is: it names which ladder
/// rung refused the version, instead of leaving the message to be guessed at.
#[test]
fn the_resolver_calls_this_fixture_downloadable() {
    tauri::async_runtime::block_on(async {
        let server = MockServer::start().await;
        let jar = jar_bytes();
        mount_modrinth(&server, &jar).await;

        let provider =
            mm_core::providers::ModrinthClient::with_base(&server.uri()).expect("provider");
        let plan = mm_core::resolver::resolve_and_plan(
            &provider,
            &["sodium".to_string()],
            "1.21.1",
            "fabric",
            &Default::default(),
        )
        .await
        .expect("resolve failed");

        assert_eq!(plan.entries.len(), 1, "no entries: {:?}", plan.entries);
        let entry = &plan.entries[0];
        assert_eq!(
            entry.status, "downloadable",
            "the resolver refused it and said: {}",
            entry.reason
        );
        assert!(plan.versions.contains_key("vvvv"), "version map is empty");
    });
}

#[test]
fn install_refuses_an_empty_slug_list_without_emitting_anything() {
    tauri::async_runtime::block_on(async {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = mm_core::db::open(&dir.path().join("t.db")).expect("db");
        let state = AppState::from_db(db);
        let sink: Sink = Arc::new(Mutex::new(vec![]));
        // A base that cannot be reached: if the guard fires first, as it must,
        // nothing will try to talk to it.
        let provider =
            mm_core::providers::ModrinthClient::with_base("http://127.0.0.1:1").expect("provider");

        let req = InstallRequest {
            instance_root: dir.path().join("instance").to_string_lossy().into_owned(),
            projects: vec![],
            provider: "modrinth".to_string(),
            game_version: "1.21.1".to_string(),
            loader: "fabric".to_string(),
        };
        let out =
            execute_install_with(&state, &provider, req, recording_channel(&sink), true).await;

        assert!(out.is_err(), "an empty install must not succeed");
        assert!(
            sink.lock().expect("events").is_empty(),
            "a refused install must not narrate itself"
        );
    });
}

/// F-152: the same install that lands a file leaves exactly one row saying so.
///
/// The row is read back through core's own `list`, not through the command, so
/// this test covers the write side of the log and the query side in one run.
#[test]
fn spec_f152_a_successful_install_writes_one_history_row() {
    tauri::async_runtime::block_on(async {
        let server = MockServer::start().await;
        let jar = jar_bytes();
        mount_modrinth(&server, &jar).await;

        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("instance");
        std::fs::create_dir_all(root.join("mods")).expect("mkdir");
        let db = mm_core::db::open(&dir.path().join("t.db")).expect("db");
        register_instance(&db, &root);
        let state = AppState::from_db(db);

        let sink: Sink = Arc::new(Mutex::new(vec![]));
        let provider =
            mm_core::providers::ModrinthClient::with_base(&server.uri()).expect("provider");
        let req = InstallRequest {
            instance_root: root.to_string_lossy().into_owned(),
            projects: vec!["sodium".to_string()],
            provider: "modrinth".to_string(),
            game_version: "1.21.1".to_string(),
            loader: "fabric".to_string(),
        };
        execute_install_with(&state, &provider, req, recording_channel(&sink), true)
            .await
            .expect("install failed");

        let rows = state
            .with_db(|db| {
                mm_core::db::download_history::list(db, &Default::default())
                    .map_err(|e| crate::dto::IpcError::internal(e.to_string()))
            })
            .await
            .expect("history");

        assert_eq!(rows.len(), 1, "one file, one row: {rows:?}");
        let row = &rows[0];
        assert_eq!(row.status, mm_core::db::download_history::Status::Success);
        assert_eq!(row.provider, "modrinth");
        assert_eq!(row.project_id, "AANobbMI");
        assert_eq!(row.slug, "sodium");
        assert_eq!(row.title, "Sodium");
        assert_eq!(row.version_id.as_deref(), Some("vvvv"));
        assert_eq!(row.version_number.as_deref(), Some("0.5.8"));
        assert_eq!(row.game_version, "1.21.1");
        assert_eq!(row.loader, "fabric");
        assert_eq!(row.file_name, "sodium-0.5.8.jar");
        let sha1 = sha1_hex(&jar);
        assert_eq!(row.file_sha1.as_deref(), Some(sha1.as_str()));
        assert!(
            row.destination_path
                .as_deref()
                .unwrap_or_default()
                .ends_with("sodium-0.5.8.jar"),
            "the row must name the file that landed: {:?}",
            row.destination_path
        );
        assert!(row.created_at > 0, "a row with no time cannot be listed");
    });
}

/// F-152: a file that fails verification is the row a user goes looking for,
/// so a batch that stops early still writes it.
#[test]
fn spec_f152_a_failed_download_still_writes_a_failed_row() {
    tauri::async_runtime::block_on(async {
        let server = MockServer::start().await;
        let jar = jar_bytes();
        mount_modrinth_serving(&server, &jar, b"not the bytes the hashes promise").await;

        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("instance");
        std::fs::create_dir_all(root.join("mods")).expect("mkdir");
        let db = mm_core::db::open(&dir.path().join("t.db")).expect("db");
        register_instance(&db, &root);
        let state = AppState::from_db(db);

        let sink: Sink = Arc::new(Mutex::new(vec![]));
        let provider =
            mm_core::providers::ModrinthClient::with_base(&server.uri()).expect("provider");
        let req = InstallRequest {
            instance_root: root.to_string_lossy().into_owned(),
            projects: vec!["sodium".to_string()],
            provider: "modrinth".to_string(),
            game_version: "1.21.1".to_string(),
            loader: "fabric".to_string(),
        };
        let out =
            execute_install_with(&state, &provider, req, recording_channel(&sink), true).await;
        assert!(out.is_err(), "a hash mismatch must not install");

        let rows = state
            .with_db(|db| {
                mm_core::db::download_history::list(db, &Default::default())
                    .map_err(|e| crate::dto::IpcError::internal(e.to_string()))
            })
            .await
            .expect("history");

        assert_eq!(
            rows.len(),
            1,
            "the failure is the point of the row: {rows:?}"
        );
        assert_eq!(
            rows[0].status,
            mm_core::db::download_history::Status::Failed
        );
        assert!(
            !rows[0].message.is_empty(),
            "a failed row with no reason is a dead end"
        );
        assert_eq!(rows[0].destination_path, None);
        assert!(
            !root.join("mods").join("sodium-0.5.8.jar").exists(),
            "a failed verification must not leave a jar"
        );
    });
}
