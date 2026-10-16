//! Orchestration tests over an in-memory FakeProvider (zero network).
//! Call-shape assertions follow the old `FakeModrinthClient.calls` style.

use super::*;
use crate::providers::{
    HashAlgo, LatestFilter, Project, ProjectFile, SearchPage, SearchQuery, Version,
};
use crate::resolver::graph::tests::{file, resolved, version};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Call {
    method: &'static str,
    arg: String,
}

struct FakeProvider {
    calls: Mutex<Vec<Call>>,
    projects: HashMap<String, Project>,
    versions: HashMap<String, Vec<Version>>,
    hashes: HashMap<String, Version>,
    fail_projects: HashSet<String>,
}

impl FakeProvider {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            projects: HashMap::new(),
            versions: HashMap::new(),
            hashes: HashMap::new(),
            fail_projects: HashSet::new(),
        }
    }

    fn with_project(mut self, p: Project, vs: Vec<Version>) -> Self {
        for v in &vs {
            for f in &v.files {
                if let Some(h) = &f.sha1 {
                    self.hashes.insert(h.to_lowercase(), v.clone());
                }
            }
        }
        self.versions.insert(p.id.clone(), vs);
        self.projects.insert(p.id.clone(), p);
        self
    }

    fn calls(&self, method: &str) -> Vec<String> {
        self.calls
            .lock()
            .expect("calls")
            .iter()
            .filter(|c| c.method == method)
            .map(|c| c.arg.clone())
            .collect()
    }

    fn push(&self, method: &'static str, arg: String) {
        self.calls.lock().expect("calls").push(Call { method, arg });
    }
}

#[async_trait::async_trait]
impl Provider for FakeProvider {
    fn provider_id(&self) -> &'static str {
        "fake"
    }

    async fn search(
        &self,
        _q: &SearchQuery,
    ) -> Result<SearchPage, crate::providers::ProviderError> {
        Ok(SearchPage::default())
    }

    async fn get_project(
        &self,
        id_or_slug: &str,
    ) -> Result<Project, crate::providers::ProviderError> {
        self.push("get_project", id_or_slug.to_string());
        if self.fail_projects.contains(id_or_slug) {
            return Err(crate::providers::ProviderError::Transport);
        }
        self.projects
            .get(id_or_slug)
            .cloned()
            .ok_or(crate::providers::ProviderError::Api { status: 404 })
    }

    async fn get_version(&self, id: &str) -> Result<Version, crate::providers::ProviderError> {
        self.push("get_version", id.to_string());
        self.versions
            .values()
            .flatten()
            .find(|v| v.id == id)
            .cloned()
            .ok_or(crate::providers::ProviderError::Api { status: 404 })
    }

    async fn get_versions(
        &self,
        id_or_slug: &str,
        game_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Vec<Version>, crate::providers::ProviderError> {
        self.push(
            "get_versions",
            format!(
                "{id_or_slug}|{}|{}",
                game_version.unwrap_or(""),
                loader.unwrap_or("")
            ),
        );
        // Server-like: a project's listing only carries its own versions.
        Ok(self
            .versions
            .get(id_or_slug)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|v| v.project_id == id_or_slug)
            .collect())
    }

    async fn get_versions_by_hashes(
        &self,
        hashes: &[String],
        _algo: HashAlgo,
        _latest: Option<&LatestFilter>,
    ) -> Result<HashMap<String, Version>, crate::providers::ProviderError> {
        self.push("get_versions_by_hashes", hashes.join(","));
        Ok(hashes
            .iter()
            .filter_map(|h| {
                self.hashes
                    .get(&h.to_lowercase())
                    .map(|v| (h.to_lowercase(), v.clone()))
            })
            .collect())
    }
}

fn full_project(id: &str) -> Project {
    Project {
        id: id.to_string(),
        slug: Some(id.to_string()),
        title: format!("Title {id}"),
        description: "d".to_string(),
        author: "a".to_string(),
        project_type: "mod".to_string(),
        categories: vec!["fabric".to_string()],
        versions: vec!["v1".to_string()],
        downloads: 10,
        icon_url: None,
    }
}

fn stub_project(id: &str) -> Project {
    // Empty metadata signals "needs hydrate" (spec §4-1 step 1).
    Project {
        id: id.to_string(),
        slug: Some(id.to_string()),
        title: format!("Title {id}"),
        description: String::new(),
        author: String::new(),
        project_type: String::new(),
        categories: Vec::new(),
        versions: Vec::new(),
        downloads: 0,
        icon_url: None,
    }
}

fn params(project: Project, ptype: &str) -> ResolveParams {
    ResolveParams {
        project,
        project_type: ptype.to_string(),
        game_version: "1.21".to_string(),
        loader: "fabric".to_string(),
        allow_alpha: false,
        pin: PreferredPin::default(),
    }
}

fn cache() -> RunCache {
    RunCache::new(true)
}

#[tokio::test]
async fn spec_f056_preferred_id_skips_candidate_loop() {
    let v1 = version("v1", "a");
    let v2 = version("v2", "a");
    let p = FakeProvider::new().with_project(full_project("a"), vec![v1, v2]);
    let mut c = cache();
    let mut ps = params(full_project("a"), "mod");
    ps.pin.version_id = Some("v2".to_string());
    let r = resolve_project(&p, &mut c, &ps).await;
    assert_eq!(r.status, "compatible");
    assert_eq!(r.version.as_ref().expect("v").id, "v2");
    // Single-version endpoint, no list scan, no per-candidate refetch.
    assert_eq!(p.calls("get_version"), vec!["v2".to_string()]);
    assert!(p.calls("get_versions").is_empty());
}

#[tokio::test]
async fn spec_f056_unknown_pin_falls_back_to_list_scan() {
    let p = FakeProvider::new().with_project(full_project("a"), vec![version("v1", "a")]);
    let mut c = cache();
    let mut ps = params(full_project("a"), "mod");
    ps.pin.version_id = Some("nope".to_string());
    let r = resolve_project(&p, &mut c, &ps).await;
    assert_eq!(r.status, "compatible");
    assert_eq!(r.version.as_ref().expect("v").id, "v1");
    assert_eq!(p.calls("get_version"), vec!["nope".to_string()]);
    assert!(!p.calls("get_versions").is_empty());
}

#[tokio::test]
async fn spec_f056_sha1_fallback_uses_hash_lookup() {
    let mut v2 = version("v2", "a");
    v2.files = vec![ProjectFile {
        sha1: Some("abc123".to_string()),
        ..file("y.jar", true)
    }];
    let p = FakeProvider::new().with_project(full_project("a"), vec![version("v1", "a"), v2]);
    let mut c = cache();
    let mut ps = params(full_project("a"), "mod");
    ps.pin.file_sha1 = Some("ABC123".to_string());
    let r = resolve_project(&p, &mut c, &ps).await;
    assert_eq!(r.version.as_ref().expect("v").id, "v2");
    // The request carries the lowered hash (providers bite contract).
    assert_eq!(
        p.calls("get_versions_by_hashes"),
        vec!["abc123".to_string()]
    );
}

#[tokio::test]
async fn spec_f057_incompatible_pin_falls_back_to_latest() {
    let mut pinned = version("pinned", "a");
    pinned.game_versions = vec!["1.20".to_string()];
    let p = FakeProvider::new().with_project(full_project("a"), vec![pinned, version("good", "a")]);
    let mut c = cache();
    let mut ps = params(full_project("a"), "mod");
    ps.pin.version_id = Some("pinned".to_string());
    let r = resolve_project(&p, &mut c, &ps).await;
    assert_eq!(r.status, "compatible");
    assert_eq!(r.version.as_ref().expect("v").id, "good");
}

#[tokio::test]
async fn spec_f051_no_forge_call_for_fabric_mod() {
    let p = FakeProvider::new().with_project(full_project("a"), vec![version("v1", "a")]);
    let mut c = cache();
    let r = resolve_project(&p, &mut c, &params(full_project("a"), "mod")).await;
    assert_eq!(r.status, "compatible");
    let calls = p.calls("get_versions");
    assert_eq!(calls.len(), 1);
    assert!(
        !calls.iter().any(|a| a.contains("forge")),
        "no forge call: {calls:?}"
    );
}

#[tokio::test]
async fn spec_f058_cache_reuses_duplicate_resolve() {
    // Alpha-only listing: channel flips the verdict without refetching.
    let mut alpha = version("av", "a");
    alpha.version_type = "alpha".to_string();
    let p = FakeProvider::new().with_project(full_project("a"), vec![alpha]);
    let mut c = cache();
    let ps = params(full_project("a"), "mod");
    let r1 = resolve_project(&p, &mut c, &ps).await;
    let r2 = resolve_project(&p, &mut c, &ps).await;
    assert_eq!(r1, r2);
    assert_eq!(r1.status, "incompatible");
    assert_eq!(p.calls("get_versions").len(), 1);
    // allow_alpha flips the resolved key: new verdict, same fetched list.
    let mut ps2 = ps.clone();
    ps2.allow_alpha = true;
    let r3 = resolve_project(&p, &mut c, &ps2).await;
    assert_eq!(r3.status, "compatible");
    assert_eq!(p.calls("get_versions").len(), 1);
}

#[tokio::test]
async fn spec_f050_browse_only_yields_version_none() {
    let p = FakeProvider::new().with_project(full_project("s"), vec![version("v", "s")]);
    let mut c = cache();
    let r = resolve_project(
        &p,
        &mut c,
        &params(full_project("s"), "minecraft_java_server"),
    )
    .await;
    assert_eq!(r.status, "browse-only");
    assert_eq!(r.version, None);
    assert_eq!(p.calls("get_versions").len(), 0);
}

#[tokio::test]
async fn spec_f050_hydrate_failure_returns_stub_silently() {
    // Stub project + failing fetch: never raises, returns the input.
    let mut p = FakeProvider::new();
    p.fail_projects.insert("a".to_string());
    let mut c = cache();
    let r = resolve_project(&p, &mut c, &params(stub_project("a"), "mod")).await;
    assert_eq!(r.project_id, "a");
    assert_eq!(r.status, "incompatible");
}

#[tokio::test]
async fn spec_f053_required_dep_version_pin_respected() {
    let dep_v = version("dv", "dep");
    let p = FakeProvider::new()
        .with_project(full_project("a"), vec![version("v1", "a")])
        .with_project(full_project("dep"), vec![dep_v]);
    let mut c = cache();
    let dep = VersionDependency {
        project_id: Some("dep".to_string()),
        version_id: Some("dv".to_string()),
        file_name: None,
        dependency_type: "required".to_string(),
    };
    let r = resolve_dependency(&p, &mut c, &dep, "1.21", "fabric", &HashMap::new())
        .await
        .expect("resolved");
    assert_eq!(r.status, "compatible");
    assert_eq!(r.version.as_ref().expect("v").id, "dv");
}

#[tokio::test]
async fn spec_f053_dep_fetch_failure_is_stub() {
    let mut p = FakeProvider::new();
    p.fail_projects.insert("ghost".to_string());
    let mut c = cache();
    let dep = VersionDependency {
        project_id: Some("ghost".to_string()),
        version_id: None,
        file_name: None,
        dependency_type: "required".to_string(),
    };
    let r = resolve_dependency(&p, &mut c, &dep, "1.21", "fabric", &HashMap::new())
        .await
        .expect("stub");
    assert_eq!(r.status, "dependency-error");
    assert_eq!(r.title, "Dependency ghost");
    assert!(!r.reason.is_empty());
}

#[tokio::test]
async fn spec_f053_dep_pin_project_mismatch_swallowed() {
    // Pinned version belongs to another project: fall through, never error.
    let mut foreign = version("fv", "other");
    foreign.project_id = "other".to_string();
    let mut p = FakeProvider::new().with_project(full_project("dep"), vec![foreign]);
    p.projects
        .insert("other".to_string(), full_project("other"));
    let mut c = cache();
    let dep = VersionDependency {
        project_id: Some("dep".to_string()),
        version_id: Some("fv".to_string()),
        file_name: None,
        dependency_type: "required".to_string(),
    };
    let r = resolve_dependency(&p, &mut c, &dep, "1.21", "fabric", &HashMap::new())
        .await
        .expect("fallback");
    // "fv" is fabric/1.21-compatible but belongs to "other": skipped, and no
    // other dep version exists -> incompatible, not an error.
    assert_eq!(r.status, "incompatible");
}

#[tokio::test]
async fn spec_f053_dep_ignores_allow_alpha() {
    // Dep offers alpha only: method 3 resolves stable-only -> incompatible
    // even though a parent could allow alpha (not propagated).
    let mut alpha = version("av", "dep");
    alpha.version_type = "alpha".to_string();
    let p = FakeProvider::new().with_project(full_project("dep"), vec![alpha]);
    let mut c = cache();
    let dep = VersionDependency {
        project_id: Some("dep".to_string()),
        version_id: None,
        file_name: None,
        dependency_type: "required".to_string(),
    };
    let r = resolve_dependency(&p, &mut c, &dep, "1.21", "fabric", &HashMap::new())
        .await
        .expect("resolved");
    assert_eq!(r.status, "incompatible");
}

#[test]
fn spec_f050_incompatible_reason_names_game_and_loader() {
    let v = version("v1", "a");
    let r = resolved("a", Some(v));
    let _ = r;
    // Reason-string contract lives in resolve tests viatoml-table below;
    // this guards the exact fallback wording for game+loader.
    let msg = super::not_found_reason("mod", "1.21", "fabric", &["fabric".to_string()]);
    assert!(msg.contains("1.21"), "{msg}");
    assert!(msg.contains("fabric"), "{msg}");
}
