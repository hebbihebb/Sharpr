//! File identity in the library index: cached content fingerprints
//! (`ROADMAP.md` Phase 4).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::params;

use super::{now_secs, path_to_string, LibraryIndex};

impl LibraryIndex {
    /// Every cached fingerprint as `path → (size, mtime_ns, hash)`.
    pub fn cached_fingerprints(&self) -> rusqlite::Result<HashMap<String, (u64, i64, String)>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT path, size, mtime_ns, hash FROM file_fingerprints")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                (row.get::<_, i64>(1)? as u64, row.get(2)?, row.get(3)?),
            ))
        })?;
        rows.collect()
    }

    /// Stores `(path, size, mtime_ns, hash)` fingerprints in one transaction.
    pub fn store_fingerprints(&self, rows: &[(PathBuf, u64, i64, String)]) -> rusqlite::Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO file_fingerprints (path, size, mtime_ns, hash, hashed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(path) DO UPDATE SET size = ?2, mtime_ns = ?3, hash = ?4, hashed_at = ?5",
            )?;
            let now = now_secs();
            for (path, size, mtime, hash) in rows {
                stmt.execute(params![
                    path_to_string(path),
                    *size as i64,
                    mtime,
                    hash,
                    now
                ])?;
            }
        }
        tx.commit()
    }

    /// The cached fingerprint of `path`, if it is still fresh for a file of
    /// `size` bytes modified at `mtime_ns`.
    pub fn fresh_fingerprint(
        &self,
        path: &Path,
        size: u64,
        mtime_ns: i64,
    ) -> rusqlite::Result<Option<String>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare_cached(
            "SELECT hash FROM file_fingerprints WHERE path = ?1 AND size = ?2 AND mtime_ns = ?3",
        )?;
        let mut rows = stmt.query(params![path_to_string(path), size as i64, mtime_ns])?;
        rows.next()?.map(|row| row.get(0)).transpose()
    }
}
