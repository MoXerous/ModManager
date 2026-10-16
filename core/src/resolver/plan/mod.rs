//! Read-only batch preview (spec §8): downloadable/blocked split,
//! counts summary, confirmation rule, and non-blocking safety warnings.
//! Never touches network, disk, or hashes.
//!
//! `PlanOp Add/Update/Remove` from the master plan is intentionally absent:
//! profile-diff `Update`/`Remove` arrive with the `instances/` bite. This
//! preview's `source` (`selected`|`dependency`) is what F-063 requires.

use super::graph::{primary_file, ResolvedContent, ReviewCandidate};
use std::collections::{HashMap, HashSet};

/// One install row. `status`: `downloadable` | `blocked`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanEntry {
    /// `selected` | `dependency`.
    pub source: String,
    pub project_id: String,
    pub title: String,
    pub project_type: String,
    pub version_id: Option<String>,
    pub version_number: Option<String>,
    pub filename: Option<String>,
    pub status: String,
    pub reason: String,
    pub dependency_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlanSummary {
    pub selected_count: usize,
    pub dependency_count: usize,
    pub total_count: usize,
    pub downloadable_count: usize,
    pub blocked_count: usize,
    pub game_version: String,
    pub loader: String,
    /// Filled by the caller after [`build_safety_warnings`]; the builder
    /// cannot know warnings yet.
    pub warnings_count: usize,
}

/// `warning` | `info`. Nothing here ever blocks (F-066).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    pub severity: String,
    pub message: String,
}

fn project_key(r: &ResolvedContent) -> String {
    if !r.project_id.is_empty() {
        r.project_id.clone()
    } else if !r.slug.is_empty() {
        r.slug.clone()
    } else {
        r.title.clone()
    }
}

fn entry_for(r: &ResolvedContent, source: &str, dependency_type: &str) -> PlanEntry {
    match &r.version {
        None => PlanEntry {
            source: source.to_string(),
            project_id: r.project_id.clone(),
            title: r.title.clone(),
            project_type: r.project_type.clone(),
            version_id: None,
            version_number: None,
            filename: None,
            status: "blocked".to_string(),
            reason: if !r.reason.is_empty() {
                r.reason.clone()
            } else if !r.status.is_empty() {
                r.status.clone()
            } else {
                "No compatible downloadable version was resolved.".to_string()
            },
            dependency_type: dependency_type.to_string(),
        },
        Some(v) => match primary_file(v).filter(|f| !f.url.is_empty()) {
            None => PlanEntry {
                source: source.to_string(),
                project_id: r.project_id.clone(),
                title: r.title.clone(),
                project_type: r.project_type.clone(),
                version_id: Some(v.id.clone()),
                version_number: Some(v.version_number.clone()),
                filename: None,
                status: "blocked".to_string(),
                reason: "A compatible version was resolved, but it has no primary downloadable file URL."
                    .to_string(),
                dependency_type: dependency_type.to_string(),
            },
            Some(f) => PlanEntry {
                source: source.to_string(),
                project_id: r.project_id.clone(),
                title: r.title.clone(),
                project_type: r.project_type.clone(),
                version_id: Some(v.id.clone()),
                version_number: Some(v.version_number.clone()),
                filename: Some(f.filename.clone()),
                status: "downloadable".to_string(),
                reason: "Ready for download.".to_string(),
                dependency_type: dependency_type.to_string(),
            },
        },
    }
}

/// Read-only final preview over already-resolved metadata (spec §8-1..8-5).
/// Selected first; repeated deps drop from `entries` but stay counted in
/// `dependency_count`.
pub fn build_batch_install_plan(
    selected: &[ResolvedContent],
    dependencies: &[ReviewCandidate],
    game_version: &str,
    loader: &str,
) -> (Vec<PlanEntry>, PlanSummary) {
    let mut entries: Vec<PlanEntry> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for r in selected {
        if seen.insert(project_key(r)) {
            entries.push(entry_for(r, "selected", ""));
        }
    }
    for c in dependencies {
        if seen.insert(project_key(&c.content)) {
            entries.push(entry_for(&c.content, "dependency", &c.dependency_type));
        }
    }
    let downloadable_count = entries
        .iter()
        .filter(|e| e.status == "downloadable")
        .count();
    let summary = PlanSummary {
        selected_count: selected.len(),
        dependency_count: dependencies.len(),
        total_count: entries.len(),
        downloadable_count,
        blocked_count: entries.len() - downloadable_count,
        game_version: game_version.to_string(),
        loader: loader.to_string(),
        warnings_count: 0,
    };
    (entries, summary)
}

/// Confirmation rule (spec §8-3). The backup flag arrives with the caller
/// that owns real backup state; until then the four data conditions apply.
pub fn requires_confirmation(summary: &PlanSummary) -> bool {
    summary.total_count > 1
        || summary.dependency_count > 0
        || summary.blocked_count > 0
        || summary.warnings_count > 0
}

/// Local preflight warnings (spec §9): conservative, read-only, and never
/// blocking — severity is only `warning` or `info`. `existing_filenames`
/// is caller-observed state; this function never stats the disk (F-065).
pub fn build_safety_warnings(
    entries: &[PlanEntry],
    resolved: &[ResolvedContent],
    existing_filenames: &HashSet<String>,
) -> Vec<Warning> {
    let mut out: Vec<Warning> = Vec::new();

    // 1. Same project key under two titles.
    let mut titles: HashMap<&str, &str> = HashMap::new();
    for r in resolved {
        let key = if !r.project_id.is_empty() {
            r.project_id.as_str()
        } else if !r.slug.is_empty() {
            r.slug.as_str()
        } else {
            r.title.as_str()
        };
        match titles.get(key) {
            Some(prev) if **prev != r.title => out.push(Warning {
                severity: "warning".to_string(),
                message: format!(
                    "The same Modrinth project appears more than once: {prev} and {}.",
                    r.title
                ),
            }),
            _ => {
                titles.insert(key, r.title.as_str());
            }
        }
    }

    // 2+3. Per-entry verdicts.
    for e in entries {
        if e.status == "blocked" {
            out.push(Warning {
                severity: "warning".to_string(),
                message: if e.reason == "Ready for download." {
                    "No compatible downloadable version was resolved for the selected Minecraft version/platform.".to_string()
                } else {
                    e.reason.clone()
                },
            });
        }
        // 4. Destination file already present.
        if let Some(f) = e.filename.as_deref() {
            if existing_filenames.contains(f) {
                out.push(Warning {
                    severity: "info".to_string(),
                    message: "A file with this name already exists in the destination. The downloader will skip it if the SHA1 matches, or save a unique filename if it differs.".to_string(),
                });
            }
        }
    }

    // 5. Two projects, one filename.
    let mut owners: HashMap<&str, &str> = HashMap::new();
    for e in entries {
        if let Some(f) = e.filename.as_deref() {
            match owners.get(f) {
                Some(prev) if **prev != e.project_id => out.push(Warning {
                    severity: "warning".to_string(),
                    message: format!(
                        "Two different projects resolve to the same filename: {prev} and {} ({f}).",
                        e.project_id
                    ),
                }),
                _ => {
                    owners.insert(f, e.project_id.as_str());
                }
            }
        }
    }

    // 6. Required dep metadata outside the download set (first 6, then ...).
    let keys: HashSet<&str> = entries.iter().map(|e| e.project_id.as_str()).collect();
    let mut missing: Vec<String> = Vec::new();
    for r in resolved {
        if let Some(v) = &r.version {
            for d in &v.dependencies {
                if d.dependency_type != "required" {
                    continue;
                }
                if let Some(id) = d.project_id.as_deref() {
                    if !id.is_empty() && !keys.contains(id) && !missing.iter().any(|m| m == id) {
                        missing.push(id.to_string());
                    }
                }
            }
        }
    }
    if !missing.is_empty() {
        let shown: Vec<&str> = missing.iter().take(6).map(String::as_str).collect();
        let mut msg = format!(
            "Required dependency metadata exists but was not included in the resolved download set: {}.",
            shown.join(", ")
        );
        if missing.len() > 6 {
            msg.push_str(" ...");
        }
        out.push(Warning {
            severity: "warning".to_string(),
            message: msg,
        });
    }

    // 7. Modrinth marks something incompatible.
    for r in resolved {
        let flagged = r.status == "incompatible"
            || r.version
                .as_ref()
                .map(|v| {
                    v.dependencies
                        .iter()
                        .any(|d| d.dependency_type == "incompatible")
                })
                .unwrap_or(false);
        if flagged {
            out.push(Warning {
                severity: "warning".to_string(),
                message: format!(
                    "Modrinth metadata marks one of the selected projects/versions as incompatible: {}.",
                    r.title
                ),
            });
        }
    }

    // 8+9. Optional / embedded counts (counted, never auto-resolved).
    let mut optional = 0usize;
    let mut embedded = 0usize;
    for r in resolved {
        if let Some(v) = &r.version {
            for d in &v.dependencies {
                match d.dependency_type.as_str() {
                    "optional" => optional += 1,
                    "embedded" => embedded += 1,
                    _ => {}
                }
            }
        }
    }
    if optional > 0 {
        let (noun, verb) = if optional == 1 {
            ("dependency", "was")
        } else {
            ("dependencies", "were")
        };
        out.push(Warning {
            severity: "info".to_string(),
            message: format!(
                "This version lists {optional} optional {noun} which {verb} not auto-selected; they are shown unchecked. If you include one, its required dependencies are included automatically."
            ),
        });
    }
    if embedded > 0 {
        let noun = if embedded == 1 {
            "dependency"
        } else {
            "dependencies"
        };
        out.push(Warning {
            severity: "info".to_string(),
            message: format!("This version lists {embedded} embedded {noun} in Modrinth metadata."),
        });
    }

    out
}

#[cfg(test)]
pub(crate) mod tests;
