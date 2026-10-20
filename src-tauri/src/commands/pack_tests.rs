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

use std::io::{Cursor, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::pack::{execute_modpack_install_with, plan_modpack_install};
use crate::dto::{ErrorCode, ModpackInstallRequest, PackEventDto};

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
    w.write_all(b"built = by-the-test\n")
        .expect("override bytes");
    w.finish().expect("finish").into_inner()
}

fn request(pack_path: &Path, instance_root: &Path) -> ModpackInstallRequest {
    ModpackInstallRequest {
        pack_path: pack_path.to_string_lossy().into_owned(),
        instance_root: instance_root.to_string_lossy().into_owned(),
    }
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

        let pack_path = dir.path().join("the-pack.mrpack");
        std::fs::write(
            &pack_path,
            pack_bytes("https://cdn.example.com/x.jar", &jar_bytes()),
        )
        .expect("write pack");
        let before = inventory(&root);

        let plan = plan_modpack_install(request(&pack_path, &root))
            .await
            .expect("plan refused a valid pack");

        // What the pack is, straight from the index.
        assert_eq!(plan.name, "Wiremock Pack");
        assert_eq!(plan.version_id, "v1.2.3");
        assert_eq!(plan.summary.as_deref(), Some("a pack built only for tests"));

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
        let pack_path = dir.path().join("the-pack.mrpack");
        std::fs::write(
            &pack_path,
            pack_bytes("https://cdn.example.com/x.jar", &jar_bytes()),
        )
        .expect("write pack");

        // A blank pack path never reaches the filesystem.
        let blank = plan_modpack_install(ModpackInstallRequest {
            pack_path: "   ".to_string(),
            instance_root: root.to_string_lossy().into_owned(),
        })
        .await;
        assert_eq!(
            blank.expect_err("a blank pack path must be refused").code,
            ErrorCode::InvalidInput
        );

        // The pack half of the pair is resolved first; a missing file is the
        // user's pick, not a failing disk.
        let missing_pack =
            plan_modpack_install(request(&dir.path().join("gone.mrpack"), &root)).await;
        assert_eq!(
            missing_pack
                .expect_err("a missing pack must be refused")
                .code,
            ErrorCode::NotFound
        );

        // And with a real pack, a missing instance folder is refused too, with
        // nothing created on its behalf.
        let missing_root = dir.path().join("no-such-instance");
        let out = plan_modpack_install(request(&pack_path, &missing_root)).await;
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
