//! `search`: read-only Modrinth query → table.

use super::{client, CliError, Ctx};
use mm_core::providers::{Provider, SearchQuery};

pub async fn search(
    ctx: &Ctx,
    query: &str,
    game: Option<String>,
    loader: Option<String>,
    limit: u32,
) -> Result<(), CliError> {
    let provider = client(ctx)?;
    let page = provider
        .search(&SearchQuery {
            query: query.to_string(),
            game_version: game,
            loader,
            project_type: "mod".to_string(),
            limit,
            offset: 0,
        })
        .await
        .map_err(|e| CliError::Provider(e.to_string()))?;
    super::print_table(
        &["slug", "title", "downloads", "type"],
        &page
            .hits
            .iter()
            .map(|p| {
                vec![
                    p.slug.clone().unwrap_or_default(),
                    p.title.clone(),
                    p.downloads.to_string(),
                    p.project_type.clone(),
                ]
            })
            .collect::<Vec<_>>(),
    );
    println!("total={}", page.total);
    Ok(())
}
