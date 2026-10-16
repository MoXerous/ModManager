//! Enable/disable by rename only (`.jar` ↔ `.jar.disabled`, F-067).
//! Plan first, execute second (§5.4): preview guards, then renames with
//! reverse-order rollback on first failure.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToggleAction {
    Disable,
    Enable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewEntry {
    pub source: PathBuf,
    pub dest: PathBuf,
    pub allowed: bool,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TogglePlan {
    pub action: Option<ToggleAction>,
    pub entries: Vec<PreviewEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToggleState {
    Success,
    Failed,
    Skipped,
    RolledBack,
    RollbackFailed,
    RollbackSkipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToggleResult {
    pub source: PathBuf,
    pub dest: PathBuf,
    pub state: ToggleState,
    pub reason: String,
}

fn canonical(dir: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(dir).ok()
}

fn is_link(p: &Path) -> bool {
    std::fs::symlink_metadata(p)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

fn in_disabled_dir(root: &Path, canon_src: &Path) -> bool {
    // Canonical-relative parent is exactly `disabled_mods/`.
    match canon_src.strip_prefix(root) {
        Ok(rel) => rel
            .parent()
            .and_then(|p| p.to_str())
            .map(|p| p.to_lowercase() == "disabled_mods")
            .unwrap_or(false),
        Err(_) => false,
    }
}

fn disabled_suffix(name: &str) -> bool {
    name.to_lowercase().ends_with(".jar.disabled")
}

fn jar_name(name: &str) -> bool {
    name.to_lowercase().ends_with(".jar")
}

/// Guarded preview: every file gets an allow/deny verdict with the exact
/// legacy reason. Nothing is renamed here.
pub fn preview_toggle(mods_dir: &Path, action: ToggleAction, files: &[PathBuf]) -> TogglePlan {
    let root = match canonical(mods_dir) {
        Some(r) => r,
        None => {
            return TogglePlan {
                action: Some(action),
                entries: files
                    .iter()
                    .map(|s| PreviewEntry {
                        source: s.clone(),
                        dest: s.clone(),
                        allowed: false,
                        reason: "Mods folder cannot be resolved.".to_string(),
                    })
                    .collect(),
            };
        }
    };
    if !root.is_dir() {
        return TogglePlan {
            action: Some(action),
            entries: files
                .iter()
                .map(|s| PreviewEntry {
                    source: s.clone(),
                    dest: s.clone(),
                    allowed: false,
                    reason: "Mods folder does not exist.".to_string(),
                })
                .collect(),
        };
    }
    let entries = files
        .iter()
        .map(|s| preview_one(&root, action, s))
        .collect();
    TogglePlan {
        action: Some(action),
        entries,
    }
}

fn preview_one(root: &Path, action: ToggleAction, source: &Path) -> PreviewEntry {
    let deny = |reason: &str| PreviewEntry {
        source: source.to_path_buf(),
        dest: source.to_path_buf(),
        allowed: false,
        reason: reason.to_string(),
    };
    if is_link(source) {
        return deny("Symlink source skipped for safety.");
    }
    if !source.is_file() {
        return deny("Source file does not exist.");
    }
    let canon_src = match std::fs::canonicalize(source) {
        Ok(p) => p,
        Err(_) => return deny("Source file does not exist."),
    };
    if !canon_src.starts_with(root) {
        return deny("Source file is outside the active profile mods folder.");
    }
    let name = source
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    match action {
        ToggleAction::Disable => {
            let top_level = canon_src
                .strip_prefix(root)
                .ok()
                .and_then(|rel| rel.parent())
                .map(|p| p.as_os_str().is_empty())
                .unwrap_or(false);
            if !top_level {
                return deny("Only top-level active mod files can be disabled by rename.");
            }
            if disabled_suffix(&name) || in_disabled_dir(root, &canon_src) {
                return deny("File already appears disabled.");
            }
            if !jar_name(&name) {
                return deny("Only .jar files can be disabled in this phase.");
            }
            let dest = source.with_file_name(format!("{name}.disabled"));
            if dest.exists() {
                return deny("Destination already exists; overwrite is not allowed.");
            }
            PreviewEntry {
                source: source.to_path_buf(),
                dest,
                allowed: true,
                reason: String::new(),
            }
        }
        ToggleAction::Enable => {
            if disabled_suffix(&name) {
                let base = name[..name.len() - ".disabled".len()].to_string();
                let dest = source.with_file_name(base);
                if dest.exists() {
                    return deny("Destination already exists; overwrite is not allowed.");
                }
                return PreviewEntry {
                    source: source.to_path_buf(),
                    dest,
                    allowed: true,
                    reason: String::new(),
                };
            }
            if jar_name(&name) && in_disabled_dir(root, &canon_src) {
                let dest = root.join(&name);
                if dest.exists() {
                    return deny("Destination already exists; overwrite is not allowed.");
                }
                return PreviewEntry {
                    source: source.to_path_buf(),
                    dest,
                    allowed: true,
                    reason: String::new(),
                };
            }
            deny("Only .jar.disabled files or .jar files inside disabled_mods can be enabled in this phase.")
        }
    }
}

/// Execute a preview: renames in order; the first failure marks every
/// remaining allowed entry `skipped` and rolls completed ones back in
/// reverse order (spec §4-4).
pub fn execute_toggle(plan: &TogglePlan) -> Vec<ToggleResult> {
    let mut out: Vec<ToggleResult> = Vec::with_capacity(plan.entries.len());
    let mut completed: Vec<usize> = Vec::new();
    let mut failed = false;
    for (i, e) in plan.entries.iter().enumerate() {
        if failed || !e.allowed {
            out.push(ToggleResult {
                source: e.source.clone(),
                dest: e.dest.clone(),
                state: ToggleState::Skipped,
                reason: if e.allowed {
                    "Skipped after an earlier failure.".to_string()
                } else {
                    e.reason.clone()
                },
            });
            continue;
        }
        // Re-checked at execution: std rename replaces on some platforms,
        // but overwrite is never allowed (spec §4-3).
        if e.dest.exists() {
            failed = true;
            out.push(ToggleResult {
                source: e.source.clone(),
                dest: e.dest.clone(),
                state: ToggleState::Failed,
                reason: "Destination already exists; nothing was overwritten.".to_string(),
            });
            continue;
        }
        match std::fs::rename(&e.source, &e.dest) {
            Ok(()) => {
                completed.push(i);
                out.push(ToggleResult {
                    source: e.source.clone(),
                    dest: e.dest.clone(),
                    state: ToggleState::Success,
                    reason: String::new(),
                });
            }
            Err(_) => {
                failed = true;
                out.push(ToggleResult {
                    source: e.source.clone(),
                    dest: e.dest.clone(),
                    state: ToggleState::Failed,
                    reason: "Destination already exists; nothing was overwritten.".to_string(),
                });
            }
        }
    }
    if failed {
        for &i in completed.iter().rev() {
            let e = &plan.entries[i];
            let state = if !e.dest.exists() || e.source.exists() {
                ToggleState::RollbackSkipped
            } else {
                match std::fs::rename(&e.dest, &e.source) {
                    Ok(()) => ToggleState::RolledBack,
                    Err(_) => ToggleState::RollbackFailed,
                }
            };
            out[i].state = state;
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests;
