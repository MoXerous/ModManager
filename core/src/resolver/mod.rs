//! Plan-first dependency resolution (pure + async orchestration).
//!
//! `graph`/`plan` are pure over already-fetched metadata, so `resolve`
//! doubles as the dry-run preview (AGENTS.md §5.4). `cache`/`resolve`
//! wire a [`Provider`] per run. F-060/F-061 (explicit conflict/cycle
//! detection) are deferred to M3: cycles are cut silently with a warn.

pub mod batch;
pub mod cache;
pub mod graph;
pub mod plan;
pub mod resolve;

pub use batch::{resolve_and_plan, BatchPlan};

pub use cache::{BestKey, HashKey, ResolveKey, RunCache};
pub use graph::{
    candidate_loaders_for_project_type, collect_required, compatibility_issue, primary_file,
    review_candidates, select_best_version, select_preferred, ResolvedContent, ReviewCandidate,
};
pub use plan::{
    build_batch_install_plan, build_safety_warnings, requires_confirmation, PlanEntry, PlanSummary,
    Warning,
};
pub use resolve::{resolve_dependency, resolve_project, PreferredPin, ResolveError, ResolveParams};

#[cfg(test)]
mod provider_literal_guard {
    //! ADR 0004 §2-3: provider names are filled from `Provider::provider_id()`
    //! only. This reads source text because a proc macro would be a new
    //! dependency for one assertion.
    use std::path::{Path, PathBuf};

    fn scan(dir: &Path, offenders: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                scan(&path, offenders);
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with(".rs") || name.ends_with("tests.rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            // Inline test modules may name fixtures; production code may not.
            let production = text.split("#[cfg(test)]").next().unwrap_or("");
            if production.contains("\"modrinth\"") {
                offenders.push(path.display().to_string());
            }
        }
    }

    #[test]
    fn spec_f014_no_hardcoded_provider_name_outside_provider_modules() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut offenders = Vec::new();
        for rel in ["src/resolver", "../cli/src", "../src-tauri/src"] {
            scan(&root.join(rel), &mut offenders);
        }
        assert!(
            offenders.is_empty(),
            "hard-coded \"modrinth\" outside provider modules: {offenders:?}"
        );
    }
}
