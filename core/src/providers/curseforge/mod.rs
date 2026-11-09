//! CurseForge provider (API v1, Minecraft mods only).
//!
//! ADR 0004 §2-2 is the contract: the captured surface in
//! `docs/spec/fixtures.md` §7 is the whole API we use. A capability that is
//! absent (`get_versions_by_hashes`, slug lookup, `get_version` without a
//! project id) is `Unsupported`, never `Ok(empty)` — “I cannot answer” is not
//! “there is no match”.
//!
//! Every request carries `x-api-key`; the key never enters a URL, a log line
//! or an error message. Error bodies are empty on this API (fixtures §7-7
//! هـ), so responses are classified by status only.

use super::{
    HashAlgo, LatestFilter, Project, ProjectFile, Provider, ProviderError, SearchPage, SearchQuery,
    Version, VersionDependency,
};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

pub mod secret;

pub const CURSEFORGE_BASE: &str = "https://api.curseforge.com";

/// The provider's identity, and the only place its name is written. Every
/// layer above matches on this constant instead of repeating the literal
/// (ADR 0004 §2-3).
pub const PROVIDER_ID: &str = "curseforge";

/// `gameId = 432` is Minecraft, `classId = 6` is Minecraft Mods (fixtures
/// §7-4). No other game or class id was captured; guessing one is forbidden.
const GAME_ID: u32 = 432;
const MODS_CLASS_ID: u32 = 6;

/// One `/files` page. The API's maximum is 50.
const FILES_PAGE_SIZE: u32 = 50;
// shortcut: one listing is capped at 4 pages / 200 files so a 3,946-file mod
// cannot turn one resolve into ~79 requests. Upgrade path: pass the resolver's
// game/loader filters into `get_versions` (they exist today) or add a declared
// capability for “search inside this project”; until then a pinned version id
// uses the single-file endpoint, which is not capped.
const FILES_MAX_PAGES: u32 = 4;

pub struct CurseForgeClient {
    client: reqwest::Client,
    base: String,
    api_key: String,
}

impl CurseForgeClient {
    pub fn new(api_key: &str) -> Result<Self, ProviderError> {
        Self::with_base(CURSEFORGE_BASE, api_key)
    }

    /// Custom base URL (tests and embedding point at wiremock; production
    /// uses CURSEFORGE_BASE). Only the URL is injectable; the key is not a
    /// query parameter in either mode.
    pub fn with_base(base: &str, api_key: &str) -> Result<Self, ProviderError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(crate::USER_AGENT)
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(45))
            .build()
            .map_err(|_| ProviderError::Transport)?;
        Ok(Self {
            client,
            base: base.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
        })
    }

    /// One attempt, status-only classification.
    // shortcut: there is no retry policy shared with Modrinth — this client
    // sends one attempt and surfaces 429 with its reset window. Upgrade path:
    // extract `providers/http.rs` and give both clients one measured
    // retry/backoff implementation once CurseForge limits are observed.
    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: Vec<(String, String)>,
    ) -> Result<T, ProviderError> {
        let mut request = self
            .client
            .get(format!("{}{}", self.base, path))
            .header("x-api-key", self.api_key.as_str());
        for (key, value) in &query {
            request = request.query(&[(key.as_str(), value.as_str())]);
        }
        let response = request.send().await.map_err(|e| {
            if e.is_timeout() {
                ProviderError::Timeout
            } else {
                ProviderError::Transport
            }
        })?;
        let status = response.status().as_u16();
        if status == 429 {
            // The reset header was never observed (fixtures §7-7 د); when it
            // is absent the documented fallback is 60 seconds. Bodies are
            // empty, so no body is read here.
            return Err(ProviderError::RateLimited {
                reset_seconds: reset_seconds(response.headers()),
            });
        }
        if status != 200 {
            return Err(ProviderError::Api { status });
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|_| ProviderError::Transport)?;
        serde_json::from_slice(&bytes).map_err(|_| ProviderError::InvalidJson)
    }

    /// Page through `/files`, dedup by file id, and stop on a short page or
    /// the page ceiling. `index` is a zero-based item offset, never a page
    /// number (fixtures §7-7 أ): page k sends `index = k * page_size`.
    async fn fetch_files(
        &self,
        mod_id: &str,
        game_version: Option<&str>,
        loader: Option<&str>,
        page_size: u32,
        max_pages: u32,
    ) -> Result<Vec<Version>, ProviderError> {
        let mut versions = Vec::new();
        let mut seen = HashSet::new();
        let mut index: u32 = 0;
        for _ in 0..max_pages {
            let mut query = vec![
                ("index".to_string(), index.to_string()),
                ("pageSize".to_string(), page_size.to_string()),
            ];
            push_filters(&mut query, game_version, loader);
            let page: Paginated<FileDto> = self
                .get_json(&format!("/v1/mods/{mod_id}/files"), query)
                .await?;
            let short = page.data.len() < page_size as usize;
            for file in page.data {
                // ADR §2-2-5 أ: unavailable files are excluded at the
                // provider boundary, not later.
                if !file.is_available {
                    continue;
                }
                if seen.insert(file.id) {
                    versions.push(map_file(file));
                }
            }
            if short {
                break;
            }
            index += page_size;
        }
        Ok(versions)
    }
}

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    data: T,
}

#[derive(Debug, Deserialize)]
struct Paginated<T> {
    data: Vec<T>,
    pagination: Pagination,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pagination {
    total_count: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModDto {
    id: u64,
    slug: String,
    name: String,
    summary: String,
    class_id: u32,
    #[serde(default)]
    authors: Vec<AuthorDto>,
    #[serde(default)]
    categories: Vec<CategoryDto>,
    #[serde(default)]
    latest_files_indexes: Vec<LatestFileIndexDto>,
    download_count: u64,
    logo: Option<LogoDto>,
}

#[derive(Debug, Deserialize)]
struct AuthorDto {
    name: String,
}

#[derive(Debug, Deserialize)]
struct CategoryDto {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LatestFileIndexDto {
    game_version: String,
}

#[derive(Debug, Deserialize)]
struct LogoDto {
    url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileDto {
    id: u64,
    mod_id: u64,
    is_available: bool,
    display_name: String,
    file_name: String,
    release_type: u32,
    #[serde(default)]
    hashes: Vec<FileHashDto>,
    file_date: String,
    file_length: u64,
    #[serde(default)]
    download_url: Option<String>,
    #[serde(default)]
    game_versions: Vec<String>,
    #[serde(default)]
    dependencies: Vec<DependencyDto>,
}

#[derive(Debug, Deserialize)]
struct FileHashDto {
    value: String,
    algo: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DependencyDto {
    mod_id: u64,
    relation_type: u32,
}

/// One `gameVersions[]` token after `trim().to_lowercase()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VersionToken {
    Loader,
    GameVersion,
    Dropped,
}

fn classify_token(raw: &str) -> VersionToken {
    let token = raw.trim().to_lowercase();
    match token.as_str() {
        // The loader names in `gameVersions` (fixtures §7-7 ج). `Client` and
        // `Server` are side markers, not loaders, and are dropped below.
        "fabric"
        | "forge"
        | "neoforge"
        | "quilt"
        | "rift"
        | "risugami's modloader"
        | "flint loader" => VersionToken::Loader,
        // The captured shape: starts with an ASCII digit and contains a dot
        // (`1.20.1`, `26.1`, `1.14-Snapshot`). Unknown shapes are dropped,
        // never stored in `loaders` and never a reason to reject the record.
        _ if is_version_token(&token) => VersionToken::GameVersion,
        _ => VersionToken::Dropped,
    }
}

fn is_version_token(token: &str) -> bool {
    token.as_bytes().first().is_some_and(|b| b.is_ascii_digit()) && token.contains('.')
}

fn split_tokens(tokens: &[String]) -> (Vec<String>, Vec<String>) {
    let mut game_versions = Vec::new();
    let mut loaders = Vec::new();
    for raw in tokens {
        let token = raw.trim().to_lowercase();
        match classify_token(&token) {
            VersionToken::Loader => loaders.push(token),
            VersionToken::GameVersion => game_versions.push(token),
            VersionToken::Dropped => {}
        }
    }
    (game_versions, loaders)
}

fn hash_of(hashes: &[FileHashDto], algo: u32) -> Option<String> {
    hashes
        .iter()
        .find(|hash| hash.algo == algo)
        .map(|hash| hash.value.clone())
}

fn map_file(file: FileDto) -> Version {
    let (game_versions, loaders) = split_tokens(&file.game_versions);
    Version {
        id: file.id.to_string(),
        project_id: file.mod_id.to_string(),
        name: file.display_name.clone(),
        // No independent version field: `displayName` is what the user sees.
        version_number: file.display_name,
        changelog: None,
        date_published: file.file_date,
        // No release-type mapping: every raw value is unverified (ADR
        // §2-2-5 ب), so the vocabulary is preserved as `unknown:<n>`.
        version_type: format!("unknown:{}", file.release_type),
        status: "listed".to_string(),
        files: vec![ProjectFile {
            filename: file.file_name,
            // null downloadUrl => "" ; the plan blocks it (F-017).
            url: file.download_url.unwrap_or_default(),
            size: file.file_length,
            // Only algo == 1 is SHA-1; algo == 2 is MD5 and is never passed.
            sha1: hash_of(&file.hashes, 1),
            sha512: None,
            primary: true,
        }],
        game_versions,
        loaders,
        dependencies: file
            .dependencies
            .into_iter()
            .map(|dependency| VersionDependency {
                project_id: Some(dependency.mod_id.to_string()),
                version_id: None,
                file_name: None,
                // relationType meaning is unverified (ADR §2-2-4); keep the
                // raw value and never guess `required`.
                dependency_type: format!("unknown:{}", dependency.relation_type),
            })
            .collect(),
    }
}

fn map_project(project: ModDto) -> Project {
    let mut versions = Vec::new();
    for index in &project.latest_files_indexes {
        if !versions.contains(&index.game_version) {
            versions.push(index.game_version.clone());
        }
    }
    Project {
        id: project.id.to_string(),
        slug: Some(project.slug),
        title: project.name,
        description: project.summary,
        author: project
            .authors
            .into_iter()
            .map(|author| author.name)
            .collect::<Vec<_>>()
            .join(", "),
        // This slice declares one content type only.
        project_type: "mod".to_string(),
        categories: project
            .categories
            .into_iter()
            .map(|category| category.name)
            .collect(),
        versions,
        downloads: project.download_count,
        icon_url: project
            .logo
            .map(|logo| logo.url)
            .filter(|url| !url.is_empty()),
    }
}

fn is_numeric_id(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
}

/// Loader name → `modLoader` enum. Only the four values verified in
/// fixtures §7-7 ج; anything else (including `any`) sends no parameter.
fn loader_param(loader: &str) -> Option<&'static str> {
    match loader.trim().to_lowercase().as_str() {
        "fabric" => Some("4"),
        "quilt" => Some("5"),
        "neoforge" => Some("6"),
        "forge" => Some("1"),
        _ => None,
    }
}

// shortcut: `gameVersion` and `modLoader` were not exercised in the captured
// search responses. The enum values 1/4/5/6 come from fixtures §7-7 ج, and
// the parameter names are the documented camelCase ones. Upgrade path: one
// recorded search per filter turns this into a fixture-backed test.
fn push_filters(
    query: &mut Vec<(String, String)>,
    game_version: Option<&str>,
    loader: Option<&str>,
) {
    if let Some(version) = game_version {
        if !version.is_empty() {
            query.push(("gameVersion".to_string(), version.to_string()));
        }
    }
    if let Some(loader) = loader {
        if let Some(param) = loader_param(loader) {
            query.push(("modLoader".to_string(), param.to_string()));
        }
    }
}

fn reset_seconds(headers: &reqwest::header::HeaderMap) -> u64 {
    // HeaderMap lookups are case-insensitive; the order matches the ADR
    // (X-Ratelimit-Reset first, then Retry-After), then the 60 s fallback.
    for name in ["x-ratelimit-reset", "retry-after"] {
        if let Some(value) = headers.get(name).and_then(|value| value.to_str().ok()) {
            if let Ok(seconds) = value.trim().parse::<u64>() {
                return seconds;
            }
        }
    }
    60
}

#[async_trait::async_trait]
impl Provider for CurseForgeClient {
    fn provider_id(&self) -> &'static str {
        PROVIDER_ID
    }

    async fn search(&self, q: &SearchQuery) -> Result<SearchPage, ProviderError> {
        if !matches!(q.project_type.as_str(), "" | "mod") {
            // `capability` is `&'static str`, so the rejected value cannot be
            // interpolated; the typed refusal still says which request failed.
            return Err(ProviderError::Unsupported {
                capability: "project_type",
            });
        }
        let mut query = vec![
            ("gameId".to_string(), GAME_ID.to_string()),
            ("classId".to_string(), MODS_CLASS_ID.to_string()),
            ("index".to_string(), q.offset.to_string()),
            ("pageSize".to_string(), q.limit.min(50).to_string()),
            ("sortField".to_string(), "2".to_string()),
            ("sortOrder".to_string(), "desc".to_string()),
        ];
        if !q.query.is_empty() {
            query.push(("searchFilter".to_string(), q.query.clone()));
        }
        push_filters(&mut query, q.game_version.as_deref(), q.loader.as_deref());
        let response: Paginated<ModDto> = self.get_json("/v1/mods/search", query).await?;
        Ok(SearchPage {
            total: response.pagination.total_count,
            hits: response.data.into_iter().map(map_project).collect(),
        })
    }

    async fn get_project(&self, id_or_slug: &str) -> Result<Project, ProviderError> {
        // No slug endpoint was captured; numeric project ids only.
        if !is_numeric_id(id_or_slug) {
            return Err(ProviderError::Unsupported {
                capability: "get_project by slug",
            });
        }
        let response: Envelope<ModDto> = self
            .get_json(&format!("/v1/mods/{id_or_slug}"), Vec::new())
            .await?;
        if response.data.class_id != MODS_CLASS_ID {
            return Err(ProviderError::Unsupported {
                capability: "project_type",
            });
        }
        Ok(map_project(response.data))
    }

    async fn get_version(&self, _id: &str) -> Result<Version, ProviderError> {
        // File ids are only unique inside their mod (`/mods/{mod}/files/{id}`).
        Err(ProviderError::Unsupported {
            capability: "get_version without a project id",
        })
    }

    async fn get_version_in_project(
        &self,
        project_id: &str,
        version_id: &str,
    ) -> Result<Version, ProviderError> {
        if !is_numeric_id(project_id) || !is_numeric_id(version_id) {
            return Err(ProviderError::Unsupported {
                capability: "get_version by slug",
            });
        }
        let response: Envelope<FileDto> = self
            .get_json(
                &format!("/v1/mods/{project_id}/files/{version_id}"),
                Vec::new(),
            )
            .await?;
        // The response must be the file of the project that was asked for.
        // A mismatched modId (or an unavailable file) is not this project's
        // version; returning it would let the resolver accept something it
        // never asked for.
        if response.data.mod_id.to_string() != project_id || !response.data.is_available {
            return Err(ProviderError::Api { status: 404 });
        }
        Ok(map_file(response.data))
    }

    async fn get_versions(
        &self,
        id_or_slug: &str,
        game_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Vec<Version>, ProviderError> {
        if !is_numeric_id(id_or_slug) {
            return Err(ProviderError::Unsupported {
                capability: "get_versions by slug",
            });
        }
        self.fetch_files(
            id_or_slug,
            game_version,
            loader,
            FILES_PAGE_SIZE,
            FILES_MAX_PAGES,
        )
        .await
    }

    async fn get_versions_by_hashes(
        &self,
        _hashes: &[String],
        _algo: HashAlgo,
        _latest: Option<&LatestFilter>,
    ) -> Result<HashMap<String, Version>, ProviderError> {
        // No hash lookup was captured; `POST /v1/fingerprints` takes u32
        // MurmurHash2 fingerprints, not SHA-1 (ADR §2-2-3).
        Err(ProviderError::Unsupported {
            capability: "get_versions_by_hashes",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param, query_param_is_missing};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SEARCH: &str = include_str!("../../../tests/fixtures/json/curseforge/mods_search.json");
    const MOD: &str = include_str!("../../../tests/fixtures/json/curseforge/mod_238222.json");
    const FILE: &str = include_str!("../../../tests/fixtures/json/curseforge/file_9277601.json");
    const FILES_PAGE_0: &str =
        include_str!("../../../tests/fixtures/json/curseforge/mod_files_page_index0.json");
    const FILES_PAGE_1: &str =
        include_str!("../../../tests/fixtures/json/curseforge/mod_files_page_index1.json");
    const FILES_PAGE_2: &str =
        include_str!("../../../tests/fixtures/json/curseforge/mod_files_page_index2.json");
    const FILES_NO_DISTRIBUTION: &str =
        include_str!("../../../tests/fixtures/json/curseforge/mod_files_no_distribution.json");

    /// All eleven captured responses, used by the classifier guard. Two of
    /// them carry no files; walking the JSON is still the honest check that
    /// the guard sees every token the capture contains.
    const ALL_FIXTURES: &[(&str, &str)] = &[
        ("mods_search.json", SEARCH),
        ("mod_238222.json", MOD),
        (
            "mod_238222_description.json",
            include_str!("../../../tests/fixtures/json/curseforge/mod_238222_description.json"),
        ),
        (
            "mod_448233_no_distribution.json",
            include_str!("../../../tests/fixtures/json/curseforge/mod_448233_no_distribution.json"),
        ),
        ("mod_files_page_index0.json", FILES_PAGE_0),
        ("mod_files_page_index1.json", FILES_PAGE_1),
        ("mod_files_page_index2.json", FILES_PAGE_2),
        ("file_9277601.json", FILE),
        ("mod_files_no_distribution.json", FILES_NO_DISTRIBUTION),
        (
            "fingerprints_request.json",
            include_str!("../../../tests/fixtures/json/curseforge/fingerprints_request.json"),
        ),
        (
            "fingerprints_response.json",
            include_str!("../../../tests/fixtures/json/curseforge/fingerprints_response.json"),
        ),
    ];

    fn test_client(server: &MockServer) -> CurseForgeClient {
        CurseForgeClient::with_base(&server.uri(), "test-key").expect("client")
    }

    fn query() -> SearchQuery {
        SearchQuery {
            query: "jei".to_string(),
            project_type: "mod".to_string(),
            limit: 2,
            offset: 0,
            ..Default::default()
        }
    }

    fn collect_tokens(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, child) in map {
                    if key == "gameVersions" {
                        if let serde_json::Value::Array(items) = child {
                            for item in items {
                                if let serde_json::Value::String(token) = item {
                                    out.push(token.clone());
                                }
                            }
                        }
                    }
                    collect_tokens(child, out);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    collect_tokens(item, out);
                }
            }
            _ => {}
        }
    }

    #[tokio::test]
    async fn spec_f014_search_maps_every_project_field() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/mods/search"))
            .and(query_param("gameId", "432"))
            .and(query_param("classId", "6"))
            .and(query_param("index", "0"))
            .and(query_param("pageSize", "2"))
            .and(query_param("sortField", "2"))
            .and(query_param("sortOrder", "desc"))
            .and(query_param("searchFilter", "jei"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SEARCH))
            .mount(&server)
            .await;

        let page = test_client(&server).search(&query()).await.expect("search");
        assert_eq!(page.total, 436);
        assert_eq!(page.hits.len(), 2);
        let hit = &page.hits[0];
        assert_eq!(hit.id, "238222");
        assert_eq!(hit.slug.as_deref(), Some("jei"));
        assert_eq!(hit.title, "Just Enough Items (JEI)");
        assert_eq!(hit.description, "View Items and Recipes");
        assert_eq!(hit.author, "Example Author 0");
        assert_eq!(hit.project_type, "mod");
        assert_eq!(
            hit.categories,
            vec![
                "Map and Information".to_string(),
                "API and Library".to_string()
            ]
        );
        // latestFilesIndexes has 1.20.1 twice; dedup keeps first-seen order.
        assert_eq!(
            hit.versions,
            vec!["1.20.1".to_string(), "1.19.2".to_string()]
        );
        assert_eq!(hit.downloads, 638527706);
        assert_eq!(
            hit.icon_url.as_deref(),
            Some("https://media.forgecdn.net/avatars/29/69/635838945588716414.jpeg")
        );
        assert_eq!(
            page.hits[1].versions,
            vec!["26.3".to_string(), "26.2".to_string()]
        );

        let requests = server.received_requests().await.expect("requests");
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0]
                .headers
                .get("x-api-key")
                .and_then(|value| value.to_str().ok()),
            Some("test-key"),
            "every request must carry x-api-key"
        );
    }

    #[tokio::test]
    async fn spec_f014_project_maps_authors_categories_and_versions() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/mods/238222"))
            .respond_with(ResponseTemplate::new(200).set_body_string(MOD))
            .mount(&server)
            .await;

        let project = test_client(&server)
            .get_project("238222")
            .await
            .expect("project");
        assert_eq!(project.id, "238222");
        assert_eq!(project.slug.as_deref(), Some("jei"));
        assert_eq!(project.title, "Just Enough Items (JEI)");
        assert_eq!(project.description, "View Items and Recipes");
        assert_eq!(project.author, "Example Author 0");
        assert_eq!(project.project_type, "mod");
        assert_eq!(
            project.categories,
            vec![
                "Map and Information".to_string(),
                "API and Library".to_string()
            ]
        );
        assert_eq!(
            project.versions,
            vec!["1.20.1".to_string(), "1.19.2".to_string()]
        );
        assert_eq!(project.downloads, 638527706);
        assert!(project.icon_url.is_some());
    }

    #[tokio::test]
    async fn spec_f014_file_maps_to_version_with_sha1_only() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/mods/238222/files/9277601"))
            .respond_with(ResponseTemplate::new(200).set_body_string(FILE))
            .mount(&server)
            .await;

        let version = test_client(&server)
            .get_version_in_project("238222", "9277601")
            .await
            .expect("version");
        assert_eq!(version.id, "9277601");
        assert_eq!(version.project_id, "238222");
        assert_eq!(version.name, "15.74.0.228 for Fabric 1.20.1");
        assert_eq!(version.version_number, "15.74.0.228 for Fabric 1.20.1");
        assert_eq!(version.changelog, None);
        assert_eq!(version.date_published, "2026-10-21T14:31:24.613Z");
        assert_eq!(version.version_type, "unknown:2");
        assert_eq!(version.status, "listed");
        assert_eq!(version.game_versions, vec!["1.20.1".to_string()]);
        assert_eq!(version.loaders, vec!["fabric".to_string()]);

        assert_eq!(version.files.len(), 1);
        let file = &version.files[0];
        assert_eq!(file.filename, "jei-1.20.1-fabric-15.74.0.228.jar");
        assert_eq!(
            file.url,
            "https://edge.forgecdn.net/files/9277/601/jei-1.20.1-fabric-15.74.0.228.jar"
        );
        assert_eq!(file.size, 6929002);
        // algo == 1 only; the algo == 2 value is MD5 and never becomes sha1.
        assert_eq!(
            file.sha1.as_deref(),
            Some("85d8e1ebf0be1c83cd21f97afe89903c8643b2c3")
        );
        assert_eq!(file.sha512, None);
        assert!(file.primary);

        assert_eq!(version.dependencies.len(), 2);
        assert_eq!(
            version.dependencies[0],
            VersionDependency {
                project_id: Some("1700987".to_string()),
                version_id: None,
                file_name: None,
                dependency_type: "unknown:2".to_string(),
            }
        );
        assert_eq!(
            version.dependencies[1],
            VersionDependency {
                project_id: Some("1689768".to_string()),
                version_id: None,
                file_name: None,
                dependency_type: "unknown:3".to_string(),
            }
        );
    }

    #[tokio::test]
    async fn spec_f014_null_download_url_is_empty_but_stays_listed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/mods/448233/files"))
            .and(query_param("index", "0"))
            .and(query_param("pageSize", "50"))
            .respond_with(ResponseTemplate::new(200).set_body_string(FILES_NO_DISTRIBUTION))
            .mount(&server)
            .await;

        let versions = test_client(&server)
            .get_versions("448233", None, None)
            .await
            .expect("versions");
        assert_eq!(versions.len(), 2);
        assert!(versions.iter().all(|version| version.status == "listed"));
        assert!(
            versions
                .iter()
                .flat_map(|version| &version.files)
                .all(|file| file.url.is_empty()),
            "a null downloadUrl becomes \"\", never dropped"
        );
        assert_eq!(
            versions[0].files[0].sha1.as_deref(),
            Some("75df6fa16449e91f9a20cd5f6ce5db9b3b972938")
        );
        assert!(versions[0].files[0].sha512.is_none());
        assert_eq!(versions[0].game_versions, vec!["26.3".to_string()]);
        assert_eq!(versions[0].loaders, vec!["neoforge".to_string()]);
    }

    #[tokio::test]
    async fn spec_f014_version_from_another_mod_is_never_returned() {
        let server = MockServer::start().await;
        // The fixture's modId is 238222, but the request names mod 999.
        Mock::given(method("GET"))
            .and(path("/v1/mods/999/files/9277601"))
            .respond_with(ResponseTemplate::new(200).set_body_string(FILE))
            .mount(&server)
            .await;

        let err = test_client(&server)
            .get_version_in_project("999", "9277601")
            .await
            .expect_err("a file from another mod must not be accepted");
        assert!(matches!(err, ProviderError::Api { status: 404 }), "{err:?}");
    }

    #[tokio::test]
    async fn spec_f014_files_pagination_uses_index_offsets_and_dedups() {
        let server = MockServer::start().await;
        for (index, body) in [
            ("0", FILES_PAGE_0),
            ("2", FILES_PAGE_1),
            ("4", FILES_PAGE_2),
        ] {
            Mock::given(method("GET"))
                .and(path("/v1/mods/238222/files"))
                .and(query_param("index", index))
                .and(query_param("pageSize", "2"))
                .respond_with(ResponseTemplate::new(200).set_body_string(body))
                .mount(&server)
                .await;
        }

        let versions = test_client(&server)
            .fetch_files("238222", None, None, 2, 3)
            .await
            .expect("pages");
        let ids: Vec<&str> = versions.iter().map(|version| version.id.as_str()).collect();
        assert_eq!(
            ids,
            ["9277601", "9277600", "9268986", "9268985"],
            "each page overlaps the previous one by a file, so dedup by id keeps four"
        );

        let requests = server.received_requests().await.expect("requests");
        assert_eq!(requests.len(), 3);
        let indexes: Vec<String> = requests
            .iter()
            .map(|request| {
                request
                    .url
                    .query_pairs()
                    .find(|(key, _)| key == "index")
                    .expect("index parameter")
                    .1
                    .into_owned()
            })
            .collect();
        assert_eq!(
            indexes,
            ["0", "2", "4"],
            "index is an item offset, never a page number"
        );
    }

    #[test]
    fn spec_f014_every_captured_game_version_token_has_one_bucket() {
        const KNOWN_LOADERS: &[&str] = &[
            "fabric",
            "forge",
            "neoforge",
            "quilt",
            "rift",
            "risugami's modloader",
            "flint loader",
        ];

        let mut tokens = Vec::new();
        for (name, body) in ALL_FIXTURES {
            let value: serde_json::Value =
                serde_json::from_str(body).unwrap_or_else(|e| panic!("{name}: {e}"));
            collect_tokens(&value, &mut tokens);
        }
        assert!(!tokens.is_empty(), "no gameVersions token was captured");

        for raw in &tokens {
            let token = raw.trim().to_lowercase();
            match classify_token(raw) {
                VersionToken::Loader => assert!(
                    KNOWN_LOADERS.contains(&token.as_str()),
                    "unknown token reached loaders: {raw}"
                ),
                VersionToken::GameVersion => assert!(
                    is_version_token(&token),
                    "game_versions received a non-version token: {raw}"
                ),
                VersionToken::Dropped => assert!(
                    token == "client" || token == "server",
                    "a captured token was dropped without a rule: {raw}"
                ),
            }
        }

        // Bucket boundaries, independent of the capture.
        assert_eq!(classify_token("Fabric"), VersionToken::Loader);
        assert_eq!(
            classify_token(" Risugami's ModLoader "),
            VersionToken::Loader
        );
        assert_eq!(classify_token("1.14-Snapshot"), VersionToken::GameVersion);
        assert_eq!(classify_token("26.1"), VersionToken::GameVersion);
        assert_eq!(classify_token("OptiFine"), VersionToken::Dropped);
        assert_eq!(classify_token("Client"), VersionToken::Dropped);
        assert_eq!(classify_token("Server"), VersionToken::Dropped);
    }

    #[tokio::test]
    async fn spec_f014_get_versions_passes_known_filters_only() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/mods/238222/files"))
            .and(query_param("gameVersion", "1.20.1"))
            .and(query_param("modLoader", "4"))
            .and(query_param("index", "0"))
            .and(query_param("pageSize", "50"))
            .respond_with(ResponseTemplate::new(200).set_body_string(FILES_PAGE_0))
            .mount(&server)
            .await;

        let versions = test_client(&server)
            .get_versions("238222", Some("1.20.1"), Some("fabric"))
            .await
            .expect("filtered versions");
        assert_eq!(versions.len(), 2);

        let requests = server.received_requests().await.expect("requests");
        assert_eq!(requests.len(), 1, "a short page ends the listing");
        let pairs: Vec<(String, String)> = requests[0]
            .url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        assert!(pairs.contains(&("gameVersion".to_string(), "1.20.1".to_string())));
        assert!(pairs.contains(&("modLoader".to_string(), "4".to_string())));

        // `any` drops only the loader; an unknown loader leaves both filters
        // out. Mount each shape separately so a query the client should not
        // send has no matching mock and turns into a 404.
        Mock::given(method("GET"))
            .and(path("/v1/mods/238222/files"))
            .and(query_param("gameVersion", "1.20.1"))
            .and(query_param_is_missing("modLoader"))
            .and(query_param("index", "0"))
            .and(query_param("pageSize", "50"))
            .respond_with(ResponseTemplate::new(200).set_body_string(FILES_PAGE_0))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/mods/238222/files"))
            .and(query_param_is_missing("gameVersion"))
            .and(query_param_is_missing("modLoader"))
            .and(query_param("index", "0"))
            .and(query_param("pageSize", "50"))
            .respond_with(ResponseTemplate::new(200).set_body_string(FILES_PAGE_0))
            .mount(&server)
            .await;

        // `any` and an unknown loader send no modLoader; an absent game
        // version sends no gameVersion either.
        let _ = test_client(&server)
            .get_versions("238222", Some("1.20.1"), Some("any"))
            .await
            .expect("any loader");
        let _ = test_client(&server)
            .get_versions("238222", None, Some("mystery"))
            .await
            .expect("unknown loader");

        let requests = server.received_requests().await.expect("requests");
        assert_eq!(requests.len(), 3);
        let any_pairs: Vec<(String, String)> = requests[1]
            .url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        assert!(any_pairs.contains(&("gameVersion".to_string(), "1.20.1".to_string())));
        assert!(!any_pairs.iter().any(|(key, _)| key == "modLoader"));
        let unknown_pairs: Vec<(String, String)> = requests[2]
            .url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        assert!(!unknown_pairs.iter().any(|(key, _)| key == "gameVersion"));
        assert!(!unknown_pairs.iter().any(|(key, _)| key == "modLoader"));
    }

    #[tokio::test]
    async fn spec_f014_hash_lookup_is_unsupported() {
        let server = MockServer::start().await;
        let err = test_client(&server)
            .get_versions_by_hashes(&["aa".to_string()], HashAlgo::Sha1, None)
            .await
            .expect_err("hash lookup is unsupported");
        assert!(matches!(
            err,
            ProviderError::Unsupported {
                capability: "get_versions_by_hashes"
            }
        ));
        assert!(
            server
                .received_requests()
                .await
                .expect("requests")
                .is_empty(),
            "an unsupported capability must not spend a request"
        );
    }

    #[tokio::test]
    async fn spec_f014_get_version_without_project_is_unsupported() {
        let server = MockServer::start().await;
        let err = test_client(&server)
            .get_version("9277601")
            .await
            .expect_err("no project id");
        assert!(matches!(
            err,
            ProviderError::Unsupported {
                capability: "get_version without a project id"
            }
        ));
    }

    #[tokio::test]
    async fn spec_f014_get_project_refuses_slug_and_non_mod_class() {
        let server = MockServer::start().await;
        let err = test_client(&server)
            .get_project("jei")
            .await
            .expect_err("slug lookup is unsupported");
        assert!(matches!(
            err,
            ProviderError::Unsupported {
                capability: "get_project by slug"
            }
        ));
        assert!(server
            .received_requests()
            .await
            .expect("requests")
            .is_empty());

        // Synthetic: the capture only contains classId 6, so the non-mod
        // branch has no fixture. The shape is the smallest valid mod record.
        Mock::given(method("GET"))
            .and(path("/v1/mods/1"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"data":{"id":1,"slug":"x","name":"X","summary":"s","classId":5,"authors":[],"categories":[],"latestFilesIndexes":[],"downloadCount":0,"logo":null}}"#,
            ))
            .mount(&server)
            .await;
        let err = test_client(&server)
            .get_project("1")
            .await
            .expect_err("a non-mod class must be refused");
        assert!(matches!(
            err,
            ProviderError::Unsupported {
                capability: "project_type"
            }
        ));
    }

    #[tokio::test]
    async fn spec_f014_403_is_api_error_without_key_or_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/mods/search"))
            .respond_with(ResponseTemplate::new(403).set_body_string("private-response-body"))
            .mount(&server)
            .await;

        let err = test_client(&server)
            .search(&query())
            .await
            .expect_err("403");
        assert!(matches!(err, ProviderError::Api { status: 403 }));
        let text = err.to_string();
        assert!(!text.contains("test-key"), "the key leaked: {text}");
        assert!(
            !text.contains("private-response-body"),
            "the response body leaked: {text}"
        );
    }

    #[tokio::test]
    async fn spec_f014_rate_limit_reads_reset_header_or_defaults_to_60() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/mods/search"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "12"))
            .mount(&server)
            .await;
        let err = test_client(&server)
            .search(&query())
            .await
            .expect_err("429");
        assert!(matches!(
            err,
            ProviderError::RateLimited { reset_seconds: 12 }
        ));

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/mods/search"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;
        let err = test_client(&server)
            .search(&query())
            .await
            .expect_err("429 without headers");
        assert!(matches!(
            err,
            ProviderError::RateLimited { reset_seconds: 60 }
        ));
    }

    #[tokio::test]
    async fn spec_f014_unavailable_file_is_skipped_at_the_boundary() {
        // Synthetic: every captured file has isAvailable == true (725/725,
        // fixtures §7-5), so the false branch has no recorded response.
        const BODY: &str = r#"{
            "data": [
                {"id": 1, "modId": 238222, "isAvailable": false, "displayName": "gone",
                 "fileName": "gone.jar", "releaseType": 1, "fileDate": "2026-01-01T00:00:00Z",
                 "fileLength": 1, "downloadUrl": null, "gameVersions": ["1.20.1"]},
                {"id": 2, "modId": 238222, "isAvailable": true, "displayName": "live",
                 "fileName": "live.jar", "releaseType": 1, "fileDate": "2026-01-01T00:00:00Z",
                 "fileLength": 1, "downloadUrl": null, "gameVersions": ["1.20.1"]}
            ],
            "pagination": {"index": 0, "pageSize": 50, "resultCount": 2, "totalCount": 2}
        }"#;
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/mods/238222/files"))
            .respond_with(ResponseTemplate::new(200).set_body_string(BODY))
            .mount(&server)
            .await;

        let versions = test_client(&server)
            .get_versions("238222", None, None)
            .await
            .expect("versions");
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].id, "2");
    }
}
