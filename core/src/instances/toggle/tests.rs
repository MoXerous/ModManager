//! Toggle tests: preview guards + execute with reverse rollback.

use super::*;
use std::path::PathBuf;

fn mods() -> tempfile::TempDir {
    let d = crate::test_tempdir();
    std::fs::create_dir_all(d.path().join("mods")).expect("mods");
    d
}

fn touch(dir: &std::path::Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, b"jar-bytes").expect("write");
    p
}

#[test]
fn spec_f110_preview_disable_guards() {
    let d = mods();
    let m = d.path().join("mods");
    let a = touch(&m, "a.jar");
    // Outside root.
    let out = touch(d.path(), "out.jar");
    // Already disabled + non-jar + collision target.
    let dis = touch(&m, "b.jar.disabled");
    let txt = touch(&m, "c.txt");
    let victim = touch(&m, "victim.jar");
    touch(&m, "victim.jar.disabled");
    let plan = preview_toggle(
        &m,
        ToggleAction::Disable,
        &[
            a.clone(),
            out.clone(),
            dis.clone(),
            txt.clone(),
            victim.clone(),
        ],
    );
    let by_src = |s: &PathBuf| {
        plan.entries
            .iter()
            .find(|e| &e.source == s)
            .expect("entry")
            .clone()
    };
    let e = by_src(&a);
    assert!(e.allowed);
    assert_eq!(e.dest, m.join("a.jar.disabled"));
    assert!(!by_src(&out).allowed);
    assert!(by_src(&out).reason.contains("outside"));
    assert!(!by_src(&dis).allowed);
    assert!(by_src(&dis).reason.contains("already"));
    assert!(!by_src(&txt).allowed);
    assert!(by_src(&txt).reason.contains(".jar"));
    assert!(!by_src(&victim).allowed);
    assert!(by_src(&victim).reason.contains("exists"));
}

#[test]
fn spec_f110_preview_enable_rules() {
    let d = mods();
    let m = d.path().join("mods");
    let dis = touch(&m, "a.jar.disabled");
    let plan = preview_toggle(&m, ToggleAction::Enable, std::slice::from_ref(&dis));
    assert!(plan.entries[0].allowed);
    assert_eq!(plan.entries[0].dest, m.join("a.jar"));
    // Enabling an active jar is rejected.
    let active = touch(&m, "b.jar");
    let plan = preview_toggle(&m, ToggleAction::Enable, &[active]);
    assert!(!plan.entries[0].allowed);
}

#[test]
fn spec_f067_execute_disable_is_rename_only() {
    let d = mods();
    let m = d.path().join("mods");
    let a = touch(&m, "a.jar");
    let plan = preview_toggle(&m, ToggleAction::Disable, std::slice::from_ref(&a));
    let rs = execute_toggle(&plan);
    assert_eq!(rs.len(), 1);
    assert_eq!(rs[0].state, ToggleState::Success);
    assert!(!a.exists());
    assert_eq!(
        std::fs::read(m.join("a.jar.disabled")).expect("read"),
        b"jar-bytes"
    );
}

#[test]
fn spec_f110_execute_rolls_back_reversed_on_failure() {
    let d = mods();
    let m = d.path().join("mods");
    let a = touch(&m, "a.jar");
    let b = touch(&m, "b.jar");
    // Plan where the second rename collides mid-execution.
    let plan = preview_toggle(&m, ToggleAction::Disable, &[a.clone(), b.clone()]);
    // Sabotage: pre-create b's destination so b fails after a succeeded.
    std::fs::write(m.join("b.jar.disabled"), b"taken").expect("write");
    let rs = execute_toggle(&plan);
    let states: Vec<ToggleState> = rs.iter().map(|r| r.state).collect();
    assert_eq!(states[0], ToggleState::RolledBack);
    assert_eq!(states[1], ToggleState::Failed);
    // a was rolled back: original restored, destination gone.
    assert!(a.exists());
    assert!(!m.join("a.jar.disabled").exists());
}
