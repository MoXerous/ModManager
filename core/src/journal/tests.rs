//! Journal model tests: transitions, counters, revision, strict serde.

use super::*;

fn item(id: &str, state: ItemState) -> OperationItem {
    OperationItem {
        item_id: id.to_string(),
        project_id: "P1".to_string(),
        title: format!("Title {id}"),
        filename: format!("{id}.jar"),
        state,
        partial_path: String::new(),
        final_path: String::new(),
        message: String::new(),
    }
}

fn record(items: Vec<OperationItem>) -> OperationRecord {
    OperationRecord::new(OperationKind::DownloadBatch, "prof", "/target", items)
}

fn pack_meta() -> PackApplyMeta {
    PackApplyMeta {
        pack_name: "Test pack".to_string(),
        pack_version_id: "ver-1".to_string(),
        archive_sha512: "ab".repeat(64),
        archive_path: "C:/packs/test.mrpack".to_string(),
    }
}

#[test]
fn spec_f141_new_record_starts_at_revision_one() {
    let r = record(vec![item("a", ItemState::Queued)]);
    assert_eq!(r.revision, 1);
    assert_eq!(r.state, OperationState::Planned);
    assert_eq!(r.created_at, r.updated_at);
    assert_eq!(r.schema, SCHEMA);
    assert!(r.verify().is_ok());
}

#[test]
fn spec_f141_operation_id_is_canonical_lowercase_uuid() {
    let r = record(vec![]);
    r.verify_id().expect("canonical");
    // Uppercase and brace forms are NOT canonical (parse_str alone would accept).
    let mut upper = r.clone();
    upper.operation_id = r.operation_id.to_uppercase();
    assert_eq!(upper.verify_id(), Err(JournalError::NotCanonicalId));
    let mut braced = r;
    braced.operation_id = format!("{{{}}}", braced.operation_id);
    assert_eq!(braced.verify_id(), Err(JournalError::NotCanonicalId));
}

#[test]
fn spec_f142_operation_transition_matrix() {
    let all = [
        OperationState::Planned,
        OperationState::Running,
        OperationState::Completed,
        OperationState::Failed,
        OperationState::Cancelled,
    ];
    for from in all {
        for to in all {
            let mut r = record(vec![item("a", ItemState::Queued)]);
            r.state = from;
            let got = r.transition_operation(to);
            assert_eq!(got.is_ok(), from.may_go_to(to), "{from:?} -> {to:?}");
        }
    }
}

#[test]
fn spec_f142_item_matrix_denies_committing_cancel() {
    let all = [
        ItemState::Queued,
        ItemState::Downloading,
        ItemState::Verifying,
        ItemState::Committing,
        ItemState::Completed,
        ItemState::Skipped,
        ItemState::Failed,
        ItemState::Cancelled,
    ];
    for from in all {
        for to in all {
            let mut r = record(vec![item("a", from)]);
            let got = r.transition_item("a", to);
            assert_eq!(got.is_ok(), from.may_go_to(to), "{from:?} -> {to:?}");
        }
    }
    // The designed-in non-edge, stated explicitly.
    assert!(!ItemState::Committing.may_go_to(ItemState::Cancelled));
}

#[test]
fn spec_f142_terminal_states_have_no_exits() {
    for s in [
        OperationState::Completed,
        OperationState::Failed,
        OperationState::Cancelled,
    ] {
        assert!(s.is_terminal());
        for to in [
            OperationState::Planned,
            OperationState::Running,
            OperationState::Completed,
            OperationState::Failed,
            OperationState::Cancelled,
        ] {
            assert!(!s.may_go_to(to), "{s:?} -> {to:?} must be denied");
        }
    }
    for s in [
        ItemState::Completed,
        ItemState::Skipped,
        ItemState::Failed,
        ItemState::Cancelled,
    ] {
        assert!(s.is_terminal());
    }
}

#[test]
fn spec_f142_revision_bumps_per_mutation_but_same_bind_does_not() {
    let mut r = record(vec![item("a", ItemState::Queued)]);
    assert_eq!(r.revision, 1);
    r.transition_operation(OperationState::Running)
        .expect("run");
    assert_eq!(r.revision, 2);
    r.transition_operation(OperationState::Running)
        .expect("same");
    assert_eq!(r.revision, 3, "same-state transition still bumps");
    r.transition_item("a", ItemState::Downloading).expect("dl");
    assert_eq!(r.revision, 4);
    r.bind_profile("prof").expect("same id");
    assert_eq!(r.revision, 4, "bind with same id does not bump");
    r.bind_profile("other").expect("new id");
    assert_eq!(r.revision, 5);
}

#[test]
fn spec_f141_counters_are_derived_and_verified() {
    let mut r = record(vec![
        item("a", ItemState::Completed),
        item("b", ItemState::Failed),
        item("c", ItemState::Skipped),
        item("d", ItemState::Cancelled),
        item("e", ItemState::Queued),
    ]);
    let c = r.counters();
    assert_eq!(c.planned_count, 5);
    assert_eq!(c.completed_count, 1);
    assert_eq!(c.failed_count, 1);
    assert_eq!(c.skipped_count, 1);
    assert_eq!(c.cancelled_count, 1);
    r.verify_counters().expect("match");
    // Tampered counter is a load-time failure, not an auto-fix.
    r.completed_count = 9;
    assert_eq!(
        r.verify_counters(),
        Err(JournalError::CounterMismatch("completed_count"))
    );
}

#[test]
fn spec_f142_missing_item_is_an_error() {
    let mut r = record(vec![item("a", ItemState::Queued)]);
    assert_eq!(
        r.transition_item("ghost", ItemState::Downloading),
        Err(JournalError::ItemMissing("ghost".to_string()))
    );
}

#[test]
fn spec_f141_duplicate_item_ids_rejected() {
    let mut r = record(vec![item("a", ItemState::Queued)]);
    assert_eq!(
        r.replace_items(vec![
            item("a", ItemState::Queued),
            item("a", ItemState::Queued)
        ]),
        Err(JournalError::DuplicateItem("a".to_string()))
    );
}

#[test]
fn spec_f141_strict_key_set_rejects_unknown_and_missing() {
    let r = record(vec![item("a", ItemState::Queued)]);
    let json = serde_json::to_value(&r).expect("json");
    // 18 top-level keys exactly.
    assert_eq!(json.as_object().expect("obj").len(), 18);
    let mut extra = json.clone();
    extra["extra_key"] = serde_json::json!(1);
    assert!(serde_json::from_value::<OperationRecord>(extra).is_err());
    let mut missing = json.clone();
    missing.as_object_mut().expect("obj").remove("message");
    assert!(serde_json::from_value::<OperationRecord>(missing).is_err());

    // Items carry exactly 8 keys, strict both ways.
    let item_json = json["items"][0].clone();
    assert_eq!(item_json.as_object().expect("obj").len(), 8);
    let mut item_extra = item_json.clone();
    item_extra["nope"] = serde_json::json!(true);
    let rec = record(vec![item("a", ItemState::Queued)]);
    let mut patched = serde_json::to_value(&rec).expect("json");
    patched["items"] = serde_json::json!([item_extra]);
    assert!(serde_json::from_value::<OperationRecord>(patched).is_err());
    let mut item_missing = item_json;
    item_missing.as_object_mut().expect("obj").remove("message");
    let mut patched = serde_json::to_value(&rec).expect("json");
    patched["items"] = serde_json::json!([item_missing]);
    assert!(serde_json::from_value::<OperationRecord>(patched).is_err());
}

#[test]
fn spec_f143_unfinished_classification() {
    assert!(unfinished(OperationState::Planned));
    assert!(unfinished(OperationState::Running));
    assert!(!unfinished(OperationState::Completed));
    assert!(!unfinished(OperationState::Failed));
    assert!(!unfinished(OperationState::Cancelled));
}

#[test]
fn spec_f141_a_v1_record_is_still_read() {
    // A record this app wrote before the pack block existed: it says so in
    // its schema and it has no `pack` key. Refusing it would turn every
    // record already on disk into a corrupt file.
    let mut r = record(vec![item("a", ItemState::Queued)]);
    r.schema = SCHEMA_V1.to_string();
    let mut json = serde_json::to_value(&r).expect("json");
    json.as_object_mut().expect("obj").remove("pack");
    let back: OperationRecord = serde_json::from_value(json).expect("a v1 record still decodes");
    assert_eq!(back.schema, SCHEMA_V1);
    assert!(back.pack.is_none());
    assert!(back.verify().is_ok(), "a v1 record is not corrupt");
    // And a v1 record of a kind that is not a pack apply is still consistent.
    assert!(back.verify_pack().is_ok());
}

#[test]
fn spec_f141_an_unknown_schema_is_refused() {
    // A record written by a newer build may carry keys this one would read
    // as something else. The honest answer to "I do not know this file" is
    // to refuse it, not to interpret it.
    let mut r = record(vec![item("a", ItemState::Queued)]);
    r.schema = "modmanager.operation_journal.v99".to_string();
    let err = r.verify().expect_err("a newer schema must be refused");
    assert!(
        matches!(err, JournalError::UnsupportedSchema { .. }),
        "got {err:?}"
    );
    assert!(ACCEPTED_SCHEMAS.contains(&SCHEMA));
    assert!(ACCEPTED_SCHEMAS.contains(&SCHEMA_V1));
}

#[test]
fn spec_f076_a_pack_apply_record_carries_its_pack() {
    let pack = pack_meta();
    let r = OperationRecord::new(
        OperationKind::MrpackInstall,
        "prof",
        "/target",
        vec![item("a", ItemState::Queued)],
    )
    .with_pack(pack.clone());
    assert!(r.verify().is_ok());
    let json = serde_json::to_value(&r).expect("json");
    assert_eq!(
        json.as_object().expect("obj").len(),
        19,
        "the pack block is the nineteenth top-level key"
    );
    let back: OperationRecord = serde_json::from_value(json).expect("round trip");
    assert_eq!(back.pack, Some(pack));
    assert_eq!(back.kind, OperationKind::MrpackInstall);
}

#[test]
fn spec_f076_a_pack_apply_without_its_pack_is_refused() {
    // A record that names no archive cannot be resumed: there is no "what"
    // to resume against.
    let mut r = record(vec![item("a", ItemState::Queued)]);
    r.kind = OperationKind::MrpackInstall;
    assert!(matches!(
        r.verify(),
        Err(JournalError::KindPackMismatch {
            kind: OperationKind::MrpackInstall,
            has_pack: false
        })
    ));
}

#[test]
fn spec_f076_a_record_that_is_not_a_pack_apply_carries_no_pack() {
    // The other direction: a pack block on an operation that is not a pack
    // apply is a field that kind has no meaning for.
    let r = OperationRecord::new(
        OperationKind::DownloadBatch,
        "prof",
        "/target",
        vec![item("a", ItemState::Queued)],
    )
    .with_pack(pack_meta());
    assert!(matches!(
        r.verify(),
        Err(JournalError::KindPackMismatch {
            kind: OperationKind::DownloadBatch,
            has_pack: true
        })
    ));
}

#[test]
fn spec_f141_transition_items_walks_several_transitions_in_one_revision() {
    // A record is one document: every write rewrites the whole file. So the
    // several transitions a file needs are applied in one write, and the
    // revision moves once for the whole batch.
    let mut r = record(vec![
        item("a", ItemState::Queued),
        item("b", ItemState::Queued),
    ]);
    let before = r.revision;
    r.transition_items(&[
        ("a".to_string(), ItemState::Downloading),
        ("a".to_string(), ItemState::Verifying),
        ("a".to_string(), ItemState::Committing),
        ("b".to_string(), ItemState::Downloading),
    ])
    .expect("transitions");
    assert_eq!(r.revision, before + 1, "one write, one revision");
    assert_eq!(r.items[0].state, ItemState::Committing);
    assert_eq!(r.items[1].state, ItemState::Downloading);
    assert_eq!(r.completed_count, 0);
    assert_eq!(r.planned_count, 2);
}

#[test]
fn spec_f141_transition_items_still_gates_every_edge() {
    let mut r = record(vec![item("a", ItemState::Queued)]);
    assert!(
        r.transition_items(&[("a".to_string(), ItemState::Completed)])
            .is_err(),
        "queued may not jump to completed"
    );
    assert!(r
        .transition_items(&[("ghost".to_string(), ItemState::Downloading)])
        .is_err());
    // The commit edge is still uncancellable.
    let mut r = record(vec![item("a", ItemState::Committing)]);
    assert!(r
        .transition_items(&[("a".to_string(), ItemState::Cancelled)])
        .is_err());
}

#[test]
fn spec_f076_pack_item_uses_the_eight_key_encoding() {
    let item = pack_item(
        "mods/a.jar",
        "Test pack",
        "/inst/mods/a.jar.part",
        "/inst/mods/a.jar",
    );
    let json = serde_json::to_value(&item).expect("json");
    assert_eq!(
        json.as_object().expect("obj").len(),
        8,
        "items carry exactly eight keys"
    );
    assert_eq!(item.item_id, "mods/a.jar");
    assert_eq!(item.project_id, "", "a pack file belongs to no project");
    assert_eq!(item.title, "Test pack");
    assert_eq!(item.filename, "a.jar");
    assert_eq!(item.state, ItemState::Queued);
    assert_eq!(item.partial_path, "/inst/mods/a.jar.part");
    assert_eq!(item.final_path, "/inst/mods/a.jar");
    assert_eq!(item.message, "");
}
