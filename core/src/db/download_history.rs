//! The download history: one row per install outcome (F-152), searchable
//! (F-153).
//!
//! The live queue is a channel, not a table: it ends when the window closes.
//! This module is the other half — what actually happened, kept across
//! restarts, so the user can answer "did that mod go in, and into where?".
//!
//! Every row is written with the outcome already known. There is no "started"
//! row and no update: a row that changes after it is written would be a
//! second source of truth next to the journal.

use rusqlite::types::Value;

use super::Db;

/// The most rows one screen may ask for.
///
/// The reference app capped at 1000 with 500/300 defaults; the number here is
/// the cap, not a page size, because the screen scrolls rather than pages.
pub const MAX_LIMIT: u32 = 500;

/// The outcome of one file. The database enforces the same four values with a
/// `CHECK`, so a mismatch between this enum and the schema fails loudly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Downloaded, hashed and renamed into place.
    Success,
    /// Already present with the right bytes; nothing was transferred.
    Skipped,
    /// The download or the verification failed. The file is not installed.
    Failed,
    /// The user stopped the batch (or the app did) before this item finished.
    Cancelled,
}

impl Status {
    pub const ALL: [Status; 4] = [
        Status::Success,
        Status::Skipped,
        Status::Failed,
        Status::Cancelled,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Status::Success => "success",
            Status::Skipped => "skipped",
            Status::Failed => "failed",
            Status::Cancelled => "cancelled",
        }
    }

    /// The value as stored, or `None` for anything else.
    ///
    /// Callers use this on input (a filter from the webview) and on rows. A
    /// row that fails to parse is a `CHECK` that was bypassed; an input that
    /// fails is simply not a filter we offer.
    pub fn parse(s: &str) -> Option<Status> {
        Status::ALL.into_iter().find(|v| v.as_str() == s)
    }
}

/// One row to write, with the outcome already known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDownload {
    pub provider: String,
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub version_id: Option<String>,
    pub version_number: Option<String>,
    pub game_version: String,
    pub loader: String,
    pub file_name: String,
    pub file_sha1: Option<String>,
    pub destination_path: Option<String>,
    pub status: Status,
    /// Why it failed, or empty. Never the raw response body (F-019).
    pub message: String,
    pub created_at: i64,
}

/// A row as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub id: i64,
    pub provider: String,
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub version_id: Option<String>,
    pub version_number: Option<String>,
    pub game_version: String,
    pub loader: String,
    pub file_name: String,
    pub file_sha1: Option<String>,
    pub destination_path: Option<String>,
    pub status: Status,
    pub message: String,
    pub created_at: i64,
}

/// What the screen asked for.
///
/// The default is the screen's default: everything, newest first, one page.
#[derive(Debug, Clone)]
pub struct Query {
    pub status: Option<Status>,
    pub search: String,
    pub limit: u32,
}

impl Default for Query {
    fn default() -> Self {
        Query {
            status: None,
            search: String::new(),
            limit: 200,
        }
    }
}

/// Escape a user's text for a `LIKE` pattern.
///
/// Without this, `%` typed into the search box is a wildcard: `100%` would
/// match every row, and `_` would match any character. The reference app
/// interpolated the pattern with no escape at all.
fn like_pattern(term: &str) -> String {
    let mut out = String::with_capacity(term.len() + 2);
    out.push('%');
    for ch in term.chars() {
        match ch {
            '\\' | '%' | '_' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out.push('%');
    out
}

/// The limit the database will use, whatever the caller asked for.
pub fn clamp_limit(limit: u32) -> u32 {
    limit.clamp(1, MAX_LIMIT)
}

/// Append one outcome. Returns the new row's id.
pub fn log(db: &Db, entry: &NewDownload) -> Result<i64, super::DbError> {
    let conn = db.conn();
    conn.execute(
        "INSERT INTO download_history
            (provider, project_id, slug, title, version_id, version_number,
             game_version, loader, filename, file_sha1, destination_path,
             status, message, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        rusqlite::params![
            entry.provider,
            entry.project_id,
            entry.slug,
            entry.title,
            entry.version_id,
            entry.version_number,
            entry.game_version,
            entry.loader,
            entry.file_name,
            entry.file_sha1,
            entry.destination_path,
            entry.status.as_str(),
            entry.message,
            entry.created_at,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// The rows matching `query`, newest first.
///
/// Newest first is not a default that callers may override: `id` breaks ties
/// so two rows written in the same second keep the order they were written
/// in, and a stable order is what makes a list usable at all.
pub fn list(db: &Db, query: &Query) -> Result<Vec<HistoryEntry>, super::DbError> {
    let mut sql = String::from(
        "SELECT id, provider, project_id, slug, title, version_id, version_number,
                game_version, loader, filename, file_sha1, destination_path,
                status, message, created_at
         FROM download_history",
    );
    let mut conditions: Vec<&str> = Vec::new();
    let mut params: Vec<Value> = Vec::new();

    if let Some(status) = query.status {
        conditions.push("status = ?");
        params.push(Value::Text(status.as_str().to_string()));
    }
    let term = query.search.trim();
    if !term.is_empty() {
        // The four fields the reference app searched. `project_id` is what a
        // user copies out of a Modrinth URL, so it earns its place.
        conditions.push(
            "(slug LIKE ? ESCAPE '\\' OR title LIKE ? ESCAPE '\\'
              OR filename LIKE ? ESCAPE '\\' OR project_id LIKE ? ESCAPE '\\')",
        );
        let pattern = like_pattern(term);
        for _ in 0..4 {
            params.push(Value::Text(pattern.clone()));
        }
    }
    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }
    sql.push_str(" ORDER BY created_at DESC, id DESC LIMIT ?");
    params.push(Value::Integer(i64::from(clamp_limit(query.limit))));

    let conn = db.conn();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), entry_from)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Delete every row. Returns how many were removed.
///
/// Deliberately not wrapped in a transaction with anything else: the screen
/// asks one question and this is its one answer.
pub fn clear(db: &Db) -> Result<usize, super::DbError> {
    let conn = db.conn();
    Ok(conn.execute("DELETE FROM download_history", [])?)
}

fn entry_from(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryEntry> {
    let raw_status: String = row.get("status")?;
    let status = Status::parse(&raw_status).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown download status {raw_status:?}"),
            )),
        )
    })?;
    Ok(HistoryEntry {
        id: row.get("id")?,
        provider: row.get("provider")?,
        project_id: row.get("project_id")?,
        slug: row.get("slug")?,
        title: row.get("title")?,
        version_id: row.get("version_id")?,
        version_number: row.get("version_number")?,
        game_version: row.get("game_version")?,
        loader: row.get("loader")?,
        file_name: row.get("filename")?,
        file_sha1: row.get("file_sha1")?,
        destination_path: row.get("destination_path")?,
        status,
        message: row.get("message")?,
        created_at: row.get("created_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open;

    fn db() -> (tempfile::TempDir, Db) {
        let dir = crate::test_tempdir();
        let db = open(&dir.path().join("t.db")).expect("open");
        (dir, db)
    }

    fn entry(title: &str, status: Status, created_at: i64) -> NewDownload {
        NewDownload {
            provider: "modrinth".to_string(),
            project_id: format!("proj-{title}"),
            slug: title.to_lowercase().replace(' ', "-"),
            title: title.to_string(),
            version_id: Some(format!("v-{title}")),
            version_number: Some("1.0.0".to_string()),
            game_version: "1.21.1".to_string(),
            loader: "fabric".to_string(),
            file_name: format!("{title}.jar"),
            file_sha1: Some("a".repeat(40)),
            destination_path: Some(format!("mods/{title}.jar")),
            status,
            message: String::new(),
            created_at,
        }
    }

    #[test]
    fn spec_f152_log_then_list_returns_the_newest_first() {
        let (_dir, db) = db();
        log(&db, &entry("Older", Status::Success, 100)).expect("first");
        log(&db, &entry("Newer", Status::Failed, 200)).expect("second");
        log(&db, &entry("Middle", Status::Skipped, 150)).expect("third");

        let rows = list(&db, &Query::default()).expect("list");
        let titles: Vec<&str> = rows.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(titles, ["Newer", "Middle", "Older"]);
        assert_eq!(rows[0].status, Status::Failed);
        assert_eq!(rows[2].destination_path.as_deref(), Some("mods/Older.jar"));
    }

    #[test]
    fn spec_f152_two_rows_in_one_second_keep_their_write_order() {
        let (_dir, db) = db();
        log(&db, &entry("First", Status::Success, 100)).expect("first");
        log(&db, &entry("Second", Status::Success, 100)).expect("second");

        let rows = list(&db, &Query::default()).expect("list");
        assert_eq!(rows[0].title, "Second", "the later write wins the tie");
        assert_eq!(rows[1].title, "First");
    }

    #[test]
    fn spec_f152_status_filter_returns_only_that_status() {
        let (_dir, db) = db();
        log(&db, &entry("Ok", Status::Success, 1)).expect("ok");
        log(&db, &entry("Bad", Status::Failed, 2)).expect("bad");
        log(&db, &entry("Gone", Status::Cancelled, 3)).expect("gone");

        let rows = list(
            &db,
            &Query {
                status: Some(Status::Failed),
                ..Query::default()
            },
        )
        .expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Bad");
    }

    #[test]
    fn spec_f152_search_matches_title_slug_filename_and_project_id() {
        let (_dir, db) = db();
        log(&db, &entry("Sodium", Status::Success, 1)).expect("sodium");
        log(&db, &entry("Lithium", Status::Success, 2)).expect("lithium");

        for term in ["Sodium", "sodium", "Sodium.jar", "proj-Sodium", "sod"] {
            let rows = list(
                &db,
                &Query {
                    search: term.to_string(),
                    ..Query::default()
                },
            )
            .expect("list");
            assert_eq!(rows.len(), 1, "term {term:?} should match one row");
            assert_eq!(rows[0].title, "Sodium");
        }
    }

    #[test]
    fn spec_f153_search_treats_wildcards_as_text() {
        let (_dir, db) = db();
        log(&db, &entry("Fifty", Status::Success, 1)).expect("first");
        let mut percent = entry("100% Cotton", Status::Success, 2);
        percent.slug = "hundred".to_string();
        percent.project_id = "proj-hundred".to_string();
        percent.file_name = "hundred.jar".to_string();
        log(&db, &percent).expect("second");

        // `%` must not match everything, and `_` must not match one character.
        let rows = list(
            &db,
            &Query {
                search: "100%".to_string(),
                ..Query::default()
            },
        )
        .expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "100% Cotton");

        let rows = list(
            &db,
            &Query {
                search: "%".to_string(),
                ..Query::default()
            },
        )
        .expect("list");
        assert_eq!(
            rows.len(),
            1,
            "a lone percent matches the row containing it"
        );

        let rows = list(
            &db,
            &Query {
                search: "S_dium".to_string(),
                ..Query::default()
            },
        )
        .expect("list");
        assert!(rows.is_empty(), "underscore is a literal, not a wildcard");
    }

    #[test]
    fn spec_f152_limit_is_clamped_and_one_is_the_floor() {
        let (_dir, db) = db();
        for i in 0..5 {
            log(&db, &entry(&format!("Mod {i}"), Status::Success, i)).expect("row");
        }

        let rows = list(
            &db,
            &Query {
                limit: 2,
                ..Query::default()
            },
        )
        .expect("list");
        assert_eq!(rows.len(), 2);

        let rows = list(
            &db,
            &Query {
                limit: 0,
                ..Query::default()
            },
        )
        .expect("list");
        assert_eq!(rows.len(), 1, "zero is not an empty page, it is one row");

        assert_eq!(clamp_limit(u32::MAX), MAX_LIMIT);
    }

    #[test]
    fn spec_f152_clear_removes_every_row_and_reports_the_count() {
        let (_dir, db) = db();
        log(&db, &entry("A", Status::Success, 1)).expect("a");
        log(&db, &entry("B", Status::Success, 2)).expect("b");

        assert_eq!(clear(&db).expect("clear"), 2);
        assert!(list(&db, &Query::default()).expect("list").is_empty());
        assert_eq!(clear(&db).expect("clear again"), 0);
    }

    #[test]
    fn spec_f152_status_round_trips_all_four_values() {
        for status in Status::ALL {
            assert_eq!(Status::parse(status.as_str()), Some(status));
        }
        assert_eq!(Status::parse("SUCCESS"), None);
        assert_eq!(Status::parse("queued"), None);
    }
}
