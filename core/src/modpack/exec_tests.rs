//! Execution-engine tests: overrides, downloads, path refusals and dry run.
//!
//! Everything runs in `crate::test_tempdir()` and against wiremock on
//! loopback, exactly like the downloader's own tests. No real instance, no
//! real network.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sha2::Digest as _;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::archive::{InstallOp, InstallPlan};
use super::exec::{
    dry_run, phase, AppliedSource, ExecError, PackEvent, PlanExecutor, PlannedWriteKind,
};
use super::mrpack::{Pack, PackFile, Side};
use crate::downloader::client::DownloadError;
use crate::downloader::Downloader;
use crate::providers::ModrinthClient;

fn sha512_hex(data: &[u8]) -> String {
    let mut hasher = sha2::Sha512::new();
    hasher.update(data);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn pack_of(layers: &[&str], files: Vec<PackFile>) -> Pack {
    Pack {
        format_version: 1,
        version_id: "ver".to_string(),
        name: "Test pack".to_string(),
        summary: None,
        dependencies: BTreeMap::new(),
        files,
        override_layers: layers.iter().map(|s| s.to_string()).collect(),
    }
}

fn pack_file(safe_path: &str, sha512: &str, downloads: Vec<String>, size: Option<u64>) -> PackFile {
    PackFile {
        safe_path: safe_path.to_string(),
        sha1: String::new(),
        sha512: sha512.to_string(),
        downloads,
        file_size: size,
        client: Side::Required,
        server: Side::Required,
    }
}

fn write_fixture(root: &Path, rel: &str, bytes: &[u8]) {
    let target = root.join(rel);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).expect("fixture dir");
    }
    std::fs::write(target, bytes).expect("fixture file");
}

/// Every file below `dir`, so "nothing was left behind" is a claim about the
/// whole tree and not just the destination.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out
}

fn client(server_base: &str) -> ModrinthClient {
    ModrinthClient::with_base(server_base).expect("provider client")
}

/// A `/version_files` response whose single version carries one primary file
/// with the requested hash, served by the mock server.
fn version_map(base: &str, sha512: &str, size: usize) -> serde_json::Value {
    let version = serde_json::json!({
        "id": "ver-1",
        "project_id": "proj-1",
        "name": "Test version",
        "version_number": "1.0.0",
        "changelog": null,
        "date_published": "2026-01-01T00:00:00Z",
        "version_type": "release",
        "status": "listed",
        "files": [{
            "hashes": {"sha1": "", "sha512": sha512},
            "url": format!("{base}/dl/mod.bin"),
            "filename": "mod.bin",
            "primary": true,
            "size": size,
        }],
        "game_versions": ["1.21.1"],
        "loaders": ["fabric"],
        "dependencies": [],
    });
    let mut map = serde_json::Map::new();
    map.insert(sha512.to_string(), version);
    serde_json::Value::Object(map)
}

#[tokio::test]
async fn mrpack_override_is_copied_with_its_bytes() {
    let dir = crate::test_tempdir();
    let staging = dir.path().join("staging");
    let instance = dir.path().join("instance");
    write_fixture(&staging, "overrides/config/a.toml", b"hello");

    let pack = pack_of(&["overrides"], vec![]);
    let plan = InstallPlan {
        ops: vec![InstallOp::CopyOverride {
            safe_path: "config/a.toml".to_string(),
        }],
    };
    let downloader = Downloader::new(1).expect("downloader");
    let provider = client("http://127.0.0.1:9");
    let executor = PlanExecutor::new(&provider, &downloader, &pack, &plan, &staging, &instance);

    let report = executor.apply(&mut |_| {}).await.expect("applies");
    assert_eq!(
        std::fs::read(instance.join("config/a.toml")).expect("read"),
        b"hello"
    );
    assert_eq!(report.written.len(), 1);
    assert_eq!(
        report.written[0].source,
        AppliedSource::Override {
            layer: "overrides".to_string()
        }
    );
    assert!(
        files_under(&instance)
            .iter()
            .all(|p| p.extension().is_none_or(|e| e != "part")),
        "a .part file survived a successful copy"
    );
}

#[tokio::test]
async fn mrpack_later_layer_overwrites_earlier_one() {
    let dir = crate::test_tempdir();
    let staging = dir.path().join("staging");
    let instance = dir.path().join("instance");
    write_fixture(&staging, "client-overrides/config/a.toml", b"client");
    write_fixture(&staging, "overrides/config/a.toml", b"main");

    let pack = pack_of(
        &["client-overrides", "overrides", "server-overrides"],
        vec![],
    );
    // The plan already collapsed the duplicate path: one op, two candidates.
    let plan = InstallPlan {
        ops: vec![InstallOp::CopyOverride {
            safe_path: "config/a.toml".to_string(),
        }],
    };
    let downloader = Downloader::new(1).expect("downloader");
    let provider = client("http://127.0.0.1:9");
    let executor = PlanExecutor::new(&provider, &downloader, &pack, &plan, &staging, &instance);

    let report = executor.apply(&mut |_| {}).await.expect("applies");
    assert_eq!(
        std::fs::read(instance.join("config/a.toml")).expect("read"),
        b"main",
        "the later layer must win"
    );
    assert_eq!(
        report.written[0].source,
        AppliedSource::Override {
            layer: "overrides".to_string()
        }
    );
}

#[tokio::test]
async fn mrpack_missing_override_is_refused() {
    let dir = crate::test_tempdir();
    let staging = dir.path().join("staging");
    let instance = dir.path().join("instance");
    std::fs::create_dir_all(&staging).expect("staging");

    let pack = pack_of(&["overrides"], vec![]);
    let plan = InstallPlan {
        ops: vec![InstallOp::CopyOverride {
            safe_path: "config/gone.toml".to_string(),
        }],
    };
    let downloader = Downloader::new(1).expect("downloader");
    let provider = client("http://127.0.0.1:9");
    let executor = PlanExecutor::new(&provider, &downloader, &pack, &plan, &staging, &instance);

    let error = executor.apply(&mut |_| {}).await.expect_err("must refuse");
    assert!(matches!(error, ExecError::MissingOverride { .. }));
    assert!(!instance.join("config/gone.toml").exists());
}

#[tokio::test]
async fn mrpack_download_lands_hash_verified_from_the_provider() {
    let dir = crate::test_tempdir();
    let staging = dir.path().join("staging");
    std::fs::create_dir_all(&staging).expect("staging");
    let instance = dir.path().join("instance");

    let body = b"config-from-provider";
    let sha512 = sha512_hex(body);
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/version_files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(version_map(
            &server.uri(),
            &sha512,
            body.len(),
        )))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/dl/mod.bin"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body.to_vec()))
        .mount(&server)
        .await;
    // If the pack URL were consulted first this decoy would poison the file;
    // the bytes below prove the provider's URL won.
    Mock::given(method("GET"))
        .and(path("/decoy.bin"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"decoy-bytes".to_vec()))
        .mount(&server)
        .await;

    let pack = pack_of(
        &["overrides"],
        vec![pack_file(
            "config/mod.bin",
            &sha512,
            vec![format!("{}/decoy.bin", server.uri())],
            Some(body.len() as u64),
        )],
    );
    let plan = InstallPlan {
        ops: vec![InstallOp::Download {
            safe_path: "config/mod.bin".to_string(),
            sha512: sha512.clone(),
            size: Some(body.len() as u64),
        }],
    };
    let downloader = Downloader::new(1).expect("downloader");
    let provider = client(&server.uri());
    let executor = PlanExecutor::new(&provider, &downloader, &pack, &plan, &staging, &instance)
        .allow_insecure_loopback(true);

    let mut events: Vec<PackEvent> = Vec::new();
    let report = executor
        .apply(&mut |e| events.push(e.clone()))
        .await
        .expect("downloads");

    assert_eq!(
        std::fs::read(instance.join("config/mod.bin")).expect("read"),
        body
    );
    assert_eq!(report.written.len(), 1);
    assert_eq!(report.written[0].source, AppliedSource::Download);
    assert_eq!(report.bytes, body.len() as u64);
    assert!(events.iter().any(|e| e.phase == phase::DOWNLOADING));
    assert!(
        !instance.join(".mm-staging").exists(),
        "staging must be cleaned after a commit"
    );
}

#[tokio::test]
async fn mrpack_wrong_hash_is_refused_and_leaves_no_file() {
    let dir = crate::test_tempdir();
    let staging = dir.path().join("staging");
    std::fs::create_dir_all(&staging).expect("staging");
    let instance = dir.path().join("instance");

    let expected = b"good-bytes";
    let served = b"evil-bytes";
    assert_eq!(
        expected.len(),
        served.len(),
        "fixture: same size on purpose"
    );
    let sha512 = sha512_hex(expected);

    let server = MockServer::start().await;
    // The provider knows nothing about this hash, so the pack URL is the only
    // candidate and the hash is the only thing that can save the install.
    Mock::given(method("POST"))
        .and(path("/version_files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/bad.bin"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(served.to_vec()))
        .mount(&server)
        .await;

    let pack = pack_of(
        &["overrides"],
        vec![pack_file(
            "config/bad.bin",
            &sha512,
            vec![format!("{}/bad.bin", server.uri())],
            Some(expected.len() as u64),
        )],
    );
    let plan = InstallPlan {
        ops: vec![InstallOp::Download {
            safe_path: "config/bad.bin".to_string(),
            sha512: sha512.clone(),
            size: Some(expected.len() as u64),
        }],
    };
    let downloader = Downloader::new(1).expect("downloader");
    let provider = client(&server.uri());
    let executor = PlanExecutor::new(&provider, &downloader, &pack, &plan, &staging, &instance)
        .allow_insecure_loopback(true);

    let error = executor.apply(&mut |_| {}).await.expect_err("must refuse");
    assert!(
        matches!(error, ExecError::Download(DownloadError::HashMismatch)),
        "unexpected error: {error}"
    );
    assert!(
        !instance.join("config/bad.bin").exists(),
        "a rejected download must not land"
    );
    assert!(
        files_under(&instance).is_empty(),
        "no file or .part may survive a hash rejection"
    );
}

#[tokio::test]
async fn mrpack_path_escaping_the_instance_is_refused() {
    let dir = crate::test_tempdir();
    let staging = dir.path().join("staging");
    let instance = dir.path().join("instance");
    std::fs::create_dir_all(&staging).expect("staging");

    let pack = pack_of(&["overrides"], vec![]);
    let plan = InstallPlan {
        ops: vec![InstallOp::CopyOverride {
            safe_path: "../evil.txt".to_string(),
        }],
    };
    let downloader = Downloader::new(1).expect("downloader");
    let provider = client("http://127.0.0.1:9");
    let executor = PlanExecutor::new(&provider, &downloader, &pack, &plan, &staging, &instance);

    let mut events: Vec<PackEvent> = Vec::new();
    let error = executor
        .apply(&mut |e| events.push(e.clone()))
        .await
        .expect_err("must refuse");
    assert!(matches!(error, ExecError::UnsafePath { .. }));
    assert!(!dir.path().join("evil.txt").exists());
    assert!(
        !instance.exists(),
        "a refused plan must not even create the instance root"
    );
    assert_eq!(
        events.last().map(|e| e.phase),
        Some(phase::FAILED),
        "a refusal still ends with a terminal event"
    );
}

#[test]
fn mrpack_dry_run_writes_nothing() {
    let dir = crate::test_tempdir();
    let staging = dir.path().join("staging");
    let instance = dir.path().join("instance");
    std::fs::create_dir_all(&staging).expect("staging");

    let pack = pack_of(&["overrides"], vec![]);
    let plan = InstallPlan {
        ops: vec![
            InstallOp::Download {
                safe_path: "mods/a.jar".to_string(),
                sha512: "ab".repeat(64),
                size: Some(42),
            },
            InstallOp::CopyOverride {
                safe_path: "config/a.toml".to_string(),
            },
        ],
    };
    let downloader = Downloader::new(1).expect("downloader");
    let provider = client("http://127.0.0.1:9");
    let executor = PlanExecutor::new(&provider, &downloader, &pack, &plan, &staging, &instance);

    let writes = executor.dry_run().expect("dry run");
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0].safe_path, "mods/a.jar");
    assert_eq!(
        writes[0].kind,
        PlannedWriteKind::Download {
            sha512: "ab".repeat(64),
            size: Some(42),
        }
    );
    assert_eq!(writes[1].safe_path, "config/a.toml");
    assert_eq!(writes[1].kind, PlannedWriteKind::Override);

    // The free function produces the same list, and nothing touched the disk.
    assert_eq!(dry_run(&plan).expect("dry run"), writes);
    assert!(!instance.exists(), "a dry run must not create the instance");
    assert!(
        files_under(&staging).is_empty(),
        "a dry run must not touch the staging tree"
    );
}

#[test]
fn mrpack_dry_run_refuses_an_escaping_path() {
    let plan = InstallPlan {
        ops: vec![InstallOp::CopyOverride {
            safe_path: "mods/../../evil.txt".to_string(),
        }],
    };
    assert!(matches!(dry_run(&plan), Err(ExecError::UnsafePath { .. })));
}

#[tokio::test]
async fn mrpack_callback_receives_the_terminal_event_last() {
    let dir = crate::test_tempdir();
    let staging = dir.path().join("staging");
    let instance = dir.path().join("instance");
    write_fixture(&staging, "overrides/a.txt", b"a");
    write_fixture(&staging, "overrides/b.txt", b"b");

    let pack = pack_of(&["overrides"], vec![]);
    let plan = InstallPlan {
        ops: vec![
            InstallOp::CopyOverride {
                safe_path: "a.txt".to_string(),
            },
            InstallOp::CopyOverride {
                safe_path: "b.txt".to_string(),
            },
        ],
    };
    let downloader = Downloader::new(1).expect("downloader");
    let provider = client("http://127.0.0.1:9");
    let executor = PlanExecutor::new(&provider, &downloader, &pack, &plan, &staging, &instance);

    let mut events: Vec<PackEvent> = Vec::new();
    executor
        .apply(&mut |e| events.push(e.clone()))
        .await
        .expect("applies");

    assert!(
        events.len() >= 5,
        "expected copying/written pairs plus a terminal event, got {events:?}"
    );
    assert_eq!(
        events.last().map(|e| e.phase),
        Some(phase::DONE),
        "the terminal event must be last"
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.phase == phase::DONE || e.phase == phase::FAILED)
            .count(),
        1,
        "exactly one terminal event"
    );
}
