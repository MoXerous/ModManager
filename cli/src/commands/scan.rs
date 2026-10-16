//! `scan`: read-only mod facts table.

use super::{instance_dir, CliError, Ctx};
use mm_core::instances::{scan_mods, Instance};

pub async fn scan(ctx: &Ctx) -> Result<(), CliError> {
    let root = instance_dir(ctx)?;
    let facts = scan_mods(&Instance {
        root: root.clone(),
        minecraft_root: root,
    });
    if facts.is_empty() {
        println!("no mod files");
        return Ok(());
    }
    super::print_table(
        &["file", "size", "sha1", "type", "disabled"],
        &facts
            .iter()
            .map(|f| {
                vec![
                    f.filename.clone(),
                    f.size.to_string(),
                    f.sha1.chars().take(12).collect(),
                    format!("{:?}", f.resource_type).to_lowercase(),
                    f.disabled.to_string(),
                ]
            })
            .collect::<Vec<_>>(),
    );
    Ok(())
}
