//! Target-planner tests (F-125, F-126, F-127, F-131, F-132).
//!
//! Real files on a temp dir, a provider that serves a canned catalogue and
//! records every question. No mocks of the scan or the resolver: the whole
//! point of F-131 is that the two real sets are compared, not a stand-in.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::target::{
    plan_targets, plan_targets_with_cache, TargetError, TargetRequest, TargetStatus, TARGET_COLUMNS,
};
use crate::instances::lock::{write_lock, LockEntry, LockFile, LOCK_FILE_NAME};
use crate::providers::{
    HashAlgo, LatestFilter, Project, ProjectFile, Provider, ProviderError, SearchPage, SearchQuery,
    Version,
};
use crate::resolver::cache::RunCache;

const MODRINTH: &str = crate::providers::modrinth::PROVIDER_ID;

/// (id_or_slug, game_version filter, loader filter)
type VersionsCall = (String, Option<String>, Option<String>);

// --- a provider that answers from a table and records the questions --------

struct FakeProvider {
    projects: HashMap<String, Project>,
    versions: HashMap<String, Vec<Version>>,
    project_calls: Mutex<Vec<String>>,
    versions_calls: Mutex<Vec<VersionsCall>>,
}

impl FakeProvider {
    fn new() -> Self {
        Self {
            projects: HashMap::new(),
            versions: HashMap::new(),
            project_calls: Mutex::new(Vec::new()),
            versions_calls: Mutex::new(Vec::new()),
        }
    }

    fn with(mut self, project: Project, versions: Vec<Version>) -> Self {
        self.versions.insert(project.id.clone(), versions);
        self.projects.insert(project.id.clone(), project);
        self
    }

    fn project_calls(&self) -> usize {
        self.project_calls.lock().expect("calls").len()
    }

    fn versions_calls(&self) -> Vec<VersionsCall> {
        self.versions_calls.lock().expect("calls").clone()
    }
}

#[async_trait::async_trait]
impl Provider for FakeProvider {
    fn provider_id(&self) -> &'static str {
        MODRINTH
    }

    async fn search(&self, _q: &SearchQuery) -> Result<SearchPage, ProviderError> {
        Err(ProviderError::InvalidJson)
    }

    async fn get_project(&self, id_or_slug: &str) -> Result<Project, ProviderError> {
        self.project_calls
            .lock()
            .expect("calls")
            .push(id_or_slug.to_string());
        self.projects
            .get(id_or_slug)
            .cloned()
            .ok_or(ProviderError::InvalidJson)
    }

    async fn get_version(&self, _id: &str) -> Result<Version, ProviderError> {
        Err(ProviderError::InvalidJson)
    }

    async fn get_versions(
        &self,
        id_or_slug: &str,
        game_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Vec<Version>, ProviderError> {
        self.versions_calls.lock().expect("calls").push((
            id_or_slug.to_string(),
            game_version.map(str::to_string),
            loader.map(str::to_string),
        ));
        let all = self
            .versions
            .get(id_or_slug)
            .cloned()
            .ok_or(ProviderError::InvalidJson)?;
        Ok(all
            .into_iter()
            .filter(|version| {
                game_version.is_none_or(|game| version.game_versions.iter().any(|v| v == game))
            })
            .filter(|version| {
                loader.is_none_or(|loader| {
                    version
                        .loaders
                        .iter()
                        .any(|candidate| candidate.eq_ignore_ascii_case(loader))
                })
            })
            .collect())
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

// --- fixtures --------------------------------------------------------------

fn temp_root() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/test-tmp");
    std::fs::create_dir_all(&base).expect("test root");
    tempfile::Builder::new()
        .prefix("mm-target-")
        .tempdir_in(&base)
        .expect("temp dir")
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    mods: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = temp_root();
        let root = dir.path().join("instance");
        let mods = root.join("mods");
        std::fs::create_dir_all(&mods).expect("mods");
        Self {
            _dir: dir,
            root,
            mods,
        }
    }

    fn write(&self, name: &str, bytes: &[u8]) -> String {
        let path = self.mods.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent dir");
        }
        std::fs::write(path, bytes).expect("jar");
        sha1_hex(bytes)
    }

    fn lock(&self, entries: Vec<LockEntry>) {
        write_lock(&self.root.join(LOCK_FILE_NAME), &LockFile { entries }).expect("lock");
    }

    async fn plan(&self, provider: &FakeProvider) -> super::target::TargetPlan {
        plan_targets(provider, &self.root, &TargetRequest::new("1.21", "fabric"))
            .await
            .expect("plan")
    }
}

fn sha1_hex(bytes: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    let mut hasher = Sha1::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn project(id: &str, title: &str) -> Project {
    Project {
        id: id.to_string(),
        slug: Some(id.to_string()),
        title: title.to_string(),
        project_type: "mod".to_string(),
        ..Default::default()
    }
}

fn version(
    project_id: &str,
    id: &str,
    number: &str,
    filename: &str,
    sha1: &str,
    game_versions: &[&str],
    loaders: &[&str],
) -> Version {
    Version {
        id: id.to_string(),
        project_id: project_id.to_string(),
        name: number.to_string(),
        version_number: number.to_string(),
        changelog: None,
        date_published: String::new(),
        version_type: "release".to_string(),
        status: "listed".to_string(),
        files: vec![ProjectFile {
            filename: filename.to_string(),
            url: format!("https://example.invalid/{filename}"),
            size: 42,
            sha1: Some(sha1.to_string()),
            sha512: None,
            primary: true,
        }],
        game_versions: game_versions.iter().map(|v| (*v).to_string()).collect(),
        loaders: loaders.iter().map(|v| (*v).to_string()).collect(),
        dependencies: Vec::new(),
    }
}

fn entry(filename: &str, project_id: &str, version_id: &str, sha1: &str) -> LockEntry {
    LockEntry {
        provider: MODRINTH.to_string(),
        project_id: project_id.to_string(),
        slug: project_id.to_string(),
        version_id: version_id.to_string(),
        filename: filename.to_string(),
        sha1: sha1.to_string(),
        sha512: String::new(),
        game_versions: vec!["1.20.1".to_string()],
    }
}

fn row<'a>(plan: &'a super::target::TargetPlan, project_id: &str) -> &'a super::target::TargetRow {
    plan.rows
        .iter()
        .find(|row| row.project_id == project_id)
        .expect("row for project")
}

// --- F-125: the planner table ---------------------------------------------

#[tokio::test]
async fn spec_f125_rejects_any_as_a_game_version() {
    // `any` is the old app's "no filter" sentinel and `unknown` is this app's
    // "could not detect" one. Neither is a Minecraft version, and a plan
    // against them would have to guess.
    let fixture = Fixture::new();
    let provider = FakeProvider::new();
    for game in ["any", "ANY", "unknown", ""] {
        let error = plan_targets(
            &provider,
            &fixture.root,
            &TargetRequest::new(game, "fabric"),
        )
        .await
        .expect_err("must refuse a non-concrete game version");
        assert!(
            matches!(error, TargetError::NeedsConcreteTarget),
            "game version {game:?} must not be planned against"
        );
    }
}

#[tokio::test]
async fn spec_f125_plans_each_locked_project_against_the_target() {
    // The 8-column contract itself: a rename here is a spec change, not a
    // refactor, so the test names every column.
    assert_eq!(
        TARGET_COLUMNS,
        [
            "title",
            "currentVersion",
            "targetVersion",
            "gameVersion",
            "loader",
            "download",
            "sync",
            "status"
        ]
    );

    let fixture = Fixture::new();
    let alpha_sha = fixture.write("alpha-1.0.jar", b"alpha one");
    let beta_sha = fixture.write("beta-1.0.jar", b"beta one");
    fixture.lock(vec![
        entry("alpha-1.0.jar", "alpha", "alpha-v1", &alpha_sha),
        entry("beta-1.0.jar", "beta", "beta-v1", &beta_sha),
    ]);

    let provider = FakeProvider::new()
        .with(
            project("alpha", "Alpha"),
            vec![
                version(
                    "alpha",
                    "alpha-v1",
                    "1.0",
                    "alpha-1.0.jar",
                    &alpha_sha,
                    &["1.20.1"],
                    &["fabric"],
                ),
                version(
                    "alpha",
                    "alpha-v2",
                    "2.0",
                    "alpha-2.0.jar",
                    "aa22",
                    &["1.21"],
                    &["fabric"],
                ),
            ],
        )
        .with(
            project("beta", "Beta"),
            vec![
                version(
                    "beta",
                    "beta-v1",
                    "1.0",
                    "beta-1.0.jar",
                    &beta_sha,
                    &["1.20.1"],
                    &["fabric"],
                ),
                version(
                    "beta",
                    "beta-v2",
                    "2.0",
                    "beta-2.0.jar",
                    "bb22",
                    &["1.21"],
                    &["fabric"],
                ),
            ],
        );

    let plan = fixture.plan(&provider).await;
    assert_eq!(plan.rows.len(), 2);

    let alpha = row(&plan, "alpha");
    assert_eq!(alpha.title, "Alpha");
    assert_eq!(alpha.current_version, "alpha-v1");
    assert_eq!(alpha.target_version, "2.0");
    assert_eq!(alpha.game_version, "1.21");
    assert_eq!(alpha.loader, "fabric");
    assert!(alpha.download);
    assert!(alpha.sync);
    assert_eq!(alpha.status, TargetStatus::Download);
    assert_eq!(alpha.target_filename, "alpha-2.0.jar");
    assert_eq!(alpha.target_size, Some(42));
    assert!(alpha.file_present);

    let beta = row(&plan, "beta");
    assert_eq!(beta.target_filename, "beta-2.0.jar");
}

#[tokio::test]
async fn spec_f125_reports_up_to_date_when_the_lock_already_has_the_target() {
    let fixture = Fixture::new();
    let sha = fixture.write("alpha-2.0.jar", b"alpha two");
    fixture.lock(vec![entry("alpha-2.0.jar", "alpha", "alpha-v2", &sha)]);

    let provider = FakeProvider::new().with(
        project("alpha", "Alpha"),
        vec![version(
            "alpha",
            "alpha-v2",
            "2.0",
            "alpha-2.0.jar",
            &sha,
            &["1.21"],
            &["fabric"],
        )],
    );

    let plan = fixture.plan(&provider).await;
    let alpha = row(&plan, "alpha");
    assert_eq!(alpha.status, TargetStatus::UpToDate);
    assert!(!alpha.download);
    assert!(!alpha.sync);
    assert_eq!(alpha.reason.code, "sameVersion");
}

#[tokio::test]
async fn spec_f125_syncs_metadata_when_the_target_file_is_already_on_disk() {
    let fixture = Fixture::new();
    let old_sha = fixture.write("alpha-1.0.jar", b"alpha one");
    let target_sha = fixture.write("alpha-2.0.jar", b"alpha two");
    fixture.lock(vec![entry("alpha-1.0.jar", "alpha", "alpha-v1", &old_sha)]);

    let provider = FakeProvider::new().with(
        project("alpha", "Alpha"),
        vec![
            version(
                "alpha",
                "alpha-v1",
                "1.0",
                "alpha-1.0.jar",
                &old_sha,
                &["1.20.1"],
                &["fabric"],
            ),
            version(
                "alpha",
                "alpha-v2",
                "2.0",
                "alpha-2.0.jar",
                &target_sha,
                &["1.21"],
                &["fabric"],
            ),
        ],
    );

    let plan = fixture.plan(&provider).await;
    let alpha = row(&plan, "alpha");
    assert_eq!(alpha.status, TargetStatus::SyncOnly);
    assert!(!alpha.download);
    assert!(alpha.sync);
    assert_eq!(alpha.reason.code, "alreadyOnDisk");
}

#[tokio::test]
async fn spec_f125_a_missing_file_is_planned_not_an_error() {
    let fixture = Fixture::new();
    fixture.lock(vec![entry("beta-1.0.jar", "beta", "beta-v1", "bb11")]);
    let provider = FakeProvider::new().with(
        project("beta", "Beta"),
        vec![version(
            "beta",
            "beta-v2",
            "2.0",
            "beta-2.0.jar",
            "bb22",
            &["1.21"],
            &["fabric"],
        )],
    );

    let plan = fixture.plan(&provider).await;
    let beta = row(&plan, "beta");
    assert!(!beta.file_present);
    assert!(beta.download);
    assert_eq!(beta.status, TargetStatus::Download);
    assert_eq!(
        plan.prune_candidates
            .iter()
            .map(|c| c.filename.as_str())
            .collect::<Vec<_>>(),
        vec!["beta-1.0.jar"]
    );
}

#[tokio::test]
async fn spec_f125_a_foreign_provider_row_is_named_not_judged() {
    // Asking one catalogue about another catalogue's files answers nothing:
    // the row stays visible with its own status and never enters the
    // intersection.
    let fixture = Fixture::new();
    let mut foreign = entry("x.jar", "gamma", "gamma-v1", "");
    foreign.provider = "curseforge".to_string();
    fixture.lock(vec![foreign]);

    let provider = FakeProvider::new();
    let plan = fixture.plan(&provider).await;

    let gamma = row(&plan, "gamma");
    assert_eq!(gamma.status, TargetStatus::OtherProvider);
    assert_eq!(gamma.reason.code, "otherProvider");
    assert!(
        plan.common.is_none(),
        "a foreign row is not a common target"
    );
    assert_eq!(
        provider.project_calls(),
        0,
        "a foreign provider is never asked"
    );
}

// --- F-126: common target --------------------------------------------------

#[tokio::test]
async fn spec_f126_common_target_is_the_loader_verified_intersection() {
    let fixture = Fixture::new();
    let a_sha = fixture.write("a.jar", b"a");
    let b_sha = fixture.write("b.jar", b"b");
    fixture.lock(vec![
        entry("a.jar", "alpha", "a-v1", &a_sha),
        entry("b.jar", "beta", "b-v1", &b_sha),
    ]);

    let provider = FakeProvider::new()
        .with(
            project("alpha", "Alpha"),
            vec![
                version(
                    "alpha",
                    "a-v1",
                    "1.0",
                    "a.jar",
                    &a_sha,
                    &["1.21", "1.20.1", "1.19.4"],
                    &["fabric"],
                ),
                version(
                    "alpha",
                    "a-v0",
                    "0.9",
                    "a-old.jar",
                    "aa00",
                    &["1.20.1", "1.19.4"],
                    &["fabric"],
                ),
            ],
        )
        .with(
            project("beta", "Beta"),
            vec![
                version(
                    "beta",
                    "b-v1",
                    "1.0",
                    "b.jar",
                    &b_sha,
                    &["1.20.1", "1.19.4"],
                    &["fabric"],
                ),
                version(
                    "beta",
                    "b-v0",
                    "0.9",
                    "b-old.jar",
                    "bb00",
                    &["1.19.4"],
                    &["fabric"],
                ),
            ],
        );

    let plan = fixture.plan(&provider).await;
    let common = plan.common.expect("common target");
    assert_eq!(common.suggested.as_deref(), Some("1.20.1"));
    assert_eq!(common.common_versions, vec!["1.20.1", "1.19.4"]);
    assert_eq!(common.loader, "fabric");
    assert_eq!(common.checked, 2);
}

#[tokio::test]
async fn spec_f126_loader_verification_excludes_a_loader_only_target() {
    let fixture = Fixture::new();
    let a_sha = fixture.write("a.jar", b"a");
    let b_sha = fixture.write("b.jar", b"b");
    fixture.lock(vec![
        entry("a.jar", "alpha", "a-v1", &a_sha),
        entry("b.jar", "beta", "b-v1", &b_sha),
    ]);

    let provider = FakeProvider::new()
        .with(
            project("alpha", "Alpha"),
            vec![version(
                "alpha",
                "a-v1",
                "1.0",
                "a.jar",
                &a_sha,
                &["1.21", "1.20.1"],
                &["fabric"],
            )],
        )
        .with(
            project("beta", "Beta"),
            vec![
                version(
                    "beta",
                    "b-v1",
                    "1.0",
                    "b.jar",
                    &b_sha,
                    &["1.20.1"],
                    &["fabric"],
                ),
                // 1.21 fits the game-version axis but not the loader one. If
                // the loader gate disappeared, this row alone would make 1.21
                // common and the suggestion would move.
                version(
                    "beta",
                    "b-v2",
                    "2.0",
                    "b-2.jar",
                    "bb22",
                    &["1.21"],
                    &["forge"],
                ),
            ],
        );

    let plan = fixture.plan(&provider).await;
    let common = plan.common.expect("common target");
    assert_eq!(common.suggested.as_deref(), Some("1.20.1"));
    assert_eq!(common.common_versions, vec!["1.20.1"]);
    assert!(
        !common
            .common_versions
            .iter()
            .any(|version| version == "1.21"),
        "a forge-only 1.21 must not widen a fabric intersection"
    );
}

#[tokio::test]
async fn spec_f126_best_upgrade_names_its_blockers() {
    let fixture = Fixture::new();
    let a_sha = fixture.write("a.jar", b"a");
    let b_sha = fixture.write("b.jar", b"b");
    fixture.lock(vec![
        entry("a.jar", "alpha", "a-v1", &a_sha),
        entry("b.jar", "beta", "b-v1", &b_sha),
    ]);

    let provider = FakeProvider::new()
        .with(
            project("alpha", "Alpha"),
            vec![version(
                "alpha",
                "a-v2",
                "2.0",
                "a-2.jar",
                &a_sha,
                &["1.21", "1.20.1"],
                &["fabric"],
            )],
        )
        .with(
            project("beta", "Beta"),
            vec![version(
                "beta",
                "b-v1",
                "1.0",
                "b.jar",
                &b_sha,
                &["1.20.1"],
                &["fabric"],
            )],
        );

    let plan = fixture.plan(&provider).await;
    let common = plan.common.expect("common target");
    assert_eq!(common.suggested.as_deref(), Some("1.20.1"));
    let upgrade = common.best_upgrade.expect("advisory upgrade");
    assert_eq!(upgrade.target, "1.21");
    assert_eq!(upgrade.supported, 1);
    assert_eq!(upgrade.total, 2);
    assert_eq!(upgrade.blockers, vec!["Beta".to_string()]);
    // Advisory only: the strict suggestion does not move.
    assert_eq!(common.suggested.as_deref(), Some("1.20.1"));
}

#[tokio::test]
async fn spec_f126_an_empty_intersection_is_no_suggestion() {
    // Two projects with no shared game version: the honest answer is "none",
    // not the newest row of either project.
    let fixture = Fixture::new();
    let a_sha = fixture.write("a.jar", b"a");
    let b_sha = fixture.write("b.jar", b"b");
    fixture.lock(vec![
        entry("a.jar", "alpha", "a-v1", &a_sha),
        entry("b.jar", "beta", "b-v1", &b_sha),
    ]);

    let provider = FakeProvider::new()
        .with(
            project("alpha", "Alpha"),
            vec![version(
                "alpha",
                "a-v1",
                "1.0",
                "a.jar",
                &a_sha,
                &["1.21", "1.20.1"],
                &["fabric"],
            )],
        )
        .with(
            project("beta", "Beta"),
            vec![version(
                "beta",
                "b-v1",
                "1.0",
                "b.jar",
                &b_sha,
                &["1.19.4"],
                &["fabric"],
            )],
        );

    let plan = fixture.plan(&provider).await;
    let common = plan.common.expect("common target");
    assert_eq!(common.suggested, None);
    assert!(common.common_versions.is_empty());
    assert!(common.skipped.is_empty(), "both projects answered");
    assert_eq!(common.checked, 2);
    assert!(
        plan.warnings
            .iter()
            .any(|warning| warning.code() == "noCommonTarget"),
        "an empty intersection is reported, not silent: {:?}",
        plan.warnings
    );
    // The advisory may still name the least-blocked newer target; it must
    // never fill the strict suggestion.
    if let Some(upgrade) = common.best_upgrade {
        assert!(!upgrade.target.is_empty());
        assert!(!upgrade.blockers.is_empty());
    }
}

// --- F-127: cleanup keeps unknowns ----------------------------------------

#[tokio::test]
async fn spec_f127_unknown_local_files_are_kept_not_removed() {
    let fixture = Fixture::new();
    let a_sha = fixture.write("a.jar", b"a");
    let unknown_sha = fixture.write("hand-installed.jar", b"hand made");
    fixture.lock(vec![entry("a.jar", "alpha", "a-v1", &a_sha)]);

    let provider = FakeProvider::new().with(
        project("alpha", "Alpha"),
        vec![version(
            "alpha",
            "a-v2",
            "2.0",
            "a-2.jar",
            "aa22",
            &["1.21"],
            &["fabric"],
        )],
    );

    let plan = fixture.plan(&provider).await;
    assert_eq!(plan.cleanup.kept_unknown.len(), 1);
    let kept = &plan.cleanup.kept_unknown[0];
    assert_eq!(kept.filename, "hand-installed.jar");
    assert_eq!(kept.reason, "notInLock");
    assert_eq!(kept.sha1, unknown_sha);
    // Kept means kept: the plan never names it for removal, and the file is
    // still there after planning.
    assert!(plan
        .prune_candidates
        .iter()
        .all(|candidate| candidate.filename != "hand-installed.jar"));
    assert!(fixture.mods.join("hand-installed.jar").is_file());
}

#[tokio::test]
async fn spec_f127_recognized_incompatible_files_are_named_for_cleanup() {
    let fixture = Fixture::new();
    let a_sha = fixture.write("a.jar", b"a");
    fixture.lock(vec![entry("a.jar", "alpha", "a-v1", &a_sha)]);

    let provider = FakeProvider::new().with(
        project("alpha", "Alpha"),
        vec![version(
            "alpha",
            "a-v1",
            "1.0",
            "a.jar",
            &a_sha,
            &["1.20.1"],
            &["fabric"],
        )],
    );

    let plan = fixture.plan(&provider).await;
    assert_eq!(
        plan.cleanup.recognized_incompatible,
        vec!["a.jar".to_string()]
    );
    assert!(plan.cleanup.kept_unknown.is_empty());
    assert_eq!(row(&plan, "alpha").status, TargetStatus::Incompatible);
}

// --- F-131: present is not eligible ----------------------------------------

#[tokio::test]
async fn spec_f131_present_but_ineligible_entry_is_not_a_prune_candidate() {
    // The old app's data-deletion bug (lessons.md §4.2): a project present on
    // disk but skipped as incompatible was counted as missing and pruned.
    let fixture = Fixture::new();
    let a_sha = fixture.write("a.jar", b"a");
    fixture.lock(vec![entry("a.jar", "alpha", "a-v1", &a_sha)]);

    let provider = FakeProvider::new().with(
        project("alpha", "Alpha"),
        vec![version(
            "alpha",
            "a-v1",
            "1.0",
            "a.jar",
            &a_sha,
            &["1.20.1"],
            &["fabric"],
        )],
    );

    let plan = fixture.plan(&provider).await;
    let alpha = row(&plan, "alpha");
    assert_eq!(alpha.status, TargetStatus::Incompatible);
    assert!(alpha.file_present, "the file is on disk");
    assert!(
        plan.prune_candidates.is_empty(),
        "an ineligible file is not an absent file"
    );
    assert_eq!(plan.cleanup.eligible_files, 1);
}

#[tokio::test]
async fn spec_f131_present_and_eligible_are_counted_as_two_sets() {
    let fixture = Fixture::new();
    let a_sha = fixture.write("a.jar", b"a");
    fixture.write("hand-installed.jar", b"hand made");
    fixture.lock(vec![entry("a.jar", "alpha", "a-v1", &a_sha)]);

    let provider = FakeProvider::new().with(
        project("alpha", "Alpha"),
        vec![version(
            "alpha",
            "a-v2",
            "2.0",
            "a-2.jar",
            "aa22",
            &["1.21"],
            &["fabric"],
        )],
    );

    let plan = fixture.plan(&provider).await;
    assert_eq!(plan.cleanup.present_files, 2, "PRESENT is the disk scan");
    assert_eq!(plan.cleanup.eligible_files, 1, "ELIGIBLE is the lock");
    assert!(plan.prune_candidates.is_empty());
    assert_eq!(plan.cleanup.kept_unknown.len(), 1);
}

#[tokio::test]
async fn spec_f131_a_disabled_file_still_satisfies_its_lock_entry() {
    let fixture = Fixture::new();
    let sha = fixture.write("b.jar.disabled", b"b");
    fixture.lock(vec![entry("b.jar", "beta", "b-v1", &sha)]);

    let provider = FakeProvider::new().with(
        project("beta", "Beta"),
        vec![version(
            "beta",
            "b-v2",
            "2.0",
            "b-2.jar",
            "bb22",
            &["1.21"],
            &["fabric"],
        )],
    );

    let plan = fixture.plan(&provider).await;
    assert!(row(&plan, "beta").file_present);
    assert!(plan.prune_candidates.is_empty());
    assert!(plan.cleanup.kept_unknown.is_empty());
}

#[tokio::test]
async fn spec_f131_a_jar_in_a_subfolder_still_satisfies_its_record() {
    // Presence is recursive for cleanup: moving a jar into `mods/<sub>/` is
    // still having the file, so its record is not absent.
    let fixture = Fixture::new();
    let sha = fixture.write("nested/a.jar", b"a");
    fixture.lock(vec![entry("a.jar", "alpha", "a-v1", &sha)]);

    let provider = FakeProvider::new().with(
        project("alpha", "Alpha"),
        vec![version(
            "alpha",
            "a-v2",
            "2.0",
            "a-2.jar",
            "aa22",
            &["1.21"],
            &["fabric"],
        )],
    );

    let plan = fixture.plan(&provider).await;
    assert!(row(&plan, "alpha").file_present);
    assert!(
        plan.prune_candidates.is_empty(),
        "a record whose jar sits in a subfolder is not missing"
    );
}

#[tokio::test]
async fn spec_f131_a_reenabled_file_still_satisfies_a_disabled_record() {
    // The record says `x.jar.disabled`; the user re-enabled it to `x.jar`.
    // The lock entry and the file are the same artifact (F-067).
    let fixture = Fixture::new();
    fixture.write("b.jar", b"b");
    fixture.lock(vec![entry("b.jar.disabled", "beta", "b-v1", "")]);

    let provider = FakeProvider::new().with(
        project("beta", "Beta"),
        vec![version(
            "beta",
            "b-v2",
            "2.0",
            "b-2.jar",
            "bb22",
            &["1.21"],
            &["fabric"],
        )],
    );

    let plan = fixture.plan(&provider).await;
    assert!(row(&plan, "beta").file_present);
    assert!(plan.prune_candidates.is_empty());
}

// --- F-132: the per-run cache ---------------------------------------------

#[tokio::test]
async fn spec_f132_planner_reads_a_seeded_run_cache() {
    let fixture = Fixture::new();
    let sha = fixture.write("a.jar", b"a");
    fixture.lock(vec![entry("a.jar", "alpha", "a-v1", &sha)]);

    let provider = FakeProvider::new().with(
        project("alpha", "Alpha"),
        vec![version(
            "alpha",
            "a-v2",
            "2.0",
            "a-2.jar",
            "aa22",
            &["1.21"],
            &["fabric"],
        )],
    );

    // Seed both keys the run uses: the full metadata key and the unfiltered
    // list the common-target pass reads.
    let versions = provider
        .versions
        .get("alpha")
        .cloned()
        .expect("seeded versions");
    let mut cache = RunCache::new(true);
    cache.put_project(project("alpha", "Alpha"));
    cache.put_project_versions("alpha", "1.21", "fabric", versions.clone());
    cache.put_project_versions("alpha", "", "", versions);

    let plan = plan_targets_with_cache(
        &provider,
        &fixture.root,
        &TargetRequest::new("1.21", "fabric"),
        &mut cache,
    )
    .await
    .expect("plan");

    assert_eq!(row(&plan, "alpha").target_version, "2.0");
    assert_eq!(
        provider.project_calls(),
        0,
        "a seeded project must not be fetched again"
    );
    assert!(
        provider.versions_calls().is_empty(),
        "a seeded version list must not be fetched again: {:?}",
        provider.versions_calls()
    );
}

#[tokio::test]
async fn spec_f132_duplicate_project_rows_share_one_provider_question() {
    let fixture = Fixture::new();
    let sha_a = fixture.write("alpha-1.0.jar", b"alpha one");
    let sha_b = fixture.write("alpha-1.0-copy.jar", b"alpha one copy");
    fixture.lock(vec![
        entry("alpha-1.0.jar", "alpha", "alpha-v1", &sha_a),
        entry("alpha-1.0-copy.jar", "alpha", "alpha-v1", &sha_b),
    ]);

    let provider = FakeProvider::new().with(
        project("alpha", "Alpha"),
        vec![version(
            "alpha",
            "alpha-v2",
            "2.0",
            "alpha-2.0.jar",
            "aa22",
            &["1.21"],
            &["fabric"],
        )],
    );

    let plan = fixture.plan(&provider).await;
    assert_eq!(plan.rows.len(), 2);
    let filtered: Vec<_> = provider
        .versions_calls()
        .into_iter()
        .filter(|(_, game, _)| game.as_deref() == Some("1.21"))
        .collect();
    assert_eq!(
        filtered.len(),
        1,
        "the second row is a cache hit, not a second question: {:?}",
        provider.versions_calls()
    );
    assert_eq!(provider.project_calls(), 1);
}
