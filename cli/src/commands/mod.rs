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

use mm_core::providers::{ModrinthClient, Provider, Version, VersionDependency};
use mm_core::resolver::{
    build_batch_install_plan, build_safety_warnings, requires_confirmation, resolve_dependency,
    resolve_project, PlanEntry, PlanSummary, PreferredPin, ResolveParams, ResolvedContent,
    ReviewCandidate, RunCache, Warning,
};
use std::collections::{HashMap, HashSet};
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
pub struct PlanData {
    pub selected: Vec<ResolvedContent>,
    pub deps: Vec<ReviewCandidate>,
    pub entries: Vec<PlanEntry>,
    pub summary: PlanSummary,
    pub warnings: Vec<Warning>,
    pub versions: HashMap<String, Version>,
}

/// Resolve slugs → selected + transitive required deps → batch plan.
/// Never writes; pure coordination over [`Provider`].
pub async fn resolve_slugs(
    provider: &dyn Provider,
    slugs: &[String],
    game: &str,
    loader: &str,
    existing: &HashSet<String>,
) -> Result<PlanData, CliError> {
    if slugs.is_empty() {
        return Err(CliError::Usage("no projects given".to_string()));
    }
    let mut cache = RunCache::new(true);
    let mut selected: Vec<ResolvedContent> = Vec::new();
    for slug in slugs {
        let project = provider
            .get_project(slug)
            .await
            .map_err(|e| CliError::Provider(format!("{slug}: {e}")))?;
        let ptype = if project.project_type.is_empty() {
            "mod".to_string()
        } else {
            project.project_type.clone()
        };
        selected.push(
            resolve_project(
                provider,
                &mut cache,
                &ResolveParams {
                    project,
                    project_type: ptype,
                    game_version: game.to_string(),
                    loader: loader.to_string(),
                    allow_alpha: false,
                    pin: PreferredPin::default(),
                },
            )
            .await,
        );
    }
    // Transitive required deps; cycles cut silently (F-061 deferred).
    let mut deps: Vec<ReviewCandidate> = Vec::new();
    let mut known: HashSet<String> = selected.iter().map(|r| r.project_id.clone()).collect();
    let mut grown: Vec<ResolvedContent> = selected.clone();
    let mut i = 0;
    while i < grown.len() {
        let parent = grown[i].project_id.clone();
        let reqs: Vec<VersionDependency> = grown[i]
            .version
            .as_ref()
            .map(|v| {
                v.dependencies
                    .iter()
                    .filter(|d| d.dependency_type == "required")
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        for dep in reqs {
            let id = dep.project_id.clone().unwrap_or_default();
            if id.is_empty() || !known.insert(id.clone()) {
                continue;
            }
            if let Some(r) =
                resolve_dependency(provider, &mut cache, &dep, game, loader, &HashMap::new()).await
            {
                if r.version.is_some() {
                    deps.push(ReviewCandidate {
                        content: r.clone(),
                        dependency_type: "required".to_string(),
                        selected: true,
                        parents: vec![parent.clone()],
                        requires_parent: false,
                    });
                    grown.push(r);
                }
            }
        }
        i += 1;
    }
    let resolved_all: Vec<ResolvedContent> = grown;
    let (entries, summary) = build_batch_install_plan(&selected, &deps, game, loader);
    let warnings = build_safety_warnings(&entries, &resolved_all, existing);
    let mut versions: HashMap<String, Version> = HashMap::new();
    for r in &resolved_all {
        if let Some(v) = &r.version {
            versions.insert(v.id.clone(), v.clone());
        }
    }
    Ok(PlanData {
        selected,
        deps,
        entries,
        summary,
        warnings,
        versions,
    })
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
