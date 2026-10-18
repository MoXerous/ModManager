//! `install`: plan → confirm → download → write lock (last).
//!
//! The lock is written once, after every download verifies. On failure
//! the lock is left untouched (landed `.part` files allow resume).

use super::{
    allow_insecure, client, confirm, instance_dir, io_err, lock_path, mods_dir, print_plan,
    resolve_slugs, CliError, Ctx,
};
use mm_core::downloader::{DownloadRequest, Downloader};
use mm_core::instances::{read_lock, write_lock, LockEntry, LockFile};
use mm_core::resolver::primary_file;
use std::collections::HashSet;
use std::ops::ControlFlow;

pub async fn install(
    ctx: &Ctx,
    slugs: &[String],
    game: Option<String>,
    loader: Option<String>,
    yes: bool,
    dry_run: bool,
) -> Result<(), CliError> {
    let game = game.unwrap_or_else(|| "any".to_string());
    let loader = loader.unwrap_or_else(|| "any".to_string());
    let provider = client(ctx)?;
    let data = resolve_slugs(&provider, slugs, &game, &loader, &HashSet::new()).await?;
    print_plan(&data.entries, &data.summary, &data.warnings);
    if dry_run {
        return Ok(());
    }
    if data.summary.blocked_count > 0 {
        return Err(CliError::Blocked(format!(
            "{} entr{} blocked",
            data.summary.blocked_count,
            if data.summary.blocked_count == 1 {
                "y"
            } else {
                "ies"
            }
        )));
    }
    confirm(&data.summary, yes)?;

    let mods = mods_dir(ctx)?;
    std::fs::create_dir_all(&mods).map_err(|e| io_err("create mods dir", e))?;
    let downloader = Downloader::new(3).map_err(|e| CliError::Download(e.to_string()))?;
    let mut lock_entries: Vec<LockEntry> = Vec::new();
    for e in &data.entries {
        let version_id = e.version_id.clone().unwrap_or_default();
        let version = data
            .versions
            .get(&version_id)
            .ok_or_else(|| CliError::Download(format!("resolved version {version_id} missing")))?;
        let file = primary_file(version).ok_or_else(|| {
            CliError::Download(format!("version {version_id} has no downloadable file"))
        })?;
        let dest = mods.join(&file.filename);
        let req = DownloadRequest {
            url: file.url.clone(),
            dest: dest.clone(),
            expected_size: Some(file.size),
            sha1_hex: file.sha1.clone(),
            sha512_hex: file.sha512.clone(),
            allow_insecure_loopback: allow_insecure(&file.url),
        };
        let go = || ControlFlow::Continue(());
        let outcome = downloader
            .download(&req, &mut |_| go(), &|| false)
            .await
            .map_err(|e| CliError::Download(e.to_string()))?;
        let landed = outcome
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&file.filename)
            .to_string();
        println!("downloaded {landed}");
        let slug = data
            .selected
            .iter()
            .chain(data.deps.iter().map(|d| &d.content))
            .find(|r| r.project_id == e.project_id)
            .and_then(|r| {
                if r.slug.is_empty() {
                    None
                } else {
                    Some(r.slug.clone())
                }
            })
            .unwrap_or_default();
        lock_entries.push(LockEntry {
            provider: "modrinth".to_string(),
            project_id: e.project_id.clone(),
            slug,
            version_id: version.id.clone(),
            filename: landed,
            sha1: file.sha1.clone().unwrap_or_default(),
            sha512: file.sha512.clone().unwrap_or_default(),
        });
    }
    // Lock last: crash before this leaves jars without claims, never the
    // reverse. Merge with existing entries (re-install overwrites by key).
    let path = lock_path(ctx)?;
    let mut merged: Vec<LockEntry> = match read_lock(&path) {
        Ok(l) => l.entries,
        Err(_) => Vec::new(),
    };
    for new in lock_entries {
        merged.retain(|e| e.project_id != new.project_id);
        merged.push(new);
    }
    let _ = instance_dir(ctx)?;
    write_lock(&path, &LockFile { entries: merged }).map_err(|e| CliError::Io(e.to_string()))?;
    println!("installed {} mod(s)", data.entries.len());
    Ok(())
}
