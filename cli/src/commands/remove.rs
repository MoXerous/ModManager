//! `remove`: jars to a timestamped backup dir, drop lock entries.
//! Never deletes (F-111 stays M2: no restore UI yet, but no data loss).

use super::{instance_dir, io_err, lock_path, mods_dir, CliError, Ctx};
use mm_core::instances::{read_lock, scan_mods, write_lock, Instance, LockFile};

/// `<instance>/.modmanager-removed/<unix-secs>/`, created on demand.
pub fn backup_dir(ctx: &Ctx) -> Result<std::path::PathBuf, CliError> {
    let dir = instance_dir(ctx)?.join(".modmanager-removed");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let stamped = dir.join(stamp.to_string());
    std::fs::create_dir_all(&stamped).map_err(|e| io_err("create backup dir", e))?;
    Ok(stamped)
}

pub async fn remove(
    ctx: &Ctx,
    selectors: &[String],
    yes: bool,
    dry_run: bool,
) -> Result<(), CliError> {
    if selectors.is_empty() {
        return Err(CliError::Usage("no mods given".to_string()));
    }
    let root = instance_dir(ctx)?;
    let mods = mods_dir(ctx)?;
    let lock = match read_lock(&lock_path(ctx)?) {
        Ok(l) => l,
        Err(_) => LockFile { entries: vec![] },
    };
    let facts = scan_mods(&Instance {
        root: root.clone(),
        minecraft_root: root,
    });
    // Resolve each selector to exactly one jar: lock filename/project
    // first, then scan fact filename.
    let mut targets: Vec<(String, std::path::PathBuf, Option<String>)> = Vec::new();
    for s in selectors {
        let by_lock = lock
            .entries
            .iter()
            .find(|e| e.project_id == *s || e.slug == *s || e.filename == *s);
        if let Some(e) = by_lock {
            targets.push((
                e.project_id.clone(),
                mods.join(&e.filename),
                Some(e.project_id.clone()),
            ));
            continue;
        }
        let hits: Vec<_> = facts.iter().filter(|f| f.filename == *s).collect();
        if hits.len() == 1 {
            targets.push((s.clone(), hits[0].path.clone(), None));
        } else if hits.is_empty() {
            return Err(CliError::Usage(format!("no such mod: {s}")));
        } else {
            return Err(CliError::Usage(format!("ambiguous filename: {s}")));
        }
    }
    super::print_table(
        &["project", "file"],
        &targets
            .iter()
            .map(|(p, path, _)| {
                vec![
                    p.clone(),
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string(),
                ]
            })
            .collect::<Vec<_>>(),
    );
    if dry_run {
        return Ok(());
    }
    super::confirm_forced(targets.len(), yes)?;

    let backup = backup_dir(ctx)?;
    let mut removed_projects: Vec<String> = Vec::new();
    for (project, path, locked) in &targets {
        if !path.is_file() {
            return Err(CliError::Io(format!("missing file: {}", path.display())));
        }
        let dest = backup.join(
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("mod.jar"),
        );
        std::fs::rename(path, &dest).map_err(|e| io_err("move to backup", e))?;
        println!("removed {project}");
        if let Some(pid) = locked {
            removed_projects.push(pid.clone());
        }
    }
    if !removed_projects.is_empty() {
        let path = lock_path(ctx)?;
        let kept: Vec<_> = lock
            .entries
            .into_iter()
            .filter(|e| !removed_projects.contains(&e.project_id))
            .collect();
        write_lock(&path, &LockFile { entries: kept }).map_err(|e| CliError::Io(e.to_string()))?;
    }
    Ok(())
}
