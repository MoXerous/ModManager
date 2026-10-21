//! `plan_modpack_install` and `execute_modpack_install` driven for real.
//!
//! Same reasoning as `channel_tests.rs`: `Channel::new` takes a plain closure
//! and needs no window, wiremock stands in for Modrinth, and tempfile stands
//! in for the filesystem. `tauri::test::mock_builder` is avoided on purpose —
//! on Windows it makes the whole test binary fail to load
//! (STATUS_ENTRYPOINT_NOT_FOUND).
//!
//! What the plan command must prove, above everything else: it writes
//! NOTHING. No instance file, no staging directory, nothing. The dry run's
//! whole value is that the user can close the dialog without a half install
//! on disk.
//!
//! What the apply must prove beyond installing: it only writes into an
//! instance the registry owns, it only applies the archive the plan described,
//! and a failure before core's executor starts still ends the stream and
//! clears the staging tree.

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::pack::{execute_modpack_install_with, plan_modpack_install_with, run_apply};
use crate::dto::{ErrorCode, ModpackInstallRequest, PackEventDto};
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

/// Builds a minimal `.mrpack`: the index names one hash-gated download
/// pointed at `file_url`, and the archive carries one override file. The
/// hashes are computed from the real bytes, so the later verification passes
/// for the right reason.
fn pack_bytes(file_url: &str, jar: &[u8]) -> Vec<u8> {
    pack_bytes_with_override(file_url, jar, "built = by-the-test\n")
}

/// The same pack with a chosen override body, so a test can replace the file
/// on disk with a different archive that is still a valid pack.
fn pack_bytes_with_override(file_url: &str, jar: &[u8], override_body: &str) -> Vec<u8> {
    let index = serde_json::json!({
        "formatVersion": 1,
        "game": "minecraft",
        "versionId": "v1.2.3",
        "name": "Wiremock Pack",
        "summary": "a pack built only for tests",
        "files": [{
            "path": "mods/packfile.jar",
            "hashes": {
                "sha1": sha1_hex(jar),
                "sha512": sha512_hex(jar),
            },
            "downloads": [file_url],
            "fileSize": jar.len(),
        }],
        "overrides": "overrides",
    })
    .to_string();

    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    w.start_file(
        "modrinth.index.json",
        zip::write::SimpleFileOptions::default(),
    )
    .expect("index member");
    w.write_all(index.as_bytes()).expect("index bytes");
    w.start_file(
        "overrides/config/pack.toml",
        zip::write::SimpleFileOptions::default(),
    )
    .expect("override member");
    w.write_all(override_body.as_bytes())
        .expect("override bytes");
    w.finish().expect("finish").into_inner()
}

/// A request that carries the digest of the pack on disk, which is what the
/// screen does after a plan. A pack that is not there gets no digest, so the
/// path gates can still be tested for their own refusal.
fn request(pack_path: &Path, instance_root: &Path) -> ModpackInstallRequest {
    ModpackInstallRequest {
        pack_path: pack_path.to_string_lossy().into_owned(),
        instance_root: instance_root.to_string_lossy().into_owned(),
        pack_sha512: std::fs::read(pack_path).ok().map(|b| sha512_hex(&b)),
    }
}

/// An `AppState` whose registry owns `instance_root`, and nothing else.
///
/// This is the production gate the commands consult: a folder that exists but
/// has no row here must be refused, so every test that wants to get past the
/// gate has to register its root the way `execute_create_instance` would.
fn app_state(dir: &Path, instance_root: &Path) -> AppState {
    let db = mm_core::db::open(&dir.join("registry.db")).expect("open db");
    let path = std::fs::canonicalize(instance_root).expect("canonical instance");
    mm_core::db::instances::insert(
        &db,
        &mm_core::db::instances::InstanceRow {
            id: "i1".into(),
            name: "Test Instance".into(),
            path,
            minecraft_root: PathBuf::new(),
            minecraft_version: None,
            loader: None,
            source: mm_core::db::instances::InstanceSource::Managed,
            created_at: 1,
            scanned_at: None,
        },
    )
    .expect("seed instance row");
    AppState::from_db(db)
}

/// The file inventory under `root`: every file, relative to it, sorted.
fn inventory(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in walk(root) {
        out.push(
            entry
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        );
    }
    out.sort();
    out
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.path().is_dir() {
                out.extend(walk(&e.path()));
            } else {
                out.push(e.path());
            }
        }
    }
    out
}

type Sink = Arc<Mutex<Vec<Value>>>;

fn recording_channel(sink: &Sink) -> tauri::ipc::Channel<PackEventDto> {
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

fn phase(e: &Value) -> &str {
    e["phase"].as_str().unwrap_or("<not a string>")
}

#[test]
fn pack_plan_writes_nothing_and_cleans_up_staging() {
    tauri::async_runtime::block_on(async {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("instance");
        std::fs::create_dir_all(root.join("mods")).expect("mkdir mods");
        // A pre-existing destination is the one real signal in the plan: it
        // must come back as `overwrites`, not as "the apply may create it".
        std::fs::write(root.join("mods/packfile.jar"), b"pre-existing").expect("existing jar");
        let state = app_state(dir.path(), &root);

        let pack = pack_bytes("https://cdn.example.com/x.jar", &jar_bytes());
        let pack_path = dir.path().join("the-pack.mrpack");
        std::fs::write(&pack_path, &pack).expect("write pack");
        let before = inventory(&root);

        let plan = plan_modpack_install_with(&state, request(&pack_path, &root))
            .await
            .expect("plan refused a valid pack");

        // What the pack is, straight from the index.
        assert_eq!(plan.name, "Wiremock Pack");
        assert_eq!(plan.version_id, "v1.2.3");
        assert_eq!(plan.summary.as_deref(), Some("a pack built only for tests"));

        // The plan names the archive it read, byte for byte, so the apply can
        // ask "still that file?" before writing anything.
        assert_eq!(
            plan.pack_sha512,
            sha512_hex(&pack),
            "the plan's digest is not the sha512 of the pack it read"
        );

        // One download (the jar), one override (config/pack.toml), and
        // exactly one replacement: the jar the instance already had.
        assert_eq!(plan.download_count, 1, "{plan:?}");
        assert_eq!(plan.override_count, 1, "{plan:?}");
        assert_eq!(plan.replace_count, 1, "{plan:?}");
        assert_eq!(plan.writes.len(), 2, "{plan:?}");

        let jar_entry = plan
            .writes
            .iter()
            .find(|w| w.safe_path == "mods/packfile.jar")
            .expect("the jar write is missing");
        assert!(
            jar_entry.overwrites,
            "an existing file must read as replaced"
        );
        assert!(matches!(
            jar_entry.kind,
            crate::dto::PackWriteKindDto::Download
        ));
        assert_eq!(jar_entry.size, Some(jar_bytes().len() as u64));

        let toml_entry = plan
            .writes
            .iter()
            .find(|w| w.safe_path == "config/pack.toml")
            .expect("the override write is missing");
        assert!(
            !toml_entry.overwrites,
            "an absent destination must read as created"
        );
        assert!(matches!(
            toml_entry.kind,
            crate::dto::PackWriteKindDto::Override
        ));
        assert_eq!(toml_entry.size, None, "an override declares no size");

        // The point of the whole command: the instance is byte-for-byte what
        // it was, and no staging tree exists after the run.
        assert_eq!(inventory(&root), before, "the plan moved the instance");
        assert!(
            !root.join(".mm-staging").exists(),
            "the plan left a staging directory behind"
        );
        assert_eq!(
            std::fs::read(root.join("mods/packfile.jar")).expect("read"),
            b"pre-existing",
            "the plan replaced a file"
        );
    });
}

/// The two paths in the request are webview input, and each gate must refuse
/// before anything is read. The codes are the ones the rest of the IPC surface
/// already uses: a blank path is the caller's mistake, a pack file that is not
/// there is `notFound` (the user chose a file), and a folder that canonicalizes
/// to nothing is the `io` failure `resolve_dir` reports for every command.
#[test]
fn pack_plan_refuses_bad_paths_before_reading_anything() {
    tauri::async_runtime::block_on(async {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("instance");
        std::fs::create_dir_all(&root).expect("mkdir instance");
        let state = app_state(dir.path(), &root);
        let pack_path = dir.path().join("the-pack.mrpack");
        std::fs::write(
            &pack_path,
            pack_bytes("https://cdn.example.com/x.jar", &jar_bytes()),
        )
        .expect("write pack");

        // A blank pack path never reaches the filesystem.
        let blank = plan_modpack_install_with(
            &state,
            ModpackInstallRequest {
                pack_path: "   ".to_string(),
                instance_root: root.to_string_lossy().into_owned(),
                pack_sha512: None,
            },
        )
        .await;
        assert_eq!(
            blank.expect_err("a blank pack path must be refused").code,
            ErrorCode::InvalidInput
        );

        // The pack half of the pair is resolved first; a missing file is the
        // user's pick, not a failing disk.
        let missing_pack =
            plan_modpack_install_with(&state, request(&dir.path().join("gone.mrpack"), &root))
                .await;
        assert_eq!(
            missing_pack
                .expect_err("a missing pack must be refused")
                .code,
            ErrorCode::NotFound
        );

        // And with a real pack, a missing instance folder is refused too, with
        // nothing created on its behalf.
        let missing_root = dir.path().join("no-such-instance");
        let out = plan_modpack_install_with(&state, request(&pack_path, &missing_root)).await;
        assert_eq!(
            out.expect_err("a missing instance folder must be refused")
                .code,
            ErrorCode::Io
        );
        assert!(
            !missing_root.exists(),
            "the refusal created the folder it refused"
        );
    });
}

/// A folder the registry does not own is not an install target, however
/// absolute and however real it is. Both commands must refuse it, and the
/// apply must refuse before it touches anything inside: a refused folder is
/// somebody's home directory, not our staging ground.
#[test]
fn pack_plan_and_apply_refuse_an_unregistered_instance_root() {
    tauri::async_runtime::block_on(async {
        let dir = tempfile::tempdir().expect("tempdir");
        // Registered: the install gate has something legitimate to compare
        // against, so the refusal below is about the target, not about an
        // empty table.
        let registered = dir.path().join("registered");
        std::fs::create_dir_all(&registered).expect("mkdir registered");
        let state = app_state(dir.path(), &registered);

        let stranger = dir.path().join("stranger");
        std::fs::create_dir_all(stranger.join("mods")).expect("mkdir stranger");
        std::fs::write(stranger.join("keep.txt"), b"do not touch").expect("write keep");
        // Leftovers a killed apply would leave: still not ours to delete.
        let stale = stranger.join(".mm-staging/leftover");
        std::fs::create_dir_all(&stale).expect("mkdir stale");
        std::fs::write(stale.join("x.bin"), b"stale").expect("write stale");
        let before = inventory(&stranger);

        let pack_path = dir.path().join("the-pack.mrpack");
        std::fs::write(
            &pack_path,
            pack_bytes("https://cdn.example.com/x.jar", &jar_bytes()),
        )
        .expect("write pack");

        let planned = plan_modpack_install_with(&state, request(&pack_path, &stranger)).await;
        assert_eq!(
            planned
                .expect_err("an unregistered folder must be refused")
                .code,
            ErrorCode::InvalidInput
        );

        let sink: Sink = Arc::new(Mutex::new(vec![]));
        let provider =
            mm_core::providers::ModrinthClient::with_base("http://127.0.0.1:1").expect("provider");
        let applied = execute_modpack_install_with(
            &state,
            &provider,
            request(&pack_path, &stranger),
            recording_channel(&sink),
            true,
        )
        .await;
        assert_eq!(
            applied
                .expect_err("an unregistered folder must be refused")
                .code,
            ErrorCode::InvalidInput
        );

        assert_eq!(
            inventory(&stranger),
            before,
            "the refusal wrote into a folder it does not own"
        );
        // Nothing was narrated, so there is no stream to have left hanging.
        assert!(
            sink.lock().expect("events").is_empty(),
            "a refusal before the run starts must not open with progress"
        );
    });
}

#[test]
fn pack_apply_installs_overrides_and_verified_downloads_over_a_real_channel() {
    tauri::async_runtime::block_on(async {
        let server = MockServer::start().await;

        let jar = jar_bytes();
        // No /version_files mount on purpose: the provider lookup degrades
        // gracefully and the pack's own URL carries the download, which is
        // the production path for a pack when Modrinth is unreachable.
        Mock::given(method("GET"))
            .and(path("/data/packfile.jar"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(jar.clone())
                    .insert_header("content-type", "application/java-archive"),
            )
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("instance");
        std::fs::create_dir_all(root.join("mods")).expect("mkdir mods");
        let state = app_state(dir.path(), &root);

        let pack_path = dir.path().join("the-pack.mrpack");
        std::fs::write(
            &pack_path,
            pack_bytes(&format!("{}/data/packfile.jar", server.uri()), &jar),
        )
        .expect("write pack");

        let sink: Sink = Arc::new(Mutex::new(vec![]));
        let provider =
            mm_core::providers::ModrinthClient::with_base(&server.uri()).expect("provider");

        let result = execute_modpack_install_with(
            &state,
            &provider,
            request(&pack_path, &root),
            recording_channel(&sink),
            true,
        )
        .await
        .expect("apply failed");

        // One override and one verified download landed.
        assert_eq!(result.written, 2, "{result:?}");
        assert_eq!(result.skipped, 0, "{result:?}");
        let landed = root.join("mods/packfile.jar");
        assert_eq!(
            std::fs::read(&landed).expect("read"),
            jar,
            "the jar bytes lie"
        );
        let override_file = root.join("config/pack.toml");
        assert_eq!(
            std::fs::read(&override_file).expect("read"),
            b"built = by-the-test\n",
            "the override bytes lie"
        );

        // The staging tree is executor cleanup's contract: gone when the run
        // ended, success included.
        assert!(
            !root.join(".mm-staging").exists(),
            "the apply left its staging directory behind"
        );

        // Every phase is a key, never a display word, and the run ends on the
        // one event that says it ended.
        let events = sink.lock().expect("events").clone();
        assert!(!events.is_empty(), "the channel carried nothing at all");
        assert!(
            events.iter().all(|e| phase(e).starts_with("pack.phase.")),
            "a phase was not an i18n key: {events:?}"
        );
        assert_eq!(
            phase(events.last().expect("last")),
            "pack.phase.done",
            "the last event was not terminal: {events:?}"
        );

        // Both destinations must settle on the channel. These two events land
        // inside one throttle window, so this is also the assertion that the
        // throttle never swallows a state: a dropped `written` leaves its row
        // standing at `copying` or `downloading` for the rest of the run.
        let mut settled: Vec<&str> = events
            .iter()
            .filter(|e| phase(e) == "pack.phase.written")
            .filter_map(|e| e["safePath"].as_str())
            .collect();
        settled.sort_unstable();
        assert_eq!(
            settled,
            vec!["config/pack.toml", "mods/packfile.jar"],
            "a written file never reached the channel: {events:?}"
        );

        // A progress event carries real byte counters when it carries any.
        for e in events
            .iter()
            .filter(|e| phase(e) == "pack.phase.downloading")
        {
            assert!(e["done"].is_number(), "{e}");
            assert!(e["total"].is_number(), "{e}");
        }

        // Applying the same pack again is not a re-download: the jar already
        // holds the right bytes, so it is skipped; the override, which the
        // format has no hash for, is re-copied.
        let sink2: Sink = Arc::new(Mutex::new(vec![]));
        let again = execute_modpack_install_with(
            &state,
            &provider,
            request(&pack_path, &root),
            recording_channel(&sink2),
            true,
        )
        .await
        .expect("second apply failed");
        assert_eq!(again.skipped, 1, "{again:?}");
        assert_eq!(again.written, 1, "{again:?}");
    });
}

/// A refused apply must not go silent: the run never reaches core's executor,
/// so the refusal event is this module's job — and the staging tree it
/// started must not survive the refusal either.
#[test]
fn pack_apply_refuses_a_non_mrpack_file_ends_the_stream_and_cleans_staging() {
    tauri::async_runtime::block_on(async {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("instance");
        std::fs::create_dir_all(root.join("mods")).expect("mkdir mods");
        let state = app_state(dir.path(), &root);
        // The instance as it stands, before a stale run's leftovers are added:
        // this is what "nothing moved" has to be measured against, because the
        // staging tree below is exactly what the apply must delete.
        let before = inventory(&root);
        // A stale staging tree from a killed run: the apply must remove it
        // first, and a refused archive must not resurrect it.
        let stale = root.join(".mm-staging/staging-guts");
        std::fs::create_dir_all(&stale).expect("mkdir staging");
        std::fs::write(stale.join("x.bin"), b"stale").expect("write stale");

        let pack_path = dir.path().join("not-a-pack.mrpack");
        std::fs::write(&pack_path, b"this is not a zip").expect("write pack");

        let sink: Sink = Arc::new(Mutex::new(vec![]));
        let provider =
            mm_core::providers::ModrinthClient::with_base("http://127.0.0.1:1").expect("provider");

        let out = execute_modpack_install_with(
            &state,
            &provider,
            request(&pack_path, &root),
            recording_channel(&sink),
            true,
        )
        .await;

        let err = out.expect_err("a non-zip pack must be refused");
        assert_eq!(
            err.code,
            ErrorCode::InvalidInput,
            "the wrong file is the caller's mistake, not a failing disk"
        );

        // The stream still ends on a terminal event, so the progress view
        // never sits on nothing while the promise rejects.
        let events = sink.lock().expect("events").clone();
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(phase(&events[0]), "pack.phase.failed");
        assert!(
            events[0]["safePath"].is_null(),
            "a pre-archive refusal has no path to name"
        );

        // Nothing was written, the staging tree is gone, not resumed.
        assert_eq!(
            inventory(&root),
            before,
            "a refused apply moved the instance"
        );
        assert!(
            !root.join(".mm-staging").exists(),
            "the refusal left the staging tree behind"
        );
    });
}

/// The plan describes one archive; the apply must only ever write that one.
///
/// The pack file is replaced between the two calls with a different valid
/// pack — the case a timed race or a replaced download would produce. The
/// digest the plan handed out no longer names the file, so the apply refuses
/// and writes nothing, instead of installing a plan nobody saw.
#[test]
fn pack_apply_refuses_a_pack_file_that_changed_after_the_plan() {
    tauri::async_runtime::block_on(async {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("instance");
        std::fs::create_dir_all(root.join("mods")).expect("mkdir mods");
        let state = app_state(dir.path(), &root);

        let jar = jar_bytes();
        let pack_path = dir.path().join("the-pack.mrpack");
        std::fs::write(
            &pack_path,
            pack_bytes_with_override("https://cdn.example.com/x.jar", &jar, "v1\n"),
        )
        .expect("write pack");

        // What the user saw.
        let planned = plan_modpack_install_with(&state, request(&pack_path, &root))
            .await
            .expect("plan refused a valid pack");
        let planned_digest = planned.pack_sha512.clone();

        // What the file is now: still a valid pack, different overrides.
        std::fs::write(
            &pack_path,
            pack_bytes_with_override("https://cdn.example.com/x.jar", &jar, "v2-NEVER-SHOWN\n"),
        )
        .expect("replace pack");
        assert_ne!(
            sha512_hex(&std::fs::read(&pack_path).expect("read")),
            planned_digest,
            "the fixture did not change the file"
        );

        let before = inventory(&root);
        let sink: Sink = Arc::new(Mutex::new(vec![]));
        let provider =
            mm_core::providers::ModrinthClient::with_base("http://127.0.0.1:1").expect("provider");

        // The request carries the digest of the pack as it was at plan time.
        let mut req = request(&pack_path, &root);
        req.pack_sha512 = Some(planned_digest);
        let out =
            execute_modpack_install_with(&state, &provider, req, recording_channel(&sink), true)
                .await;

        let err = out.expect_err("a pack replaced after its plan must be refused");
        assert_eq!(
            err.code,
            ErrorCode::InvalidInput,
            "the request names an archive that is no longer there: {err:?}"
        );

        // Nothing of the unseen pack was written, and the refusal cleaned up.
        assert_eq!(
            inventory(&root),
            before,
            "the unseen pack wrote into the instance"
        );
        assert!(
            !root.join(".mm-staging").exists(),
            "the refused apply left its staging tree behind"
        );
        let events = sink.lock().expect("events").clone();
        assert_eq!(
            events.last().map(phase),
            Some("pack.phase.failed"),
            "the refusal did not end the stream: {events:?}"
        );
    });
}

/// An apply that fails before core's executor exists still owes the screen an
/// ending and the instance a clean staging tree.
///
/// The downloader is the first thing an apply builds and the last thing that
/// used to be able to fail silently: the function returned, the tree stayed,
/// and the channel never spoke again. The seam passes the failure in, because
/// no environment can be made to fail `Client::build` on demand.
#[test]
fn pack_apply_cleans_staging_and_ends_the_stream_when_it_fails_before_starting() {
    tauri::async_runtime::block_on(async {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("instance");
        std::fs::create_dir_all(root.join("mods")).expect("mkdir mods");
        let state = app_state(dir.path(), &root);

        let pack_path = dir.path().join("the-pack.mrpack");
        std::fs::write(
            &pack_path,
            pack_bytes("https://cdn.example.com/x.jar", &jar_bytes()),
        )
        .expect("write pack");

        // The instance as it stands, before a stale run's leftovers are added:
        // the staging tree below is exactly what the failed run must delete.
        let before = inventory(&root);
        // A staging tree that a previous, killed run left behind.
        let stale = root.join(".mm-staging/staging-guts");
        std::fs::create_dir_all(&stale).expect("mkdir staging");
        std::fs::write(stale.join("x.bin"), b"stale").expect("write stale");

        let sink: Sink = Arc::new(Mutex::new(vec![]));
        let provider =
            mm_core::providers::ModrinthClient::with_base("http://127.0.0.1:1").expect("provider");

        let out = run_apply(
            &state,
            &provider,
            request(&pack_path, &root),
            recording_channel(&sink),
            true,
            Err(mm_core::downloader::client::DownloadError::Internal(
                "client build failed",
            )),
        )
        .await;

        let err = out.expect_err("a downloader that cannot be built must fail the apply");
        assert_eq!(err.code, ErrorCode::Internal, "{err:?}");

        // Both halves of the failure, in one report: the run ends on the
        // terminal event and the tree it would have used does not survive.
        let events = sink.lock().expect("events").clone();
        let staging_gone = !root.join(".mm-staging").exists();
        assert!(
            staging_gone && events.len() == 1,
            "a pre-apply failure must clean its staging tree and end the stream \
             (staging gone: {staging_gone}, events: {events:?})"
        );
        assert_eq!(phase(&events[0]), "pack.phase.failed");

        assert_eq!(
            inventory(&root),
            before,
            "the failed apply moved the instance"
        );
    });
}

/// Two applies into one instance must not run at the same time.
///
/// They share `<instance>/.mm-staging`, so without serialization one run's
/// reset or cleanup deletes the other's extracted overrides. The observable
/// consequence is the second run re-downloading the jar the first one just
/// committed (written 2, skipped 0 instead of written 1, skipped 1), and an
/// event log in which the two runs alternate.
#[test]
fn pack_apply_serializes_two_runs_on_one_instance() {
    tauri::async_runtime::block_on(async {
        let server = MockServer::start().await;
        let jar = jar_bytes();
        // Slow enough that the second run is certainly inside the first one's
        // download window when it starts.
        Mock::given(method("GET"))
            .and(path("/data/packfile.jar"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(jar.clone())
                    .set_delay(Duration::from_millis(400))
                    .insert_header("content-type", "application/java-archive"),
            )
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("instance");
        std::fs::create_dir_all(root.join("mods")).expect("mkdir mods");
        let state = app_state(dir.path(), &root);

        let pack_path = dir.path().join("the-pack.mrpack");
        std::fs::write(
            &pack_path,
            pack_bytes(&format!("{}/data/packfile.jar", server.uri()), &jar),
        )
        .expect("write pack");

        // One log for both runs, each event tagged with the run it came from:
        // serialization is exactly "the runs do not alternate in this log".
        let log: Arc<Mutex<Vec<(usize, String)>>> = Arc::new(Mutex::new(vec![]));
        let mut channels = Vec::new();
        for run in 0..2 {
            let log = log.clone();
            channels.push(tauri::ipc::Channel::new(move |body| {
                if let tauri::ipc::InvokeResponseBody::Json(text) = body {
                    let value: Value = serde_json::from_str(&text).expect("event json");
                    let phase = value["phase"].as_str().unwrap_or("?").to_string();
                    log.lock().expect("log").push((run, phase));
                }
                Ok(())
            }));
        }

        let state_a = state.clone();
        let state_b = state.clone();
        let provider_a =
            mm_core::providers::ModrinthClient::with_base(&server.uri()).expect("provider a");
        let provider_b =
            mm_core::providers::ModrinthClient::with_base(&server.uri()).expect("provider b");
        let req_a = request(&pack_path, &root);
        let req_b = request(&pack_path, &root);
        let mut channels = channels.into_iter();
        let channel_a = channels.next().expect("channel a");
        let channel_b = channels.next().expect("channel b");

        let first = tauri::async_runtime::spawn(async move {
            execute_modpack_install_with(&state_a, &provider_a, req_a, channel_a, true).await
        });
        let second = tauri::async_runtime::spawn(async move {
            execute_modpack_install_with(&state_b, &provider_b, req_b, channel_b, true).await
        });

        let a = first
            .await
            .expect("join first run")
            .expect("first apply failed");
        let b = second
            .await
            .expect("join second run")
            .expect("second apply failed");

        // Serialized, the second run finds the jar the first one committed:
        // one run writes two files, the other writes the override and skips
        // the jar. Which one goes first is the scheduler's business.
        let mut outcomes: Vec<(usize, usize)> =
            vec![(a.written, a.skipped), (b.written, b.skipped)];
        outcomes.sort_unstable();
        assert_eq!(
            outcomes,
            vec![(1, 1), (2, 0)],
            "the two runs overlapped: {a:?} {b:?}"
        );

        // And the channel says the same thing: no run's events are split by
        // the other's.
        let events = log.lock().expect("log").clone();
        let transitions = events.windows(2).filter(|w| w[0].0 != w[1].0).count();
        assert!(
            transitions <= 1,
            "the two applies interleaved on the channel: {events:?}"
        );
        assert!(
            events.iter().any(|(_, p)| p == "pack.phase.done"),
            "a run never reported done: {events:?}"
        );
    });
}
