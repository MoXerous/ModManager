//! `list`: locked mods + on-disk presence.

use super::{instance_dir, lock_path, CliError, Ctx};
use mm_core::instances::{read_lock, scan_mods, Instance};

pub async fn list(ctx: &Ctx) -> Result<(), CliError> {
    let root = instance_dir(ctx)?;
    let lock = match read_lock(&lock_path(ctx)?) {
        Ok(l) => l,
        Err(_) => {
            println!("no locked mods");
            return Ok(());
        }
    };
    if lock.entries.is_empty() {
        println!("no locked mods");
        return Ok(());
    }
    let facts = scan_mods(&Instance {
        root: root.clone(),
        minecraft_root: root,
    });
    super::print_table(
        &["project", "version", "file", "state"],
        &lock
            .entries
            .iter()
            .map(|e| {
                let state = facts
                    .iter()
                    .find(|f| f.filename == e.filename)
                    .map(|f| {
                        if f.disabled {
                            "disabled".to_string()
                        } else {
                            "enabled".to_string()
                        }
                    })
                    .unwrap_or_else(|| "missing".to_string());
                let name = if e.slug.is_empty() {
                    e.project_id.clone()
                } else {
                    e.slug.clone()
                };
                vec![name, e.version_id.clone(), e.filename.clone(), state]
            })
            .collect::<Vec<_>>(),
    );
    Ok(())
}
