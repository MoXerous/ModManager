//! S7 mapping tests: the browse table's and inspector's wire shapes.
//!
//! These are the boundary, not the providers: core's own tests pin what the
//! catalogue answers, and these pin what the webview is allowed to see.

use crate::dto::{ErrorCode, ProjectDto, ProjectVersionDto};
use mm_core::providers::{
    HashAlgo, LatestFilter, Project, ProjectFile, Provider, ProviderError, SearchPage, SearchQuery,
    Version,
};
use std::collections::HashMap;
use std::sync::Mutex;

use super::super::project_page_url;

fn project() -> Project {
    Project {
        id: "P7dR8mSH".to_string(),
        slug: Some("fabric-api".to_string()),
        title: "Fabric API".to_string(),
        description: "d".to_string(),
        author: "modmuss50".to_string(),
        project_type: "mod".to_string(),
        categories: vec!["fabric".to_string(), "library".to_string()],
        versions: vec!["1.21".to_string()],
        downloads: 100,
        followers: 42,
        icon_url: None,
        published: "2024-01-01T00:00:00Z".to_string(),
        client_side: "required".to_string(),
        server_side: "optional".to_string(),
        latest_version: Some("1.0.0".to_string()),
        web_url: Some("https://modrinth.com/mod/fabric-api".to_string()),
    }
}

/// F-165: every column the old results table rendered has a wire field. A
/// missing one is a column the screen would have to invent.
#[test]
fn spec_f165_project_dto_carries_the_table_columns() {
    let dto = ProjectDto::from_core(&project(), "modrinth");
    assert_eq!(dto.followers, 42);
    assert_eq!(dto.categories, vec!["fabric", "library"]);
    assert_eq!(dto.published, "2024-01-01T00:00:00Z");
    assert_eq!(dto.client_side, "required");
    assert_eq!(dto.server_side, "optional");
    assert_eq!(dto.latest_version.as_deref(), Some("1.0.0"));
    assert_eq!(
        dto.web_url.as_deref(),
        Some("https://modrinth.com/mod/fabric-api")
    );
    // The columns that were already there keep their values.
    assert_eq!(dto.downloads, 100);
    assert_eq!(dto.provider_id, "modrinth");
}

/// F-165: the URL column opens the project's own page, and the opener command
/// refuses everything that is not a catalogue page: non-https, unknown hosts,
/// and non-URLs never reach the operating system.
#[test]
fn spec_f165_open_project_url_opens_only_catalogue_pages() {
    for url in [
        "https://modrinth.com/mod/fabric-api",
        "https://www.curseforge.com/minecraft/mc-mods/jei",
        "https://cdn.modrinth.com/icon.png",
    ] {
        assert!(
            project_page_url(url).is_ok(),
            "a provider page passes: {url}"
        );
    }
    for url in [
        "http://modrinth.com/mod/fabric-api",
        "file://C:/Windows/System32/calc.exe",
        "https://evil.example.com/mod/fabric-api",
        "not a url at all",
    ] {
        let error = project_page_url(url).expect_err("a non-catalogue URL is refused");
        assert_eq!(error.code, ErrorCode::InvalidInput, "refusal for {url}");
    }
}

/// F-164: the versions list is its own small record — the download plan is a
/// different command and a different shape.
#[test]
fn spec_f164_version_dto_carries_the_versions_list() {
    let version = Version {
        id: "oGwyXeEI".to_string(),
        project_id: "P7dR8mSH".to_string(),
        name: "1.0".to_string(),
        version_number: "1.0.0".to_string(),
        changelog: None,
        date_published: "2024-01-01T00:00:00Z".to_string(),
        version_type: "release".to_string(),
        status: "listed".to_string(),
        files: vec![ProjectFile {
            filename: "x.jar".to_string(),
            url: String::new(),
            size: 6,
            sha1: None,
            sha512: None,
            primary: true,
        }],
        game_versions: vec!["1.21".to_string()],
        loaders: vec!["fabric".to_string()],
        dependencies: Vec::new(),
    };

    let dto = ProjectVersionDto::from_core(&version);
    assert_eq!(dto.id, "oGwyXeEI");
    assert_eq!(dto.project_id, "P7dR8mSH");
    assert_eq!(dto.version_number, "1.0.0");
    assert_eq!(dto.date_published, "2024-01-01T00:00:00Z");
    assert_eq!(dto.version_type, "release");
    assert_eq!(dto.game_versions, vec!["1.21"]);
    assert_eq!(dto.loaders, vec!["fabric"]);
    // A file with no URL is kept (F-017), so the list can still show its name.
    assert_eq!(dto.filename.as_deref(), Some("x.jar"));
}

/// A provider that answers the one call these tests make and records the
/// filters it saw, so "did the command pass the target through" is assertable.
/// `(project id, game version, loader)` as the provider received them.
type SeenFilters = (String, Option<String>, Option<String>);

struct VersionProvider {
    versions: Vec<Version>,
    seen: Mutex<Vec<SeenFilters>>,
}

#[async_trait::async_trait]
impl Provider for VersionProvider {
    fn provider_id(&self) -> &'static str {
        "modrinth"
    }

    async fn search(&self, _q: &SearchQuery) -> Result<SearchPage, ProviderError> {
        Err(ProviderError::Unsupported {
            capability: "search",
        })
    }

    async fn get_project(&self, _id_or_slug: &str) -> Result<Project, ProviderError> {
        Err(ProviderError::Unsupported {
            capability: "get_project",
        })
    }

    async fn get_version(&self, _id: &str) -> Result<Version, ProviderError> {
        Err(ProviderError::Unsupported {
            capability: "get_version",
        })
    }

    async fn get_versions(
        &self,
        id_or_slug: &str,
        game_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Vec<Version>, ProviderError> {
        if let Ok(mut seen) = self.seen.lock() {
            seen.push((
                id_or_slug.to_string(),
                game_version.map(str::to_string),
                loader.map(str::to_string),
            ));
        }
        Ok(self.versions.clone())
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

fn version_of(id: &str, number: &str) -> Version {
    Version {
        id: id.to_string(),
        project_id: "P7dR8mSH".to_string(),
        name: number.to_string(),
        version_number: number.to_string(),
        changelog: None,
        date_published: "2024-01-01T00:00:00Z".to_string(),
        version_type: "release".to_string(),
        status: "listed".to_string(),
        files: vec![ProjectFile {
            filename: format!("{id}.jar"),
            url: String::new(),
            size: 1,
            sha1: None,
            sha512: None,
            primary: true,
        }],
        game_versions: vec!["1.21".to_string()],
        loaders: vec!["fabric".to_string()],
        dependencies: Vec::new(),
    }
}

/// F-164: "Show versions" is one provider call, and the target the screen
/// knows narrows it — a lower-case loader, because that is the facet's
/// spelling, not the label's.
#[tokio::test]
async fn spec_f164_versions_with_asks_the_provider_and_maps_the_list() {
    let provider = VersionProvider {
        versions: vec![version_of("v2", "1.1.0"), version_of("v1", "1.0.0")],
        seen: Mutex::new(Vec::new()),
    };
    let list =
        crate::commands::versions_with(&provider, " P7dR8mSH ", Some("1.21"), Some("Fabric"))
            .await
            .expect("versions");
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].version_number, "1.1.0");
    assert_eq!(list[0].filename.as_deref(), Some("v2.jar"));
    let seen = provider.seen.lock().expect("seen");
    assert_eq!(
        seen.as_slice(),
        &[(
            "P7dR8mSH".to_string(),
            Some("1.21".to_string()),
            Some("fabric".to_string())
        )]
    );
}

/// An empty id is a caller bug, not an empty list: the list would read as
/// "this project has no versions".
#[tokio::test]
async fn spec_f164_versions_with_refuses_an_empty_project_id() {
    let provider = VersionProvider {
        versions: Vec::new(),
        seen: Mutex::new(Vec::new()),
    };
    let err = crate::commands::versions_with(&provider, "   ", None, None)
        .await
        .expect_err("must refuse");
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(provider.seen.lock().expect("seen").is_empty());
}
