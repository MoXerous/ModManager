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
pub use resolve::{resolve_dependency, resolve_project, PreferredPin, ResolveParams};
