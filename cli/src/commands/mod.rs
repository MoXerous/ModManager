//! CLI command implementations: clap-thin wiring over mm-core.

pub mod install;
pub mod list;
pub mod plan;
pub mod remove;
pub mod scan;
pub mod search;
pub mod update;

pub use install::install;
pub use list::list;
pub use plan::plan;
pub use remove::remove;
pub use scan::scan;
pub use search::search;
pub use update::update;

use mm_core::providers::ModrinthClient;
use mm_core::resolver::{
    requires_confirmation, resolve_and_plan, BatchPlan, PlanEntry, PlanSummary, Warning,
};
use std::collections::HashSet;
use std::path::PathBuf;

/// Shared context: paths come from flags, never defaults.
#[derive(Debug, Clone, Default)]
pub struct Ctx {
    pub instance: Option<PathBuf>,
    pub api_base: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("usage: {0}")]
    Usage(String),
    #[error("provider: {0}")]
    Provider(String),
    #[error("download: {0}")]
    Download(String),
    #[error("io: {0}")]
    Io(String),
    #[error("blocked: {0}")]
    Blocked(String),
    #[error("confirmation required: pass --yes")]
    ConfirmationRequired,
}

impl CliError {
    /// Exit codes: 0 ok · 1 operational error · 2 usage / confirmation.
    pub fn code(&self) -> i32 {
        match self {
            CliError::Usage(_) | CliError::ConfirmationRequired => 2,
            _ => 1,
        }
    }
}

pub fn info() -> Result<(), CliError> {
    let info = mm_core::app_info();
    println!(
        "{} {} ({} / {})",
        info.name, info.version, info.os, info.arch
    );
    Ok(())
}

fn io_err(op: &str, e: std::io::Error) -> CliError {
    CliError::Io(format!("{op}: {e}"))
}

pub fn client(ctx: &Ctx) -> Result<ModrinthClient, CliError> {
    match ctx.api_base.as_deref() {
        Some(base) => ModrinthClient::with_base(base),
        None => ModrinthClient::new(),
    }
    .map_err(|e| CliError::Provider(e.to_string()))
}

pub fn instance_dir(ctx: &Ctx) -> Result<PathBuf, CliError> {
    ctx.instance
        .clone()
        .ok_or_else(|| CliError::Usage("missing --instance <DIR>".to_string()))
}

pub fn mods_dir(ctx: &Ctx) -> Result<PathBuf, CliError> {
    Ok(instance_dir(ctx)?.join("mods"))
}

pub fn lock_path(ctx: &Ctx) -> Result<PathBuf, CliError> {
    Ok(instance_dir(ctx)?.join("instance.lock.json"))
}

/// Loopback CDN (tests) may use http; everything else stays https-only
/// inside the downloader.
fn allow_insecure(url: &str) -> bool {
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', ':'])
        .next()
        .unwrap_or("");
    host == "localhost" || host == "::1" || host.starts_with("127.")
}

pub fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    println!("{}", headers.join(" | "));
    for r in rows {
        println!("{}", r.join(" | "));
    }
}

/// Plan-first gate: `--yes` bypasses; a TTY prompts; anything else refuses.
pub fn confirm(summary: &PlanSummary, yes: bool) -> Result<(), CliError> {
    if !requires_confirmation(summary) || yes {
        return Ok(());
    }
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        return Err(CliError::ConfirmationRequired);
    }
    eprint!(
        "Apply {} change(s) ({} blocked)? [y/N] ",
        summary.total_count, summary.blocked_count
    );
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| io_err("read stdin", e))?;
    let ok = matches!(line.trim().to_lowercase().as_str(), "y" | "yes");
    if ok {
        Ok(())
    } else {
        Err(CliError::ConfirmationRequired)
    }
}

/// Unconditional gate for update/remove: any row needs confirmation.
pub fn confirm_forced(count: usize, yes: bool) -> Result<(), CliError> {
    if count == 0 || yes {
        return Ok(());
    }
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        return Err(CliError::ConfirmationRequired);
    }
    eprint!("Apply {count} change(s)? [y/N] ");
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| io_err("read stdin", e))?;
    if matches!(line.trim().to_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err(CliError::ConfirmationRequired)
    }
}
/// Resolve slugs through the shared core facade (one implementation for
/// CLI and Tauri).
pub async fn resolve_slugs(
    provider: &dyn mm_core::providers::Provider,
    slugs: &[String],
    game: &str,
    loader: &str,
    existing: &HashSet<String>,
) -> Result<BatchPlan, CliError> {
    if slugs.is_empty() {
        return Err(CliError::Usage("no projects given".to_string()));
    }
    resolve_and_plan(provider, slugs, game, loader, existing)
        .await
        .map_err(|e| CliError::Provider(e.to_string()))
}
pub fn print_plan(entries: &[PlanEntry], summary: &PlanSummary, warnings: &[Warning]) {
    print_table(
        &["status", "source", "title", "version", "file", "reason"],
        &entries
            .iter()
            .map(|e| {
                vec![
                    e.status.clone(),
                    e.source.clone(),
                    e.title.clone(),
                    e.version_number.clone().unwrap_or_default(),
                    e.filename.clone().unwrap_or_default(),
                    e.reason.clone(),
                ]
            })
            .collect::<Vec<_>>(),
    );
    println!(
        "selected={} dependencies={} downloadable={} blocked={}",
        summary.selected_count,
        summary.dependency_count,
        summary.downloadable_count,
        summary.blocked_count
    );
    for w in warnings {
        println!("[{}] {}", w.severity, w.message);
    }
}
