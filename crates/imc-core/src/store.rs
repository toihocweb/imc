use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::types::{Chunk, Edge, EdgeKind, Symbol, SymbolKind};

pub struct Store {
    pub conn: Connection,
}

pub fn open(db_path: &Path) -> Result<Store> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let conn = Connection::open(db_path)
        .with_context(|| format!("open sqlite {}", db_path.display()))?;
    conn.execute_batch(
        r#"
        PRAGMA journal_mode=WAL;
        PRAGMA synchronous=NORMAL;
        CREATE TABLE IF NOT EXISTS files (
            path TEXT PRIMARY KEY,
            hash TEXT NOT NULL,
            lang TEXT,
            size INTEGER DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS symbols (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            qualname TEXT NOT NULL,
            kind TEXT NOT NULL,
            file TEXT NOT NULL,
            line INTEGER NOT NULL,
            end_line INTEGER NOT NULL,
            start_byte INTEGER NOT NULL,
            end_byte INTEGER NOT NULL,
            parent TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_symbols_file ON symbols(file);
        CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
        CREATE TABLE IF NOT EXISTS edges (
            src TEXT NOT NULL,
            dst TEXT NOT NULL,
            kind TEXT NOT NULL,
            file TEXT NOT NULL,
            line INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_edges_src ON edges(src);
        CREATE INDEX IF NOT EXISTS idx_edges_dst ON edges(dst);
        CREATE TABLE IF NOT EXISTS chunks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            file TEXT NOT NULL,
            start_line INTEGER NOT NULL,
            end_line INTEGER NOT NULL,
            text TEXT NOT NULL,
            sym_id TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_chunks_file ON chunks(file);
        CREATE TABLE IF NOT EXISTS embeddings (
            chunk_id INTEGER PRIMARY KEY,
            dim INTEGER NOT NULL,
            vec BLOB NOT NULL
        );
        CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);
        "#,
    )?;
    Ok(Store { conn })
}

impl Store {
    pub fn file_hash(&self, rel: &str) -> Option<String> {
        self.conn
            .query_row("SELECT hash FROM files WHERE path = ?1", params![rel], |r| {
                r.get::<_, String>(0)
            })
            .ok()
    }

    pub fn all_file_hashes(&self) -> Vec<(String, String)> {
        let mut stmt = match self
            .conn
            .prepare("SELECT path, hash FROM files")
        {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map(|it| it.filter_map(Result::ok).collect())
            .unwrap_or_default();
        rows
    }

    pub fn upsert_file(&self, rel: &str, hash: &str, lang: Option<&str>, size: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO files(path, hash, lang, size) VALUES (?1,?2,?3,?4)
             ON CONFLICT(path) DO UPDATE SET hash=excluded.hash, lang=excluded.lang, size=excluded.size",
            params![rel, hash, lang, size as i64],
        )?;
        Ok(())
    }

    /// Replace every row belonging to a file — the Rust port re-derives per-file state.
    /// Remove the file row itself (called on deleted paths).
    pub fn remove_file(&self, rel: &str) -> Result<()> {
        self.conn.execute("DELETE FROM files WHERE path = ?1", params![rel])?;
        Ok(())
    }

    pub fn clear_file(&self, rel: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM symbols WHERE file = ?1", params![rel])?;
        self.conn
            .execute("DELETE FROM edges WHERE file = ?1", params![rel])?;
        self.conn.execute(
            "DELETE FROM embeddings WHERE chunk_id IN (SELECT id FROM chunks WHERE file = ?1)",
            params![rel],
        )?;
        self.conn
            .execute("DELETE FROM chunks WHERE file = ?1", params![rel])?;
        Ok(())
    }

    pub fn insert_symbols(&mut self, rel: &str, symbols: &[Symbol]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO symbols
                 (id,name,qualname,kind,file,line,end_line,start_byte,end_byte,parent)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            )?;
            for s in symbols {
                stmt.execute(params![
                    s.id,
                    s.name,
                    s.qualname,
                    s.kind.as_str(),
                    rel,
                    s.line as i64,
                    s.end_line as i64,
                    s.start_byte as i64,
                    s.end_byte as i64,
                    s.parent
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn insert_edges(&mut self, rel: &str, edges: &[Edge]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO edges(src,dst,kind,file,line) VALUES (?1,?2,?3,?4,?5)",
            )?;
            for e in edges {
                stmt.execute(params![e.src, e.dst, e.kind.as_str(), rel, e.line as i64])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn insert_chunk(&self, file: &str, start_line: usize, end_line: usize, text: &str, sym_id: Option<&str>) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO chunks(file,start_line,end_line,text,sym_id) VALUES (?1,?2,?3,?4,?5)",
            params![file, start_line as i64, end_line as i64, text, sym_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn insert_embedding(&self, chunk_id: i64, dim: usize, vec: &[f32]) -> Result<()> {
        let mut buf = Vec::with_capacity(vec.len() * 4);
        for v in vec {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        self.conn.execute(
            "INSERT OR REPLACE INTO embeddings(chunk_id,dim,vec) VALUES (?1,?2,?3)",
            params![chunk_id, dim as i64, buf],
        )?;
        Ok(())
    }

    pub fn count_chunks(&self) -> usize {
        self.conn
            .query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as usize
    }

    pub fn count_symbols(&self) -> usize {
        self.conn
            .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as usize
    }

    pub fn count_edges(&self) -> usize {
        self.conn
            .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as usize
    }

    pub fn count_files(&self) -> usize {
        self.conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as usize
    }

    pub fn symbols_in_file(&self, rel: &str) -> Vec<Symbol> {
        let mut stmt = match self.conn.prepare(
            "SELECT id,name,qualname,kind,file,line,end_line,start_byte,end_byte,parent FROM symbols WHERE file=?1 ORDER BY start_byte",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt
            .query_map(params![rel], row_to_symbol)
            .map(|it| it.filter_map(Result::ok).collect())
            .unwrap_or_default();
        rows
    }

    pub fn find_symbols_by_name(&self, name: &str, limit: usize) -> Vec<Symbol> {
        let mut stmt = match self.conn.prepare(
            "SELECT id,name,qualname,kind,file,line,end_line,start_byte,end_byte,parent
             FROM symbols WHERE name=?1 ORDER BY length(qualname) LIMIT ?2",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map(params![name, limit as i64], row_to_symbol)
            .map(|it| it.filter_map(Result::ok).collect())
            .unwrap_or_default()
    }

    /// Definitions in a file, ordered by position — the graph-search building block.
    pub fn definitions_in_file(&self, rel: &str) -> Vec<Symbol> {
        self.symbols_in_file(rel)
            .into_iter()
            .filter(|s| s.kind.is_definition())
            .collect()
    }

    pub fn callees(&self, sym_id: &str) -> Vec<String> {
        let mut stmt = match self
            .conn
            .prepare("SELECT dst FROM edges WHERE src=?1 AND kind='call'")
        {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map(params![sym_id], |r| r.get::<_, String>(0))
            .map(|it| it.filter_map(Result::ok).collect())
            .unwrap_or_default()
    }

    pub fn callers(&self, name: &str) -> Vec<String> {
        let mut stmt = match self
            .conn
            .prepare("SELECT src FROM edges WHERE dst=?1 AND kind='call'")
        {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map(params![name], |r| r.get::<_, String>(0))
            .map(|it| it.filter_map(Result::ok).collect())
            .unwrap_or_default()
    }

    pub fn chunk_by_id(&self, id: i64) -> Option<Chunk> {
        self.conn
            .query_row(
                "SELECT id,file,start_line,end_line,text,sym_id FROM chunks WHERE id=?1",
                params![id],
                |r| {
                    Ok(Chunk {
                        id: r.get(0)?,
                        file: r.get(1)?,
                        start_line: r.get::<_, i64>(2)? as usize,
                        end_line: r.get::<_, i64>(3)? as usize,
                        text: r.get(4)?,
                        sym_id: r.get(5)?,
                    })
                },
            )
            .ok()
    }

    /// All chunks with vectors, for brute-force cosine scoring.
    pub fn all_vectors(&self) -> Vec<(i64, Vec<f32>)> {
        let mut stmt = match self
            .conn
            .prepare("SELECT chunk_id, vec FROM embeddings")
        {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt
            .query_map([], |r| {
                let id: i64 = r.get(0)?;
                let blob: Vec<u8> = r.get(1)?;
                let mut v = Vec::with_capacity(blob.len() / 4);
                for c in blob.chunks_exact(4) {
                    v.push(f32::from_le_bytes([c[0], c[1], c[2], c[3]]));
                }
                Ok((id, v))
            })
            .map(|it| it.filter_map(Result::ok).collect())
            .unwrap_or_default();
        rows
    }

    pub fn memory_count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM memories", [], |r| r.get::<usize, i64>(0usize))?)
    }

    pub fn stats(&self) -> Vec<(String, i64)> {
        let mut out = Vec::new();
        for t in ["files", "symbols", "edges", "chunks", "embeddings"] {
            let n: i64 = self
                .conn
                .query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0))
                .unwrap_or(0);
            out.push((t.to_string(), n));
        }
        out
    }
}

fn row_to_symbol(r: &rusqlite::Row<'_>) -> rusqlite::Result<Symbol> {
    let kind: String = r.get(3)?;
    Ok(Symbol {
        id: r.get(0)?,
        name: r.get(1)?,
        qualname: r.get(2)?,
        kind: SymbolKind::from_str(&kind),
        file: r.get(4)?,
        line: r.get::<_, i64>(5)? as usize,
        end_line: r.get::<_, i64>(6)? as usize,
        start_byte: r.get::<_, i64>(7)? as usize,
        end_byte: r.get::<_, i64>(8)? as usize,
        parent: r.get(9)?,
    })
}

pub fn _edge_kind_roundtrip(k: EdgeKind) -> EdgeKind {
    EdgeKind::from_str(k.as_str())
}