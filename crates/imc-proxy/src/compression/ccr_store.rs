//! CCR store — Compress-Cache-Retrieve.
//!
//! Original (pre-compression) content is stored here so nothing is lost when
//! the proxy sends a compressed summary to the LLM. The model can later ask
//! for the full original via the retrieve-style endpoint.
//!
//! Stored in the project's SQLite DB alongside imc-core's tables.

use std::path::Path;
use std::time::SystemTime;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

pub struct CcrStore {
    pub conn: Connection,
}

impl CcrStore {
    pub fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(db_path)
            .with_context(|| format!("open ccr sqlite {db_path:?}"))?;
        conn.execute_batch(
            r#"
            PRAGMA journal_mode=WAL;
            PRAGMA busy_timeout=5000;
            CREATE TABLE IF NOT EXISTS ccr_originals (
                id           TEXT PRIMARY KEY,
                kind         TEXT NOT NULL,
                original     TEXT NOT NULL,
                compressed   TEXT NOT NULL,
                orig_tokens  INTEGER NOT NULL,
                comp_tokens  INTEGER NOT NULL,
                created_at   INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_ccr_kind ON ccr_originals(kind);
            CREATE INDEX IF NOT EXISTS idx_ccr_created ON ccr_originals(created_at DESC);
            "#,
        )?;
        Ok(Self { conn })
    }

    /// Store an original + its compressed form. Returns the id.
    pub fn store(
        &mut self,
        kind: &str,
        original: &str,
        compressed: &str,
        orig_tokens: i64,
        comp_tokens: i64,
    ) -> Result<String> {
        let id = imc_core::hash::md5_hex(format!("{original}{compressed}{orig_tokens}").as_bytes())
            [..16]
            .to_string();
        self.conn.execute(
            "INSERT OR REPLACE INTO ccr_originals
             (id, kind, original, compressed, orig_tokens, comp_tokens, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                id,
                kind,
                original,
                compressed,
                orig_tokens,
                comp_tokens,
                SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64
            ],
        )?;
        Ok(id)
    }

    /// Store an original + its compressed form with an explicit deterministic id.
    pub fn store_with_id(
        &mut self,
        id: &str,
        kind: &str,
        original: &str,
        compressed: &str,
        orig_tokens: i64,
        comp_tokens: i64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO ccr_originals
             (id, kind, original, compressed, orig_tokens, comp_tokens, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                id,
                kind,
                original,
                compressed,
                orig_tokens,
                comp_tokens,
                SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64
            ],
        )?;
        Ok(())
    }

    /// Retrieve an original by id.
    pub fn get(&self, id: &str) -> Result<Option<String>> {
        let row: Option<String> = self
            .conn
            .query_row("SELECT original FROM ccr_originals WHERE id = ?1", params![id], |r| {
                r.get(0)
            })
            .ok();
        Ok(row)
    }

    /// List recent originals (newest first), optionally filtered by kind.
    pub fn list(&self, kind: Option<&str>, limit: usize) -> Result<Vec<CcrEntry>> {
        let (sql, params) = match kind {
            Some(k) => (
                "SELECT id, kind, original, compressed, orig_tokens, comp_tokens, created_at
                 FROM ccr_originals WHERE kind = ?1 ORDER BY created_at DESC LIMIT ?2"
                    .to_string(),
                rusqlite::params![k.to_string(), limit as i64],
            ),
            None => (
                "SELECT id, kind, original, compressed, orig_tokens, comp_tokens, created_at
                 FROM ccr_originals ORDER BY created_at DESC LIMIT ?1"
                    .to_string(),
                rusqlite::params![limit as i64],
            ),
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params, |r| {
            Ok(CcrEntry {
                id: r.get(0)?,
                kind: r.get(1)?,
                original: r.get(2)?,
                compressed: r.get(3)?,
                orig_tokens: r.get(4)?,
                comp_tokens: r.get(5)?,
                created_at: r.get(6)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Total count of stored originals.
    pub fn count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM ccr_originals", [], |r| r.get(0))?)
    }

    /// Delete originals older than `before_ts` (unix seconds).
    pub fn prune_before(&mut self, before_ts: i64) -> Result<usize> {
        let n = self.conn.execute(
            "DELETE FROM ccr_originals WHERE created_at < ?1",
            params![before_ts],
        )?;
        Ok(n)
    }

    /// Delete originals older than `days` days.
    pub fn prune_older_than_days(&mut self, days: i64) -> Result<usize> {
        let cutoff = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
            - days * 86_400;
        self.prune_before(cutoff)
    }
}

#[derive(Debug, Clone)]
pub struct CcrEntry {
    pub id: String,
    pub kind: String,
    pub original: String,
    pub compressed: String,
    pub orig_tokens: i64,
    pub comp_tokens: i64,
    pub created_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_db(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("imc-ccr-{tag}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn store_and_get() {
        let db = tmp_db("get");
        let mut store = CcrStore::open(&db).unwrap();
        let id = store
            .store(
                "json",
                r#"{"a":1,"b":2}"#,
                r#"{"a":1}"#,
                10,
                5,
            )
            .unwrap();
        let got = store.get(&id).unwrap().unwrap();
        assert_eq!(got, r#"{"a":1,"b":2}"#);
    }

    #[test]
    fn count_and_list() {
        let db = tmp_db("count");
        let mut store = CcrStore::open(&db).unwrap();
        for i in 0..5 {
            let _ = store.store("json", &format!("orig {i}"), &format!("comp {i}"), 10, 5).unwrap();
        }
        assert_eq!(store.count().unwrap(), 5);
        let list = store.list(None, 3).unwrap();
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn auto_prunes_records_older_than_7_days() {
        let db = tmp_db("auto-prune");
        let mut store = CcrStore::open(&db).unwrap();
        let old_ts = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - 8 * 86_400;
        store
            .conn
            .execute(
                "INSERT INTO ccr_originals (id, kind, original, compressed, orig_tokens, comp_tokens, created_at)
                 VALUES ('old_id', 'test', 'old content', 'comp', 10, 5, ?1)",
                params![old_ts],
            )
            .unwrap();

        let fresh_id = store.store("test", "fresh content", "comp", 10, 5).unwrap();

        let pruned = store.prune_older_than_days(7).unwrap();
        assert_eq!(pruned, 1);
        assert!(store.get("old_id").unwrap().is_none(), "records older than 7 days must be pruned");
        assert!(store.get(&fresh_id).unwrap().is_some(), "recent records must be preserved");
    }
}
