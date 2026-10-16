//! Plan-first dependency resolution (pure: no network, no fs, no Tauri).
//!
//! Async `Provider` wiring (hydrate, preferred-sha1 lookup over the wire,
//! run-scoped cache) belongs to a later bite. Everything here runs over
//! already-fetched metadata, so `resolve` doubles as the dry-run preview
//! (AGENTS.md §5.4). F-060/F-061 (explicit conflict/cycle detection) are
//! deferred to M3: cycles are cut silently with a `tracing::warn`.

pub mod graph;
pub mod plan;

pub use graph::{
    candidate_loaders_for_project_type, collect_required, compatibility_issue, primary_file,
    review_candidates, select_best_version, select_preferred, ResolvedContent, ReviewCandidate,
};
pub use plan::{
    build_batch_install_plan, build_safety_warnings, requires_confirmation, PlanEntry, PlanSummary,
    Warning,
};
