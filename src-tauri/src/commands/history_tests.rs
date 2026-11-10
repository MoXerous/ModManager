//! The pure half of F-152: a terminal download event becomes a history row.

use std::collections::HashMap;

use mm_core::db::download_history::Status;
use mm_core::downloader::ItemProgress;

use super::{history_rows, HistorySeed};

fn seed(id: &str) -> (String, HistorySeed) {
    (
        id.to_string(),
        HistorySeed {
            provider: "modrinth".to_string(),
            project_id: format!("proj-{id}"),
            slug: format!("slug-{id}"),
            title: format!("Title {id}"),
            version_number: Some("1.2.3".to_string()),
            file_name: format!("{id}.jar"),
            file_sha1: Some("b".repeat(40)),
        },
    )
}

fn seeds(ids: &[&str]) -> HashMap<String, HistorySeed> {
    ids.iter().map(|id| seed(id)).collect()
}

#[test]
fn spec_f152_landed_becomes_a_success_row_with_the_file_and_its_identity() {
    let rows = history_rows(
        &seeds(&["v1"]),
        &[(
            "v1".to_string(),
            ItemProgress::Landed {
                path: std::path::PathBuf::from("/instances/a/mods/sodium.jar"),
            },
        )],
        "1.21.1",
        "fabric",
        1_700_000_000,
    );

    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.status, Status::Success);
    assert_eq!(row.provider, "modrinth");
    assert_eq!(row.project_id, "proj-v1");
    assert_eq!(row.slug, "slug-v1");
    assert_eq!(row.title, "Title v1");
    assert_eq!(row.version_id.as_deref(), Some("v1"));
    assert_eq!(row.version_number.as_deref(), Some("1.2.3"));
    assert_eq!(row.game_version, "1.21.1");
    assert_eq!(row.loader, "fabric");
    assert_eq!(row.file_name, "v1.jar");
    assert_eq!(row.file_sha1, Some("b".repeat(40)));
    assert_eq!(
        row.destination_path.as_deref(),
        Some("/instances/a/mods/sodium.jar")
    );
    assert_eq!(row.created_at, 1_700_000_000);
    assert!(row.message.is_empty());
}

#[test]
fn spec_f152_skipped_is_its_own_status_not_a_success() {
    let rows = history_rows(
        &seeds(&["v1"]),
        &[(
            "v1".to_string(),
            ItemProgress::Skipped {
                path: std::path::PathBuf::from("/mods/v1.jar"),
            },
        )],
        "1.21.1",
        "fabric",
        1,
    );

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, Status::Skipped);
    assert_eq!(rows[0].destination_path.as_deref(), Some("/mods/v1.jar"));
}

#[test]
fn spec_f152_failed_keeps_its_message_and_no_destination() {
    let rows = history_rows(
        &seeds(&["v1"]),
        &[(
            "v1".to_string(),
            ItemProgress::Failed {
                message: "hash mismatch".to_string(),
            },
        )],
        "1.21.1",
        "fabric",
        1,
    );

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, Status::Failed);
    assert_eq!(rows[0].message, "hash mismatch");
    assert_eq!(rows[0].destination_path, None);
}

#[test]
fn spec_f152_cancelled_is_recorded_without_a_file() {
    let rows = history_rows(
        &seeds(&["v1"]),
        &[("v1".to_string(), ItemProgress::Cancelled)],
        "1.21.1",
        "fabric",
        1,
    );

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, Status::Cancelled);
    assert_eq!(rows[0].destination_path, None);
}

#[test]
fn spec_f152_an_outcome_with_no_identity_is_not_written() {
    let rows = history_rows(
        &seeds(&["v1"]),
        &[
            ("v1".to_string(), ItemProgress::Cancelled),
            ("unknown".to_string(), ItemProgress::Cancelled),
        ],
        "1.21.1",
        "fabric",
        1,
    );

    assert_eq!(rows.len(), 1, "a row with no title would be unreadable");
    assert_eq!(rows[0].version_id.as_deref(), Some("v1"));
}

#[test]
fn spec_f152_progress_that_is_not_terminal_never_becomes_a_row() {
    let rows = history_rows(
        &seeds(&["v1"]),
        &[
            (
                "v1".to_string(),
                ItemProgress::Started {
                    done: 10,
                    total: Some(100),
                },
            ),
            ("v1".to_string(), ItemProgress::Verifying),
        ],
        "1.21.1",
        "fabric",
        1,
    );

    assert!(rows.is_empty());
}

#[test]
fn spec_f152_a_long_failure_is_cut_on_characters() {
    // Arabic, so a byte-index cut would panic or split a character.
    let long = "فشل ".repeat(400);
    let rows = history_rows(
        &seeds(&["v1"]),
        &[("v1".to_string(), ItemProgress::Failed { message: long })],
        "1.21.1",
        "fabric",
        1,
    );

    let kept = &rows[0].message;
    assert_eq!(kept.chars().count(), super::HISTORY_MESSAGE_LIMIT);
    assert!(kept
        .chars()
        .all(|c| c == 'ف' || c == 'ش' || c == 'ل' || c == ' '));
}
