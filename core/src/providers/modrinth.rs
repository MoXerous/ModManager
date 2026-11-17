//! Modrinth provider (API v2).
//!
//! Constants from `providers.md` §2: base `https://api.modrinth.com/v2`,
//! 10 s connect / 45 s total timeouts, max 2 attempts on read-only paths,
//! retryable = transport ∪ {429, 500, 502, 503, 504}, never sleep > 10 s.

use super::{
    HashAlgo, LatestFilter, Project, ProjectFile, Provider, ProviderError, SearchPage, SearchQuery,
    Version, VersionDependency,
};
use std::collections::HashMap;

pub const MODRINTH_BASE: &str = "https://api.modrinth.com/v2";

/// The provider's identity, and the only place its name is written. Every
/// layer above matches on this constant instead of repeating the literal
/// (ADR 0004 §2-3).
pub const PROVIDER_ID: &str = "modrinth";

pub struct ModrinthClient {
    client: reqwest::Client,
    base: String,
}

impl ModrinthClient {
    pub fn new() -> Result<Self, ProviderError> {
        Self::with_base(MODRINTH_BASE)
    }

    /// Custom base URL (tests and embedding point at wiremock; production
    /// uses MODRINTH_BASE).
    pub fn with_base(base: &str) -> Result<Self, ProviderError> {
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
        })
    }
}

/// Statuses the provider retries on read-only paths (spec §2-3).
const RETRYABLE_5XX: &[u16] = &[500, 502, 503, 504];
/// Never sleep longer than this; surface `RateLimited` instead (spec §2-5).
const MAX_WAIT_SECS: f64 = 10.0;
/// Transport-error retry sleeps exactly this (spec §2-2).
const TRANSPORT_RETRY_SECS: f64 = 1.0;
/// Attempts cap on every read-only path (spec §2-2).
const MAX_ATTEMPTS: u32 = 2;

#[derive(Debug, serde::Deserialize)]
struct SearchHit {
    project_id: String,
    #[serde(default)]
    project_type: String,
    slug: Option<String>,
    title: String,
    description: String,
    author: String,
    categories: Vec<String>,
    versions: Vec<String>,
    downloads: u64,
    /// Search hits spell the count `follows`; the project endpoint spells the
    /// same number `followers`.
    #[serde(default)]
    follows: u64,
    #[serde(default)]
    date_created: String,
    #[serde(default)]
    client_side: String,
    #[serde(default)]
    server_side: String,
    /// The newest version's **id**; the label and date come from the bulk
    /// versions call `fill_latest_versions` makes once per page.
    #[serde(default)]
    latest_version: Option<String>,
    icon_url: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct SearchResp {
    hits: Vec<SearchHit>,
    total_hits: u64,
}

#[derive(Debug, serde::Deserialize)]
struct ProjectDto {
    id: String,
    slug: Option<String>,
    title: String,
    #[serde(default)]
    project_type: String,
    description: String,
    downloads: u64,
    /// The project endpoint's spelling; `follows` is accepted too so one shape
    /// serves both endpoints.
    #[serde(default, alias = "follows")]
    followers: u64,
    #[serde(default)]
    date_created: String,
    #[serde(default)]
    client_side: String,
    #[serde(default)]
    server_side: String,
    categories: Vec<String>,
    versions: Vec<String>,
    icon_url: Option<String>,
}

#[derive(Debug, serde::Deserialize, Default)]
struct HashDto {
    sha1: Option<String>,
    sha512: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct FileDto {
    hashes: HashDto,
    url: Option<String>,
    filename: String,
    #[serde(default)]
    primary: bool,
    size: u64,
}

#[derive(Debug, serde::Deserialize)]
struct DependencyDto {
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    version_id: Option<String>,
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    dependency_type: String,
}

#[derive(Debug, serde::Deserialize)]
struct VersionDto {
    id: String,
    project_id: String,
    name: String,
    version_number: String,
    changelog: Option<String>,
    date_published: String,
    version_type: String,
    #[serde(default = "listed_status")]
    status: String,
    files: Vec<FileDto>,
    game_versions: Vec<String>,
    loaders: Vec<String>,
    #[serde(default)]
    dependencies: Vec<DependencyDto>,
}

fn listed_status() -> String {
    "listed".to_string()
}

/// The project's own page. Built from the documented URL shape
/// (`modrinth.com/{type}/{slug}`), never from a field Modrinth might not send.
fn project_page_url(project_type: &str, slug_or_id: &str) -> Option<String> {
    let kind = if project_type.is_empty() {
        "mod"
    } else {
        project_type
    };
    if slug_or_id.is_empty() {
        return None;
    }
    Some(format!("https://modrinth.com/{kind}/{slug_or_id}"))
}

fn map_file(f: FileDto) -> ProjectFile {
    ProjectFile {
        // F-017: missing URL becomes "", the file is kept.
        url: f.url.unwrap_or_default(),
        filename: f.filename,
        size: f.size,
        sha1: f.hashes.sha1,
        sha512: f.hashes.sha512,
        primary: f.primary,
    }
}

fn map_version(v: VersionDto) -> Version {
    Version {
        id: v.id,
        project_id: v.project_id,
        name: v.name,
        version_number: v.version_number,
        changelog: v.changelog,
        date_published: v.date_published,
        version_type: v.version_type,
        status: v.status,
        files: v.files.into_iter().map(map_file).collect(),
        game_versions: v.game_versions,
        loaders: v.loaders,
        dependencies: v
            .dependencies
            .into_iter()
            .map(|d| VersionDependency {
                project_id: d.project_id,
                version_id: d.version_id,
                file_name: d.file_name,
                dependency_type: d.dependency_type,
            })
            .collect(),
    }
}

/// Strict numeric: digits with one optional dot (`10`, `10.01`); rejects
/// `NaN`, `Infinity`, `1_0`, `-2`, `` (spec §2-4).
fn parse_retry_seconds(text: &str) -> Option<f64> {
    if text.is_empty() {
        return None;
    }
    let mut dots = 0;
    for b in text.bytes() {
        match b {
            b'0'..=b'9' => {}
            b'.' => {
                dots += 1;
                if dots > 1 {
                    return None;
                }
            }
            _ => return None,
        }
    }
    text.parse::<f64>().ok()
}

/// Days since 1970-01-01 → (year, month, day). Howard Hinnant's
/// `civil_from_days`: no date dependency, ~15 lines.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// (year, month, day) → days since 1970-01-01 (Hinnant's `days_from_civil`).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// F-009: the wait an HTTP-date `Retry-After` asks for, in seconds (never
/// negative). RFC 850 dates carry a two-digit year, which `httpdate` maps
/// into its fixed 1970–2069 window; HTTP's own rule (the old app's
/// `_server_retry_delay`) is a rolling ±50-year window around now, so from
/// 2026 on `11-Sep-76` means 2076 — a future reset the >10 s fail-fast rule
/// fires on — not 1976.
fn http_date_wait_secs(date: std::time::SystemTime, two_digit_year: bool) -> f64 {
    let now = std::time::SystemTime::now();
    if !two_digit_year {
        return date
            .duration_since(now)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
    }
    let date_secs = date
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64);
    let now_secs = now
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64);
    let (Some(date_secs), Some(now_secs)) = (date_secs, now_secs) else {
        return 0.0;
    };
    let (parsed_year, month, day) = civil_from_days(date_secs.div_euclid(86_400));
    let date_sod = date_secs.rem_euclid(86_400);
    let (now_year, now_month, now_day) = civil_from_days(now_secs.div_euclid(86_400));
    let now_sod = now_secs.rem_euclid(86_400);
    // The two-digit year inside the window [now.year - 50, now.year + 50].
    let mut year = now_year + (parsed_year % 100 - now_year).rem_euclid(100);
    // Past (now + 50 years) at now's own month/day/time, the window looked
    // a century ahead: step back into the past.
    let candidate = days_from_civil(year, month, day) * 86_400 + date_sod;
    let cutoff = days_from_civil(now_year + 50, now_month, now_day) * 86_400 + now_sod;
    if candidate > cutoff {
        year -= 100;
    }
    // A Feb-29 the target year does not have is unrepresentable; that is
    // the old app's ValueError branch: no delay, the next header is tried.
    let final_days = days_from_civil(year, month, day);
    if civil_from_days(final_days) != (year, month, day) {
        return 0.0;
    }
    ((final_days * 86_400 + date_sod - now_secs).max(0)) as f64
}

/// First valid wins: numeric `Retry-After`, then HTTP-date `Retry-After`,
/// then `X-Ratelimit-Reset` (epoch seconds). Malformed values are skipped.
fn server_delay_secs(headers: &reqwest::header::HeaderMap) -> Option<f64> {
    use reqwest::header::RETRY_AFTER;
    if let Some(v) = headers.get(RETRY_AFTER).and_then(|v| v.to_str().ok()) {
        let v = v.trim();
        if let Some(secs) = parse_retry_seconds(v) {
            return Some(secs);
        }
        if let Ok(date) = httpdate::parse_http_date(v) {
            // F-009: RFC 850 dates carry a two-digit year (the `-` in the
            // text) and get HTTP's rolling ±50-year rule; other shapes keep
            // their absolute year.
            let wait = http_date_wait_secs(date, v.contains('-'));
            return Some(wait);
        }
    }
    if let Some(v) = headers
        .get("X-Ratelimit-Reset")
        .and_then(|v| v.to_str().ok())
    {
        if let Ok(epoch) = v.trim().parse::<u64>() {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            return Some(epoch.saturating_sub(now) as f64);
        }
    }
    None
}

struct RawResp {
    status: u16,
    body: Vec<u8>,
}

impl ModrinthClient {
    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        query: Vec<(String, String)>,
        body: Option<serde_json::Value>,
    ) -> Result<RawResp, ProviderError> {
        let mut attempt: u32 = 0;
        loop {
            attempt += 1;
            let mut rb = self
                .client
                .request(method.clone(), format!("{}{}", self.base, path));
            for (k, v) in &query {
                rb = rb.query(&[(k, v)]);
            }
            if let Some(b) = &body {
                rb = rb.json(b);
            }
            let resp = rb.send().await.map_err(|e| {
                if e.is_timeout() {
                    ProviderError::Timeout
                } else {
                    ProviderError::Transport
                }
            });
            let resp = match resp {
                Err(_) if attempt < MAX_ATTEMPTS => {
                    tokio::time::sleep(std::time::Duration::from_secs_f64(TRANSPORT_RETRY_SECS))
                        .await;
                    continue;
                }
                Err(e) => return Err(e),
                Ok(r) => r,
            };
            let status = resp.status().as_u16();
            if status == 429 || RETRYABLE_5XX.contains(&status) {
                let delay = server_delay_secs(resp.headers()).unwrap_or(if status == 429 {
                    60.0
                } else {
                    1.0
                });
                if attempt < MAX_ATTEMPTS && delay <= MAX_WAIT_SECS {
                    tokio::time::sleep(std::time::Duration::from_secs_f64(delay)).await;
                    continue;
                }
                if status == 429 {
                    return Err(ProviderError::RateLimited {
                        reset_seconds: delay.ceil() as u64,
                    });
                }
                return Err(ProviderError::Api { status });
            }
            let bytes = resp.bytes().await.map_err(|_| ProviderError::Transport)?;
            return Ok(RawResp {
                status,
                body: bytes.to_vec(),
            });
        }
    }

    fn parse<T: serde::de::DeserializeOwned>(r: &RawResp) -> Result<T, ProviderError> {
        serde_json::from_slice(&r.body).map_err(|_| ProviderError::InvalidJson)
    }

    /// Facets as JSON array-of-arrays; empty groups omitted.
    fn facets(q: &SearchQuery) -> String {
        let mut groups: Vec<Vec<String>> = Vec::new();
        if !q.project_type.is_empty() {
            groups.push(vec![format!("project_type:{}", q.project_type)]);
        }
        if let Some(v) = &q.game_version {
            groups.push(vec![format!("versions:{v}")]);
        }
        if let Some(l) = &q.loader {
            groups.push(vec![format!("categories:{l}")]);
        }
        serde_json::to_string(&groups).unwrap_or_else(|_| "[]".to_string())
    }

    /// F-165: swap each row's newest-version **id** for its label and date.
    ///
    /// One bulk call answers the whole page (`/versions?ids=[…]`), so a row
    /// never costs a request of its own. Best effort by design: a failed lookup
    /// leaves the project's own date in place and an empty version cell, which
    /// is worth more than an empty screen.
    async fn fill_latest_versions(&self, hits: &mut [Project]) {
        let ids: Vec<String> = hits
            .iter()
            .filter_map(|h| h.latest_version.clone())
            .collect();
        if ids.is_empty() {
            return;
        }
        let query = vec![(
            "ids".to_string(),
            serde_json::to_string(&ids).unwrap_or_default(),
        )];
        let filled = match self
            .request(reqwest::Method::GET, "/versions", query, None)
            .await
        {
            Ok(r) if r.status == 200 => Self::parse::<Vec<VersionDto>>(&r).ok(),
            _ => None,
        };
        let mut labels: HashMap<String, (String, String)> = HashMap::new();
        for v in filled.unwrap_or_default() {
            labels.insert(v.id, (v.version_number, v.date_published));
        }
        for hit in hits.iter_mut() {
            let Some(id) = hit.latest_version.take() else {
                continue;
            };
            if let Some((label, date)) = labels.remove(&id) {
                hit.latest_version = Some(label);
                if !date.is_empty() {
                    hit.published = date;
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl Provider for ModrinthClient {
    fn provider_id(&self) -> &'static str {
        PROVIDER_ID
    }

    async fn search(&self, q: &SearchQuery) -> Result<SearchPage, ProviderError> {
        let mut page = self.search_inner(q, true).await?;
        self.fill_latest_versions(&mut page.hits).await;
        Ok(page)
    }

    async fn get_project(&self, id_or_slug: &str) -> Result<Project, ProviderError> {
        let r = self
            .request(
                reqwest::Method::GET,
                &format!("/project/{id_or_slug}"),
                Vec::new(),
                None,
            )
            .await?;
        if r.status == 404 {
            return Err(ProviderError::Api { status: 404 });
        }
        if r.status != 200 {
            return Err(ProviderError::Api { status: r.status });
        }
        let p: ProjectDto = Self::parse(&r)?;
        let project_type = if p.project_type.is_empty() {
            "mod".to_string()
        } else {
            p.project_type
        };
        let web_url = project_page_url(&project_type, p.slug.as_deref().unwrap_or(p.id.as_str()));
        Ok(Project {
            id: p.id,
            slug: p.slug,
            title: p.title,
            description: p.description,
            // The project endpoint carries a team id, not a username;
            // usernames come from search hits. Team resolution is deferred.
            author: String::new(),
            project_type,
            categories: p.categories,
            versions: p.versions,
            downloads: p.downloads,
            followers: p.followers,
            icon_url: p.icon_url,
            published: p.date_created,
            client_side: p.client_side,
            server_side: p.server_side,
            // The project record lists version ids and no label; the details
            // pane's versions list is what answers a project's versions.
            latest_version: None,
            web_url,
        })
    }

    async fn get_version(&self, id: &str) -> Result<Version, ProviderError> {
        let r = self
            .request(
                reqwest::Method::GET,
                &format!("/version/{id}"),
                Vec::new(),
                None,
            )
            .await?;
        if r.status != 200 {
            return Err(ProviderError::Api { status: r.status });
        }
        Self::parse::<VersionDto>(&r).map(map_version)
    }

    async fn get_versions(
        &self,
        id_or_slug: &str,
        game_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Vec<Version>, ProviderError> {
        let mut query = Vec::new();
        if let Some(v) = game_version {
            query.push((
                "game_versions".to_string(),
                serde_json::to_string(&[v]).unwrap_or_default(),
            ));
        }
        if let Some(l) = loader {
            query.push((
                "loaders".to_string(),
                serde_json::to_string(&[l]).unwrap_or_default(),
            ));
        }
        let r = self
            .request(
                reqwest::Method::GET,
                &format!("/project/{id_or_slug}/version"),
                query,
                None,
            )
            .await?;
        if r.status != 200 {
            return Err(ProviderError::Api { status: r.status });
        }
        let vs: Vec<VersionDto> = Self::parse(&r)?;
        Ok(vs.into_iter().map(map_version).collect())
    }

    async fn get_versions_by_hashes(
        &self,
        hashes: &[String],
        algo: HashAlgo,
        latest: Option<&LatestFilter>,
    ) -> Result<HashMap<String, Version>, ProviderError> {
        let lowered: Vec<String> = hashes.iter().map(|h| h.to_lowercase()).collect();
        let mut body = serde_json::json!({ "hashes": lowered, "algorithm": algo.as_str() });
        let path = match latest {
            Some(f) => {
                body["loaders"] = f
                    .loaders
                    .iter()
                    .map(|s| serde_json::Value::String(s.clone()))
                    .collect::<Vec<_>>()
                    .into();
                body["game_versions"] = f
                    .game_versions
                    .iter()
                    .map(|s| serde_json::Value::String(s.clone()))
                    .collect::<Vec<_>>()
                    .into();
                "/version_files/update"
            }
            None => "/version_files",
        };
        let r = self
            .request(reqwest::Method::POST, path, Vec::new(), Some(body))
            .await?;
        if r.status != 200 {
            return Err(ProviderError::Api { status: r.status });
        }
        let map: HashMap<String, VersionDto> = Self::parse(&r)?;
        Ok(map
            .into_iter()
            .map(|(k, v)| (k.to_lowercase(), map_version(v)))
            .collect())
    }
}

impl ModrinthClient {
    /// Search with the 400-fallback: `plugin`/`datapack` fall back to `mod`.
    async fn search_inner(
        &self,
        q: &SearchQuery,
        allow_fallback: bool,
    ) -> Result<SearchPage, ProviderError> {
        let query = vec![
            ("query".to_string(), q.query.clone()),
            ("facets".to_string(), Self::facets(q)),
            ("index".to_string(), "relevance".to_string()),
            ("offset".to_string(), q.offset.to_string()),
            ("limit".to_string(), q.limit.to_string()),
        ];
        let r = self
            .request(reqwest::Method::GET, "/search", query, None)
            .await?;
        if r.status == 400
            && allow_fallback
            && (q.project_type == "plugin" || q.project_type == "datapack")
        {
            let mut retry = q.clone();
            retry.project_type = "mod".to_string();
            if retry.loader.is_none() {
                retry.loader = Some(q.project_type.clone());
            }
            return Box::pin(self.search_inner(&retry, false)).await;
        }
        if r.status != 200 {
            return Err(ProviderError::Api { status: r.status });
        }
        let resp: SearchResp = Self::parse(&r)?;
        Ok(SearchPage {
            total: resp.total_hits,
            hits: resp
                .hits
                .into_iter()
                .map(|h| {
                    let project_type = if h.project_type.is_empty() {
                        "mod".to_string()
                    } else {
                        h.project_type
                    };
                    let web_url = project_page_url(
                        &project_type,
                        h.slug.as_deref().unwrap_or(h.project_id.as_str()),
                    );
                    Project {
                        id: h.project_id,
                        slug: h.slug,
                        title: h.title,
                        description: h.description,
                        author: h.author,
                        project_type,
                        categories: h.categories,
                        versions: h.versions,
                        downloads: h.downloads,
                        followers: h.follows,
                        icon_url: h.icon_url,
                        // Until `fill_latest_versions` says otherwise, the
                        // project's own creation date is the honest date.
                        published: h.date_created,
                        client_side: h.client_side,
                        server_side: h.server_side,
                        // Held as the version *id* so the bulk call can swap in
                        // the label; `None` when the hit carried no version.
                        latest_version: h.latest_version,
                        web_url,
                    }
                })
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SEARCH_BODY: &str = r#"{
        "hits": [{
            "project_id": "P7dR8mSH", "project_type": "mod", "slug": "fabric-api",
            "author": "modmuss50", "title": "Fabric API", "description": "d",
            "categories": ["fabric", "technology"], "display_categories": ["fabric"],
            "versions": ["1.21"], "downloads": 100, "follows": 10,
            "icon_url": null, "date_created": "2021-01-01T00:00:00Z",
            "date_modified": "2024-01-01T00:00:00Z", "latest_version": "oGwyXeEI",
            "license": "MIT", "client_side": "required", "server_side": "optional",
            "gallery": [], "featured_gallery": null, "color": null
        }],
        "offset": 0, "limit": 10, "total_hits": 3905
    }"#;

    const PROJECT_BODY: &str = r#"{
        "id": "P7dR8mSH", "slug": "fabric-api", "title": "Fabric API",
        "description": "d", "body": "b", "client_side": "required",
        "server_side": "optional", "game_versions": ["1.21"],
        "project_type": "mod", "downloads": 100, "followers": 10,
        "categories": ["fabric"], "loaders": ["fabric"], "versions": ["oGwyXeEI"],
        "icon_url": null, "team": "t", "status": "approved",
        "date_created": "2021-01-01T00:00:00Z",
        "license": {"id": "MIT"}, "gallery": []
    }"#;

    const VERSIONS_BODY: &str = r#"[{
        "id": "oGwyXeEI", "project_id": "P7dR8mSH", "author_id": "a",
        "featured": false, "name": "1.0", "version_number": "1.0.0",
        "changelog": null, "date_published": "2024-01-01T00:00:00Z",
        "downloads": 5, "version_type": "release", "status": "listed",
        "game_versions": ["1.21"], "loaders": ["fabric"],
        "files": [{
            "hashes": {"sha512": "aa", "sha1": "43035a1c6f506285a9910bc8038d1b1b925f8dd1"},
            "url": "https://cdn.modrinth.com/x.jar",
            "filename": "x.jar", "primary": true, "size": 6, "file_type": null
        }, {
            "hashes": {"sha512": "bb", "sha1": "11"},
            "url": null, "filename": "x-sources.jar", "primary": false,
            "size": 1, "file_type": null
        }],
        "dependencies": []
    }]"#;

    const HASH_BODY: &str = r#"{
        "43035a1c6f506285a9910bc8038d1b1b925f8dd1": {
            "id": "oGwyXeEI", "project_id": "P7dR8mSH", "author_id": "a",
            "featured": false, "name": "1.0", "version_number": "1.0.0",
            "changelog": null, "date_published": "2024-01-01T00:00:00Z",
            "downloads": 5, "version_type": "release", "status": "listed",
            "game_versions": ["1.21"], "loaders": ["fabric"],
            "files": [{
                "hashes": {"sha512": "aa", "sha1": "43035a1c6f506285a9910bc8038d1b1b925f8dd1"},
                "url": "https://cdn.modrinth.com/x.jar",
                "filename": "x.jar", "primary": true, "size": 6, "file_type": null
            }],
            "dependencies": []
        }
    }"#;

    fn test_client(server: &MockServer) -> ModrinthClient {
        ModrinthClient::with_base(&server.uri()).expect("client")
    }

    fn query() -> SearchQuery {
        SearchQuery {
            query: "fabric".to_string(),
            game_version: Some("1.21".to_string()),
            loader: Some("fabric".to_string()),
            project_type: "mod".to_string(),
            limit: 10,
            offset: 0,
        }
    }

    /// Only the `/search` calls: a successful page also makes one bulk
    /// versions call (F-165), which these retry tests do not count.
    async fn search_calls(server: &MockServer) -> usize {
        server
            .received_requests()
            .await
            .expect("requests")
            .iter()
            .filter(|r| r.url.path() == "/search")
            .count()
    }

    #[tokio::test]
    async fn spec_f001_search_parses_hits_and_total() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("query", "fabric"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SEARCH_BODY))
            .mount(&server)
            .await;
        let page = test_client(&server).search(&query()).await.expect("search");
        assert_eq!(page.total, 3905);
        assert_eq!(page.hits.len(), 1);
        assert_eq!(page.hits[0].id, "P7dR8mSH");
        assert_eq!(page.hits[0].title, "Fabric API");
    }

    #[tokio::test]
    async fn spec_f165_search_rows_carry_the_table_columns() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SEARCH_BODY))
            .mount(&server)
            .await;
        // The Version and Date columns are the newest version's, and one bulk
        // call answers them for the whole page: a row must never cost a request
        // of its own (a 20-row page would be 21 round trips).
        Mock::given(method("GET"))
            .and(path("/versions"))
            .and(query_param("ids", r#"["oGwyXeEI"]"#))
            .respond_with(ResponseTemplate::new(200).set_body_string(VERSIONS_BODY))
            .mount(&server)
            .await;

        let page = test_client(&server).search(&query()).await.expect("search");
        let row = &page.hits[0];
        assert_eq!(row.followers, 10);
        assert_eq!(row.client_side, "required");
        assert_eq!(row.server_side, "optional");
        assert_eq!(row.categories, vec!["fabric", "technology"]);
        // F-165: the resolved version label, not the version *id* the search
        // hit carries.
        assert_eq!(row.latest_version.as_deref(), Some("1.0.0"));
        assert_eq!(row.published, "2024-01-01T00:00:00Z");
        assert_eq!(
            row.web_url.as_deref(),
            Some("https://modrinth.com/mod/fabric-api")
        );
    }

    /// The bulk call is an enrichment, not the search: a catalogue that
    /// answers hits and then fails the versions call still has rows, and the
    /// table shows the project's own date instead of a blank one.
    #[tokio::test]
    async fn spec_f165_a_failed_version_lookup_keeps_the_row() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SEARCH_BODY))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/versions"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let page = test_client(&server).search(&query()).await.expect("search");
        let row = &page.hits[0];
        assert_eq!(row.latest_version, None);
        // `date_created` is still a real published date.
        assert_eq!(row.published, "2021-01-01T00:00:00Z");
        assert_eq!(row.followers, 10);
    }

    #[tokio::test]
    async fn spec_f002_project_maps_fields() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/project/fabric-api"))
            .respond_with(ResponseTemplate::new(200).set_body_string(PROJECT_BODY))
            .mount(&server)
            .await;
        let p = test_client(&server)
            .get_project("fabric-api")
            .await
            .expect("project");
        assert_eq!(p.id, "P7dR8mSH");
        assert_eq!(p.slug.as_deref(), Some("fabric-api"));
        assert_eq!(p.downloads, 100);
    }

    /// F-164: the details pane reads the one project record, so it must carry
    /// the same columns the search row does.
    #[tokio::test]
    async fn spec_f164_project_record_carries_the_details_pane_fields() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/project/fabric-api"))
            .respond_with(ResponseTemplate::new(200).set_body_string(PROJECT_BODY))
            .mount(&server)
            .await;
        let p = test_client(&server)
            .get_project("fabric-api")
            .await
            .expect("project");
        // The project endpoint spells the count `followers`, search spells it
        // `follows`: both are the same number.
        assert_eq!(p.followers, 10);
        assert_eq!(p.client_side, "required");
        assert_eq!(p.server_side, "optional");
        assert_eq!(p.published, "2021-01-01T00:00:00Z");
        assert_eq!(
            p.web_url.as_deref(),
            Some("https://modrinth.com/mod/fabric-api")
        );
    }

    #[tokio::test]
    async fn spec_f002_get_version_maps_single() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/version/oGwyXeEI"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{
                    "id": "oGwyXeEI", "project_id": "P7dR8mSH", "author_id": "a",
                    "featured": false, "name": "1.0", "version_number": "1.0.0",
                    "changelog": null, "date_published": "2024-01-01T00:00:00Z",
                    "downloads": 5, "version_type": "release", "status": "listed",
                    "game_versions": ["1.21"], "loaders": ["fabric"],
                    "files": [{
                        "hashes": {"sha512": "aa", "sha1": "bb"},
                        "url": "https://cdn.modrinth.com/x.jar",
                        "filename": "x.jar", "primary": true, "size": 6, "file_type": null
                    }],
                    "dependencies": []
                }"#,
            ))
            .mount(&server)
            .await;
        let v = test_client(&server)
            .get_version("oGwyXeEI")
            .await
            .expect("version");
        assert_eq!(v.id, "oGwyXeEI");
        assert_eq!(v.project_id, "P7dR8mSH");
        assert_eq!(v.status, "listed");
    }

    #[tokio::test]
    async fn spec_f002_versions_map_files_and_hashes() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/project/P7dR8mSH/version"))
            .respond_with(ResponseTemplate::new(200).set_body_string(VERSIONS_BODY))
            .mount(&server)
            .await;
        let vs = test_client(&server)
            .get_versions("P7dR8mSH", Some("1.21"), Some("fabric"))
            .await
            .expect("versions");
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].files.len(), 2);
        assert_eq!(
            vs[0].files[0].sha1.as_deref(),
            Some("43035a1c6f506285a9910bc8038d1b1b925f8dd1")
        );
        // F-017: empty-URL file is kept, never dropped.
        assert_eq!(vs[0].files[1].url, "");
        assert_eq!(vs[0].date_published, "2024-01-01T00:00:00Z");
        // Resolver reads these raw (spec §3 step 3, §5 BFS).
        assert_eq!(vs[0].status, "listed");
        assert!(vs[0].dependencies.is_empty());
    }

    #[tokio::test]
    async fn spec_f004_hash_lookup_batch() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/version_files"))
            .and(body_json(serde_json::json!({
                "hashes": ["43035a1c6f506285a9910bc8038d1b1b925f8dd1"],
                "algorithm": "sha1"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_string(HASH_BODY))
            .mount(&server)
            .await;
        let map = test_client(&server)
            .get_versions_by_hashes(
                &["43035A1C6F506285A9910BC8038D1B1B925F8DD1".to_string()],
                HashAlgo::Sha1,
                None,
            )
            .await
            .expect("lookup");
        assert_eq!(map.len(), 1);
        assert!(map.contains_key("43035a1c6f506285a9910bc8038d1b1b925f8dd1"));
        assert_eq!(map.values().next().expect("v").id, "oGwyXeEI");
    }

    #[tokio::test]
    async fn spec_f007_rate_limit_surfaces_reset() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "120"))
            .mount(&server)
            .await;
        let t0 = std::time::Instant::now();
        let r = test_client(&server).search(&query()).await;
        assert!(matches!(
            r,
            Err(ProviderError::RateLimited { reset_seconds: 120 })
        ));
        assert!(t0.elapsed() < std::time::Duration::from_secs(10));
        // `/search` only: a successful page also fires the one bulk versions
        // call that fills the table's Version/Date columns (F-165).
        assert_eq!(search_calls(&server).await, 1);
    }

    #[tokio::test]
    async fn spec_f008_short_wait_retries_once() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(503).insert_header("Retry-After", "0"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SEARCH_BODY))
            .mount(&server)
            .await;
        let page = test_client(&server).search(&query()).await.expect("retry");
        assert_eq!(page.total, 3905);
        assert_eq!(search_calls(&server).await, 2);
    }

    #[tokio::test]
    async fn spec_f010_no_retry_on_501() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(501))
            .mount(&server)
            .await;
        let r = test_client(&server).search(&query()).await;
        assert!(matches!(r, Err(ProviderError::Api { status: 501 })));
        assert_eq!(server.received_requests().await.expect("count").len(), 1);
    }

    #[tokio::test]
    async fn spec_f011_fallback_plugin_to_mod() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(400))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SEARCH_BODY))
            .mount(&server)
            .await;
        let mut q = query();
        q.project_type = "plugin".to_string();
        let page = test_client(&server).search(&q).await.expect("fallback");
        assert_eq!(page.total, 3905);
        let reqs: Vec<wiremock::Request> = server
            .received_requests()
            .await
            .expect("reqs")
            .into_iter()
            .filter(|r| r.url.path() == "/search")
            .collect();
        assert_eq!(reqs.len(), 2);
        // Second request carries project_type:mod, never plugin.
        let facets = reqs[1]
            .url
            .query_pairs()
            .find(|(k, _)| k == "facets")
            .expect("facets")
            .1
            .into_owned();
        assert!(facets.contains("project_type:mod"));
        assert!(!facets.contains("plugin"));
    }

    #[tokio::test]
    async fn spec_f019_invalid_json_never_leaks_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(200).set_body_string("private{garbage"))
            .mount(&server)
            .await;
        let r = test_client(&server).search(&query()).await;
        assert!(matches!(r, Err(ProviderError::InvalidJson)));
        if let Err(e) = r {
            assert!(!e.to_string().contains("private"), "{e}");
        }
    }

    #[tokio::test]
    async fn spec_f005_update_endpoint_filters() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/version_files/update"))
            .and(body_json(serde_json::json!({
                "hashes": ["aa"],
                "algorithm": "sha1",
                "loaders": ["fabric"],
                "game_versions": ["1.21"]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_string(HASH_BODY))
            .mount(&server)
            .await;
        let f = LatestFilter {
            loaders: vec!["fabric".to_string()],
            game_versions: vec!["1.21".to_string()],
        };
        let map = test_client(&server)
            .get_versions_by_hashes(&["aa".to_string()], HashAlgo::Sha1, Some(&f))
            .await
            .expect("update");
        assert_eq!(map.len(), 1);
    }

    fn delay_from(value: &'static str) -> Option<f64> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static(value),
        );
        server_delay_secs(&headers)
    }

    /// F-009: RFC 850 dates carry a two-digit year, which `httpdate` maps
    /// into its fixed 1970–2069 window. HTTP's own rule (the old app's
    /// `_server_retry_delay`) is a rolling ±50-year window around now, so
    /// `11-Sep-76` means 2076 from 2026 on — a future reset the >10 s
    /// fail-fast rule fires on, not 1976's 0 s wait.
    #[test]
    fn spec_f009_two_digit_year_rolls_forward_into_the_future() {
        let wait = delay_from("Saturday, 11-Sep-76 09:14:33 GMT").expect("http date");
        assert!(
            wait > 10.0,
            "a mapped-future reset must fail fast, got {wait}"
        );
        // Inside the ±50-year window: not further than 51 years out.
        assert!(
            wait < 51.0 * 365.25 * 86_400.0,
            "outside the window: {wait}"
        );
    }

    /// F-009: a two-digit year that maps into the recent past keeps its
    /// past answer (0 s wait), the other side of the rolling window.
    #[test]
    fn spec_f009_recent_two_digit_year_stays_in_the_past() {
        let wait = delay_from("Wednesday, 11-Sep-24 09:14:33 GMT").expect("http date");
        assert_eq!(wait, 0.0, "a past reset waits nothing, got {wait}");
    }

    /// F-009: the pivot is (now + 50 years) at now's own month/day/time.
    /// A mapped year past that point steps back a century, so `Dec-76`
    /// (2076-12-11 is past the 2076-11-17 cutoff) stays 1976, while
    /// `Sep-76` stays 2076.
    #[test]
    fn spec_f009_the_pivot_month_and_day_decide_the_direction() {
        let wait = delay_from("Saturday, 11-Dec-76 09:14:33 GMT").expect("http date");
        assert_eq!(wait, 0.0, "past the cutoff the year steps back, got {wait}");
    }

    /// F-009 integration: a future HTTP-date `Retry-After` must trip the
    /// >10 s fail-fast — no retry sleep, one attempt, and the surfaced
    /// reset window is the (huge) wait, not 0.
    #[tokio::test]
    async fn spec_f009_a_future_http_date_fails_fast() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("Retry-After", "Saturday, 11-Sep-76 09:14:33 GMT"),
            )
            .mount(&server)
            .await;
        let r = test_client(&server).search(&query()).await;
        assert!(matches!(
            r,
            Err(ProviderError::RateLimited { reset_seconds }) if reset_seconds > 10
        ));
        assert_eq!(search_calls(&server).await, 1, "a >10 s wait never retries");
    }
}
