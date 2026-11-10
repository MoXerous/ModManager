//! Export over IPC: the registry gate, the stale-lock refusal, and a pack that
//! reads back. The provider is a map, because the subject is the command, not
//! Modrinth.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::pack::{plan_modpack_export_with, write_modpack_export_with};
use crate::dto::{
    ErrorCode, ModpackExportRequest, ModpackExportSettingsDto, ModpackExportWriteRequest,
};
use crate::state::AppState;
use mm_core::instances::lock::{write_lock, LockEntry, LockFile};
use mm_core::providers::{
    HashAlgo, LatestFilter, Project, ProjectFile, Provider, ProviderError, SearchPage, SearchQuery,
    Version,
};
use mm_core::safety::inspect::ArchiveLimits;

fn hex40(ch: char) -> String {
    (0..40).map(|_| ch).collect()
}

fn hex128(ch: char) -> String {
    (0..128).map(|_| ch).collect()
}

fn file(name: &str, url: &str, size: u64) -> ProjectFile {
    ProjectFile {
        filename: name.to_string(),
        url: url.to_string(),
        size,
        sha1: Some(hex40('a')),
        sha512: Some(hex128('b')),
        primary: true,
    }
}

fn project(id: &str, title: &str, kind: &str) -> Project {
    Project {
        id: id.to_string(),
        slug: Some(id.to_string()),
        title: title.to_string(),
        description: String::new(),
        author: "author".to_string(),
        project_type: kind.to_string(),
        categories: Vec::new(),
        versions: Vec::new(),
        downloads: 0,
        icon_url: None,
    }
}

fn version(id: &str, project_id: &str, files: Vec<ProjectFile>) -> Version {
    Version {
        id: id.to_string(),
        project_id: project_id.to_string(),
        name: format!("{id} name"),
        version_number: "1.0.0".to_string(),
        changelog: None,
        date_published: "2026-01-01T00:00:00Z".to_string(),
        version_type: "release".to_string(),
        status: "listed".to_string(),
        files,
        game_versions: vec!["1.21.1".to_string()],
        loaders: vec!["fabric".to_string()],
        dependencies: Vec::new(),
    }
}

struct FakeProvider {
    projects: HashMap<String, Project>,
    versions: HashMap<String, Version>,
}

fn fake_provider() -> FakeProvider {
    FakeProvider {
        projects: HashMap::from([("p1".to_string(), project("p1", "Sodium", "mod"))]),
        versions: HashMap::from([(
            "v1".to_string(),
            version(
                "v1",
                "p1",
                vec![file("sodium.jar", "https://cdn.modrinth.com/s.jar", 10)],
            ),
        )]),
    }
}

#[async_trait::async_trait]
impl Provider for FakeProvider {
    fn provider_id(&self) -> &'static str {
        "modrinth"
    }

    async fn search(&self, _q: &SearchQuery) -> Result<SearchPage, ProviderError> {
        Err(ProviderError::Unsupported {
            capability: "search",
        })
    }

    async fn get_project(&self, id_or_slug: &str) -> Result<Project, ProviderError> {
        self.projects
            .get(id_or_slug)
            .cloned()
            .ok_or(ProviderError::Api { status: 404 })
    }

    async fn get_version(&self, id: &str) -> Result<Version, ProviderError> {
        self.versions
            .get(id)
            .cloned()
            .ok_or(ProviderError::Api { status: 404 })
    }

    async fn get_versions(
        &self,
        _id_or_slug: &str,
        _game_version: Option<&str>,
        _loader: Option<&str>,
    ) -> Result<Vec<Version>, ProviderError> {
        Err(ProviderError::Unsupported {
            capability: "get_versions",
        })
    }

    async fn get_versions_by_hashes(
        &self,
        _hashes: &[String],
        _algo: HashAlgo,
        _latest: Option<&LatestFilter>,
    ) -> Result<HashMap<String, Version>, ProviderError> {
        Err(ProviderError::Unsupported {
            capability: "get_versions_by_hashes",
        })
    }
}

fn entry(provider: &str, project_id: &str, version_id: &str) -> LockEntry {
    LockEntry {
        provider: provider.to_string(),
        project_id: project_id.to_string(),
        slug: project_id.to_string(),
        version_id: version_id.to_string(),
        filename: format!("{project_id}.jar"),
        sha1: hex40('a'),
        sha512: hex128('b'),
    }
}

fn app_state(dir: &Path, instance_root: &Path) -> AppState {
    let db = mm_core::db::open(&dir.join("registry.db")).expect("open db");
    mm_core::db::instances::insert(
        &db,
        &mm_core::db::instances::InstanceRow {
            id: "i1".to_string(),
            name: "Test Instance".to_string(),
            path: std::fs::canonicalize(instance_root).expect("canonical"),
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

fn settings() -> ModpackExportSettingsDto {
    ModpackExportSettingsDto {
        name: "My Pack".to_string(),
        version_id: "1.0.0".to_string(),
        summary: Some("Summary".to_string()),
        game_version: "1.21.1".to_string(),
        loader: "fabric".to_string(),
        override_folders: vec!["config".to_string()],
    }
}

fn instance_dir(dir: &Path) -> PathBuf {
    let root = dir.join("instance");
    std::fs::create_dir_all(root.join("config")).expect("instance dirs");
    std::fs::write(root.join("config/client.toml"), b"key = true").expect("override");
    root
}

fn seed_lock(root: &Path, entries: Vec<LockEntry>) {
    write_lock(
        &root.join(mm_core::instances::lock::LOCK_FILE_NAME),
        &LockFile { entries },
    )
    .expect("lock");
}

#[tokio::test]
async fn spec_f081_plan_export_over_ipc_returns_the_review_and_writes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = instance_dir(dir.path());
    seed_lock(&root, vec![entry("modrinth", "p1", "v1")]);
    let state = app_state(dir.path(), &root);
    let provider = fake_provider();

    let plan = plan_modpack_export_with(
        &state,
        &provider,
        ModpackExportRequest {
            instance_root: root.to_string_lossy().into_owned(),
            settings: settings(),
        },
    )
    .await
    .expect("plan");

    assert_eq!(plan.name, "My Pack");
    assert_eq!(plan.files.len(), 1);
    assert_eq!(plan.files[0].path, "mods/sodium.jar");
    assert_eq!(plan.override_count, 1);
    assert_eq!(
        plan.overrides[0].archive_path,
        "overrides/config/client.toml"
    );
    assert_eq!(plan.lock_sha512.len(), 128);
    assert!(!dir.path().join("My Pack.mrpack").exists());
}

#[tokio::test]
async fn spec_f081_export_refuses_a_folder_the_registry_does_not_own() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = instance_dir(dir.path());
    seed_lock(&root, vec![entry("modrinth", "p1", "v1")]);
    let state = app_state(dir.path(), &root);
    let provider = fake_provider();
    let str = root.to_string_lossy().into_owned();

    let error = plan_modpack_export_with(
        &state,
        &provider,
        ModpackExportRequest {
            instance_root: str.clone(),
            settings: settings(),
        },
    )
    .await
    .expect("registered");
    assert_eq!(error.files.len(), 1);

    // Same existing folder, different registry: the gate is the row, not the
    // path existing on disk.
    let other = tempfile::tempdir().expect("tempdir");
    let other_root = instance_dir(other.path());
    let other_state = app_state(other.path(), &other_root);
    let error = plan_modpack_export_with(
        &other_state,
        &provider,
        ModpackExportRequest {
            instance_root: str,
            settings: settings(),
        },
    )
    .await
    .expect_err("not our folder");
    assert_eq!(error.code, ErrorCode::InvalidInput);
}

#[tokio::test]
async fn spec_f081_write_export_refuses_a_lock_that_moved_after_the_preview() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = instance_dir(dir.path());
    seed_lock(&root, vec![entry("modrinth", "p1", "v1")]);
    let state = app_state(dir.path(), &root);
    let provider = fake_provider();
    let str = root.to_string_lossy().into_owned();

    let plan = plan_modpack_export_with(
        &state,
        &provider,
        ModpackExportRequest {
            instance_root: str.clone(),
            settings: settings(),
        },
    )
    .await
    .expect("plan");

    // The instance now pins different content than the preview described.
    seed_lock(
        &root,
        vec![entry("modrinth", "p1", "v1"), entry("modrinth", "p2", "v2")],
    );

    let output = dir.path().join("out.mrpack");
    let error = write_modpack_export_with(
        &state,
        &provider,
        ModpackExportWriteRequest {
            instance_root: str,
            settings: settings(),
            expected_lock_sha512: plan.lock_sha512,
            output_path: output.to_string_lossy().into_owned(),
        },
    )
    .await
    .expect_err("stale preview");
    assert_eq!(error.code, ErrorCode::InstanceChanged);
    assert!(!output.exists(), "a refused export writes nothing");
}

#[tokio::test]
async fn spec_f081_write_export_writes_a_pack_that_reads_back() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = instance_dir(dir.path());
    seed_lock(&root, vec![entry("modrinth", "p1", "v1")]);
    let state = app_state(dir.path(), &root);
    let provider = fake_provider();
    let str = root.to_string_lossy().into_owned();

    let plan = plan_modpack_export_with(
        &state,
        &provider,
        ModpackExportRequest {
            instance_root: str.clone(),
            settings: settings(),
        },
    )
    .await
    .expect("plan");

    let output = dir.path().join("out.mrpack");
    let summary = write_modpack_export_with(
        &state,
        &provider,
        ModpackExportWriteRequest {
            instance_root: str,
            settings: settings(),
            expected_lock_sha512: plan.lock_sha512,
            output_path: output.to_string_lossy().into_owned(),
        },
    )
    .await
    .expect("write");

    assert_eq!(summary.output, output.to_string_lossy());
    assert_eq!(summary.file_count, 1);
    assert_eq!(summary.override_count, 1);
    let bytes = std::fs::read(&output).expect("read");
    let pack = mm_core::modpack::archive::read_pack_bytes(&bytes, &ArchiveLimits::default())
        .expect("the written pack reads back");
    assert_eq!(pack.name, "My Pack");
    assert_eq!(pack.files.len(), 1);
    assert_eq!(pack.dependencies["fabric-loader"], "*");
}

#[tokio::test]
async fn spec_f081_export_reports_a_missing_lock_as_its_own_code() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = instance_dir(dir.path());
    // No `seed_lock`: the folder exists and is registered, but nothing is
    // recorded as installed. That is a state, not malformed input.
    let state = app_state(dir.path(), &root);
    let provider = fake_provider();

    let error = plan_modpack_export_with(
        &state,
        &provider,
        ModpackExportRequest {
            instance_root: root.to_string_lossy().into_owned(),
            settings: settings(),
        },
    )
    .await
    .expect_err("no lock");
    assert_eq!(error.code, ErrorCode::NoInstanceLock);
}

#[tokio::test]
async fn spec_f082_export_reports_an_empty_pack_as_its_own_code() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = instance_dir(dir.path());
    // A lock that only names another provider resolves to no files, and no
    // override folder is selected: the pack would contain nothing at all.
    seed_lock(&root, vec![entry("curseforge", "c1", "cv1")]);
    let state = app_state(dir.path(), &root);
    let provider = fake_provider();
    let mut empty_folders = settings();
    empty_folders.override_folders = Vec::new();

    let error = plan_modpack_export_with(
        &state,
        &provider,
        ModpackExportRequest {
            instance_root: root.to_string_lossy().into_owned(),
            settings: empty_folders,
        },
    )
    .await
    .expect_err("an empty pack is not a pack");
    assert_eq!(error.code, ErrorCode::EmptyExport);
}

#[tokio::test]
async fn spec_f082_export_offers_top_level_folders_but_never_mods_logs_or_worlds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    for folder in [
        "config",
        "kubejs",
        "resourcepacks",
        "mods",
        "logs",
        "crash-reports",
        "saves",
        ".mm-staging",
        ".git",
    ] {
        std::fs::create_dir_all(root.join(folder)).expect("folder");
    }
    std::fs::write(root.join("options.txt"), b"fov:0.5").expect("file");

    let folders = super::pack::export_folders_in(root).expect("listing");
    assert_eq!(folders, vec!["config", "kubejs", "resourcepacks"]);
}

#[tokio::test]
async fn spec_f081_export_reports_a_curseforge_lock_entry_as_skipped() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = instance_dir(dir.path());
    seed_lock(
        &root,
        vec![
            entry("modrinth", "p1", "v1"),
            entry("curseforge", "c1", "cv1"),
        ],
    );
    let state = app_state(dir.path(), &root);
    let provider = fake_provider();

    let plan = plan_modpack_export_with(
        &state,
        &provider,
        ModpackExportRequest {
            instance_root: root.to_string_lossy().into_owned(),
            settings: settings(),
        },
    )
    .await
    .expect("plan");
    assert_eq!(plan.files.len(), 1);
    assert_eq!(plan.skipped.len(), 1);
    assert_eq!(plan.skipped[0].project_id, "c1");
    assert_eq!(plan.skipped[0].reason, "unsupported_provider");
}

#[tokio::test]
async fn spec_f081_export_destination_must_be_absolute_and_land_in_an_existing_folder() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = instance_dir(dir.path());
    seed_lock(&root, vec![entry("modrinth", "p1", "v1")]);
    let state = app_state(dir.path(), &root);
    let provider = fake_provider();
    let str = root.to_string_lossy().into_owned();

    let plan = plan_modpack_export_with(
        &state,
        &provider,
        ModpackExportRequest {
            instance_root: str.clone(),
            settings: settings(),
        },
    )
    .await
    .expect("plan");

    let write = |output_path: String| {
        let req = ModpackExportWriteRequest {
            instance_root: str.clone(),
            settings: settings(),
            expected_lock_sha512: plan.lock_sha512.clone(),
            output_path,
        };
        async { write_modpack_export_with(&state, &provider, req).await }
    };

    let error = write("relative.mrpack".to_string())
        .await
        .expect_err("relative");
    assert_eq!(error.code, ErrorCode::InvalidInput);

    let missing = dir.path().join("missing").join("out.mrpack");
    let error = write(missing.to_string_lossy().into_owned())
        .await
        .expect_err("missing parent");
    assert_eq!(error.code, ErrorCode::NotFound);
}
