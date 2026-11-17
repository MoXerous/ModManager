//! Graph tests: fixtures + spec-ID assertions (all pure, no network).

use super::*;
use crate::providers::{ProjectFile, Version, VersionDependency};

pub(crate) fn file(name: &str, primary: bool) -> ProjectFile {
    ProjectFile {
        filename: name.to_string(),
        url: format!("https://cdn.modrinth.com/{name}"),
        size: 8,
        sha1: Some(format!("sha1-{name}")),
        sha512: None,
        primary,
    }
}

pub(crate) fn version(id: &str, project: &str) -> Version {
    Version {
        id: id.to_string(),
        project_id: project.to_string(),
        name: id.to_string(),
        version_number: id.to_string(),
        changelog: None,
        date_published: "2024-01-01T00:00:00Z".to_string(),
        version_type: "release".to_string(),
        status: "listed".to_string(),
        files: vec![file("x.jar", true)],
        game_versions: vec!["1.21".to_string()],
        loaders: vec!["fabric".to_string()],
        dependencies: Vec::new(),
    }
}

fn dep(project: &str, kind: &str) -> VersionDependency {
    VersionDependency {
        project_id: Some(project.to_string()),
        version_id: None,
        file_name: None,
        dependency_type: kind.to_string(),
    }
}

pub(crate) fn resolved(project: &str, v: Option<Version>) -> ResolvedContent {
    let status = if v.is_some() {
        "compatible"
    } else {
        "incompatible"
    };
    ResolvedContent {
        provider: "fake".to_string(),
        project_id: project.to_string(),
        slug: project.to_string(),
        title: format!("Title {project}"),
        project_type: "mod".to_string(),
        version: v,
        status: status.to_string(),
        reason: String::new(),
    }
}

fn cands(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn spec_f051_strict_mod_returns_only_selected() {
    // The false-conflict guard: Fabric must never fall through to Forge.
    assert_eq!(
        candidate_loaders_for_project_type("mod", "fabric"),
        vec!["fabric".to_string()]
    );
    assert_eq!(
        candidate_loaders_for_project_type("modpack", "forge"),
        vec!["forge".to_string()]
    );
    assert_eq!(
        candidate_loaders_for_project_type("MOD", " Fabric "),
        vec!["fabric".to_string()]
    );
}

#[test]
fn spec_f052_non_strict_fallback_tables() {
    assert_eq!(
        candidate_loaders_for_project_type("shader", "optifine"),
        ["optifine", "iris", "vanilla", "minecraft"]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    );
    // Unknown selection is ignored: full table back.
    assert_eq!(
        candidate_loaders_for_project_type("shader", "neoforge"),
        ["iris", "optifine", "vanilla", "minecraft"]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        candidate_loaders_for_project_type("resourcepack", ""),
        vec!["minecraft".to_string()]
    );
    assert_eq!(
        candidate_loaders_for_project_type("datapack", "any"),
        vec!["datapack".to_string(), "minecraft".to_string()]
    );
    assert_eq!(
        candidate_loaders_for_project_type("plugin", "paper"),
        [
            "paper",
            "spigot",
            "bukkit",
            "folia",
            "purpur",
            "velocity",
            "bungeecord",
            "sponge"
        ]
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
    );
}

#[test]
fn spec_f050_compat_messages_in_exact_order() {
    // 1: no version.
    assert_eq!(
        compatibility_issue(None, "1.21", &cands(&["fabric"])).as_deref(),
        Some("No version was selected.")
    );
    // 2: no primary file (empty files).
    let mut v = version("v1", "a");
    v.files.clear();
    assert_eq!(
        compatibility_issue(Some(&v), "1.21", &cands(&["fabric"])).as_deref(),
        Some("Selected version has no downloadable primary file.")
    );
    // 2b: files but none primary.
    let mut v = version("v1", "a");
    v.files = vec![file("x.jar", false)];
    assert_eq!(
        compatibility_issue(Some(&v), "1.21", &cands(&["fabric"])).as_deref(),
        Some("Selected version has no downloadable primary file.")
    );
    // 3: status beats game version (order guard: both fail -> status wins).
    let mut v = version("v1", "a");
    v.status = "archived".to_string();
    v.game_versions = vec!["1.20".to_string()];
    assert_eq!(
        compatibility_issue(Some(&v), "1.21", &cands(&["fabric"])).as_deref(),
        Some("Selected version is not listed for download (status: archived).")
    );
    // 4: game version names target + lists available.
    let mut v = version("v1", "a");
    v.game_versions = vec!["1.20".to_string()];
    let m = compatibility_issue(Some(&v), "1.21", &cands(&["fabric"])).expect("issue");
    assert!(m.contains("1.21"), "{m}");
    assert!(m.contains("1.20"), "{m}");
    // 4b: empty game_versions prints "none listed".
    let mut v = version("v1", "a");
    v.game_versions.clear();
    let m = compatibility_issue(Some(&v), "1.21", &cands(&["fabric"])).expect("issue");
    assert!(m.contains("none listed"), "{m}");
    // 5: loader checks ALL candidates, names loader + lists.
    let mut v = version("v1", "a");
    v.loaders = vec!["forge".to_string()];
    let m = compatibility_issue(Some(&v), "1.21", &cands(&["fabric"])).expect("issue");
    assert!(m.contains("fabric"), "{m}");
    assert!(m.contains("forge"), "{m}");
    // Compatible -> None.
    let v = version("v1", "a");
    assert_eq!(
        compatibility_issue(Some(&v), "1.21", &cands(&["fabric"])),
        None
    );
    // "any" target skips game/loader checks.
    let mut v = version("v1", "a");
    v.game_versions = vec!["1.20".to_string()];
    v.loaders = vec!["forge".to_string()];
    assert_eq!(compatibility_issue(Some(&v), "any", &cands(&["any"])), None);
}

#[test]
fn spec_f050_select_best_skips_alpha_unless_allowed() {
    let mut beta = version("b", "a");
    beta.version_type = "beta".to_string();
    let mut alpha = version("a", "a");
    alpha.id = "a".to_string();
    alpha.version_number = "a".to_string();
    alpha.name = "a".to_string();
    alpha.version_type = "alpha".to_string();
    let vs = vec![alpha, beta];
    // F-003: the ladder wins first — with beta allowed, beta is the pick even
    // though alpha is allowed too.
    let best = select_best_version(&vs, "1.21", &cands(&["fabric"]), true, false).expect("best");
    assert_eq!(best.version_type, "beta");
    // The alpha gate stays: with beta not allowed and alpha allowed, the
    // ladder reaches alpha.
    let best = select_best_version(&vs, "1.21", &cands(&["fabric"]), false, true).expect("best");
    assert_eq!(best.version_type, "alpha");
    // Nothing compatible -> None.
    let mut v = version("v1", "a");
    v.game_versions = vec!["1.20".to_string()];
    assert_eq!(
        select_best_version(&[v], "1.21", &cands(&["fabric"]), true, true),
        None
    );
}

/// F-003: the old app's ladder is release > beta (`allow_beta` defaults
/// true) > alpha, newest by `date_published` inside a channel. A newer beta
/// must NOT beat an older stable.
#[test]
fn spec_f003_a_newer_beta_does_not_beat_an_older_stable() {
    let mut release = version("r", "a");
    release.date_published = "2024-01-01T00:00:00Z".to_string();
    let mut beta = version("b", "a");
    beta.version_type = "beta".to_string();
    beta.date_published = "2024-06-01T00:00:00Z".to_string();
    // Newest-first input order, as the providers answer: the beta is first.
    let vs = vec![beta, release];
    let best = select_best_version(&vs, "1.21", &cands(&["fabric"]), true, false).expect("best");
    assert_eq!(
        best.id, "r",
        "the older stable must win over the newer beta"
    );
}

/// F-003: within one channel the pick is the newest by `date_published`,
/// never the first compatible row in input order.
#[test]
fn spec_f003_the_newest_of_the_channel_wins_not_the_first_in_input() {
    let mut old = version("old", "a");
    old.date_published = "2024-01-01T00:00:00Z".to_string();
    let mut new = version("new", "a");
    new.date_published = "2024-06-01T00:00:00Z".to_string();
    // An older release listed first must not win.
    let vs = vec![old, new];
    let best = select_best_version(&vs, "1.21", &cands(&["fabric"]), true, false).expect("best");
    assert_eq!(best.id, "new", "the newest release wins by date_published");
}

/// F-003: the switches gate their channels, and when no allowed channel has
/// a candidate the fallback is the newest compatible version of all — the
/// old app's final `sorted(versions)[0]` (CurseForge's `unknown:<n>` labels
/// land there).
#[test]
fn spec_f003_the_switches_gate_their_channels_and_the_fallback_is_newest_of_all() {
    let mut beta = version("b", "a");
    beta.version_type = "beta".to_string();
    beta.date_published = "2024-06-01T00:00:00Z".to_string();
    let mut alpha = version("a", "a");
    alpha.id = "a2".to_string();
    alpha.version_number = "a2".to_string();
    alpha.name = "a2".to_string();
    alpha.version_type = "alpha".to_string();
    alpha.date_published = "2024-03-01T00:00:00Z".to_string();
    let vs = vec![alpha.clone(), beta.clone()];
    // No release exists: the ladder falls through to beta.
    let best = select_best_version(&vs, "1.21", &cands(&["fabric"]), true, false).expect("beta");
    assert_eq!(best.id, "b");
    // `allow_beta = false` closes the beta channel: only alpha is left.
    let best = select_best_version(&vs, "1.21", &cands(&["fabric"]), false, true).expect("alpha");
    assert_eq!(best.id, "a2");
    // Both rated channels closed: a closed channel stays closed — only an
    // unrated channel label (CurseForge's `unknown:<n>`) reaches the
    // newest-of-all fallback.
    assert_eq!(
        select_best_version(&vs, "1.21", &cands(&["fabric"]), false, false),
        None
    );
}

/// F-003: CurseForge files carry `unknown:<n>` instead of a channel name.
/// The ladder never matches them, so they answer through the fallback: the
/// newest compatible unlabelled version, never the first in input order.
#[test]
fn spec_f003_an_unlabelled_channel_still_answers_via_the_fallback() {
    let mut old = version("old", "a");
    old.version_type = "unknown:1".to_string();
    old.date_published = "2024-01-01T00:00:00Z".to_string();
    let mut new = version("new", "a");
    new.version_type = "unknown:1".to_string();
    new.date_published = "2024-06-01T00:00:00Z".to_string();
    let vs = vec![old, new];
    let best = select_best_version(&vs, "1.21", &cands(&["fabric"]), true, false).expect("fb");
    assert_eq!(best.id, "new", "the newest unrated version wins");
}

#[test]
fn spec_f056_preferred_pin_order_id_sha1_number() {
    let v1 = version("v1", "a");
    let mut v2 = version("v2", "a");
    v2.files = vec![ProjectFile {
        sha1: Some("abc123".to_string()),
        ..file("y.jar", true)
    }];
    let mut v3 = version("v3", "a");
    v3.version_number = "1.0.0".to_string();
    let vs = vec![v1, v2, v3];
    // id wins even when not first.
    let got = select_preferred(
        &vs,
        "1.21",
        &cands(&["fabric"]),
        Some("v3"),
        None,
        None,
        false,
    )
    .expect("pin");
    assert_eq!(got.id, "v3");
    // sha1 fallback.
    let got = select_preferred(
        &vs,
        "1.21",
        &cands(&["fabric"]),
        None,
        Some("ABC123"),
        None,
        false,
    )
    .expect("pin");
    assert_eq!(got.id, "v2");
    // number fallback.
    let got = select_preferred(
        &vs,
        "1.21",
        &cands(&["fabric"]),
        None,
        None,
        Some("1.0.0"),
        false,
    )
    .expect("pin");
    assert_eq!(got.id, "v3");
}

#[test]
fn spec_f057_incompatible_pin_falls_back_to_best() {
    let mut pinned = version("pinned", "a");
    pinned.game_versions = vec!["1.20".to_string()];
    let good = version("good", "a");
    let vs = vec![pinned, good];
    let got = select_preferred(
        &vs,
        "1.21",
        &cands(&["fabric"]),
        Some("pinned"),
        None,
        None,
        false,
    )
    .expect("fallback");
    assert_eq!(got.id, "good");
}

#[test]
fn spec_f053_bfs_dedups_and_cuts_cycles_silently() {
    // a -> b (required), b -> a (cycle). Both listed; each added once.
    let mut va = version("va", "a");
    va.dependencies = vec![dep("b", "required")];
    let mut vb = version("vb", "b");
    vb.dependencies = vec![dep("a", "required")];
    let ra = resolved("a", Some(va));
    let rb = resolved("b", Some(vb));
    let catalog: HashMap<String, ResolvedContent> =
        [("a".to_string(), ra.clone()), ("b".to_string(), rb.clone())]
            .into_iter()
            .collect();
    let out = collect_required(&[ra], &catalog);
    let ids: Vec<&str> = out.iter().map(|r| r.project_id.as_str()).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&"a") && ids.contains(&"b"));
    // Dep without project_id is skipped, never panics.
    let mut v = version("v", "c");
    v.dependencies = vec![VersionDependency {
        project_id: None,
        version_id: Some("x".to_string()),
        file_name: None,
        dependency_type: "required".to_string(),
    }];
    let rc = resolved("c", Some(v));
    let catalog: HashMap<String, ResolvedContent> =
        [("c".to_string(), rc.clone())].into_iter().collect();
    let out = collect_required(&[rc], &catalog);
    assert_eq!(out.len(), 1);
}

#[test]
fn spec_f053_required_version_pin_respected_before_best() {
    // collect_required keeps catalog versions as-is: a pinned catalog
    // entry (v9) is what flows downstream, not a re-pick.
    let pinned = version("v9", "a");
    let ra = resolved("a", Some(pinned));
    let catalog: HashMap<String, ResolvedContent> =
        [("a".to_string(), ra.clone())].into_iter().collect();
    let out = collect_required(&[ra], &catalog);
    assert_eq!(out[0].version.as_ref().expect("v").id, "v9");
}

#[test]
fn spec_f054_optional_unchecked_by_default() {
    let mut v = version("v", "a");
    v.dependencies = vec![dep("o", "optional")];
    let ra = resolved("a", Some(v));
    let ro = resolved("o", None);
    let catalog: HashMap<String, ResolvedContent> =
        [("a".to_string(), ra.clone()), ("o".to_string(), ro.clone())]
            .into_iter()
            .collect();
    let cands = review_candidates(&[ra], &[], &catalog);
    let opt = cands
        .iter()
        .find(|c| c.content.project_id == "o")
        .expect("optional listed");
    assert_eq!(opt.dependency_type, "optional");
    assert!(!opt.selected, "optional ships unchecked");
}

#[test]
fn spec_f055_selected_optional_pulls_required_chain() {
    // o (optional, selected) -> m (required) -> n (required): 3-deep chain
    // with requires_parent flags and parent ids merged.
    let mut vo = version("vo", "o");
    vo.dependencies = vec![dep("m", "required")];
    let mut vm = version("vm", "m");
    vm.dependencies = vec![dep("n", "required")];
    let vn = version("vn", "n");
    let ro = ResolvedContent {
        provider: "fake".to_string(),
        project_id: "o".to_string(),
        slug: "o".to_string(),
        title: "Title o".to_string(),
        project_type: "mod".to_string(),
        version: Some(vo),
        status: "compatible".to_string(),
        reason: String::new(),
    };
    let catalog: HashMap<String, ResolvedContent> = [
        ("o".to_string(), ro.clone()),
        ("m".to_string(), resolved("m", Some(vm))),
        ("n".to_string(), resolved("n", Some(vn))),
    ]
    .into_iter()
    .collect();
    // The user checks the optional: it enters as a checked optional, and
    // its required chain (m -> n) is flagged + auto-selected.
    let cands = review_candidates(&[], &[ro], &catalog);
    let m = cands
        .iter()
        .find(|c| c.content.project_id == "m")
        .expect("chain m");
    let n = cands
        .iter()
        .find(|c| c.content.project_id == "n")
        .expect("chain n");
    assert!(m.requires_parent);
    assert!(n.requires_parent);
    assert!(m.parents.contains(&"o".to_string()));
    assert!(m.selected && n.selected);
}
