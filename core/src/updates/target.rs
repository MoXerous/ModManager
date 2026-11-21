//! Target-version planner (F-125..F-127, F-131, F-132).
//!
//! Read-only: given the instance's lock and the files actually on disk, work
//! out what each recorded project would become at a chosen Minecraft version
//! and loader. Nothing here downloads, renames or writes a lock.
//!
//! **Two sets, never one** (F-131, `lessons.md` §4.2). The PRESENT set is what
//! the scan found on disk; the ELIGIBLE set is what the lock records for this
//! provider. The old app pruned saved metadata by comparing it with the
//! *importable/matched* subset instead of the *present* set, and deleted rows
//! for files that were sitting on disk but were merely skipped for
//! incompatibility. Here:
//!
//! - a file that is present but absent from the lock is UNKNOWN and is always
//!   KEPT, never reported as removed (F-127);
//! - a lock entry is a prune candidate only when NO file on disk satisfies it
//!   (F-130) - incompatibility with the target is not absence from disk;
//! - popularity with the resolver never decides presence.
//!
//! **One run, one cache** (F-132). [`plan_targets`] owns a
//! [`RunCache`] and drops it with the run (`lessons.md` §6): a cache that
//! outlives its run is a cache that serves stale provider answers.
//!
//! **F-097 stays a separate slice.** Ranges (`>=1.20`, `1.20.x`, `[a,b)`) are
//! still not parsed. The planner judges compatibility against live provider
//! metadata, which is an exact list of game versions, so the recorded ranges
//! are not on this path. A half parser would turn today's honest `Unknown`
//! into a false `Incompatible`, which is the wrong direction for a cleanup
//! screen. The `is_exact_version` gate in `instances::content` remains the
//! boundary; this slice deliberately does not move it.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use crate::instances::lock::{read_lock, LockEntry, LockError, LOCK_FILE_NAME};
use crate::instances::{scan_mods_recursive, Instance, LocalContentFact};
use crate::providers::{Project, Provider, ProviderError, Version};
use crate::resolver::cache::RunCache;
use crate::resolver::graph::{
    candidate_loaders_for_project_type, compatibility_issue, primary_file,
};
use crate::resolver::resolve::{resolve_project, PreferredPin, ResolveParams};
use crate::resolver::ResolvedContent;

use super::prune::{candidates_for, is_recordable_entry, PruneCandidate};

/// The eight columns of the target table, in the old app's order of reading:
/// what the project is, where it is now, where it would go, the target it is
/// judged against, and the two follow-up actions. `reason` rides with
/// `status`; file names and hashes are payload, not extra columns.
pub const TARGET_COLUMNS: [&str; 8] = [
    "title",
    "currentVersion",
    "targetVersion",
    "gameVersion",
    "loader",
    "download",
    "sync",
    "status",
];

/// What the user asked the planner to target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetRequest {
    pub game_version: String,
    pub loader: String,
}

impl TargetRequest {
    pub fn new(game_version: &str, loader: &str) -> Self {
        Self {
            game_version: game_version.trim().to_string(),
            loader: loader.trim().to_string(),
        }
    }

    /// A concrete target is a real Minecraft version and a real loader. The
    /// two sentinels the app itself uses for "no filter" and "could not
    /// detect" are neither: a plan against them would have to guess.
    fn is_concrete(&self) -> bool {
        !is_sentinel(&self.game_version) && !is_sentinel(&self.loader)
    }
}

/// `any` is the old app's "no filter", `unknown` this app's "could not
/// detect"; an empty string is a value that was never filled.
fn is_sentinel(value: &str) -> bool {
    let value = value.trim();
    value.is_empty()
        || value.eq_ignore_ascii_case("any")
        || value.eq_ignore_ascii_case(crate::db::instances::UNKNOWN)
}

/// The one label the row shows (F-125), plus the reason code for its tooltip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetStatus {
    /// The recorded version is what the target resolves to.
    UpToDate,
    /// A different target version exists and must be downloaded.
    Download,
    /// The target file is already on disk byte-identical; only the lock's
    /// record needs updating.
    SyncOnly,
    /// The provider has no version for this target.
    Incompatible,
    /// Recorded by another provider; this run cannot judge it.
    OtherProvider,
    /// The provider failed; the row is visible, the plan is not aborted.
    Error,
}

impl TargetStatus {
    pub fn code(self) -> &'static str {
        match self {
            Self::UpToDate => "upToDate",
            Self::Download => "download",
            Self::SyncOnly => "syncOnly",
            Self::Incompatible => "incompatible",
            Self::OtherProvider => "otherProvider",
            Self::Error => "providerError",
        }
    }
}

/// A stable code plus the provider's own sentence when there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetReason {
    pub code: &'static str,
    pub detail: String,
}

impl TargetReason {
    fn new(code: &'static str) -> Self {
        Self {
            code,
            detail: String::new(),
        }
    }

    fn with_detail(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

/// One project's line in the planner (F-125).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetRow {
    pub title: String,
    pub current_version: String,
    pub target_version: String,
    pub game_version: String,
    pub loader: String,
    pub download: bool,
    pub sync: bool,
    pub status: TargetStatus,
    pub reason: TargetReason,
    // Payload behind the columns.
    pub provider: String,
    pub project_id: String,
    pub slug: String,
    pub current_filename: String,
    /// The archive the target version would install.
    pub target_filename: String,
    pub target_url: Option<String>,
    pub target_size: Option<u64>,
    pub target_sha1: Option<String>,
    pub target_sha512: Option<String>,
    /// F-131: this row's lock entry has a file on disk. Nothing about the
    /// target changes this fact.
    pub file_present: bool,
}

/// A present file the lock never recorded (F-127/F-131): UNKNOWN, KEPT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptUnknownFile {
    pub rel_path: String,
    pub filename: String,
    pub sha1: String,
    /// Always `notInLock` today; a code, not prose, so the UI translates it.
    pub reason: &'static str,
}

/// F-127: what a target cleanup may touch, and what it must leave alone.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CleanupReport {
    /// Files on disk with no lock record. Never removed, always reported.
    pub kept_unknown: Vec<KeptUnknownFile>,
    /// Locked files whose project has no version for the target. The old app
    /// moved these to a dated backup on install; this plan only names them.
    pub recognized_incompatible: Vec<String>,
    /// PRESENT set size.
    pub present_files: usize,
    /// ELIGIBLE set size: lock entries with a file on disk.
    pub eligible_files: usize,
}

/// F-126: the newest version every recorded project can actually use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommonTarget {
    pub suggested: Option<String>,
    /// Newest-first, the whole verified intersection.
    pub common_versions: Vec<String>,
    pub loader: String,
    pub checked: usize,
    /// Projects that could not contribute a verified version. This is why an
    /// empty suggestion is empty, not a shrug.
    pub skipped: Vec<CommonTargetSkip>,
    /// Advisory: a newer target that most - not all - projects support.
    pub best_upgrade: Option<BestUpgrade>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommonTargetSkip {
    pub project_id: String,
    pub title: String,
    /// `providerError` | `noVersions`.
    pub reason: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BestUpgrade {
    pub target: String,
    pub supported: usize,
    pub total: usize,
    pub blockers: Vec<String>,
}

/// A fact about the plan, not about one row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetWarning {
    EmptyLock,
    LockUnreadable,
    NoCommonTarget,
    ProviderFailure { project_id: String },
}

impl TargetWarning {
    pub fn code(&self) -> &'static str {
        match self {
            Self::EmptyLock => "emptyLock",
            Self::LockUnreadable => "lockUnreadable",
            Self::NoCommonTarget => "noCommonTarget",
            Self::ProviderFailure { .. } => "providerFailure",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetPlan {
    pub game_version: String,
    pub loader: String,
    pub rows: Vec<TargetRow>,
    pub common: Option<CommonTarget>,
    /// F-130 dry run: lock entries with no file on disk. Removal is a separate,
    /// explicit confirm.
    pub prune_candidates: Vec<PruneCandidate>,
    pub cleanup: CleanupReport,
    pub warnings: Vec<TargetWarning>,
}

#[derive(Debug, thiserror::Error)]
pub enum TargetError {
    #[error("a target needs a concrete loader and Minecraft version")]
    NeedsConcreteTarget,
    #[error(transparent)]
    Lock(#[from] LockError),
}

/// Plans with a cache owned by this run and dropped with it (F-132).
pub async fn plan_targets(
    provider: &dyn Provider,
    instance_root: &Path,
    request: &TargetRequest,
) -> Result<TargetPlan, TargetError> {
    let mut cache = RunCache::new(true);
    plan_targets_with_cache(provider, instance_root, request, &mut cache).await
}

/// The planner with an explicit per-run cache. The seam exists so a test can
/// seed the cache and prove the planning flow reads it, and so a caller that
/// already owns a run cache does not throw it away.
pub async fn plan_targets_with_cache(
    provider: &dyn Provider,
    instance_root: &Path,
    request: &TargetRequest,
    cache: &mut RunCache,
) -> Result<TargetPlan, TargetError> {
    if !request.is_concrete() {
        return Err(TargetError::NeedsConcreteTarget);
    }

    let lock_path = instance_root.join(LOCK_FILE_NAME);
    let mut warnings = Vec::new();
    let entries = match read_lock(&lock_path) {
        Ok(lock) => lock.entries,
        Err(LockError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            warnings.push(TargetWarning::EmptyLock);
            Vec::new()
        }
        // A newer schema is a typed refusal: interpreting it would be guessing.
        Err(error @ LockError::NewerSchema { .. }) => return Err(TargetError::Lock(error)),
        Err(_) => {
            warnings.push(TargetWarning::LockUnreadable);
            Vec::new()
        }
    };

    let facts = scan_mods_recursive(&Instance {
        root: instance_root.to_path_buf(),
        minecraft_root: instance_root.to_path_buf(),
    });

    // F-131: PRESENT is the scan, ELIGIBLE is the lock. Nothing below decides
    // presence from a resolver verdict.
    let present_entries: Vec<bool> = entries
        .iter()
        .map(|entry| entry_is_present(entry, &facts))
        .collect();
    let prune_candidates = candidates_for(&entries, &facts);
    let unknown = unknown_files(&entries, &facts);

    let provider_id = provider.provider_id();
    let mut rows = Vec::new();
    let mut recognized_incompatible = Vec::new();

    for (index, entry) in entries.iter().enumerate() {
        if !is_recordable_entry(entry) {
            continue;
        }
        let file_present = present_entries[index];
        let mut row = TargetRow {
            title: title_for(entry),
            current_version: current_version_label(entry),
            target_version: String::new(),
            game_version: request.game_version.clone(),
            loader: request.loader.clone(),
            download: false,
            sync: false,
            status: TargetStatus::Incompatible,
            reason: TargetReason::new("noTargetVersion"),
            provider: if entry.provider.is_empty() {
                provider_id.to_string()
            } else {
                entry.provider.clone()
            },
            project_id: entry.project_id.clone(),
            slug: entry.slug.clone(),
            current_filename: entry.filename.clone(),
            target_filename: String::new(),
            target_url: None,
            target_size: None,
            target_sha1: None,
            target_sha512: None,
            file_present,
        };

        if !entry.provider.is_empty() && !entry.provider.eq_ignore_ascii_case(provider_id) {
            row.status = TargetStatus::OtherProvider;
            row.reason = TargetReason::new("otherProvider");
            rows.push(row);
            continue;
        }
        if entry.project_id.trim().is_empty() {
            row.reason = TargetReason::new("noProjectId");
            rows.push(row);
            continue;
        }

        let params = ResolveParams {
            project: stub_project(entry),
            project_type: "mod".to_string(),
            game_version: request.game_version.clone(),
            loader: request.loader.clone(),
            allow_beta: true,
            // Alpha stays closed, as in every other install path (F-003).
            allow_alpha: false,
            pin: PreferredPin::default(),
        };
        match resolve_project(provider, cache, &params).await {
            Ok(resolved) => {
                apply_resolved(
                    &mut row,
                    entry,
                    resolved,
                    &facts,
                    &mut recognized_incompatible,
                );
                rows.push(row);
            }
            // One provider failure is a row and a warning, not a plan that
            // hides the other projects.
            Err(error) => {
                warnings.push(TargetWarning::ProviderFailure {
                    project_id: entry.project_id.clone(),
                });
                row.status = TargetStatus::Error;
                row.reason = TargetReason::with_detail("providerFailure", error.to_string());
                rows.push(row);
            }
        }
    }

    let cleanup = CleanupReport {
        kept_unknown: unknown
            .iter()
            .map(|fact| KeptUnknownFile {
                rel_path: fact.rel_path.clone(),
                filename: fact.filename.clone(),
                sha1: fact.sha1.clone(),
                reason: "notInLock",
            })
            .collect(),
        recognized_incompatible,
        present_files: facts.len(),
        eligible_files: present_entries.iter().filter(|present| **present).count(),
    };

    let projects = distinct_projects(&rows);
    let common = if projects.is_empty() {
        None
    } else {
        Some(plan_common_target(provider, cache, &projects, request, &mut warnings).await)
    };

    Ok(TargetPlan {
        game_version: request.game_version.clone(),
        loader: request.loader.clone(),
        rows,
        common,
        prune_candidates,
        cleanup,
        warnings,
    })
}

/// The identity a lock entry gives the resolver. The lock records no project
/// type, so `mod` is the same default the resolver uses for an empty one.
fn stub_project(entry: &LockEntry) -> Project {
    Project {
        id: entry.project_id.clone(),
        slug: if entry.slug.trim().is_empty() {
            None
        } else {
            Some(entry.slug.clone())
        },
        project_type: "mod".to_string(),
        ..Default::default()
    }
}

fn title_for(entry: &LockEntry) -> String {
    if !entry.slug.trim().is_empty() {
        entry.slug.clone()
    } else if !entry.project_id.trim().is_empty() {
        entry.project_id.clone()
    } else {
        entry.filename.clone()
    }
}

/// What the lock knows about "now": the pinned version id, or the file name a
/// lock from before ids existed still has.
fn current_version_label(entry: &LockEntry) -> String {
    if !entry.version_id.trim().is_empty() {
        entry.version_id.clone()
    } else {
        entry.filename.clone()
    }
}

fn target_version_label(version: &Version) -> String {
    if !version.version_number.trim().is_empty() {
        version.version_number.clone()
    } else {
        version.id.clone()
    }
}

fn hashes_match(recorded: &str, candidate: &str) -> bool {
    let recorded = recorded.trim();
    let candidate = candidate.trim();
    !recorded.is_empty() && !candidate.is_empty() && recorded.eq_ignore_ascii_case(candidate)
}

fn apply_resolved(
    row: &mut TargetRow,
    entry: &LockEntry,
    resolved: ResolvedContent,
    facts: &[LocalContentFact],
    recognized_incompatible: &mut Vec<String>,
) {
    if !resolved.title.trim().is_empty() {
        row.title = resolved.title.clone();
    }
    row.project_id = resolved.project_id.clone();
    row.slug = resolved.slug.clone();

    let Some(version) = resolved.version else {
        row.status = TargetStatus::Incompatible;
        row.reason = TargetReason::with_detail("noTargetVersion", resolved.reason);
        recognized_incompatible.push(entry.filename.clone());
        return;
    };

    let file = primary_file(&version).or_else(|| version.files.first());
    row.target_version = target_version_label(&version);
    row.target_filename = file.map(|f| f.filename.clone()).unwrap_or_default();
    row.target_url = file
        .map(|f| f.url.clone())
        .filter(|url| !url.trim().is_empty());
    row.target_size = file.map(|f| f.size);
    row.target_sha1 = file
        .and_then(|f| f.sha1.clone())
        .filter(|v| !v.trim().is_empty());
    row.target_sha512 = file
        .and_then(|f| f.sha512.clone())
        .filter(|v| !v.trim().is_empty());

    let same = (!entry.version_id.trim().is_empty() && entry.version_id == version.id)
        || version
            .files
            .iter()
            .any(|file| hashes_match(&entry.sha1, file.sha1.as_deref().unwrap_or("")));

    if same {
        row.status = TargetStatus::UpToDate;
        row.reason = TargetReason::new("sameVersion");
        return;
    }

    // Evidence, not hope: only a hash proves the file on disk is the target.
    let target_on_disk = row
        .target_sha1
        .as_deref()
        .is_some_and(|target| facts.iter().any(|fact| hashes_match(&fact.sha1, target)));
    if target_on_disk {
        row.status = TargetStatus::SyncOnly;
        row.reason = TargetReason::new("alreadyOnDisk");
        row.sync = true;
    } else {
        row.status = TargetStatus::Download;
        row.reason = TargetReason::new("targetAvailable");
        row.download = true;
        row.sync = true;
    }
}

/// One entry per project, in first-seen order: two lock rows for one project
/// are one question to the catalogue.
fn distinct_projects(rows: &[TargetRow]) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for row in rows {
        if row.status == TargetStatus::OtherProvider || row.project_id.trim().is_empty() {
            continue;
        }
        if seen.insert(row.project_id.clone()) {
            out.push((row.project_id.clone(), row.title.clone()));
        }
    }
    out
}

async fn plan_common_target(
    provider: &dyn Provider,
    cache: &mut RunCache,
    projects: &[(String, String)],
    request: &TargetRequest,
    warnings: &mut Vec<TargetWarning>,
) -> CommonTarget {
    let loader_candidates = candidate_loaders_for_project_type("mod", &request.loader);
    let mut per_project: Vec<(String, BTreeSet<String>)> = Vec::new();
    let mut skipped = Vec::new();

    for (project_id, title) in projects {
        match versions_cached_all(provider, cache, project_id).await {
            Ok(versions) => {
                let viable = viable_game_versions(&versions, &loader_candidates);
                if viable.is_empty() {
                    skipped.push(CommonTargetSkip {
                        project_id: project_id.clone(),
                        title: title.clone(),
                        reason: "noVersions",
                    });
                } else {
                    per_project.push((title.clone(), viable));
                }
            }
            Err(_) => {
                warnings.push(TargetWarning::ProviderFailure {
                    project_id: project_id.clone(),
                });
                skipped.push(CommonTargetSkip {
                    project_id: project_id.clone(),
                    title: title.clone(),
                    reason: "providerError",
                });
            }
        }
    }

    let common_versions = if skipped.is_empty() {
        intersect_versions(&per_project)
    } else {
        // A project we could not read makes any intersection a guess.
        Vec::new()
    };
    let suggested = common_versions.first().cloned();
    if suggested.is_none() {
        warnings.push(TargetWarning::NoCommonTarget);
    }
    let best_upgrade = best_upgrade(&per_project, suggested.as_deref());

    CommonTarget {
        suggested,
        common_versions,
        loader: request.loader.clone(),
        checked: per_project.len(),
        skipped,
        best_upgrade,
    }
}

/// Cached unfiltered version list. A provider failure is returned and never
/// cached: it is not "this project has no versions".
async fn versions_cached_all(
    provider: &dyn Provider,
    cache: &mut RunCache,
    project_id: &str,
) -> Result<Vec<Version>, ProviderError> {
    if let Some(hit) = cache.project_versions(project_id, "", "") {
        return Ok(hit);
    }
    let versions = provider.get_versions(project_id, None, None).await?;
    cache.put_project_versions(project_id, "", "", versions.clone());
    Ok(versions)
}

/// The game versions one project really supports on this loader: a listed
/// version with a downloadable primary file, verified by the resolver's own
/// compatibility rule rather than by a second implementation.
fn viable_game_versions(versions: &[Version], loader_candidates: &[String]) -> BTreeSet<String> {
    let mut viable = BTreeSet::new();
    for version in versions {
        for game in &version.game_versions {
            if compatibility_issue(Some(version), game, loader_candidates).is_none() {
                viable.insert(game.clone());
            }
        }
    }
    viable
}

fn intersect_versions(per_project: &[(String, BTreeSet<String>)]) -> Vec<String> {
    let mut intersection: Option<BTreeSet<String>> = None;
    for (_, set) in per_project {
        intersection = Some(match intersection {
            None => set.clone(),
            Some(accumulated) => accumulated.intersection(set).cloned().collect(),
        });
    }
    let mut out: Vec<String> = intersection.unwrap_or_default().into_iter().collect();
    out.sort_by_key(|version| std::cmp::Reverse(version_sort_key(version)));
    out
}

/// Newest-first ordering for Minecraft versions, without inventing a full
/// semver parser: numeric components in order, releases before pre-releases.
type VersionKey = (u8, [u64; 4], String);

/// (supported count, sort key, target, blockers)
type BestCandidate = (usize, VersionKey, String, Vec<String>);

/// The old app's advisory answer: the newest target more projects can reach
/// than the strict one, with the projects it leaves behind named. Strict stays
/// the default; this never replaces [`CommonTarget::suggested`].
fn best_upgrade(
    per_project: &[(String, BTreeSet<String>)],
    suggested: Option<&str>,
) -> Option<BestUpgrade> {
    if per_project.len() < 2 {
        return None;
    }
    let total = per_project.len();
    let suggested_key = suggested.map(version_sort_key);
    let mut candidates = BTreeSet::new();
    for (_, viable) in per_project {
        candidates.extend(viable.iter().cloned());
    }

    let mut best: Option<BestCandidate> = None;
    for candidate in candidates {
        let key = version_sort_key(&candidate);
        if suggested_key
            .as_ref()
            .is_some_and(|suggested| key <= *suggested)
        {
            continue;
        }
        let supported: Vec<&String> = per_project
            .iter()
            .filter(|(_, viable)| viable.contains(&candidate))
            .map(|(title, _)| title)
            .collect();
        if supported.is_empty() || supported.len() >= total {
            continue;
        }
        let blockers: Vec<String> = per_project
            .iter()
            .filter(|(_, viable)| !viable.contains(&candidate))
            .map(|(title, _)| title.clone())
            .collect();
        let count = supported.len();
        if best
            .as_ref()
            .is_none_or(|(best_count, best_key, _, _)| (count, &key) > (*best_count, best_key))
        {
            best = Some((count, key, candidate, blockers));
        }
    }

    best.map(|(supported, _, target, blockers)| BestUpgrade {
        target,
        supported,
        total,
        blockers,
    })
}

/// Newest-first ordering for Minecraft versions, without inventing a full
/// semver parser: numeric components in order, releases before pre-releases.
fn version_sort_key(version: &str) -> (u8, [u64; 4], String) {
    let text = version.trim().to_lowercase();
    let lower = text.as_str();
    let unstable = lower.contains("pre")
        || lower.contains("rc")
        || lower.contains("snapshot")
        || lower.starts_with('w');
    let mut numbers = [0u64; 4];
    for (slot, part) in text
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .take(4)
        .enumerate()
    {
        numbers[slot] = part.parse().unwrap_or(0);
    }
    (if unstable { 1 } else { 2 }, numbers, text)
}

/// The lock entry a present file satisfies. Disabled spellings still count:
/// `a.jar.disabled` and `disabled_mods/a.jar` are the same recorded file
/// (F-067), and a record written for one spelling must still be satisfied
/// after a toggle changes it — calling that missing would prune live metadata.
pub(crate) fn entry_is_present(entry: &LockEntry, facts: &[LocalContentFact]) -> bool {
    let recorded = entry.filename.trim().to_lowercase();
    let recorded_base = base_name(&recorded);
    facts.iter().any(|fact| {
        let disk = fact.filename.trim().to_lowercase();
        base_name(&disk) == recorded_base
            || (!entry.sha1.trim().is_empty() && fact.sha1.eq_ignore_ascii_case(entry.sha1.trim()))
    })
}

/// The name without a `.disabled` suffix: the same artifact on either side of
/// a toggle.
fn base_name(name: &str) -> &str {
    name.strip_suffix(".disabled").unwrap_or(name)
}

/// Present files no lock entry claims. Order follows the scan (casefolded), so
/// a report is stable.
pub(crate) fn unknown_files<'a>(
    entries: &[LockEntry],
    facts: &'a [LocalContentFact],
) -> Vec<&'a LocalContentFact> {
    facts
        .iter()
        .filter(|fact| {
            !entries
                .iter()
                .any(|entry| entry_is_present(entry, std::slice::from_ref(*fact)))
        })
        .collect()
}
