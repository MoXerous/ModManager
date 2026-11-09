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
    clear_interrupted_parts, dry_run, interrupted_parts, phase, AppliedSource, ExecError,
    PackEvent, PackIdentity, PlanExecutor, PlannedWriteKind,
};
use super::mrpack::{Pack, PackFile, Side};
use crate::downloader::client::DownloadError;
use crate::downloader::Downloader;
use crate::journal::{
    pack_item, ItemState, Journal, JournalError, JournalOp, OperationItem, OperationKind,
    OperationRecord, OperationState,
};
use crate::providers::ModrinthClient;

/// A journal for one test, and the callback the executor wants.
///
/// The callback is the production shape: a write per journal call, with the
/// handle opened and closed per write. The operation id is remembered from
/// the start call, so the test does not have to go looking for it.
struct TestJournal {
    journal: Journal,
    _holder: tempfile::TempDir,
}

impl TestJournal {
    fn new() -> Self {
        let holder = crate::test_tempdir();
        let journal = Journal::new(holder.path().join("operations"));
        Self {
            journal,
            _holder: holder,
        }
    }

    fn writer(&self) -> impl FnMut(JournalOp) -> Result<OperationRecord, JournalError> + '_ {
        let mut id: Option<String> = None;
        move |op| {
            let current = || {
                id.clone().ok_or_else(|| {
                    JournalError::ItemMissing("the apply wrote no operation yet".into())
                })
            };
            match op {
                JournalOp::Start { target, items } => {
                    let r = self
                        .journal
                        .start(OperationKind::DownloadBatch, "", &target, items)?;
                    id = Some(r.operation_id.clone());
                    Ok(r)
                }
                JournalOp::PackStart {
                    target,
                    items,
                    pack,
                } => {
                    let r = self.journal.start_pack("", &target, items, pack)?;
                    id = Some(r.operation_id.clone());
                    Ok(r)
                }
                JournalOp::Operation(state) => {
                    self.journal.transition_operation(&current()?, state, None)
                }
                JournalOp::Item { item_id, state } => {
                    self.journal
                        .transition_item(&current()?, &item_id, state, None)
                }
                JournalOp::Items { transitions } => {
                    self.journal
                        .transition_items(&current()?, &transitions, None)
                }
            }
        }
    }

    /// The one record the journal holds.
    fn record(&self) -> OperationRecord {
        self.journal
            .discover()
            .records()
            .next()
            .cloned()
            .expect("the apply must be journalled")
    }
}

/// The pack a test apply is about.
fn identity() -> PackIdentity {
    PackIdentity {
        name: "Test pack".to_string(),
        version_id: "ver".to_string(),
        archive_sha512: "ab".repeat(64),
        archive_path: "packs/test.mrpack".to_string(),
    }
}

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
            source_layer: "overrides".to_string(),
        }],
    };
    let downloader = Downloader::new(1).expect("downloader");
    let provider = client("http://127.0.0.1:9");
    let executor = PlanExecutor::new(
        &provider,
        &downloader,
        &pack,
        &plan,
        &staging,
        &instance,
        identity(),
    );

    let tj = TestJournal::new();
    let mut write = tj.writer();
    let report = executor
        .apply(&mut write, &mut |_| {})
        .await
        .expect("applies");
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
async fn mrpack_override_is_copied_from_the_layer_the_plan_names() {
    let dir = crate::test_tempdir();
    let staging = dir.path().join("staging");
    let instance = dir.path().join("instance");
    write_fixture(&staging, "client-overrides/config/a.toml", b"client");
    write_fixture(&staging, "overrides/config/a.toml", b"main");

    let pack = pack_of(
        &["client-overrides", "overrides", "server-overrides"],
        vec![],
    );
    // The planner already picked the winner; the executor must not pick again
    // by "last layer that has the file" and accidentally copy a different one.
    let plan = InstallPlan {
        ops: vec![InstallOp::CopyOverride {
            safe_path: "config/a.toml".to_string(),
            source_layer: "overrides".to_string(),
        }],
    };
    let downloader = Downloader::new(1).expect("downloader");
    let provider = client("http://127.0.0.1:9");
    let executor = PlanExecutor::new(
        &provider,
        &downloader,
        &pack,
        &plan,
        &staging,
        &instance,
        identity(),
    );

    let tj = TestJournal::new();
    let mut write = tj.writer();
    let report = executor
        .apply(&mut write, &mut |_| {})
        .await
        .expect("applies");
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
            source_layer: "overrides".to_string(),
        }],
    };
    let downloader = Downloader::new(1).expect("downloader");
    let provider = client("http://127.0.0.1:9");
    let executor = PlanExecutor::new(
        &provider,
        &downloader,
        &pack,
        &plan,
        &staging,
        &instance,
        identity(),
    );

    let tj = TestJournal::new();
    let mut write = tj.writer();
    let error = executor
        .apply(&mut write, &mut |_| {})
        .await
        .expect_err("must refuse");
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
    let executor = PlanExecutor::new(
        &provider,
        &downloader,
        &pack,
        &plan,
        &staging,
        &instance,
        identity(),
    )
    .allow_insecure_loopback(true);

    let mut events: Vec<PackEvent> = Vec::new();
    let tj = TestJournal::new();
    let mut write = tj.writer();
    let report = executor
        .apply(&mut write, &mut |e| events.push(e.clone()))
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
    let executor = PlanExecutor::new(
        &provider,
        &downloader,
        &pack,
        &plan,
        &staging,
        &instance,
        identity(),
    )
    .allow_insecure_loopback(true);

    let tj = TestJournal::new();
    let mut write = tj.writer();
    let error = executor
        .apply(&mut write, &mut |_| {})
        .await
        .expect_err("must refuse");
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
    assert!(
        !instance.join(".mm-staging").exists(),
        "a failed apply must remove its staging directory too"
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
            source_layer: "overrides".to_string(),
        }],
    };
    let downloader = Downloader::new(1).expect("downloader");
    let provider = client("http://127.0.0.1:9");
    let executor = PlanExecutor::new(
        &provider,
        &downloader,
        &pack,
        &plan,
        &staging,
        &instance,
        identity(),
    );

    let mut events: Vec<PackEvent> = Vec::new();
    let tj = TestJournal::new();
    let mut write = tj.writer();
    let error = executor
        .apply(&mut write, &mut |e| events.push(e.clone()))
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
                source_layer: "overrides".to_string(),
            },
        ],
    };
    let downloader = Downloader::new(1).expect("downloader");
    let provider = client("http://127.0.0.1:9");
    let executor = PlanExecutor::new(
        &provider,
        &downloader,
        &pack,
        &plan,
        &staging,
        &instance,
        identity(),
    );

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
    assert!(!writes[0].overwrites, "nothing exists at the destination");
    assert_eq!(writes[1].safe_path, "config/a.toml");
    assert_eq!(writes[1].kind, PlannedWriteKind::Override);
    assert!(!writes[1].overwrites, "nothing exists at the destination");

    // The free function produces the same list, and nothing touched the disk.
    assert_eq!(dry_run(&plan, &instance).expect("dry run"), writes);
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
            source_layer: "overrides".to_string(),
        }],
    };
    assert!(matches!(
        dry_run(&plan, Path::new("unused-instance")),
        Err(ExecError::UnsafePath { .. })
    ));
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
                source_layer: "overrides".to_string(),
            },
            InstallOp::CopyOverride {
                safe_path: "b.txt".to_string(),
                source_layer: "overrides".to_string(),
            },
        ],
    };
    let downloader = Downloader::new(1).expect("downloader");
    let provider = client("http://127.0.0.1:9");
    let executor = PlanExecutor::new(
        &provider,
        &downloader,
        &pack,
        &plan,
        &staging,
        &instance,
        identity(),
    );

    let mut events: Vec<PackEvent> = Vec::new();
    let tj = TestJournal::new();
    let mut write = tj.writer();
    executor
        .apply(&mut write, &mut |e| events.push(e.clone()))
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

#[test]
fn mrpack_dry_run_marks_destinations_that_will_be_overwritten() {
    let dir = crate::test_tempdir();
    let staging = dir.path().join("staging");
    std::fs::create_dir_all(&staging).expect("staging");
    let instance = dir.path().join("instance");
    std::fs::create_dir_all(instance.join("config")).expect("config");
    std::fs::write(instance.join("config/a.toml"), b"old").expect("seed");

    let pack = pack_of(&["overrides"], vec![]);
    let plan = InstallPlan {
        ops: vec![
            InstallOp::CopyOverride {
                safe_path: "config/a.toml".to_string(),
                source_layer: "overrides".to_string(),
            },
            InstallOp::CopyOverride {
                safe_path: "config/new.toml".to_string(),
                source_layer: "overrides".to_string(),
            },
        ],
    };
    let downloader = Downloader::new(1).expect("downloader");
    let provider = client("http://127.0.0.1:9");
    let executor = PlanExecutor::new(
        &provider,
        &downloader,
        &pack,
        &plan,
        &staging,
        &instance,
        identity(),
    );

    let writes = executor.dry_run().expect("dry run");
    assert!(writes[0].overwrites, "an existing destination is replaced");
    assert!(!writes[1].overwrites, "a missing destination is created");
    assert_eq!(
        std::fs::read(instance.join("config/a.toml")).expect("read"),
        b"old",
        "a dry run must not replace the file it reports"
    );
    assert!(!instance.join("config/new.toml").exists());
}

#[tokio::test]
async fn mrpack_a_stale_staging_directory_is_removed_before_apply() {
    let dir = crate::test_tempdir();
    let staging = dir.path().join("staging");
    write_fixture(&staging, "overrides/a.txt", b"a");
    let instance = dir.path().join("instance");
    std::fs::create_dir_all(instance.join(".mm-staging")).expect("stale staging");
    std::fs::write(instance.join(".mm-staging/leftover.part"), b"stale").expect("leftover");

    let pack = pack_of(&["overrides"], vec![]);
    let plan = InstallPlan {
        ops: vec![InstallOp::CopyOverride {
            safe_path: "a.txt".to_string(),
            source_layer: "overrides".to_string(),
        }],
    };
    let downloader = Downloader::new(1).expect("downloader");
    let provider = client("http://127.0.0.1:9");
    let executor = PlanExecutor::new(
        &provider,
        &downloader,
        &pack,
        &plan,
        &staging,
        &instance,
        identity(),
    );

    let tj = TestJournal::new();
    let mut write = tj.writer();
    executor
        .apply(&mut write, &mut |_| {})
        .await
        .expect("applies");

    assert_eq!(std::fs::read(instance.join("a.txt")).expect("read"), b"a");
    assert!(
        !instance.join(".mm-staging").exists(),
        "a killed run's scratch directory must not survive the next apply"
    );
}

#[tokio::test]
async fn mrpack_overrides_extracted_into_the_staging_tree_survive_until_they_are_copied() {
    let dir = crate::test_tempdir();
    let instance = dir.path().join("instance");
    // The caller is allowed to extract the pack into the same scratch tree the
    // downloads use; start-of-apply cleanup must not eat the overrides.
    let staging = instance.join(".mm-staging");
    write_fixture(&staging, "overrides/a.txt", b"a");

    let pack = pack_of(&["overrides"], vec![]);
    let plan = InstallPlan {
        ops: vec![InstallOp::CopyOverride {
            safe_path: "a.txt".to_string(),
            source_layer: "overrides".to_string(),
        }],
    };
    let downloader = Downloader::new(1).expect("downloader");
    let provider = client("http://127.0.0.1:9");
    let executor = PlanExecutor::new(
        &provider,
        &downloader,
        &pack,
        &plan,
        &staging,
        &instance,
        identity(),
    );

    let tj = TestJournal::new();
    let mut write = tj.writer();
    executor
        .apply(&mut write, &mut |_| {})
        .await
        .expect("applies");

    assert_eq!(std::fs::read(instance.join("a.txt")).expect("read"), b"a");
    assert!(
        !instance.join(".mm-staging").exists(),
        "the extraction tree is removed once the apply finishes"
    );
}

#[tokio::test]
async fn mrpack_a_plan_writing_an_app_managed_file_is_refused() {
    let dir = crate::test_tempdir();
    let staging = dir.path().join("staging");
    write_fixture(&staging, "overrides/profile.json", b"{}");
    let instance = dir.path().join("instance");

    let pack = pack_of(&["overrides"], vec![]);
    let plan = InstallPlan {
        ops: vec![InstallOp::CopyOverride {
            safe_path: "profile.json".to_string(),
            source_layer: "overrides".to_string(),
        }],
    };
    let downloader = Downloader::new(1).expect("downloader");
    let provider = client("http://127.0.0.1:9");
    let executor = PlanExecutor::new(
        &provider,
        &downloader,
        &pack,
        &plan,
        &staging,
        &instance,
        identity(),
    );

    let tj = TestJournal::new();
    let mut write = tj.writer();
    let error = executor
        .apply(&mut write, &mut |_| {})
        .await
        .expect_err("must refuse");
    assert!(
        matches!(error, ExecError::ReservedPath { .. }),
        "unexpected error: {error}"
    );
    assert!(
        !instance.join("profile.json").exists(),
        "the application's own profile must not be replaced by a plan"
    );
}

// --- F-076: the apply is journalled -------------------------------------

/// One override op, the shape the pack tests use.
fn override_op(safe_path: &str) -> InstallPlan {
    InstallPlan {
        ops: vec![InstallOp::CopyOverride {
            safe_path: safe_path.to_string(),
            source_layer: "overrides".to_string(),
        }],
    }
}

#[tokio::test]
async fn spec_f076_the_apply_writes_a_record_that_names_the_pack_and_the_instance() {
    // The whole point of the record: it says which pack was applied, where,
    // and which scratch file is ours. A record without that is a record no
    // later run can resume against.
    let dir = crate::test_tempdir();
    let staging = dir.path().join("staging");
    let instance = dir.path().join("instance");
    write_fixture(&staging, "overrides/config/a.toml", b"hello");

    let pack = pack_of(&["overrides"], vec![]);
    let plan = override_op("config/a.toml");
    let downloader = Downloader::new(1).expect("downloader");
    let provider = client("http://127.0.0.1:9");
    let executor = PlanExecutor::new(
        &provider,
        &downloader,
        &pack,
        &plan,
        &staging,
        &instance,
        identity(),
    );

    let tj = TestJournal::new();
    let mut write = tj.writer();
    executor
        .apply(&mut write, &mut |_| {})
        .await
        .expect("applies");

    let record = tj.record();
    assert_eq!(record.kind, OperationKind::MrpackInstall);
    assert_eq!(record.state, OperationState::Completed);
    let meta = record.pack.as_ref().expect("the record names the pack");
    assert_eq!(meta.pack_name, "Test pack");
    assert_eq!(meta.pack_version_id, "ver");
    assert_eq!(meta.archive_sha512, "ab".repeat(64));
    assert_eq!(record.target_path, instance.to_string_lossy());

    assert_eq!(record.items.len(), 1);
    let item = &record.items[0];
    assert_eq!(item.item_id, "config/a.toml");
    assert_eq!(
        item.final_path,
        instance.join("config/a.toml").to_string_lossy()
    );
    assert_eq!(
        item.partial_path,
        crate::safety::atomic::part_path(&instance.join("config/a.toml")).to_string_lossy(),
        "the record names the scratch file this apply owns"
    );
    assert_eq!(item.state, ItemState::Completed);
    assert_eq!(record.completed_count, 1);

    // And a finished apply is not offered again as an interrupted one.
    assert!(tj.journal.discover().unfinished().next().is_none());
}

/// The crash simulation: the journal callback fails on the Nth item write
/// and the executor propagates the error exactly as a crash would.
fn crashing_writer(
    tj: &TestJournal,
    budget: usize,
) -> impl FnMut(JournalOp) -> Result<OperationRecord, JournalError> + '_ {
    let mut inner = tj.writer();
    let mut spent = 0usize;
    move |op| {
        if let JournalOp::Items { transitions } = &op {
            spent += transitions.len();
            if spent > budget {
                return Err(JournalError::Io("simulated crash".into()));
            }
        }
        inner(op)
    }
}

#[tokio::test]
async fn spec_f076_an_interrupted_apply_leaves_a_record_that_describes_the_instance() {
    // The failure this pins: a crash used to leave half-applied files and
    // nothing that said so. The record is what makes the half-applied
    // instance describable: it names the pack, the instance, every
    // destination, and every scratch file.
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
                source_layer: "overrides".to_string(),
            },
            InstallOp::CopyOverride {
                safe_path: "b.txt".to_string(),
                source_layer: "overrides".to_string(),
            },
        ],
    };
    let downloader = Downloader::new(1).expect("downloader");
    let provider = client("http://127.0.0.1:9");
    let executor = PlanExecutor::new(
        &provider,
        &downloader,
        &pack,
        &plan,
        &staging,
        &instance,
        identity(),
    );

    let tj = TestJournal::new();
    // The first file's chain is three transitions; the crash lands on the
    // second file's write.
    let mut write = crashing_writer(&tj, 3);
    let error = executor
        .apply(&mut write, &mut |_| {})
        .await
        .expect_err("the crash must propagate");
    assert!(matches!(error, ExecError::Journal(_)), "got {error}");

    // The record exists, and it is unfinished: this is what
    // `discover().unfinished()` hands back on the next launch.
    let found = tj.journal.discover();
    assert_eq!(found.issues().count(), 0);
    let unfinished: Vec<&OperationRecord> = found.unfinished().collect();
    assert_eq!(unfinished.len(), 1, "the half-applied apply is described");
    let record = unfinished[0];
    assert_eq!(record.kind, OperationKind::MrpackInstall);
    assert!(record.pack.is_some(), "the record names the pack");
    assert_eq!(record.items.len(), 2);
    for item in &record.items {
        assert!(
            item.partial_path.ends_with(".part"),
            "every item names the scratch file it owns: {}",
            item.partial_path
        );
        assert!(item.final_path.contains("a.txt") || item.final_path.contains("b.txt"));
    }

    // The first file landed before the crash, the second did not: the disk
    // and the record agree about what happened.
    assert_eq!(std::fs::read(instance.join("a.txt")).expect("read"), b"a");
    assert!(!instance.join("b.txt").exists());

    // The record's own account: the first file is `committing` — the part is
    // verified and the rename is what the crash caught — and the second never
    // started. A broken journal is not patched up with a state nobody can
    // vouch for, so nothing was written over the crash.
    let first = record
        .items
        .iter()
        .find(|i| i.item_id == "a.txt")
        .expect("the first item");
    assert_eq!(first.state, ItemState::Committing);
    let second = record
        .items
        .iter()
        .find(|i| i.item_id == "b.txt")
        .expect("the second item");
    assert_eq!(second.state, ItemState::Queued);
    assert_eq!(record.completed_count, 0);
}

#[tokio::test]
async fn spec_f076_the_apply_clears_the_part_an_interrupted_commit_of_the_same_pack_left() {
    // The wedge: `write_part` refuses a `.part` it cannot attribute, so the
    // part an interrupted commit of this pack left behind makes the pack
    // permanently uninstallable — every retry meets the same refusal. The
    // journal is the only proof that the part is ours, so the record is what
    // lets the next run clear it.
    let dir = crate::test_tempdir();
    let staging = dir.path().join("staging");
    let instance = dir.path().join("instance");
    write_fixture(&staging, "overrides/config/a.toml", b"hello");
    std::fs::create_dir_all(instance.join("config")).expect("mkdir");

    let destination = instance.join("config/a.toml");
    let part = crate::safety::atomic::part_path(&destination);
    std::fs::write(&part, b"half-written").expect("leftover");

    let pack = pack_of(&["overrides"], vec![]);
    let plan = override_op("config/a.toml");
    let downloader = Downloader::new(1).expect("downloader");
    let provider = client("http://127.0.0.1:9");
    let executor = PlanExecutor::new(
        &provider,
        &downloader,
        &pack,
        &plan,
        &staging,
        &instance,
        identity(),
    );

    let tj = TestJournal::new();
    // The record a crashed run of the same pack left behind.
    let previous = tj
        .journal
        .start_pack(
            "prof",
            &instance.to_string_lossy(),
            vec![pack_item(
                "config/a.toml",
                "Test pack",
                &part.to_string_lossy(),
                &destination.to_string_lossy(),
            )],
            identity().to_meta(),
        )
        .expect("start");
    tj.journal
        .transition_operation(&previous.operation_id, OperationState::Running, None)
        .expect("running");

    let removed = clear_interrupted_parts(&tj.journal, &identity(), &instance);
    assert_eq!(removed, 1, "the part the record names is ours");
    assert!(!part.exists());

    let mut write = tj.writer();
    executor
        .apply(&mut write, &mut |_| {})
        .await
        .expect("the apply is not wedged");
    assert_eq!(
        std::fs::read(&destination).expect("read"),
        b"hello",
        "the pack installs after the wedge is cleared"
    );
}

#[tokio::test]
async fn spec_f076_a_part_the_journal_does_not_mention_survives_untouched() {
    // Only the part the record names is ours. A part at a path no record
    // mentions is somebody else's file: it stays, and the apply refuses the
    // destination with the honest `StalePart` error instead of destroying
    // it.
    let dir = crate::test_tempdir();
    let staging = dir.path().join("staging");
    let instance = dir.path().join("instance");
    write_fixture(&staging, "overrides/config/a.toml", b"hello");
    std::fs::create_dir_all(instance.join("config")).expect("mkdir");

    let destination = instance.join("config/a.toml");
    let part = crate::safety::atomic::part_path(&destination);
    std::fs::write(&part, b"not ours").expect("foreign");

    let pack = pack_of(&["overrides"], vec![]);
    let plan = override_op("config/a.toml");
    let downloader = Downloader::new(1).expect("downloader");
    let provider = client("http://127.0.0.1:9");
    let executor = PlanExecutor::new(
        &provider,
        &downloader,
        &pack,
        &plan,
        &staging,
        &instance,
        identity(),
    );

    let tj = TestJournal::new();
    // A record of the same pack that names a *different* destination.
    let other = instance.join("config/other.toml");
    let previous = tj
        .journal
        .start_pack(
            "prof",
            &instance.to_string_lossy(),
            vec![pack_item(
                "config/other.toml",
                "Test pack",
                &crate::safety::atomic::part_path(&other).to_string_lossy(),
                &other.to_string_lossy(),
            )],
            identity().to_meta(),
        )
        .expect("start");
    tj.journal
        .transition_operation(&previous.operation_id, OperationState::Running, None)
        .expect("running");

    let removed = clear_interrupted_parts(&tj.journal, &identity(), &instance);
    assert_eq!(removed, 0, "a part no record names is not ours");
    assert!(part.exists(), "the foreign part survives");
    assert_eq!(std::fs::read(&part).expect("read"), b"not ours");

    let mut write = tj.writer();
    let error = executor
        .apply(&mut write, &mut |_| {})
        .await
        .expect_err("must refuse");
    assert!(
        matches!(
            error,
            ExecError::Atomic(crate::safety::atomic::AtomicError::StalePart(_))
        ),
        "got {error}"
    );
    assert_eq!(
        std::fs::read(&part).expect("read"),
        b"not ours",
        "the refused part is not destroyed"
    );
}

#[test]
fn spec_f076_only_the_parts_of_the_same_pack_and_instance_are_ours() {
    // A record of another pack, or of another instance, proves nothing about
    // this destination: its parts are not ours to remove.
    let dir = crate::test_tempdir();
    let instance = dir.path().join("instance");
    let other_instance = dir.path().join("other");
    let j = Journal::new(dir.path().join("operations"));

    let ours = instance.join("a.jar");
    let part = crate::safety::atomic::part_path(&ours);
    std::fs::create_dir_all(&instance).expect("mkdir");
    std::fs::write(&part, b"ours").expect("write");

    // A record of a DIFFERENT pack, same instance.
    let other_pack = PackIdentity {
        archive_sha512: "cd".repeat(64),
        ..identity()
    };
    let rec = j
        .start_pack(
            "prof",
            &instance.to_string_lossy(),
            vec![pack_item(
                "a.jar",
                "Test pack",
                &part.to_string_lossy(),
                &ours.to_string_lossy(),
            )],
            other_pack.to_meta(),
        )
        .expect("start");
    j.transition_operation(&rec.operation_id, OperationState::Running, None)
        .expect("running");
    assert!(
        interrupted_parts(&j, &identity(), &instance).is_empty(),
        "another pack's part is not ours"
    );

    // A record of the SAME pack, another instance.
    let rec = j
        .start_pack(
            "prof",
            &other_instance.to_string_lossy(),
            vec![pack_item(
                "a.jar",
                "Test pack",
                &part.to_string_lossy(),
                &ours.to_string_lossy(),
            )],
            identity().to_meta(),
        )
        .expect("start");
    j.transition_operation(&rec.operation_id, OperationState::Running, None)
        .expect("running");
    assert!(
        interrupted_parts(&j, &identity(), &instance).is_empty(),
        "another instance's part is not ours"
    );

    // The same pack, the same instance: ours.
    let rec = j
        .start_pack(
            "prof",
            &instance.to_string_lossy(),
            vec![pack_item(
                "a.jar",
                "Test pack",
                &part.to_string_lossy(),
                &ours.to_string_lossy(),
            )],
            identity().to_meta(),
        )
        .expect("start");
    j.transition_operation(&rec.operation_id, OperationState::Running, None)
        .expect("running");
    assert_eq!(
        interrupted_parts(&j, &identity(), &instance),
        vec![part.clone()],
        "the same pack in the same instance: ours"
    );
    assert_eq!(clear_interrupted_parts(&j, &identity(), &instance), 1);
    assert!(!part.exists());
}

/// The journal-write-volume measurement (F-076).
///
/// A record is one document: every write rewrites the whole file, and the
/// file grows with the number of items. A pack apply that moved four states
/// per file would rewrite the record four times per file, so the bytes
/// written grow with the *square* of the file count. This measures both
/// paths on the same record so the two can be compared.
struct Volume {
    unbatched_writes: usize,
    unbatched_bytes: u64,
    batched_writes: usize,
    batched_bytes: u64,
}

fn record_size(j: &Journal, id: &str) -> u64 {
    std::fs::metadata(j.root().join(format!("{id}.json")))
        .map(|m| m.len())
        .unwrap_or(0)
}

fn measure(files: usize) -> Volume {
    let d = crate::test_tempdir();
    let j = Journal::new(d.path().join("operations"));
    let items: Vec<OperationItem> = (0..files)
        .map(|i| {
            pack_item(
                &format!("mods/m{i}.jar"),
                "Pack",
                &format!("/inst/mods/m{i}.jar.part"),
                &format!("/inst/mods/m{i}.jar"),
            )
        })
        .collect();

    // The unbatched path: every transition is a write of its own.
    let rec = j
        .start_pack(
            "p",
            "/inst",
            items.clone(),
            crate::journal::PackApplyMeta {
                pack_name: "Pack".to_string(),
                pack_version_id: "ver".to_string(),
                archive_sha512: "ab".repeat(64),
                archive_path: "packs/test.mrpack".to_string(),
            },
        )
        .expect("start");
    let mut unbatched_writes = 0usize;
    let mut unbatched_bytes = 0u64;
    for item in &items {
        for state in [
            ItemState::Downloading,
            ItemState::Verifying,
            ItemState::Committing,
            ItemState::Completed,
        ] {
            j.transition_item(&rec.operation_id, &item.item_id, state, None)
                .expect("transition");
            unbatched_writes += 1;
            unbatched_bytes += record_size(&j, &rec.operation_id);
        }
    }

    // The batched path: one write per file, carrying the file's chain and
    // the previous file's ending — what the executor does.
    let rec = j
        .start_pack(
            "p",
            "/inst",
            items.clone(),
            crate::journal::PackApplyMeta {
                pack_name: "Pack".to_string(),
                pack_version_id: "ver".to_string(),
                archive_sha512: "ab".repeat(64),
                archive_path: "packs/test.mrpack".to_string(),
            },
        )
        .expect("start");
    let mut batched_writes = 0usize;
    let mut batched_bytes = 0u64;
    for (i, item) in items.iter().enumerate() {
        let mut transitions = Vec::new();
        if i > 0 {
            transitions.push((items[i - 1].item_id.clone(), ItemState::Completed));
        }
        transitions.push((item.item_id.clone(), ItemState::Downloading));
        transitions.push((item.item_id.clone(), ItemState::Verifying));
        transitions.push((item.item_id.clone(), ItemState::Committing));
        j.transition_items(&rec.operation_id, &transitions, None)
            .expect("transitions");
        batched_writes += 1;
        batched_bytes += record_size(&j, &rec.operation_id);
    }
    // The last file's ending, flushed on its own.
    j.transition_items(
        &rec.operation_id,
        &[(items[items.len() - 1].item_id.clone(), ItemState::Completed)],
        None,
    )
    .expect("flush");
    batched_writes += 1;
    batched_bytes += record_size(&j, &rec.operation_id);

    Volume {
        unbatched_writes,
        unbatched_bytes,
        batched_writes,
        batched_bytes,
    }
}

#[test]
fn spec_f076_the_journal_write_volume_is_quadratic_and_the_batched_write_is_the_fix() {
    // Two sizes, so the growth is measured rather than assumed.
    let small = measure(32);
    let large = measure(64);
    println!(
        "journal volume: 32 files -> {} writes / {} bytes unbatched, {} writes / {} bytes batched",
        small.unbatched_writes, small.unbatched_bytes, small.batched_writes, small.batched_bytes
    );
    println!(
        "journal volume: 64 files -> {} writes / {} bytes unbatched, {} writes / {} bytes batched",
        large.unbatched_writes, large.unbatched_bytes, large.batched_writes, large.batched_bytes
    );

    // The unbatched path: four writes per file, each rewriting a record
    // that grows with the file count. Doubling the files more than doubles
    // the bytes — the quadratic signature.
    assert!(
        large.unbatched_bytes > small.unbatched_bytes * 3,
        "the unbatched path is not quadratic: {} -> {}",
        small.unbatched_bytes,
        large.unbatched_bytes
    );

    // The batched path: one write per file.
    assert!(
        large.batched_writes * 3 < large.unbatched_writes,
        "the batched path still writes per transition: {} vs {}",
        large.batched_writes,
        large.unbatched_writes
    );
    assert!(
        large.batched_bytes * 3 < large.unbatched_bytes * 2,
        "the batched path does not cut the volume: {} vs {}",
        large.batched_bytes,
        large.unbatched_bytes
    );
}
