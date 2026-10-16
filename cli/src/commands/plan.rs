//! `plan`: resolve slugs → preview table. Never writes.

use super::{client, print_plan, resolve_slugs, CliError, Ctx};
use std::collections::HashSet;

pub async fn plan(
    ctx: &Ctx,
    slugs: &[String],
    game: Option<String>,
    loader: Option<String>,
) -> Result<(), CliError> {
    let game = game.unwrap_or_else(|| "any".to_string());
    let loader = loader.unwrap_or_else(|| "any".to_string());
    let provider = client(ctx)?;
    let data = resolve_slugs(&provider, slugs, &game, &loader, &HashSet::new()).await?;
    print_plan(&data.entries, &data.summary, &data.warnings);
    Ok(())
}
