//! Update preview tests: fixtures on temp dirs, a provider that answers from a
//! map and records every question it was asked.

use std::collections::HashMap;
use std::sync::Mutex;

use super::{
    check_updates, UpdateCandidate, UpdateError, UpdatePreview, UpdateStatus, UpdateWarning,
};
use crate::instances::lock::{write_lock, LockEntry, LockError, LockFile, LOCK_FILE_NAME};
use crate::instances::snapshot_tree;
use crate::providers::{
    HashAlgo, LatestFilter, Project, ProjectFile, Provider, ProviderError, SearchPage, SearchQuery,
    Version,
};

const MODRINTH: &str = crate::providers::modrinth::PROVIDER_ID;
const CURSEFORGE: &str = crate::providers::curseforge::PROVIDER_ID;

/// A canned answer for one hash algorithm.
#[derive(Clone)]
enum Answer {
    Hits(HashMap<String, Version>),
    Unsupported,
}

/// Answers from two canned maps and records the questions verbatim.
struct FakeProvider {
    id: &'static str,
    sha512: Answer,
    sha1: Answer,
    calls: Mutex<Vec<(HashAlgo, Vec<String>)>>,
    filters: Mutex<Vec<LatestFilter>>,
}

impl FakeProvider {
    fn new(sha512: Answer, sha1: Answer) -> Self {
        Self {
            id: MODRINTH,
            sha512,
            sha1,
            calls: Mutex::new(Vec::new()),
            filters: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<(HashAlgo, Vec<String>)> {
        self.calls.lock().expect("calls").clone()
    }

    fn filters(&self) -> Vec<LatestFilter> {
        self.filters.lock().expect("filters").clone()
    }
}

#[async_trait::async_trait]
impl Provider for FakeProvider {
    fn provider_id(&self) -> &'static str {
        self.id
    }

    async fn search(&self, _q: &SearchQuery) -> Result<SearchPage, ProviderError> {
        Err(ProviderError::InvalidJson)
    }

    async fn get_project(&self, _id_or_slug: &str) -> Result<Project, ProviderError> {
        Err(ProviderError::InvalidJson)
    }

    async fn get_version(&self, _id: &str) -> Result<Version, ProviderError> {
        Err(ProviderError::InvalidJson)
    }

    async fn get_versions(
        &self,
        _id_or_slug: &str,
        _game_version: Option<&str>,
        _loader: Option<&str>,
    ) -> Result<Vec<Version>, ProviderError> {
        Err(ProviderError::InvalidJson)
    }

    async fn get_versions_by_hashes(
        &self,
        hashes: &[String],
        algo: HashAlgo,
        latest: Option<&LatestFilter>,
    ) -> Result<HashMap<String, Version>, ProviderError> {
        self.calls
            .lock()
            .expect("calls")
            .push((algo, hashes.to_vec()));
        if let Some(filter) = latest {
            self.filters.lock().expect("filters").push(filter.clone());
        }
        match match algo {
            HashAlgo::Sha1 => &self.sha1,
            HashAlgo::Sha512 => &self.sha512,
        } {
            Answer::Hits(map) => Ok(map.clone()),
            Answer::Unsupported => Err(ProviderError::Unsupported {
                capability: "get_versions_by_hashes",
            }),
        }
    }
}

fn hits(pairs: &[(&str, Version)]) -> Answer {
    Answer::Hits(
        pairs
            .iter()
            .map(|(hash, version)| ((*hash).to_string(), version.clone()))
            .collect(),
    )
}

fn version(id: &str, number: &str, file: &str, sha512: &str, sha1: &str) -> Version {
    let hash = |value: &str| {
        if value.is_empty() {
            None
        } else {
            Some(value.to_string())
        }
    };
    Version {
        id: id.to_string(),
        project_id: "proj".to_string(),
        name: number.to_string(),
        version_number: number.to_string(),
        changelog: None,
        date_published: String::new(),
        version_type: "release".to_string(),
        status: "listed".to_string(),
        files: vec![ProjectFile {
            filename: file.to_string(),
            url: String::new(),
            size: 1,
            sha1: hash(sha1),
            sha512: hash(sha512),
            primary: true,
        }],
        game_versions: vec!["1.21.1".to_string()],
        loaders: vec!["fabric".to_string()],
        dependencies: Vec::new(),
    }
}

fn entry(
    filename: &str,
    provider: &str,
    project_id: &str,
    version_id: &str,
    sha512: &str,
    sha1: &str,
) -> LockEntry {
    LockEntry {
        provider: provider.to_string(),
        project_id: project_id.to_string(),
        slug: String::new(),
        version_id: version_id.to_string(),
        filename: filename.to_string(),
        sha1: sha1.to_string(),
        sha512: sha512.to_string(),
    }
}

fn instance_with(entries: Vec<LockEntry>) -> tempfile::TempDir {
    let dir = crate::test_tempdir();
    std::fs::create_dir_all(dir.path().join("mods")).expect("mods");
    write_lock(&dir.path().join(LOCK_FILE_NAME), &LockFile { entries }).expect("lock");
    dir
}

fn candidate<'a>(preview: &'a UpdatePreview, filename: &str) -> &'a UpdateCandidate {
    preview
        .candidates
        .iter()
        .find(|c| c.filename == filename)
        .unwrap_or_else(|| panic!("no candidate for {filename}: {:?}", preview.candidates))
}

#[tokio::test]
async fn spec_f120_check_updates_maps_available_and_current() {
    let dir = instance_with(vec![
        entry("a.jar", MODRINTH, "p-a", "v1", "aaa", "111"),
        entry("b.jar", MODRINTH, "p-b", "v2", "bbb", "222"),
    ]);
    let provider = FakeProvider::new(
        hits(&[
            ("aaa", version("v9", "9.0", "a-9.jar", "aaa9", "999")),
            ("bbb", version("v2", "2.0", "b-2.jar", "bbb", "222")),
        ]),
        hits(&[]),
    );

    let preview = check_updates(&provider, dir.path(), "1.21.1", "fabric")
        .await
        .expect("preview");

    assert_eq!(preview.checked, 2);
    assert!(preview.warnings.is_empty(), "{:?}", preview.warnings);

    let a = candidate(&preview, "a.jar");
    assert_eq!(a.status, UpdateStatus::Available);
    assert_eq!(a.current_version_id, "v1");
    assert_eq!(a.current_sha512, "aaa");
    assert_eq!(a.latest_version_id.as_deref(), Some("v9"));
    assert_eq!(a.latest_version_number.as_deref(), Some("9.0"));
    assert_eq!(a.latest_filename.as_deref(), Some("a-9.jar"));
    assert_eq!(a.latest_sha512.as_deref(), Some("aaa9"));

    let b = candidate(&preview, "b.jar");
    assert_eq!(
        b.status,
        UpdateStatus::UpToDate,
        "the recorded version id is the latest one"
    );
    assert_eq!(b.latest_version_id.as_deref(), Some("v2"));

    // The provider was asked about the instance's own target, once, by
    // SHA-512.
    let filters = provider.filters();
    assert_eq!(filters.len(), 1);
    assert_eq!(filters[0].loaders, vec!["fabric".to_string()]);
    assert_eq!(filters[0].game_versions, vec!["1.21.1".to_string()]);
    let calls = provider.calls();
    assert_eq!(calls.len(), 1, "no SHA-1 fallback was needed: {calls:?}");
    assert_eq!(calls[0].0, HashAlgo::Sha512);
}

#[tokio::test]
async fn spec_f120_check_updates_reports_unidentified_not_current() {
    let dir = instance_with(vec![entry("c.jar", MODRINTH, "p-c", "v1", "ccc", "ddd")]);
    let provider = FakeProvider::new(hits(&[]), hits(&[]));

    let preview = check_updates(&provider, dir.path(), "1.21.1", "fabric")
        .await
        .expect("preview");

    assert_eq!(preview.checked, 1);
    assert!(
        preview.warnings.is_empty(),
        "an empty answer is not an unsupported provider"
    );
    let c = candidate(&preview, "c.jar");
    assert_eq!(
        c.status,
        UpdateStatus::Unidentified,
        "\"the provider does not know this file\" must not read as \"you are current\""
    );
    assert_eq!(c.latest_version_id, None);

    let calls = provider.calls();
    let algos: Vec<HashAlgo> = calls.iter().map(|(algo, _)| *algo).collect();
    assert_eq!(
        algos,
        vec![HashAlgo::Sha512, HashAlgo::Sha1],
        "the fallback is asked only after SHA-512 came back empty"
    );
}

#[tokio::test]
async fn spec_f120_check_updates_reports_unsupported_provider() {
    let dir = instance_with(vec![
        entry("a.jar", MODRINTH, "p-a", "v1", "aaa", "111"),
        entry("b.jar", MODRINTH, "p-b", "v2", "bbb", "222"),
    ]);
    let provider = FakeProvider::new(
        Answer::Unsupported,
        hits(&[("111", version("v9", "9.0", "a-9.jar", "aaa9", "999"))]),
    );

    let preview = check_updates(&provider, dir.path(), "1.21.1", "fabric")
        .await
        .expect("preview");

    assert_eq!(preview.checked, 2);
    assert_eq!(preview.warnings, vec![UpdateWarning::Unsupported]);
    assert!(
        preview
            .candidates
            .iter()
            .all(|c| c.status == UpdateStatus::Unsupported),
        "{:?}",
        preview.candidates
    );
    assert_eq!(
        provider.calls().len(),
        1,
        "an unsupported provider must not be asked a second time over SHA-1"
    );
}

#[tokio::test]
async fn spec_f120_check_updates_requires_concrete_loader_and_version() {
    let dir = instance_with(vec![entry("a.jar", MODRINTH, "p-a", "v1", "aaa", "111")]);

    for (game_version, loader) in [
        ("1.21.1", ""),
        ("1.21.1", "any"),
        ("1.21.1", " UNKNOWN "),
        ("", "fabric"),
        ("   ", "fabric"),
    ] {
        let provider = FakeProvider::new(hits(&[]), hits(&[]));
        let err = check_updates(&provider, dir.path(), game_version, loader)
            .await
            .expect_err("a check without a concrete target must be refused");
        assert!(
            matches!(err, UpdateError::NeedsConcreteTarget),
            "{game_version:?}/{loader:?} gave {err:?}"
        );
        assert!(
            provider.calls().is_empty(),
            "the provider must not be asked with a target that is not a target"
        );
    }
}

#[tokio::test]
async fn spec_f120_check_updates_writes_nothing() {
    let dir = instance_with(vec![entry("a.jar", MODRINTH, "p-a", "v1", "aaa", "111")]);
    std::fs::write(dir.path().join("mods/a.jar"), b"a jar").expect("jar");
    let before = snapshot_tree(dir.path());

    let provider = FakeProvider::new(
        hits(&[("aaa", version("v9", "9.0", "a-9.jar", "aaa9", "999"))]),
        hits(&[]),
    );
    let preview = check_updates(&provider, dir.path(), "1.21.1", "fabric")
        .await
        .expect("preview");
    assert_eq!(preview.checked, 1);

    let after = snapshot_tree(dir.path());
    assert_eq!(
        before, after,
        "an update check is a read: it must not add, remove or rewrite a file"
    );
}

#[tokio::test]
async fn spec_f120_check_updates_uses_sha512_first() {
    let dir = instance_with(vec![
        entry("a.jar", MODRINTH, "p-a", "v1", "aaa", "111"),
        // An old lock entry: no SHA-512 was recorded when it was installed.
        entry("b.jar", MODRINTH, "p-b", "", "", "222"),
    ]);
    let provider = FakeProvider::new(
        hits(&[]),
        hits(&[
            ("111", version("v9", "9.0", "a-9.jar", "aaa9", "999")),
            ("222", version("v2", "2.0", "b-2.jar", "bbb", "888")),
        ]),
    );

    let preview = check_updates(&provider, dir.path(), "1.21.1", "fabric")
        .await
        .expect("preview");
    assert_eq!(preview.checked, 2);
    assert_eq!(candidate(&preview, "a.jar").status, UpdateStatus::Available);
    assert_eq!(candidate(&preview, "b.jar").status, UpdateStatus::Available);

    let calls = provider.calls();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert_eq!(calls[0].0, HashAlgo::Sha512);
    assert_eq!(calls[0].1, vec!["aaa".to_string()]);
    assert_eq!(calls[1].0, HashAlgo::Sha1);
    assert_eq!(
        calls[1].1,
        vec!["111".to_string(), "222".to_string()],
        "the fallback carries exactly the records SHA-512 did not answer for"
    );
}

#[tokio::test]
async fn spec_f120_check_updates_reports_empty_lock() {
    let dir = crate::test_tempdir();
    std::fs::create_dir_all(dir.path().join("mods")).expect("mods");
    let provider = FakeProvider::new(hits(&[]), hits(&[]));

    let preview = check_updates(&provider, dir.path(), "1.21.1", "fabric")
        .await
        .expect("a missing lock is a normal state");

    assert_eq!(preview.checked, 0);
    assert!(preview.candidates.is_empty());
    assert_eq!(preview.warnings, vec![UpdateWarning::EmptyLock]);
    assert!(provider.calls().is_empty());
}

#[tokio::test]
async fn spec_f120_check_updates_warns_on_unreadable_lock() {
    let dir = crate::test_tempdir();
    std::fs::write(dir.path().join(LOCK_FILE_NAME), b"{ not json").expect("bad lock");
    let provider = FakeProvider::new(hits(&[]), hits(&[]));

    let preview = check_updates(&provider, dir.path(), "1.21.1", "fabric")
        .await
        .expect("an unreadable lock is a warning, not a crash");

    assert_eq!(preview.checked, 0);
    assert_eq!(preview.warnings, vec![UpdateWarning::LockUnreadable]);
    assert!(provider.calls().is_empty());
}

#[tokio::test]
async fn spec_f120_check_updates_refuses_a_newer_lock_schema() {
    let dir = crate::test_tempdir();
    std::fs::write(
        dir.path().join(LOCK_FILE_NAME),
        br#"{"schema_version":99,"entries":[]}"#,
    )
    .expect("newer lock");
    let provider = FakeProvider::new(hits(&[]), hits(&[]));

    let err = check_updates(&provider, dir.path(), "1.21.1", "fabric")
        .await
        .expect_err("a newer lock must be refused, not guessed at");
    assert!(
        matches!(err, UpdateError::Lock(LockError::NewerSchema { .. })),
        "{err:?}"
    );
    assert!(provider.calls().is_empty());
}

#[tokio::test]
async fn spec_f120_check_updates_only_asks_about_this_providers_jars() {
    let dir = instance_with(vec![
        entry("a.jar", MODRINTH, "p-a", "v1", "aaa", "111"),
        entry("b.jar", CURSEFORGE, "p-b", "v2", "bbb", "222"),
        entry("notes.zip", MODRINTH, "p-c", "v3", "ccc", "333"),
    ]);
    let provider = FakeProvider::new(
        hits(&[("aaa", version("v9", "9.0", "a-9.jar", "aaa9", "999"))]),
        hits(&[]),
    );

    let preview = check_updates(&provider, dir.path(), "1.21.1", "fabric")
        .await
        .expect("preview");

    assert_eq!(
        preview.checked, 1,
        "another provider's files and non-jar files are not this check's question"
    );
    assert_eq!(preview.candidates.len(), 1);
    assert_eq!(preview.candidates[0].filename, "a.jar");
    assert_eq!(provider.calls()[0].1, vec!["aaa".to_string()]);
}
