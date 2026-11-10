//! Export tests (F-081, F-082, F-156). A fake provider over a temp instance,
//! so the whole path — lock → resolved index → zip → read back — is exercised
//! without a network, and every refusal is a rule a hostile instance can hit.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use super::archive::{plan_install, read_pack, InstallOp};
use super::export::{
    collect_overrides, exclusion_reason, export_mod_from, folder_for_project_type, index_json,
    loader_key, plan_export, resolve_mods, write_export_zip, Exclusion, ExportError, ExportMod,
    ExportPlan, ExportSettings, OverrideFile, SkipReason,
};
use crate::instances::lock::{write_lock, LockEntry, LockFile};
use crate::providers::{
    HashAlgo, LatestFilter, Project, ProjectFile, Provider, ProviderError, SearchPage, SearchQuery,
    Version,
};
use crate::safety::inspect::ArchiveLimits;

// --- fixtures --------------------------------------------------------------

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

fn fake_provider(projects: Vec<Project>, versions: Vec<Version>) -> FakeProvider {
    FakeProvider {
        projects: projects.into_iter().map(|p| (p.id.clone(), p)).collect(),
        versions: versions.into_iter().map(|v| (v.id.clone(), v)).collect(),
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

fn settings(folders: &[&str]) -> ExportSettings {
    ExportSettings {
        name: "My Pack".to_string(),
        version_id: "1.0.0".to_string(),
        summary: Some("A pack".to_string()),
        game_version: "1.21.1".to_string(),
        loader: "fabric".to_string(),
        override_folders: folders.iter().map(|f| f.to_string()).collect(),
    }
}

fn write_file(root: &Path, rel: &str, bytes: &[u8]) {
    let target = root.join(rel);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).expect("fixture dir");
    }
    std::fs::write(target, bytes).expect("fixture file");
}

/// Every path and size under `root`, for "planning writes nothing" checks.
fn tree(root: &Path) -> Vec<(String, u64)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, u64)>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .expect("read_dir")
            .filter_map(|e| e.ok())
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .expect("inside root")
                .to_string_lossy()
                .replace('\\', "/");
            let meta = std::fs::symlink_metadata(&path).expect("metadata");
            if meta.is_dir() {
                out.push((format!("{rel}/"), 0));
                walk(root, &path, out);
            } else {
                out.push((rel, meta.len()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out
}

fn export_mod(name: &str) -> ExportMod {
    ExportMod {
        project_id: "p1".to_string(),
        version_id: "v1".to_string(),
        title: "Sodium".to_string(),
        project_type: "mod".to_string(),
        folder: "mods",
        file_name: name.to_string(),
        url: format!("https://cdn.modrinth.com/data/p1/versions/v1/{name}"),
        size: 1234,
        sha1: hex40('a'),
        sha512: hex128('b'),
    }
}

// --- F-081: one resolved project becomes one index entry -------------------

#[test]
fn spec_f081_export_entry_carries_the_providers_file_and_hashes() {
    let mut seen = BTreeSet::new();
    let module = export_mod_from(
        &project("p1", "Sodium", "mod"),
        &version(
            "v1",
            "p1",
            vec![file(
                "sodium.jar",
                "https://cdn.modrinth.com/data/p1/versions/v1/sodium.jar",
                1234,
            )],
        ),
        &mut seen,
    )
    .expect("a valid entry");

    assert_eq!(module.destination(), "mods/sodium.jar");
    assert_eq!(module.project_id, "p1");
    assert_eq!(module.version_id, "v1");
    assert_eq!(module.title, "Sodium");
    assert_eq!(module.size, 1234);
    // Hashes are compared case-insensitively but written lowercase.
    assert_eq!(module.sha1, hex40('a'));
    assert_eq!(module.sha512, hex128('b'));
}

#[test]
fn spec_f081_export_prefers_the_primary_file_over_the_first() {
    let mut first = file("decoy.jar", "https://cdn.modrinth.com/decoy.jar", 10);
    first.primary = false;
    let mut seen = BTreeSet::new();
    let module = export_mod_from(
        &project("p1", "Sodium", "mod"),
        &version(
            "v1",
            "p1",
            vec![
                first,
                file("real.jar", "https://cdn.modrinth.com/real.jar", 20),
            ],
        ),
        &mut seen,
    )
    .expect("a valid entry");
    assert_eq!(module.file_name, "real.jar");
}

#[test]
fn spec_f081_export_skips_a_version_with_no_file() {
    let mut seen = BTreeSet::new();
    assert_eq!(
        export_mod_from(
            &project("p1", "Sodium", "mod"),
            &version("v1", "p1", vec![]),
            &mut seen
        ),
        Err(SkipReason::NoFile)
    );
}

#[test]
fn spec_f081_export_skips_a_non_https_url() {
    for url in [
        "http://cdn.modrinth.com/x.jar",
        "https:///x.jar",
        "https://cdn.modrinth.com /x.jar",
        "cdn.modrinth.com/x.jar",
    ] {
        let mut seen = BTreeSet::new();
        assert_eq!(
            export_mod_from(
                &project("p1", "Sodium", "mod"),
                &version("v1", "p1", vec![file("sodium.jar", url, 10)]),
                &mut seen
            ),
            Err(SkipReason::InvalidUrl),
            "{url}"
        );
    }
}

#[test]
fn spec_f081_export_skips_a_zero_size_file() {
    // The format requires a positive `fileSize`; a zero would make the
    // installer's size check meaningless.
    let mut seen = BTreeSet::new();
    assert_eq!(
        export_mod_from(
            &project("p1", "Sodium", "mod"),
            &version(
                "v1",
                "p1",
                vec![file("sodium.jar", "https://cdn.modrinth.com/x.jar", 0)]
            ),
            &mut seen
        ),
        Err(SkipReason::MissingSize)
    );
}

#[test]
fn spec_f081_export_skips_missing_or_malformed_hashes() {
    let cases: Vec<(Option<String>, Option<String>)> = vec![
        (None, Some(hex128('b'))),
        (Some(hex40('a')), None),
        (Some("abc".to_string()), Some(hex128('b'))),
        (Some(hex40('a')), Some("xyz".to_string())),
        (Some("g".repeat(40)), Some(hex128('b'))),
    ];
    for (sha1, sha512) in cases {
        let mut f = file("sodium.jar", "https://cdn.modrinth.com/x.jar", 10);
        f.sha1 = sha1.clone();
        f.sha512 = sha512.clone();
        let mut seen = BTreeSet::new();
        assert_eq!(
            export_mod_from(
                &project("p1", "Sodium", "mod"),
                &version("v1", "p1", vec![f]),
                &mut seen
            ),
            Err(SkipReason::InvalidHashes),
            "{sha1:?} {sha512:?}"
        );
    }
}

#[test]
fn spec_f081_export_skips_a_project_type_with_no_instance_folder() {
    let mut seen = BTreeSet::new();
    assert_eq!(
        export_mod_from(
            &project("p1", "A pack", "modpack"),
            &version(
                "v1",
                "p1",
                vec![file("pack.mrpack", "https://cdn.modrinth.com/p.mrpack", 10)]
            ),
            &mut seen
        ),
        Err(SkipReason::UnsupportedType)
    );
}

#[test]
fn spec_f081_export_skips_a_file_name_a_zip_reader_would_refuse() {
    for name in ["CON.jar", "with:colon.jar", "trailing."] {
        let mut seen = BTreeSet::new();
        assert_eq!(
            export_mod_from(
                &project("p1", "Sodium", "mod"),
                &version(
                    "v1",
                    "p1",
                    vec![file(name, "https://cdn.modrinth.com/x.jar", 10)]
                ),
                &mut seen
            ),
            Err(SkipReason::UnsafeName),
            "{name}"
        );
    }
}

#[test]
fn spec_f081_export_skips_a_duplicate_destination_case_insensitively() {
    let mut seen = BTreeSet::new();
    export_mod_from(
        &project("p1", "Sodium", "mod"),
        &version(
            "v1",
            "p1",
            vec![file("Sodium.jar", "https://cdn.modrinth.com/x.jar", 10)],
        ),
        &mut seen,
    )
    .expect("the first claim");
    assert_eq!(
        export_mod_from(
            &project("p2", "Sodium again", "mod"),
            &version(
                "v2",
                "p2",
                vec![file("sodium.jar", "https://cdn.modrinth.com/y.jar", 10)]
            ),
            &mut seen
        ),
        Err(SkipReason::DuplicatePath)
    );
}

#[tokio::test]
async fn spec_f081_resolve_mods_reports_every_skip_with_its_reason() {
    let provider = fake_provider(
        vec![
            project("p2", "Known", "mod"),
            project("p4", "No file", "mod"),
        ],
        vec![
            version(
                "v2",
                "p2",
                vec![file("known.jar", "https://cdn.modrinth.com/k.jar", 10)],
            ),
            version("v4", "p4", vec![]),
        ],
    );
    let entries = vec![
        entry("curseforge", "p0", "v0"),
        entry("modrinth", "p1", ""),
        entry("modrinth", "p1", "missing"),
        entry("modrinth", "p3", "v3"),
        entry("modrinth", "p4", "v4"),
        entry("modrinth", "p2", "v2"),
    ];
    let (files, skipped) = resolve_mods(&provider, &entries).await;

    assert_eq!(files.len(), 1, "one file resolved");
    assert_eq!(files[0].file_name, "known.jar");
    let reasons: Vec<SkipReason> = skipped.iter().map(|s| s.reason).collect();
    assert_eq!(
        reasons,
        vec![
            SkipReason::UnsupportedProvider,
            SkipReason::MissingVersionId,
            SkipReason::VersionUnavailable,
            // p3's version is unknown; v3 is absent from the fake provider.
            SkipReason::VersionUnavailable,
            SkipReason::NoFile,
        ]
    );
}

#[tokio::test]
async fn spec_f081_plan_export_reads_the_lock_and_the_selected_folders_only() {
    let dir = crate::test_tempdir();
    let instance = dir.path();
    write_file(instance, "config/client.toml", b"key = true");
    write_file(instance, "config/other.toml", b"x = 1");
    write_file(instance, "mods/user.jar", b"not selected");
    let lock = LockFile {
        entries: vec![
            entry("modrinth", "p1", "v1"),
            entry("curseforge", "c1", "cv1"),
        ],
    };
    write_lock(&instance.join("instance.lock.json"), &lock).expect("lock");

    let provider = fake_provider(
        vec![project("p1", "Sodium", "mod")],
        vec![version(
            "v1",
            "p1",
            vec![file("sodium.jar", "https://cdn.modrinth.com/s.jar", 10)],
        )],
    );
    let plan = plan_export(&provider, instance, &settings(&["config"]))
        .await
        .expect("plan");

    assert_eq!(plan.name, "My Pack");
    assert_eq!(plan.files.len(), 1);
    assert_eq!(plan.overrides.len(), 2);
    assert_eq!(plan.skipped.len(), 1);
    assert_eq!(plan.skipped[0].reason, SkipReason::UnsupportedProvider);
    assert_eq!(plan.lock_sha512.len(), 128);
    assert!(plan.lock_sha512.bytes().all(|b| b.is_ascii_hexdigit()));
}

#[tokio::test]
async fn spec_f081_plan_export_writes_nothing() {
    let dir = crate::test_tempdir();
    let instance = dir.path();
    write_file(instance, "config/client.toml", b"key = true");
    write_lock(
        &instance.join("instance.lock.json"),
        &LockFile {
            entries: vec![entry("modrinth", "p1", "v1")],
        },
    )
    .expect("lock");
    let before = tree(instance);

    let provider = fake_provider(
        vec![project("p1", "Sodium", "mod")],
        vec![version(
            "v1",
            "p1",
            vec![file("sodium.jar", "https://cdn.modrinth.com/s.jar", 10)],
        )],
    );
    plan_export(&provider, instance, &settings(&["config"]))
        .await
        .expect("plan");

    assert_eq!(tree(instance), before, "planning must not create a file");
}

#[tokio::test]
async fn spec_f081_plan_export_refuses_an_instance_without_a_lock() {
    let dir = crate::test_tempdir();
    let provider = fake_provider(vec![], vec![]);
    let error = plan_export(&provider, dir.path(), &settings(&[]))
        .await
        .expect_err("no lock, no pack");
    assert!(matches!(error, ExportError::MissingLock));
}

#[tokio::test]
async fn spec_f082_plan_export_refuses_an_empty_pack() {
    let dir = crate::test_tempdir();
    write_lock(
        &dir.path().join("instance.lock.json"),
        &LockFile { entries: vec![] },
    )
    .expect("lock");
    let provider = fake_provider(vec![], vec![]);
    let error = plan_export(&provider, dir.path(), &settings(&[]))
        .await
        .expect_err("an empty pack is not a pack");
    assert!(matches!(error, ExportError::Empty));
}

#[tokio::test]
async fn spec_f082_overrides_keep_their_instance_paths_inside_the_archive() {
    let dir = crate::test_tempdir();
    let instance = dir.path();
    write_file(instance, "config/sub/client.toml", b"key = true");
    write_lock(
        &instance.join("instance.lock.json"),
        &LockFile {
            entries: vec![entry("modrinth", "p1", "v1")],
        },
    )
    .expect("lock");
    let provider = fake_provider(
        vec![project("p1", "Sodium", "mod")],
        vec![version(
            "v1",
            "p1",
            vec![file("sodium.jar", "https://cdn.modrinth.com/s.jar", 10)],
        )],
    );
    let plan = plan_export(&provider, instance, &settings(&["config"]))
        .await
        .expect("plan");
    assert_eq!(
        plan.overrides[0].archive_path,
        "overrides/config/sub/client.toml"
    );
}

// --- F-156: the exclusion list ---------------------------------------------

#[test]
fn spec_f156_export_excludes_transient_directories_anywhere_in_the_path() {
    assert_eq!(
        exclusion_reason("config/logs", true),
        Some(Exclusion::Transient)
    );
    assert_eq!(
        exclusion_reason("config/crash-reports/latest.txt", false),
        Some(Exclusion::Transient)
    );
    assert_eq!(
        exclusion_reason(".git/objects/ab/cdef", false),
        Some(Exclusion::Transient)
    );
    assert_eq!(
        exclusion_reason("config/.modrinth_bulk_downloader_staging/x.part", false),
        Some(Exclusion::Transient)
    );
}

#[test]
fn spec_f156_export_excludes_dot_env_files_and_their_variants() {
    for name in [".env", ".env.local", ".env.production", "sub/.env.test"] {
        assert_eq!(
            exclusion_reason(&format!("config/{name}"), false),
            Some(Exclusion::Environment),
            "{name}"
        );
    }
}

#[test]
fn spec_f156_export_excludes_every_reference_suffix() {
    // The list is the old exporter's, not an approximation: every entry here
    // must be refused, and a suffix not in it must not be.
    for suffix in [
        ".cer",
        ".crt",
        ".der",
        ".env",
        ".gpg",
        ".jks",
        ".key",
        ".keystore",
        ".log",
        ".p12",
        ".pem",
        ".pfx",
        ".pgp",
    ] {
        assert_eq!(
            exclusion_reason(&format!("config/file{suffix}"), false),
            Some(Exclusion::Secret),
            "{suffix}"
        );
    }
    assert_eq!(exclusion_reason("config/notes.txt", false), None);
}

#[test]
fn spec_f156_export_excludes_secret_like_names() {
    for name in [
        "auth.json",
        "my-account.json",
        "session.dat",
        "client-token.txt",
        "credentials.yml",
        "server-license.txt",
        "private-key.pem",
    ] {
        assert_eq!(
            exclusion_reason(&format!("config/{name}"), false),
            Some(Exclusion::Secret),
            "{name}"
        );
    }
    // A name that merely contains a token as a substring is not a secret.
    assert_eq!(
        exclusion_reason("config/authentication-notes.txt", false),
        None
    );
}

#[test]
fn spec_f156_collect_overrides_keeps_only_the_files_the_policy_allows() {
    let dir = crate::test_tempdir();
    let instance = dir.path();
    write_file(instance, "config/options.toml", b"keep");
    write_file(instance, "config/mod-config.json", b"keep");
    write_file(instance, "config/logs/latest.log", b"drop");
    write_file(instance, "config/.env", b"drop");
    write_file(instance, "config/server.key", b"drop");
    write_file(instance, "config/auth.json", b"drop");

    let (files, excluded) = collect_overrides(instance, &["config".to_string()]).expect("collect");
    let kept: BTreeSet<&str> = files.iter().map(|f| f.archive_path.as_str()).collect();
    assert_eq!(
        kept,
        BTreeSet::from([
            "overrides/config/mod-config.json",
            "overrides/config/options.toml",
        ])
    );
    let reasons: HashMap<&str, Exclusion> = excluded
        .iter()
        .map(|e| (e.path.as_str(), e.reason))
        .collect();
    assert_eq!(reasons["config/logs"], Exclusion::Transient);
    assert_eq!(reasons["config/.env"], Exclusion::Environment);
    assert_eq!(reasons["config/server.key"], Exclusion::Secret);
    assert_eq!(reasons["config/auth.json"], Exclusion::Secret);
}

#[test]
fn spec_f156_collect_overrides_never_follows_a_symlink() {
    let dir = crate::test_tempdir();
    let instance = dir.path();
    write_file(instance, "config/real.toml", b"keep");
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&outside).expect("outside");
    std::fs::write(outside.join("secret.toml"), b"outside").expect("outside file");

    #[cfg(unix)]
    std::os::unix::fs::symlink(
        outside.join("secret.toml"),
        instance.join("config/link.toml"),
    )
    .expect("symlink");
    #[cfg(windows)]
    match std::os::windows::fs::symlink_file(
        outside.join("secret.toml"),
        instance.join("config/link.toml"),
    ) {
        Ok(()) => {}
        Err(_) => return, // privilege not granted on this host
    }

    let (files, excluded) = collect_overrides(instance, &["config".to_string()]).expect("collect");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].archive_path, "overrides/config/real.toml");
    assert!(excluded
        .iter()
        .any(|e| e.path == "config/link.toml" && e.reason == Exclusion::Symlink));
}

#[test]
fn spec_f081_collect_overrides_refuses_an_unsafe_or_missing_folder() {
    let dir = crate::test_tempdir();
    assert!(matches!(
        collect_overrides(dir.path(), &["../outside".to_string()]),
        Err(ExportError::UnsafeFolder(_))
    ));
    assert!(matches!(
        collect_overrides(dir.path(), &["missing".to_string()]),
        Err(ExportError::NotAFolder(_))
    ));
}

// --- index and zip ---------------------------------------------------------

#[test]
fn spec_f081_index_json_names_the_game_the_loader_and_every_file() {
    let plan = ExportPlan {
        name: "My Pack".to_string(),
        version_id: "1.0.0".to_string(),
        summary: Some("Summary".to_string()),
        game_version: "1.21.1".to_string(),
        loader: "fabric".to_string(),
        files: vec![export_mod("sodium.jar")],
        overrides: vec![],
        skipped: vec![],
        excluded: vec![],
        lock_sha512: hex128('a'),
    };
    let index = index_json(&plan);
    assert_eq!(index["formatVersion"], 1);
    assert_eq!(index["game"], "minecraft");
    assert_eq!(index["versionId"], "1.0.0");
    assert_eq!(index["name"], "My Pack");
    assert_eq!(index["summary"], "Summary");
    assert_eq!(index["dependencies"]["minecraft"], "1.21.1");
    assert_eq!(index["dependencies"]["fabric-loader"], "*");
    assert_eq!(index["files"][0]["path"], "mods/sodium.jar");
    assert_eq!(index["files"][0]["hashes"]["sha1"], hex40('a'));
    assert_eq!(index["files"][0]["hashes"]["sha512"], hex128('b'));
    assert_eq!(
        index["files"][0]["downloads"][0],
        export_mod("sodium.jar").url
    );
    assert_eq!(index["files"][0]["fileSize"], 1234);
}

#[test]
fn spec_f081_index_json_never_invents_a_loader() {
    let plan = ExportPlan {
        loader: "rift".to_string(),
        game_version: "1.21.1".to_string(),
        ..ExportPlan::default()
    };
    let index = index_json(&plan);
    assert_eq!(index["dependencies"].as_object().expect("deps").len(), 1);
    assert!(index["dependencies"].get("rift").is_none());
}

#[test]
fn spec_f081_loader_and_folder_tables_match_the_format() {
    assert_eq!(loader_key("fabric"), Some("fabric-loader"));
    assert_eq!(loader_key("Quilt"), Some("quilt-loader"));
    assert_eq!(loader_key("forge"), Some("forge"));
    assert_eq!(loader_key("NeoForge"), Some("neoforge"));
    assert_eq!(loader_key("vanilla"), None);
    assert_eq!(folder_for_project_type("mod"), Some("mods"));
    assert_eq!(
        folder_for_project_type("resourcepack"),
        Some("resourcepacks")
    );
    assert_eq!(folder_for_project_type("shader"), Some("shaderpacks"));
    assert_eq!(folder_for_project_type("datapack"), Some("datapacks"));
    assert_eq!(folder_for_project_type("plugin"), Some("plugins"));
    assert_eq!(folder_for_project_type("modpack"), None);
}

#[test]
fn spec_f081_write_export_zip_round_trips_through_the_pack_reader() {
    let dir = crate::test_tempdir();
    let config = dir.path().join("config/client.toml");
    std::fs::create_dir_all(config.parent().expect("parent")).expect("dir");
    std::fs::write(&config, b"key = true").expect("config");

    let plan = ExportPlan {
        name: "My Pack".to_string(),
        version_id: "1.0.0".to_string(),
        summary: Some("Summary".to_string()),
        game_version: "1.21.1".to_string(),
        loader: "fabric".to_string(),
        files: vec![export_mod("sodium.jar")],
        overrides: vec![OverrideFile {
            archive_path: "overrides/config/client.toml".to_string(),
            source: config,
            size: 10,
        }],
        skipped: vec![],
        excluded: vec![],
        lock_sha512: hex128('a'),
    };
    let output = dir.path().join("out/My Pack.mrpack");
    let summary = write_export_zip(&plan, &output).expect("write");
    assert_eq!(summary.output, output);
    assert_eq!(summary.file_count, 1);
    assert_eq!(summary.override_count, 1);
    assert!(output.is_file());

    let bytes = std::fs::read(&output).expect("read");
    let limits = ArchiveLimits::default();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).expect("zip");
    let (pack, inventory) = read_pack(&mut archive, &limits).expect("the pack reads back");
    assert_eq!(pack.name, "My Pack");
    assert_eq!(pack.version_id, "1.0.0");
    assert_eq!(pack.summary.as_deref(), Some("Summary"));
    assert_eq!(pack.files.len(), 1);
    assert_eq!(pack.files[0].safe_path, "mods/sodium.jar");
    assert!(inventory
        .iter()
        .any(|m| m.name == "overrides/config/client.toml"));

    // The archive this application wrote is one its own installer can plan.
    let install = plan_install(&pack, &inventory).expect("plan the exported pack");
    assert_eq!(install.downloads(), 1);
    assert_eq!(install.overrides(), 1);
    assert!(install.ops.contains(&InstallOp::CopyOverride {
        safe_path: "config/client.toml".to_string(),
        source_layer: "overrides".to_string(),
    }));
}

#[test]
fn spec_f081_write_export_zip_leaves_no_part_file_behind() {
    let dir = crate::test_tempdir();
    let plan = ExportPlan {
        name: "My Pack".to_string(),
        version_id: "1.0.0".to_string(),
        game_version: "1.21.1".to_string(),
        loader: "fabric".to_string(),
        files: vec![export_mod("sodium.jar")],
        lock_sha512: hex128('a'),
        ..ExportPlan::default()
    };
    let output = dir.path().join("My Pack.mrpack");
    write_export_zip(&plan, &output).expect("write");
    let leftovers: Vec<PathBuf> = std::fs::read_dir(dir.path())
        .expect("read_dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".part"))
        })
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn spec_f081_write_export_zip_replaces_an_existing_pack_atomically() {
    let dir = crate::test_tempdir();
    let output = dir.path().join("My Pack.mrpack");
    std::fs::write(&output, b"old content").expect("old file");
    let plan = ExportPlan {
        name: "My Pack".to_string(),
        version_id: "1.0.0".to_string(),
        game_version: "1.21.1".to_string(),
        loader: "fabric".to_string(),
        files: vec![export_mod("sodium.jar")],
        lock_sha512: hex128('a'),
        ..ExportPlan::default()
    };
    write_export_zip(&plan, &output).expect("replace");
    let bytes = std::fs::read(&output).expect("read");
    assert_ne!(bytes, b"old content");
    super::archive::read_pack_bytes(&bytes, &ArchiveLimits::default()).expect("a valid pack");
}

#[test]
fn spec_f081_write_export_zip_refuses_an_empty_plan() {
    let dir = crate::test_tempdir();
    let error = write_export_zip(&ExportPlan::default(), &dir.path().join("empty.mrpack"))
        .expect_err("an empty pack is not a pack");
    assert!(matches!(error, ExportError::Empty));
    assert!(!dir.path().join("empty.mrpack").exists());
}
