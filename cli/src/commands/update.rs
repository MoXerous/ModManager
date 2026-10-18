//! `update`: newest compatible per lock pin via hash-batch, then swap.

use super::{allow_insecure, client, instance_dir, io_err, lock_path, mods_dir, CliError, Ctx};
use mm_core::downloader::{DownloadRequest, Downloader};
use mm_core::instances::{read_lock, write_lock, LockEntry};
use mm_core::providers::{HashAlgo, LatestFilter, Provider};
use mm_core::resolver::primary_file;
use std::ops::ControlFlow;

struct UpdateRow {
    project_id: String,
    old_version: String,
    new_number: String,
    version_id: String,
    filename: String,
    url: String,
    size: u64,
    sha1: Option<String>,
    sha512: Option<String>,
}

pub async fn update(
    ctx: &Ctx,
    game: Option<String>,
    loader: Option<String>,
    yes: bool,
    dry_run: bool,
) -> Result<(), CliError> {
    let game = game.unwrap_or_else(|| "any".to_string());
    let loader = loader.unwrap_or_else(|| "any".to_string());
    if game == "any" || loader == "any" {
        return Err(CliError::Usage(
            "update requires --game and --loader".to_string(),
        ));
    }
    let lock = match read_lock(&lock_path(ctx)?) {
        Ok(l) => l,
        Err(_) => {
            println!("no locked mods");
            return Ok(());
        }
    };
    let shas: Vec<String> = lock
        .entries
        .iter()
        .map(|e| e.sha1.clone())
        .filter(|s| !s.is_empty())
        .collect();
    if shas.is_empty() {
        println!("nothing to update");
        return Ok(());
    }
    let provider = client(ctx)?;
    let latest = provider
        .get_versions_by_hashes(
            &shas,
            HashAlgo::Sha1,
            Some(&LatestFilter {
                loaders: vec![loader.clone()],
                game_versions: vec![game.clone()],
            }),
        )
        .await
        .map_err(|e| CliError::Provider(e.to_string()))?;
    let mut rows: Vec<UpdateRow> = Vec::new();
    for e in &lock.entries {
        let key = e.sha1.to_lowercase();
        if let Some(v) = latest.get(&key) {
            if v.id != e.version_id {
                if let Some(f) = primary_file(v).filter(|f| !f.url.is_empty()) {
                    rows.push(UpdateRow {
                        project_id: e.project_id.clone(),
                        old_version: e.version_id.clone(),
                        new_number: v.version_number.clone(),
                        version_id: v.id.clone(),
                        filename: f.filename.clone(),
                        url: f.url.clone(),
                        size: f.size,
                        sha1: f.sha1.clone(),
                        sha512: f.sha512.clone(),
                    });
                }
            }
        }
    }
    if rows.is_empty() {
        println!("already up to date");
        return Ok(());
    }
    super::print_table(
        &["project", "old", "new", "file"],
        &rows
            .iter()
            .map(|r| {
                vec![
                    r.project_id.clone(),
                    r.old_version.clone(),
                    r.new_number.clone(),
                    r.filename.clone(),
                ]
            })
            .collect::<Vec<_>>(),
    );
    if dry_run {
        return Ok(());
    }
    super::confirm_forced(rows.len(), yes)?;

    let mods = mods_dir(ctx)?;
    std::fs::create_dir_all(&mods).map_err(|e| io_err("create mods dir", e))?;
    let downloader = Downloader::new(3).map_err(|e| CliError::Download(e.to_string()))?;
    let backup = super::remove::backup_dir(ctx)?;
    let mut new_entries: Vec<LockEntry> = Vec::new();
    for r in &rows {
        let req = DownloadRequest {
            url: r.url.clone(),
            dest: mods.join(&r.filename),
            expected_size: Some(r.size),
            sha1_hex: r.sha1.clone(),
            sha512_hex: r.sha512.clone(),
            allow_insecure_loopback: allow_insecure(&r.url),
        };
        let go = || ControlFlow::Continue(());
        let outcome = downloader
            .download(&req, &mut |_| go(), &|| false)
            .await
            .map_err(|e| CliError::Download(e.to_string()))?;
        // Swap only after the new jar verifies: old file goes to backup.
        let old_file = lock
            .entries
            .iter()
            .find(|e| e.project_id == r.project_id)
            .map(|e| mods.join(&e.filename));
        if let Some(old) = old_file {
            if old.is_file() && old != outcome.path {
                let staged = backup.join(
                    old.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("old.jar"),
                );
                std::fs::rename(&old, &staged).map_err(|e| io_err("stage old jar", e))?;
            }
        }
        let landed = outcome
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&r.filename)
            .to_string();
        if outcome.path != mods.join(&landed) {
            std::fs::rename(&outcome.path, mods.join(&landed))
                .map_err(|e| io_err("place new jar", e))?;
        }
        println!("updated {} -> {}", r.project_id, r.new_number);
        let slug = lock
            .entries
            .iter()
            .find(|e| e.project_id == r.project_id)
            .map(|e| e.slug.clone())
            .unwrap_or_default();
        new_entries.push(LockEntry {
            provider: "modrinth".to_string(),
            project_id: r.project_id.clone(),
            slug,
            version_id: r.version_id.clone(),
            filename: landed,
            sha1: r.sha1.clone().unwrap_or_default(),
            sha512: r.sha512.clone().unwrap_or_default(),
        });
    }
    let path = lock_path(ctx)?;
    let mut merged = match read_lock(&path) {
        Ok(l) => l.entries,
        Err(_) => Vec::new(),
    };
    for new in new_entries {
        merged.retain(|e| e.project_id != new.project_id);
        merged.push(new);
    }
    let _ = instance_dir(ctx)?;
    write_lock(&path, &mm_core::instances::LockFile { entries: merged })
        .map_err(|e| CliError::Io(e.to_string()))?;
    Ok(())
}
