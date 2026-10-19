//! CCR store — Compress-Cache-Retrieve.
//!
//! Original (pre-compression) content is stored here so nothing is lost when
//! the proxy sends a compressed summary to the LLM. The model can later ask
//! for the full original via the `headroom_retrieve`-style endpoint.
//!
//! Stored in the project's SQLite DB alongside imc-core's tables.

use std::path::Path;
use std::time::SystemTime;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde_json::Value;

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
                rusqlite::params![k.clone(), limit as i64],
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
}
