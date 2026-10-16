//! أوامر IPC. هذا الملف جسر رقيق فقط: لا منطق عمل هنا، المنطق كله في mm-core.
//! مسارات الملفات من الـwebview إدخال غير موثوق: تُحلَّل وتُتحقَّق قبل أي I/O.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use tauri::ipc::Channel;

use crate::dto::{
    AppInfo, DownloadEvent, ErrorCode, InstallPlanDto, InstanceSnapshotDto, IpcError, LocalFactDto,
    LockEntryDto, PlanEntryDto, PlanSummaryDto, PreviewEntryDto, PreviewToggleDto, ProjectDto,
    SearchPageDto, SearchQueryDto, TogglePlanDto, ToggleResultDto, WarningDto,
};

#[tauri::command]
pub fn app_info() -> AppInfo {
    mm_core::app_info().into()
}

/// تحميل تجريبي يبثّ تقدمه عبر Channel (أسرع وأكثر ترتيباً من Events).
#[tauri::command]
pub async fn start_demo_download(on_event: Channel<DownloadEvent>) -> Result<(), IpcError> {
    tauri::async_runtime::spawn_blocking(move || {
        mm_core::demo::run(5_000_000, 20, Duration::from_millis(100), |progress| {
            // إغلاق الواجهة أثناء التحميل لا يجب أن يوقف العملية.
            let _ = on_event.send(progress.into());
        });
    })
    .await
    .map_err(IpcError::internal)
}

#[tauri::command]
pub async fn search_projects(query: SearchQueryDto) -> Result<SearchPageDto, IpcError> {
    use mm_core::providers::{ModrinthClient, Provider};
    let client = ModrinthClient::new().map_err(|e| IpcError::new(ErrorCode::Internal, e))?;
    let page = client
        .search(&query.to_core())
        .await
        .map_err(|e| IpcError::from_provider(&e))?;
    Ok(SearchPageDto {
        hits: page
            .hits
            .iter()
            .map(|p| ProjectDto::from_core(p, "modrinth"))
            .collect(),
        total: page.total,
    })
}

/// Resolve-only preview: never writes, never downloads (F-063, dry-run by
/// construction). `existing_files` feeds the F-065 info warning.
#[tauri::command]
pub async fn build_install_plan(
    slugs: Vec<String>,
    game_version: String,
    loader: String,
    instance_root: Option<String>,
) -> Result<InstallPlanDto, IpcError> {
    use mm_core::providers::ModrinthClient;
    use mm_core::resolver::resolve_and_plan;
    if slugs.is_empty() {
        return Err(IpcError::invalid_input("no projects given"));
    }
    let client = ModrinthClient::new().map_err(|e| IpcError::new(ErrorCode::Internal, e))?;
    let existing = existing_filenames(instance_root)?;
    let plan = resolve_and_plan(&client, &slugs, &game_version, &loader, &existing)
        .await
        .map_err(|e| IpcError::from_provider(&e))?;
    Ok(InstallPlanDto {
        entries: plan.entries.iter().map(PlanEntryDto::from_core).collect(),
        summary: PlanSummaryDto {
            selected_count: plan.summary.selected_count,
            dependency_count: plan.summary.dependency_count,
            total_count: plan.summary.total_count,
            downloadable_count: plan.summary.downloadable_count,
            blocked_count: plan.summary.blocked_count,
            game_version: plan.summary.game_version.clone(),
            loader: plan.summary.loader.clone(),
            requires_confirmation: mm_core::resolver::requires_confirmation(&plan.summary),
        },
        warnings: plan.warnings.iter().map(WarningDto::from_core).collect(),
    })
}

/// Read-only instance inventory: mods facts + locked entries + loaders.
#[tauri::command]
pub async fn scan_instance(instance_root: String) -> Result<InstanceSnapshotDto, IpcError> {
    let root = resolve_dir(&instance_root)?;
    tauri::async_runtime::spawn_blocking(move || {
        use mm_core::instances::{read_lock, scan_mods, Instance};
        use mm_core::jar_reader::{detect_loaders, ScanCache};
        let mods = scan_mods(&Instance {
            root: root.clone(),
            minecraft_root: root.clone(),
        });
        let locked = match read_lock(&root.join("instance.lock.json")) {
            Ok(l) => l
                .entries
                .iter()
                .map(|e| LockEntryDto {
                    provider: e.provider.clone(),
                    project_id: e.project_id.clone(),
                    slug: e.slug.clone(),
                    version_id: e.version_id.clone(),
                    filename: e.filename.clone(),
                    sha1: e.sha1.clone(),
                })
                .collect(),
            // A newer lock schema is a typed refusal, never an empty list
            // (§5.10) — this is the only place that surfaces it.
            Err(e @ mm_core::instances::LockError::NewerSchema { .. }) => {
                return Err(IpcError::from_lock(&e));
            }
            Err(_) => Vec::new(),
        };
        // Authoritative launchers are empty for now; jar scan fills it
        // (F-092/F-093 sources land with profile support in M2).
        let mut cache = ScanCache::new();
        let mods_dir = root.join("mods");
        let loaders = detect_loaders(&[], &mods_dir, &mut cache);
        let source = if loaders.is_empty() {
            "unknown"
        } else {
            "jars"
        };
        Ok(InstanceSnapshotDto {
            loaders: loaders.iter().map(|l| l.as_str().to_string()).collect(),
            loader_source: source.to_string(),
            mods: mods
                .iter()
                .map(|f| LocalFactDto {
                    filename: f.filename.clone(),
                    size: f.size,
                    sha1: f.sha1.clone(),
                    resource_type: format!("{:?}", f.resource_type).to_lowercase(),
                    disabled: f.disabled,
                    project_id: locked
                        .iter()
                        .find(|l| l.filename == f.filename)
                        .map(|l| l.project_id.clone()),
                })
                .collect(),
            locked,
        })
    })
    .await
    .map_err(IpcError::internal)?
}

#[tauri::command]
pub async fn preview_toggle(req: PreviewToggleDto) -> Result<TogglePlanDto, IpcError> {
    let mods_dir = resolve_dir(&req.mods_dir)?;
    let files = filenames(&req.files);
    tauri::async_runtime::spawn_blocking(move || {
        use mm_core::instances::toggle::preview_toggle;
        let plan = preview_toggle(&mods_dir, req.action.into(), &files);
        Ok(TogglePlanDto {
            entries: plan
                .entries
                .iter()
                .map(|e| PreviewEntryDto {
                    filename: file_name(&e.source),
                    dest_filename: file_name(&e.dest),
                    allowed: e.allowed,
                    reason: e.reason.clone(),
                })
                .collect(),
        })
    })
    .await
    .map_err(IpcError::internal)?
}

/// Rename-only mutation. The plan is **recomputed server-side** from the
/// same inputs — a plan body from the webview is never trusted.
#[tauri::command]
pub async fn execute_toggle(req: PreviewToggleDto) -> Result<Vec<ToggleResultDto>, IpcError> {
    let mods_dir = resolve_dir(&req.mods_dir)?;
    let files = filenames(&req.files);
    tauri::async_runtime::spawn_blocking(move || {
        use mm_core::instances::toggle::{execute_toggle, preview_toggle};
        let action = req.action.into();
        let plan = preview_toggle(&mods_dir, action, &files);
        let results = execute_toggle(&plan);
        Ok(results
            .iter()
            .map(|r| ToggleResultDto {
                filename: file_name(&r.source),
                dest_filename: file_name(&r.dest),
                state: format!("{:?}", r.state),
                reason: r.reason.clone(),
            })
            .collect())
    })
    .await
    .map_err(IpcError::internal)?
}

// --- shared guards ------------------------------------------------------

/// Canonicalize a webview-supplied directory: missing or non-dir → typed
/// error. Symlinks resolve through canonicalize, so the caller gets the
/// real target.
fn resolve_dir(raw: &str) -> Result<PathBuf, IpcError> {
    let p = PathBuf::from(raw.trim());
    if p.as_os_str().is_empty() {
        return Err(IpcError::invalid_input("empty path"));
    }
    let canon = std::fs::canonicalize(&p)
        .map_err(|e| IpcError::io(format!("cannot open {}: {e}", p.display())))?;
    if !canon.is_dir() {
        return Err(IpcError::invalid_input("not a folder"));
    }
    Ok(canon)
}

/// Filenames only: a traversing, nested, or blank name is dropped here, and
/// core re-checks containment under `mods_dir` anyway. The check is on the
/// **raw** string: `file_name()` alone would hide `../escape.jar`.
fn filenames(raw: &[String]) -> Vec<PathBuf> {
    raw.iter()
        .filter_map(|s| {
            let trimmed = s.trim();
            if trimmed.is_empty()
                || trimmed.contains('/')
                || trimmed.contains('\\')
                || trimmed == ".."
                || trimmed.contains(':')
            {
                return None;
            }
            Some(PathBuf::from(trimmed))
        })
        .collect()
}

fn file_name(p: &std::path::Path) -> String {
    p.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string()
}

/// Filenames already present in the instance mods dir (F-065 info warning).
fn existing_filenames(instance_root: Option<String>) -> Result<HashSet<String>, IpcError> {
    let root = match instance_root {
        Some(r) => resolve_dir(&r)?,
        None => return Ok(HashSet::new()),
    };
    let mut out = HashSet::new();
    if let Ok(rd) = std::fs::read_dir(root.join("mods")) {
        for e in rd.flatten() {
            if let Some(n) = e.file_name().to_str() {
                out.insert(n.to_string());
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests;
