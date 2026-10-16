//! Plan tests: read-only preview + non-blocking warnings.

use super::*;
use crate::resolver::graph::tests::{file, resolved, version};
use crate::resolver::graph::{ResolvedContent, ReviewCandidate};
use std::collections::HashSet;

fn block(a: &str) -> ResolvedContent {
    resolved(a, None)
}

fn dep_candidate(id: &str, kind: &str) -> ReviewCandidate {
    ReviewCandidate {
        content: resolved(id, Some(version("v", id))),
        dependency_type: kind.to_string(),
        selected: kind == "required",
        parents: vec!["a".to_string()],
        requires_parent: false,
    }
}

#[test]
fn spec_f063_plan_splits_downloadable_and_blocked() {
    let good = resolved("a", Some(version("v1", "a")));
    let blocked = block("b");
    let mut no_url = version("v2", "c");
    no_url.files.iter_mut().for_each(|f| f.url.clear());
    let nourl = resolved("c", Some(no_url));
    let (entries, summary) =
        build_batch_install_plan(&[good, blocked, nourl], &[], "1.21", "fabric");
    assert_eq!(entries.len(), 3);
    let dl: Vec<&PlanEntry> = entries
        .iter()
        .filter(|e| e.status == "downloadable")
        .collect();
    let bl: Vec<&PlanEntry> = entries.iter().filter(|e| e.status == "blocked").collect();
    assert_eq!(dl.len(), 1);
    assert_eq!(bl.len(), 2);
    assert_eq!(dl[0].reason, "Ready for download.");
    assert_eq!(dl[0].source, "selected");
    // No primary URL -> the spec's exact blocked reason.
    let url_block = bl.iter().find(|e| e.project_id == "c").expect("c blocked");
    assert_eq!(
        url_block.reason,
        "A compatible version was resolved, but it has no primary downloadable file URL."
    );
    assert_eq!(summary.selected_count, 3);
    assert_eq!(summary.downloadable_count, 1);
    assert_eq!(summary.blocked_count, 2);
    assert_eq!(summary.total_count, 3);
}

#[test]
fn spec_f063_duplicates_dropped_but_counted() {
    // Selected first; a repeated dependency drops from entries but still
    // counts in dependency_count.
    let a = resolved("a", Some(version("v1", "a")));
    let deps = vec![
        dep_candidate("b", "required"),
        dep_candidate("b", "required"),
    ];
    let (entries, summary) = build_batch_install_plan(&[a], &deps, "1.21", "fabric");
    let b_entries: Vec<&PlanEntry> = entries.iter().filter(|e| e.project_id == "b").collect();
    assert_eq!(b_entries.len(), 1, "dupes collapse to one entry");
    assert_eq!(b_entries[0].source, "dependency");
    assert_eq!(summary.dependency_count, 2);
    assert_eq!(summary.total_count, entries.len());
}

#[test]
fn spec_f063_requires_confirmation_truth_table() {
    // Single downloadable, no deps/warnings -> no confirmation.
    let good = resolved("a", Some(version("v1", "a")));
    let (_, s) = build_batch_install_plan(&[good], &[], "1.21", "fabric");
    let mut clean = s;
    clean.warnings_count = 0;
    assert!(!requires_confirmation(&clean));
    // total > 1 -> confirm.
    let (_, s) = build_batch_install_plan(
        &[
            resolved("a", Some(version("v1", "a"))),
            resolved("b", Some(version("v2", "b"))),
        ],
        &[],
        "1.21",
        "fabric",
    );
    assert!(requires_confirmation(&s));
    // dependency present -> confirm even at total 1.
    let (_, s) = build_batch_install_plan(
        &[resolved("a", Some(version("v1", "a")))],
        &[dep_candidate("b", "required")],
        "1.21",
        "fabric",
    );
    assert!(requires_confirmation(&s));
    // blocked -> confirm.
    let (_, mut s) = build_batch_install_plan(&[block("a")], &[], "1.21", "fabric");
    s.warnings_count = 0;
    assert!(requires_confirmation(&s));
    // warnings -> confirm.
    s.blocked_count = 0;
    s.warnings_count = 1;
    assert!(requires_confirmation(&s));
}

#[test]
fn spec_f064_warnings_none_blocking() {
    let good = resolved("a", Some(version("v1", "a")));
    let blocked = block("b");
    let (entries, _) =
        build_batch_install_plan(&[good.clone(), blocked.clone()], &[], "1.21", "fabric");
    let ws = build_safety_warnings(&entries, &[good, blocked], &HashSet::new());
    assert!(!ws.is_empty());
    for w in &ws {
        assert!(
            w.severity == "warning" || w.severity == "info",
            "never error/blocking: {}",
            w.severity
        );
    }
    // Blocked item surfaces its reason as a warning (spec check 2).
    assert!(ws
        .iter()
        .any(|w| w.message.contains('b') || w.message.contains("compatible")));
}

#[test]
fn spec_f065_existing_destination_file_is_info() {
    let v = version("v1", "a");
    let fname = v.files[0].filename.clone();
    let good = resolved("a", Some(v));
    let (entries, _) = build_batch_install_plan(std::slice::from_ref(&good), &[], "1.21", "fabric");
    let existing: HashSet<String> = [fname].into_iter().collect();
    let ws = build_safety_warnings(&entries, &[good], &existing);
    let info = ws.iter().find(|w| w.severity == "info").expect("info warn");
    assert!(info.message.contains("already exists"), "{}", info.message);
}

#[test]
fn spec_f066_nothing_ever_blocks() {
    // Even two projects mapping to the same filename is a warning, not an error.
    let mut v1 = version("v1", "a");
    v1.files = vec![file("same.jar", true)];
    let mut v2 = version("v2", "b");
    v2.files = vec![file("same.jar", true)];
    let (entries, _) = build_batch_install_plan(
        &[resolved("a", Some(v1)), resolved("b", Some(v2))],
        &[],
        "1.21",
        "fabric",
    );
    let ws = build_safety_warnings(&entries, &[], &HashSet::new());
    assert!(ws.iter().any(|w| w.message.contains("same filename")));
    assert!(ws.iter().all(|w| w.severity != "error"));
}
