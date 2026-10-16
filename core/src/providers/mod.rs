//! Content providers: trait + Modrinth (API v2).
//!
//! Only provider-agnostic shapes cross the trait. CurseForge has zero
//! behavioural reference and arrives in M3 as a second implementation.

pub mod modrinth;

use std::collections::HashMap;
use thiserror::Error;

/// A downloadable file of a version. `url` may be empty (F-017): the file is
/// kept and the UI decides — the provider never filters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectFile {
    pub filename: String,
    pub url: String,
    pub size: u64,
    pub sha1: Option<String>,
    pub sha512: Option<String>,
    pub primary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub slug: Option<String>,
    pub title: String,
    pub description: String,
    pub author: String,
    /// Modrinth `project_type` (`mod`, `modpack`, `shader`, …); drives
    /// loader fallbacks and overrides in `resolver`.
    pub project_type: String,
    pub categories: Vec<String>,
    pub versions: Vec<String>,
    pub downloads: u64,
    pub icon_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionDependency {
    pub project_id: Option<String>,
    pub version_id: Option<String>,
    pub file_name: Option<String>,
    pub dependency_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub version_number: String,
    pub changelog: Option<String>,
    /// Raw string, never parsed (spec §2-7: no invented ISO parsing).
    pub date_published: String,
    pub version_type: String,
    /// Modrinth `status` (`listed`, `archived`, …); step 3 of the
    /// compatibility ladder (resolver §3) reads it raw.
    pub status: String,
    pub files: Vec<ProjectFile>,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub dependencies: Vec<VersionDependency>,
}

#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    pub query: String,
    pub game_version: Option<String>,
    pub loader: Option<String>,
    pub project_type: String,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Default)]
pub struct SearchPage {
    pub hits: Vec<Project>,
    pub total: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgo {
    Sha1,
    Sha512,
}

impl HashAlgo {
    pub fn as_str(self) -> &'static str {
        match self {
            HashAlgo::Sha1 => "sha1",
            HashAlgo::Sha512 => "sha512",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LatestFilter {
    pub loaders: Vec<String>,
    pub game_versions: Vec<String>,
}

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("provider api error {status}")]
    Api { status: u16 },
    #[error("rate limited, retry in {reset_seconds}s")]
    RateLimited { reset_seconds: u64 },
    #[error("request timed out")]
    Timeout,
    #[error("transport failure")]
    Transport,
    #[error("invalid response")]
    InvalidJson,
}

#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    fn provider_id(&self) -> &'static str;
    async fn search(&self, q: &SearchQuery) -> Result<SearchPage, ProviderError>;
    async fn get_project(&self, id_or_slug: &str) -> Result<Project, ProviderError>;
    async fn get_versions(
        &self,
        id_or_slug: &str,
        game_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Vec<Version>, ProviderError>;
    /// Keys are the input hashes lowercased. `latest` selects the
    /// `/version_files/update` endpoint (F-005), else `/version_files`.
    async fn get_versions_by_hashes(
        &self,
        hashes: &[String],
        algo: HashAlgo,
        latest: Option<&LatestFilter>,
    ) -> Result<HashMap<String, Version>, ProviderError>;
}
