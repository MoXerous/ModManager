//! Pure selection rules: loader candidates, compatibility ladder,
//! best-version pick, and BFS closure over already-resolved metadata.

use crate::providers::{ProjectFile, Version};
use std::collections::{HashMap, HashSet, VecDeque};

/// A project with its resolved version (`None` = incompatible/blocked).
/// `status`: `compatible` | `incompatible` | `browse-only` | `dependency-error`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedContent {
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub project_type: String,
    pub version: Option<Version>,
    pub status: String,
    pub reason: String,
}

/// One row of the dependency-review dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewCandidate {
    pub content: ResolvedContent,
    /// `required` | `optional` | `incompatible` | `embedded`.
    pub dependency_type: String,
    /// Optional deps ship unchecked (F-054); required ship checked.
    pub selected: bool,
    pub parents: Vec<String>,
    /// Set when pulled in by a selected optional's required chain (F-055).
    pub requires_parent: bool,
}

fn fallbacks(project_type: &str) -> Option<&'static [&'static str]> {
    match project_type {
        "mod" | "modpack" => Some(&["fabric", "forge", "neoforge", "quilt"]),
        "shader" => Some(&["iris", "optifine", "vanilla", "minecraft"]),
        "resourcepack" => Some(&["minecraft"]),
        "datapack" => Some(&["datapack", "minecraft"]),
        "plugin" => Some(&[
            "paper",
            "spigot",
            "bukkit",
            "folia",
            "purpur",
            "velocity",
            "bungeecord",
            "sponge",
        ]),
        _ => None,
    }
}

/// Loader candidates for a content type (spec §2).
///
/// Strict types (`mod`, `modpack`) with a concrete selection return
/// **only** that loader — never fall through (F-051). Unknown content
/// types degrade to `[selected]` or `["any"]`.
pub fn candidate_loaders_for_project_type(project_type: &str, selected: &str) -> Vec<String> {
    let pt = project_type.trim().to_lowercase();
    let pt = if pt.is_empty() { "mod".to_string() } else { pt };
    let sel = selected.trim().to_lowercase();
    let has_sel = !sel.is_empty() && sel != "any";
    match fallbacks(pt.as_str()) {
        Some(table) => {
            if has_sel {
                // Strict: the selected loader only, no cross-loader fallback.
                if pt == "mod" || pt == "modpack" {
                    return vec![sel];
                }
                if table.contains(&sel.as_str()) {
                    let mut out = vec![sel.clone()];
                    out.extend(table.iter().filter(|l| **l != sel).map(|s| s.to_string()));
                    return out;
                }
                // Non-strict with an off-table selection: ignore it.
            }
            table.iter().map(|s| s.to_string()).collect()
        }
        None => {
            if has_sel {
                vec![sel]
            } else {
                vec!["any".to_string()]
            }
        }
    }
}

/// The downloadable file of a version, if flagged primary.
pub fn primary_file(version: &Version) -> Option<&ProjectFile> {
    version.files.iter().find(|f| f.primary)
}

/// Compatibility verdict (spec §3): `None` = compatible, else the exact
/// English reason in the fixed 5-step order. The loader check compares
/// **all** candidates, never just the selected loader.
pub fn compatibility_issue(
    version: Option<&Version>,
    game_version: &str,
    loader_candidates: &[String],
) -> Option<String> {
    let v = match version {
        Some(v) => v,
        None => return Some("No version was selected.".to_string()),
    };
    let file = match primary_file(v) {
        Some(f) => f,
        None => {
            return Some("Selected version has no downloadable primary file.".to_string());
        }
    };
    let _ = file;
    if v.status != "listed" {
        return Some(format!(
            "Selected version is not listed for download (status: {}).",
            v.status
        ));
    }
    if !game_version.is_empty()
        && game_version != "any"
        && !v.game_versions.iter().any(|g| g == game_version)
    {
        let listed = if v.game_versions.is_empty() {
            "none listed".to_string()
        } else {
            v.game_versions.join(", ")
        };
        return Some(format!(
            "Selected version does not support Minecraft {game_version}. Listed game versions: {listed}."
        ));
    }
    let meaningful: Vec<&String> = loader_candidates
        .iter()
        .filter(|l| !l.is_empty() && l.as_str() != "any")
        .collect();
    if !meaningful.is_empty()
        && !meaningful
            .iter()
            .any(|l| v.loaders.iter().any(|vl| vl == *l))
    {
        let listed = if v.loaders.is_empty() {
            "none listed".to_string()
        } else {
            v.loaders.join(", ")
        };
        return Some(format!(
            "Selected version does not support loader/platform {}. Listed loaders/platforms: {listed}.",
            meaningful[0]
        ));
    }
    None
}

/// First compatible version in input (newest-first) order, honouring the
/// release channel (`allow_alpha = false` skips `alpha` builds).
pub fn select_best_version<'a>(
    versions: &'a [Version],
    game_version: &str,
    loader_candidates: &[String],
    allow_alpha: bool,
) -> Option<&'a Version> {
    versions.iter().find(|v| {
        if !allow_alpha && v.version_type.to_lowercase() == "alpha" {
            return false;
        }
        compatibility_issue(Some(v), game_version, loader_candidates).is_none()
    })
}

fn channel_ok(v: &Version, allow_alpha: bool) -> bool {
    allow_alpha || v.version_type.to_lowercase() != "alpha"
}

fn fits(v: &Version, game_version: &str, loader_candidates: &[String]) -> bool {
    compatibility_issue(Some(v), game_version, loader_candidates).is_none()
}

/// Preferred-pin pick (F-056): id → sha1 → number, each required to be
/// compatible; any incompatible pin falls back to the best version (F-057).
#[allow(clippy::too_many_arguments)]
pub fn select_preferred<'a>(
    versions: &'a [Version],
    game_version: &str,
    loader_candidates: &[String],
    preferred_id: Option<&str>,
    preferred_sha1: Option<&str>,
    preferred_number: Option<&str>,
    allow_alpha: bool,
) -> Option<&'a Version> {
    if let Some(id) = preferred_id {
        if let Some(v) = versions.iter().find(|v| {
            v.id == id && channel_ok(v, allow_alpha) && fits(v, game_version, loader_candidates)
        }) {
            return Some(v);
        }
    }
    if let Some(sha) = preferred_sha1 {
        let want = sha.to_lowercase();
        if let Some(v) = versions.iter().find(|v| {
            channel_ok(v, allow_alpha)
                && fits(v, game_version, loader_candidates)
                && v.files.iter().any(|f| {
                    f.sha1
                        .as_deref()
                        .map(|h| h.to_lowercase() == want)
                        .unwrap_or(false)
                })
        }) {
            return Some(v);
        }
    }
    if let Some(num) = preferred_number {
        if let Some(v) = versions.iter().find(|v| {
            v.version_number == num
                && channel_ok(v, allow_alpha)
                && fits(v, game_version, loader_candidates)
        }) {
            return Some(v);
        }
    }
    select_best_version(versions, game_version, loader_candidates, allow_alpha)
}

fn stub_for(id: &str) -> ResolvedContent {
    ResolvedContent {
        project_id: id.to_string(),
        slug: id.to_string(),
        title: format!("Dependency {id}"),
        project_type: "mod".to_string(),
        version: None,
        status: "dependency-error".to_string(),
        reason: "Could not fetch dependency project metadata.".to_string(),
    }
}

/// BFS over already-resolved metadata (spec §5-2): roots first, each
/// project added once. Cycles are cut silently with a warn log — explicit
/// cycle reporting is deferred to M3 (F-061). Only `required` edges are
/// followed; deps without a `project_id` are skipped.
pub fn collect_required(
    roots: &[ResolvedContent],
    catalog: &HashMap<String, ResolvedContent>,
) -> Vec<ResolvedContent> {
    let mut known: HashSet<String> = roots.iter().map(|r| r.project_id.clone()).collect();
    let mut out: Vec<ResolvedContent> = roots.to_vec();
    let mut queue: VecDeque<ResolvedContent> = roots.to_vec().into();
    while let Some(cur) = queue.pop_front() {
        let deps: Vec<String> = cur
            .version
            .as_ref()
            .map(|v| {
                v.dependencies
                    .iter()
                    .filter(|d| d.dependency_type == "required")
                    .filter_map(|d| d.project_id.clone())
                    .filter(|id| !id.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        for id in deps {
            if !known.insert(id.clone()) {
                tracing::warn!("resolver: cycle cut at {id}");
                continue;
            }
            match catalog.get(&id) {
                Some(next) => {
                    queue.push_back(next.clone());
                    out.push(next.clone());
                }
                None => out.push(stub_for(&id)),
            }
        }
    }
    out
}

/// Review rows (spec §5-3): required first (checked), then optional
/// (unchecked), then each *checked* optional's required chain with
/// `requires_parent`. `selected_optional` holds optionals the user checked;
/// their required chains are flagged so the UI auto-checks them (F-055).
/// Merged by project id with parents unioned.
pub fn review_candidates(
    resolved: &[ResolvedContent],
    selected_optional: &[ResolvedContent],
    catalog: &HashMap<String, ResolvedContent>,
) -> Vec<ReviewCandidate> {
    let mut all_roots: Vec<ResolvedContent> = resolved.to_vec();
    all_roots.extend(selected_optional.iter().cloned());
    let closure = collect_required(&all_roots, catalog);
    let root_ids: HashSet<&str> = all_roots.iter().map(|r| r.project_id.as_str()).collect();
    let mut out: Vec<ReviewCandidate> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();

    let push = |content: ResolvedContent,
                dtype: &str,
                selected: bool,
                parent: Option<&str>,
                requires_parent: bool,
                out: &mut Vec<ReviewCandidate>,
                index: &mut HashMap<String, usize>| {
        if let Some(&i) = index.get(&content.project_id) {
            // Merge parents; required outranks optional.
            if let Some(p) = parent {
                if !out[i].parents.iter().any(|x| x == p) {
                    out[i].parents.push(p.to_string());
                }
            }
            if dtype == "required" && out[i].dependency_type != "required" {
                out[i].dependency_type = "required".to_string();
                out[i].selected = true;
            }
            if requires_parent {
                out[i].requires_parent = true;
            }
            return;
        }
        let mut parents = Vec::new();
        if let Some(p) = parent {
            parents.push(p.to_string());
        }
        index.insert(content.project_id.clone(), out.len());
        out.push(ReviewCandidate {
            content,
            dependency_type: dtype.to_string(),
            selected,
            parents,
            requires_parent,
        });
    };

    // 1. Required closure (minus roots), each parented by its requirer.
    for item in &closure {
        if root_ids.contains(item.project_id.as_str()) {
            continue;
        }
        let parent = closure
            .iter()
            .find(|p| {
                p.version
                    .as_ref()
                    .map(|v| {
                        v.dependencies.iter().any(|d| {
                            d.dependency_type == "required"
                                && d.project_id.as_deref() == Some(item.project_id.as_str())
                        })
                    })
                    .unwrap_or(false)
            })
            .map(|p| p.project_id.clone());
        let content = item.clone();
        push(
            content,
            "required",
            true,
            parent.as_deref(),
            false,
            &mut out,
            &mut index,
        );
    }

    // 2. Optional deps of roots + required closure, unchecked.
    let mut optional_ids: Vec<(String, String)> = Vec::new();
    for item in &closure {
        if let Some(v) = &item.version {
            for d in &v.dependencies {
                if d.dependency_type != "optional" {
                    continue;
                }
                if let Some(id) = d.project_id.clone() {
                    if !id.is_empty() {
                        optional_ids.push((id, item.project_id.clone()));
                    }
                }
            }
        }
    }
    for (id, parent) in &optional_ids {
        if index.contains_key(id) {
            if let Some(c) = out.iter_mut().find(|c| c.content.project_id == *id) {
                if !c.parents.iter().any(|x| x == parent) {
                    c.parents.push(parent.clone());
                }
            }
            continue;
        }
        let content = catalog.get(id).cloned().unwrap_or_else(|| stub_for(id));
        push(
            content,
            "optional",
            false,
            Some(parent),
            false,
            &mut out,
            &mut index,
        );
    }

    // 3. Required chain under each checked optional (cycle-safe).
    for opt in selected_optional {
        for item in collect_required(std::slice::from_ref(opt), catalog) {
            if item.project_id == opt.project_id {
                continue;
            }
            push(
                item,
                "required",
                true,
                Some(&opt.project_id),
                true,
                &mut out,
                &mut index,
            );
        }
    }

    out
}

#[cfg(test)]
pub(crate) mod tests;
