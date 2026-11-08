//! Async orchestration over [`Provider`]: hydrate, preferred pins,
//! candidate-loader loop, and single-dependency resolution.
//!
//! The [`Provider`] trait is unchanged: pin-by-id scans one unfiltered
//! `get_versions` call instead of a dedicated `get_version` endpoint.
//! `shortcut: revisit if a second provider needs direct version fetch.`

use super::cache::{BestKey, HashKey, ResolveKey, RunCache};
use super::graph::{
    candidate_loaders_for_project_type, compatibility_issue, primary_file, select_best_version,
    ResolvedContent,
};
use crate::providers::{HashAlgo, Project, Provider, ProviderError, Version, VersionDependency};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

/// A resolution that could not produce a verdict.
///
/// A provider failure is not a version verdict. Turning it into an empty map
/// or list makes the resolver answer "no match" and then substitute a
/// different version, which is exactly what a pin must prevent. The failing
/// operation and target stay attached so the caller can name what broke
/// instead of returning a generic error.
#[derive(Debug, Error)]
#[error("{operation} failed for {target}: {source}")]
pub struct ResolveError {
    pub operation: &'static str,
    pub target: String,
    #[source]
    pub source: ProviderError,
}

impl ResolveError {
    pub fn provider(
        operation: &'static str,
        target: impl Into<String>,
        source: ProviderError,
    ) -> Self {
        Self {
            operation,
            target: target.into(),
            source,
        }
    }

    /// The provider failure as the IPC layer knows it (`F-019`).
    pub fn provider_error(&self) -> &ProviderError {
        &self.source
    }
}

/// Project types with no downloadable versions (spec §4-1 step 2).
const BROWSE_ONLY: &[&str] = &["minecraft_java_server"];
/// Types allowed the last-resort `"any"` loader pass (spec §4-1 step 5).
const LAST_RESORT_ANY: &[&str] = &["shader", "resourcepack", "datapack"];

/// Preferred-pin inputs (spec §4-1 step 3: id → sha1 → number).
#[derive(Debug, Clone, Default)]
pub struct PreferredPin {
    pub version_id: Option<String>,
    pub version_number: Option<String>,
    pub file_name: Option<String>,
    pub file_sha1: Option<String>,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ResolveParams {
    pub project: Project,
    pub project_type: String,
    pub game_version: String,
    pub loader: String,
    pub allow_alpha: bool,
    pub pin: PreferredPin,
}

fn or_any(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value.trim().to_string()
    }
}

fn resolve_key(params: &ResolveParams) -> ResolveKey {
    let sha = params
        .pin
        .file_sha1
        .clone()
        .unwrap_or_default()
        .to_lowercase();
    ResolveKey {
        project_id: params.project.id.clone(),
        slug: params.project.slug.clone().unwrap_or_default(),
        project_type: or_any(&params.project_type, "mod"),
        game_version: or_any(&params.game_version, "any"),
        loader: or_any(&params.loader, "any"),
        allow_alpha: params.allow_alpha,
        preferred_version_id: params.pin.version_id.clone().unwrap_or_default(),
        preferred_version_number: params.pin.version_number.clone().unwrap_or_default(),
        preferred_file_name: params.pin.file_name.clone().unwrap_or_default(),
        preferred_file_sha1: sha,
        preferred_game_versions: params.pin.game_versions.clone(),
        preferred_loaders: params.pin.loaders.clone(),
    }
}

/// Empty metadata signals "needs hydrate" (spec §4-1 step 1).
fn is_stub(project: &Project) -> bool {
    project.description.is_empty() && project.categories.is_empty() && project.versions.is_empty()
}

/// Hydrate that never raises: any failure returns the input untouched,
/// and failures are never cached (a 500 must not poison the run).
async fn hydrate_project(
    provider: &dyn Provider,
    cache: &mut RunCache,
    project: &Project,
) -> Project {
    if !is_stub(project) {
        return project.clone();
    }
    if let Some(hit) = cache.project(&project.id) {
        return hit;
    }
    let mut ids = vec![project.id.clone()];
    if let Some(slug) = project.slug.as_deref() {
        if !slug.is_empty() && slug != project.id {
            ids.push(slug.to_string());
        }
    }
    for id in ids {
        if let Ok(full) = provider.get_project(&id).await {
            let full = Project {
                project_type: or_any(&full.project_type, "mod"),
                ..full
            };
            cache.put_project(full.clone());
            return full;
        }
    }
    project.clone()
}

/// Cached `get_versions`. A provider failure is returned, never cached: it is
/// not "this project has no versions" and must not become an `incompatible`
/// verdict.
async fn versions_cached(
    provider: &dyn Provider,
    cache: &mut RunCache,
    id: &str,
    game: Option<&str>,
    loader: Option<&str>,
) -> Result<Vec<Version>, ProviderError> {
    let (g, l) = (game.unwrap_or(""), loader.unwrap_or(""));
    if let Some(hit) = cache.project_versions(id, g, l) {
        return Ok(hit);
    }
    let vs = provider.get_versions(id, game, loader).await?;
    cache.put_project_versions(id, g, l, vs.clone());
    Ok(vs)
}

fn compatible(content: &Project, ptype: &str, version: &Version) -> ResolvedContent {
    ResolvedContent {
        project_id: content.id.clone(),
        slug: content.slug.clone().unwrap_or_default(),
        title: content.title.clone(),
        project_type: ptype.to_string(),
        version: Some(version.clone()),
        status: "compatible".to_string(),
        reason: String::new(),
    }
}

fn blocked(content: &Project, ptype: &str, status: &str, reason: String) -> ResolvedContent {
    ResolvedContent {
        project_id: content.id.clone(),
        slug: content.slug.clone().unwrap_or_default(),
        title: content.title.clone(),
        project_type: ptype.to_string(),
        version: None,
        status: status.to_string(),
        reason,
    }
}

/// A hash pin the provider cannot answer. This stays a visible block, with the
/// provider's own error in the reason: the plan will show it as a warning, and
/// no substitute version is chosen.
fn unsupported_pin(
    content: &Project,
    ptype: &str,
    sha1: &str,
    provider_id: &str,
    error: &ProviderError,
) -> ResolvedContent {
    blocked(
        content,
        ptype,
        "unsupported",
        format!(
            "{provider_id} cannot resolve a version by SHA-1: {error}. The pinned file {sha1} was not checked, and no other version was substituted."
        ),
    )
}

/// Type-specific no-result reason (spec §4-2). Shader/resource-pack
/// wordings name the game + loader; exact legacy phrasing of the
/// surrounding sentence is reconstructed, not byte-verified.
pub fn not_found_reason(
    project_type: &str,
    game_version: &str,
    loader: &str,
    candidates: &[String],
) -> String {
    let target = if loader.is_empty() || loader == "any" {
        format!("Minecraft {game_version}")
    } else {
        format!("Minecraft {game_version} + {loader}")
    };
    match project_type {
        "shader" => format!(
            "No listed downloadable version for {target} resolved by shader platform ({}), not by Forge/NeoForge directly.",
            candidates.join(", ")
        ),
        "resourcepack" => format!(
            "No listed downloadable version for {target} resolved by resource-pack platform ({}), not by Forge/NeoForge directly.",
            candidates.join(", ")
        ),
        _ => format!("No listed downloadable version for {target}"),
    }
}

/// Seven-step project resolution (spec §4-1). Pure coordination around
/// [`Provider`] + [`RunCache`]. A provider failure on a required fetch is
/// returned as [`ResolveError`]; an unsupported hash search is a visible
/// `unsupported` block carrying the provider's error; only a real "no
/// compatible version" answer becomes an `incompatible` verdict.
pub async fn resolve_project(
    provider: &dyn Provider,
    cache: &mut RunCache,
    params: &ResolveParams,
) -> Result<ResolvedContent, ResolveError> {
    let key = resolve_key(params);
    if let Some(hit) = cache.resolved(&key) {
        return Ok(hit);
    }
    let project = hydrate_project(provider, cache, &params.project).await;
    let ptype = or_any(
        if params.project_type.trim().is_empty() {
            project.project_type.as_str()
        } else {
            params.project_type.as_str()
        },
        "mod",
    )
    .to_lowercase();
    let game = params.game_version.clone();
    let loader = params.loader.clone();

    if BROWSE_ONLY.contains(&ptype.as_str()) {
        let r = blocked(
            &project,
            &ptype,
            "browse-only",
            "This project is browse-only and has no downloadable version.".to_string(),
        );
        cache.put_resolved(key, r.clone());
        return Ok(r);
    }

    let candidates = candidate_loaders_for_project_type(&ptype, &loader);
    let finish = |cache: &mut RunCache, r: ResolvedContent| {
        cache.put_resolved(key.clone(), r.clone());
        r
    };

    // Step 3: preferred pins (id → sha1 → number). The id pin tries the
    // single-version endpoint first (963 B vs ~1.2 MB list); any miss
    // falls back to the list scan below.
    if params.pin.version_id.is_some()
        || params.pin.file_sha1.is_some()
        || params.pin.version_number.is_some()
    {
        if let Some(id) = params.pin.version_id.as_deref() {
            if let Ok(v) = provider.get_version(id).await {
                if v.project_id == project.id
                    && (params.allow_alpha || v.version_type.to_lowercase() != "alpha")
                    && compatibility_issue(Some(&v), &game, &candidates).is_none()
                {
                    return Ok(finish(cache, compatible(&project, &ptype, &v)));
                }
            }
        }
        let full = if params.pin.version_id.is_some() || params.pin.version_number.is_some() {
            versions_cached(provider, cache, &project.id, None, None)
                .await
                .map_err(|source| {
                    ResolveError::provider("get_versions", project.id.clone(), source)
                })?
        } else {
            Vec::new()
        };
        if let Some(id) = params.pin.version_id.as_deref() {
            if let Some(v) = full.iter().find(|v| {
                v.id == id
                    && (params.allow_alpha || v.version_type.to_lowercase() != "alpha")
                    && compatibility_issue(Some(v), &game, &candidates).is_none()
            }) {
                return Ok(finish(cache, compatible(&project, &ptype, v)));
            }
        }
        if let Some(sha) = params.pin.file_sha1.as_deref() {
            match hash_lookup(provider, cache, &project.id, sha, &game, &candidates).await {
                Ok(Some(v)) => return Ok(finish(cache, compatible(&project, &ptype, &v))),
                // A real "no version has this hash" answer: the existing
                // fallback may still resolve a compatible version.
                Ok(None) => {}
                // The provider cannot answer at all. That is not "no match":
                // block the entry and carry the provider's error into the
                // plan warning instead of substituting another version.
                Err(source @ ProviderError::Unsupported { .. }) => {
                    let r = unsupported_pin(&project, &ptype, sha, provider.provider_id(), &source);
                    return Ok(finish(cache, r));
                }
                Err(source) => {
                    return Err(ResolveError::provider(
                        "get_versions_by_hashes",
                        project.id.clone(),
                        source,
                    ));
                }
            }
        }
        if let Some(num) = params.pin.version_number.as_deref() {
            if let Some(v) = full.iter().find(|v| {
                v.version_number == num
                    && (params.allow_alpha || v.version_type.to_lowercase() != "alpha")
                    && compatibility_issue(Some(v), &game, &candidates).is_none()
            }) {
                return Ok(finish(cache, compatible(&project, &ptype, v)));
            }
        }
    }

    // Step 4: candidate-loader loop, first hit wins.
    let mut tried: HashSet<String> = HashSet::new();
    for cand in &candidates {
        if !tried.insert(cand.clone()) {
            continue;
        }
        let bkey = BestKey {
            project_id: project.id.clone(),
            game_version: game.clone(),
            loader: cand.clone(),
            allow_alpha: params.allow_alpha,
        };
        if let Some(hit) = cache.best(&bkey) {
            if let Some(v) = hit {
                return Ok(finish(cache, compatible(&project, &ptype, &v)));
            }
            continue;
        }
        let (g, l) = (
            if game.is_empty() || game == "any" {
                None
            } else {
                Some(game.as_str())
            },
            if cand.is_empty() || cand == "any" {
                None
            } else {
                Some(cand.as_str())
            },
        );
        let vs = versions_cached(provider, cache, &project.id, g, l)
            .await
            .map_err(|source| ResolveError::provider("get_versions", project.id.clone(), source))?;
        let pick = select_best_version(&vs, &game, std::slice::from_ref(cand), params.allow_alpha)
            .cloned();
        cache.put_best(bkey, pick.clone());
        if let Some(v) = pick {
            return Ok(finish(cache, compatible(&project, &ptype, &v)));
        }
    }

    // Step 5: last-resort "any" pass for non-mod content only.
    if LAST_RESORT_ANY.contains(&ptype.as_str()) && !tried.contains("any") {
        let g = if game.is_empty() || game == "any" {
            None
        } else {
            Some(game.as_str())
        };
        let vs = versions_cached(provider, cache, &project.id, g, None)
            .await
            .map_err(|source| ResolveError::provider("get_versions", project.id.clone(), source))?;
        if let Some(v) = select_best_version(&vs, &game, &["any".to_string()], params.allow_alpha) {
            return Ok(finish(cache, compatible(&project, &ptype, v)));
        }
    }

    // Step 6: incompatible with a type-specific reason.
    Ok(finish(
        cache,
        ResolvedContent {
            project_id: project.id.clone(),
            slug: project.slug.clone().unwrap_or_default(),
            title: project.title.clone(),
            project_type: ptype.clone(),
            version: None,
            status: "incompatible".to_string(),
            reason: not_found_reason(&ptype, &game, &loader, &candidates),
        },
    ))
}

/// SHA-1 pin via `get_versions_by_hashes` (spec §4-1 step 3b): the mapped
/// version must belong to the requesting project and be compatible. A
/// provider failure is returned, never turned into an empty map: `None` here
/// means the provider answered and no version matched.
async fn hash_lookup(
    provider: &dyn Provider,
    cache: &mut RunCache,
    project_id: &str,
    sha1: &str,
    game: &str,
    candidates: &[String],
) -> Result<Option<Version>, ProviderError> {
    let want = sha1.to_lowercase();
    let hkey = match HashKey::new(HashAlgo::Sha1, std::slice::from_ref(&want)) {
        Some(key) => key,
        None => return Ok(None),
    };
    let map = match cache.hash_versions(&hkey) {
        Some(m) => m,
        None => {
            let m = provider
                .get_versions_by_hashes(std::slice::from_ref(&want), HashAlgo::Sha1, None)
                .await?;
            cache.put_hash_versions(hkey, m.clone());
            m
        }
    };
    Ok(map
        .get(&want)
        .filter(|v| {
            v.project_id == project_id && compatibility_issue(Some(v), game, candidates).is_none()
        })
        .cloned())
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

/// A dependency failure stays visible in the review row: one broken dep must
/// not abort the whole plan, but it must not masquerade as "no compatible
/// version" either.
fn dependency_stub(id: &str, reason: String) -> ResolvedContent {
    let mut stub = stub_for(id);
    stub.reason = reason;
    stub
}

/// Single-dependency resolution (spec §5-1): pinned `version_id` →
/// pinned `file_name` → full resolve **without** `allow_alpha`.
/// `None` = skipped dep without a `project_id` (spec §5-2); provider
/// failures become `dependency-error` stubs carrying the typed error in
/// their reason, never a substitute version and never a whole-plan abort.
pub async fn resolve_dependency(
    provider: &dyn Provider,
    cache: &mut RunCache,
    dep: &VersionDependency,
    game_version: &str,
    loader: &str,
    loader_overrides: &HashMap<String, String>,
) -> Option<ResolvedContent> {
    let dep_id = dep.project_id.clone().unwrap_or_default();
    if dep_id.trim().is_empty() {
        return None;
    }
    let project = match cache.project(&dep_id) {
        Some(p) => p,
        None => match provider.get_project(&dep_id).await {
            Ok(p) => {
                let p = Project {
                    project_type: or_any(&p.project_type, "mod"),
                    ..p
                };
                cache.put_project(p.clone());
                p
            }
            // Typed error text only — response bodies never cross (F-019).
            Err(e) => {
                return Some(dependency_stub(
                    &dep_id,
                    format!("Could not fetch dependency project metadata ({e})."),
                ));
            }
        },
    };
    let ptype = or_any(&project.project_type, "mod").to_lowercase();
    let eff = loader_overrides
        .get(&ptype)
        .map(String::as_str)
        .unwrap_or(loader);
    let candidates = candidate_loaders_for_project_type(&ptype, eff);
    let game = game_version.to_string();
    let pinned_reason = format!(
        "Using {} dependency version pinned by Modrinth metadata.",
        dep.dependency_type
    );

    // Method 1: pinned version id via the single-version endpoint; a
    // miss, mismatch, or incompatibility falls back to the list scan.
    if let Some(vid) = dep.version_id.as_deref() {
        if let Ok(v) = provider.get_version(vid).await {
            if v.project_id == dep_id && compatibility_issue(Some(&v), &game, &candidates).is_none()
            {
                let mut r = compatible(&project, &ptype, &v);
                r.reason = pinned_reason.clone();
                return Some(r);
            }
        }
        let full = match versions_cached(provider, cache, &dep_id, None, None).await {
            Ok(vs) => vs,
            Err(e) => {
                return Some(dependency_stub(
                    &dep_id,
                    format!("Could not fetch dependency versions ({e})."),
                ));
            }
        };
        if let Some(v) = full.iter().find(|v| v.id == vid) {
            if v.project_id == dep_id && compatibility_issue(Some(v), &game, &candidates).is_none()
            {
                let mut r = compatible(&project, &ptype, v);
                r.reason = pinned_reason.clone();
                return Some(r);
            }
        }
    }

    // Method 2: pinned file name over the unfiltered-per-loader list.
    if let Some(name) = dep.file_name.as_deref() {
        let g = if game.is_empty() || game == "any" {
            None
        } else {
            Some(game.as_str())
        };
        let vs = match versions_cached(provider, cache, &dep_id, g, Some("any")).await {
            Ok(vs) => vs,
            Err(e) => {
                return Some(dependency_stub(
                    &dep_id,
                    format!("Could not fetch dependency versions ({e})."),
                ));
            }
        };
        if let Some(v) = vs.iter().find(|v| {
            primary_file(v).map(|f| f.filename == name).unwrap_or(false)
                && compatibility_issue(Some(v), &game, &candidates).is_none()
        }) {
            let mut r = compatible(&project, &ptype, v);
            r.reason = pinned_reason.clone();
            return Some(r);
        }
    }

    // Method 3: full resolve, stable channel only.
    Some(
        match resolve_project(
            provider,
            cache,
            &ResolveParams {
                project,
                project_type: ptype,
                game_version: game,
                loader: eff.to_string(),
                allow_alpha: false,
                pin: PreferredPin::default(),
            },
        )
        .await
        {
            Ok(r) => r,
            Err(e) => dependency_stub(&dep_id, format!("Could not resolve dependency ({e}).")),
        },
    )
}

#[cfg(test)]
pub(crate) mod tests;
