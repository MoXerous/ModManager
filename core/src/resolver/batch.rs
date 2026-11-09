//! One-shot batch orchestration: slugs → selected + required deps →
//! batch plan + warnings. Shared by the CLI and the Tauri bridge so the
//! wiring is written once (AGENTS.md §5.1). Never writes; pure
//! coordination over [`Provider`] (dry-run by construction).

use super::cache::RunCache;
use super::graph::{ResolvedContent, ReviewCandidate};
use super::plan::{build_batch_install_plan, build_safety_warnings};
use super::resolve::{resolve_dependency, resolve_project, ResolveError};
use super::{PlanEntry, PlanSummary, PreferredPin, ResolveParams, Warning};
use crate::providers::{Provider, Version, VersionDependency};
use std::collections::{HashMap, HashSet};

/// Everything a plan UI/CLI needs after one call.
pub struct BatchPlan {
    pub selected: Vec<ResolvedContent>,
    pub deps: Vec<ReviewCandidate>,
    pub entries: Vec<PlanEntry>,
    pub summary: PlanSummary,
    pub warnings: Vec<Warning>,
    /// By version id, for downloaders.
    pub versions: HashMap<String, Version>,
}

/// Resolve slugs → transitive required deps → batch plan + warnings.
/// `existing` holds filenames already in the destination (F-065 info).
/// A provider failure on a selected project aborts with the typed
/// [`ResolveError`]; dependency failures stay visible as `dependency-error`
/// stubs (one broken dep must not hide the rest of the plan).
pub async fn resolve_and_plan(
    provider: &dyn Provider,
    slugs: &[String],
    game: &str,
    loader: &str,
    existing: &HashSet<String>,
) -> Result<BatchPlan, ResolveError> {
    let mut cache = RunCache::new(true);
    let mut selected: Vec<ResolvedContent> = Vec::new();
    for slug in slugs {
        let project = provider
            .get_project(slug)
            .await
            .map_err(|source| ResolveError::provider("get_project", slug.clone(), source))?;
        let ptype = if project.project_type.is_empty() {
            "mod".to_string()
        } else {
            project.project_type.clone()
        };
        selected.push(
            resolve_project(
                provider,
                &mut cache,
                &ResolveParams {
                    project,
                    project_type: ptype,
                    game_version: game.to_string(),
                    loader: loader.to_string(),
                    allow_alpha: false,
                    pin: PreferredPin::default(),
                },
            )
            .await?,
        );
    }
    // Transitive required deps; cycles cut silently (F-061 deferred).
    let mut deps: Vec<ReviewCandidate> = Vec::new();
    let mut dependency_warnings: Vec<Warning> = Vec::new();
    let mut known: HashSet<String> = selected.iter().map(|r| r.project_id.clone()).collect();
    let mut grown: Vec<ResolvedContent> = selected.clone();
    let mut i = 0;
    while i < grown.len() {
        let parent = grown[i].project_id.clone();
        let reqs: Vec<VersionDependency> = grown[i]
            .version
            .as_ref()
            .map(|v| {
                v.dependencies
                    .iter()
                    .filter(|d| d.dependency_type == "required")
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        for dep in reqs {
            let id = dep.project_id.clone().unwrap_or_default();
            if id.is_empty() || !known.insert(id.clone()) {
                continue;
            }
            if let Some(r) =
                resolve_dependency(provider, &mut cache, &dep, game, loader, &HashMap::new()).await
            {
                if r.version.is_some() {
                    deps.push(ReviewCandidate {
                        content: r.clone(),
                        dependency_type: "required".to_string(),
                        selected: true,
                        parents: vec![parent.clone()],
                        requires_parent: false,
                    });
                    grown.push(r);
                } else if r.status == "dependency-error" {
                    // A failed dep is skipped from the download set, but the
                    // plan must say which provider call failed — not leave the
                    // user with a generic "required metadata is missing"
                    // warning that cannot distinguish a 500 from a typo.
                    dependency_warnings.push(Warning {
                        severity: "warning".to_string(),
                        message: format!(
                            "Required dependency {id} could not be resolved: {}",
                            r.reason
                        ),
                    });
                }
            }
        }
        i += 1;
    }
    let (entries, summary) = build_batch_install_plan(&selected, &deps, game, loader);
    let mut warnings = build_safety_warnings(&entries, &grown, existing);
    warnings.extend(dependency_warnings);
    let mut versions: HashMap<String, Version> = HashMap::new();
    for r in &grown {
        if let Some(v) = &r.version {
            versions.insert(v.id.clone(), v.clone());
        }
    }
    Ok(BatchPlan {
        selected,
        deps,
        entries,
        summary,
        warnings,
        versions,
    })
}
