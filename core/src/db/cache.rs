//! `api_cache` with TTL enforced in SQL (no sweeper thread).

use super::{Db, DbError};
use rusqlite::OptionalExtension;

impl Db {
    /// Cached body, or `None` on miss/expiry. Expired rows are deleted lazily.
    pub fn cache_get(&self, key: &str, ttl_secs: i64) -> Result<Option<String>, DbError> {
        if ttl_secs <= 0 {
            return Ok(None);
        }
        let hit: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM api_cache
             WHERE key = ?1 AND fetched_at + ?2 > CAST(strftime('%s', 'now') AS INTEGER)",
                rusqlite::params![key, ttl_secs],
                |r| r.get(0),
            )
            .optional()?;
        if hit.is_none() {
            self.conn.execute(
                "DELETE FROM api_cache WHERE key = ?1 AND fetched_at + ?2 <= CAST(strftime('%s', 'now') AS INTEGER)",
                rusqlite::params![key, ttl_secs],
            )?;
        }
        Ok(hit)
    }

    /// Upsert + opportunistic expiry cleanup (no sweeper thread).
    pub fn cache_set(&self, key: &str, body: &str) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO api_cache (key, value, fetched_at)
             VALUES (?1, ?2, strftime('%s', 'now'))
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, fetched_at = excluded.fetched_at",
            rusqlite::params![key, body],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn spec_storage_api_cache_ttl() {
        let dir = crate::test_tempdir();
        let db = super::super::open(&dir.path().join("t.db")).expect("open");
        db.cache_set("k", "v").expect("set");
        assert_eq!(db.cache_get("k", 3600).expect("get"), Some("v".to_string()));
        // Backdate past TTL: must miss.
        db.conn
            .execute(
                "UPDATE api_cache SET fetched_at = fetched_at - 7200 WHERE key = 'k'",
                [],
            )
            .expect("backdate");
        assert_eq!(db.cache_get("k", 3600).expect("get"), None);
    }

    #[test]
    fn spec_storage_api_cache_overwrite() {
        let dir = crate::test_tempdir();
        let db = super::super::open(&dir.path().join("t.db")).expect("open");
        db.cache_set("k", "v1").expect("set");
        db.cache_set("k", "v2").expect("set");
        assert_eq!(
            db.cache_get("k", 3600).expect("get"),
            Some("v2".to_string())
        );
    }
}
