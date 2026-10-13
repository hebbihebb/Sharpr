//! The vector file: `~/.local/share/sharpr/visual-search.sqlite`.
//!
//! Vectors are keyed by content fingerprint (Phase 4) and by a model key that
//! names the exact model files and image preparation. Vectors under any
//! other key are ignored, so a changed model or preparation reindexes
//! instead of mixing incompatible vectors. Nothing else depends on this
//! file: deleting it costs only the time to index again.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use super::files::ModelFiles;
use super::{DIMENSIONS, PREPARATION_VERSION};

pub const SCHEMA_VERSION: i64 = 1;

pub fn default_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("sharpr")
        .join("visual-search.sqlite")
}

/// One connection; each thread that needs the store opens its own.
pub struct Store {
    conn: Connection,
}

fn sql_error(e: rusqlite::Error) -> String {
    format!("Visual search index: {e}")
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, String> {
        crate::db_version::ensure_not_newer(path, SCHEMA_VERSION, "visual search index")
            .map_err(sql_error)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("Visual search index: {e}"))?;
        }
        let conn = Connection::open(path).map_err(sql_error)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(sql_error)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(sql_error)?;
        Self::setup(conn)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, String> {
        Self::setup(Connection::open_in_memory().map_err(sql_error)?)
    }

    fn setup(conn: Connection) -> Result<Self, String> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS vectors (
                fingerprint TEXT NOT NULL,
                model TEXT NOT NULL,
                vector BLOB NOT NULL,
                indexed_at INTEGER NOT NULL,
                PRIMARY KEY (fingerprint, model)
            );
            CREATE TABLE IF NOT EXISTS failures (
                fingerprint TEXT NOT NULL,
                model TEXT NOT NULL,
                error TEXT NOT NULL,
                failed_at INTEGER NOT NULL,
                PRIMARY KEY (fingerprint, model)
            );
            CREATE TABLE IF NOT EXISTS model_files (
                path TEXT PRIMARY KEY,
                size INTEGER NOT NULL,
                mtime_ns INTEGER NOT NULL,
                hash TEXT NOT NULL
            );",
        )
        .map_err(sql_error)?;
        crate::db_version::stamp(&conn, SCHEMA_VERSION).map_err(sql_error)?;
        Ok(Self { conn })
    }

    /// Names the exact model files and preparation. Each file's hash is
    /// cached by size and modification time, so only a changed file is read
    /// again.
    pub fn model_key(&self, files: &ModelFiles) -> Result<String, String> {
        let model = self.file_hash(&files.model)?;
        let projector = self.file_hash(&files.projector)?;
        Ok(format!(
            "embeddinggemma-2:{}:{}:p{PREPARATION_VERSION}",
            &model[..16],
            &projector[..16]
        ))
    }

    fn file_hash(&self, path: &Path) -> Result<String, String> {
        let metadata = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let (size, mtime) = crate::fingerprint::file_stamp(&metadata);
        let key = path.to_string_lossy();
        let cached: Option<String> = self
            .conn
            .query_row(
                "SELECT hash FROM model_files WHERE path = ?1 AND size = ?2 AND mtime_ns = ?3",
                params![key, size as i64, mtime],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        if let Some(hash) = cached {
            return Ok(hash);
        }
        let hash =
            crate::fingerprint::hash_file(path).map_err(|e| format!("{}: {e}", path.display()))?;
        self.conn
            .execute(
                "INSERT OR REPLACE INTO model_files (path, size, mtime_ns, hash) VALUES (?1, ?2, ?3, ?4)",
                params![key, size as i64, mtime, hash],
            )
            .map_err(sql_error)?;
        Ok(hash)
    }

    /// Fingerprints that have a vector, or a recorded failure, for `model`.
    pub fn done(&self, model: &str) -> Result<HashSet<String>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT fingerprint FROM vectors WHERE model = ?1
                 UNION SELECT fingerprint FROM failures WHERE model = ?1",
            )
            .map_err(sql_error)?;
        let rows = stmt
            .query_map([model], |row| row.get::<_, String>(0))
            .map_err(sql_error)?;
        rows.collect::<Result<_, _>>().map_err(sql_error)
    }

    /// Fingerprints whose image could not be indexed, with the reason.
    pub fn failures(&self, model: &str) -> Result<HashMap<String, String>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT fingerprint, error FROM failures WHERE model = ?1")
            .map_err(sql_error)?;
        let rows = stmt
            .query_map([model], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(sql_error)?;
        rows.collect::<Result<_, _>>().map_err(sql_error)
    }

    pub fn put_vector(&self, model: &str, fingerprint: &str, vector: &[f32]) -> Result<(), String> {
        if vector.len() != DIMENSIONS {
            return Err(format!("Vector has {} numbers", vector.len()));
        }
        let blob: Vec<u8> = vector.iter().flat_map(|v| v.to_le_bytes()).collect();
        self.conn
            .execute(
                "INSERT OR REPLACE INTO vectors (fingerprint, model, vector, indexed_at)
                 VALUES (?1, ?2, ?3, strftime('%s','now'))",
                params![fingerprint, model, blob],
            )
            .map_err(sql_error)?;
        Ok(())
    }

    pub fn put_failure(&self, model: &str, fingerprint: &str, error: &str) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO failures (fingerprint, model, error, failed_at)
                 VALUES (?1, ?2, ?3, strftime('%s','now'))",
                params![fingerprint, model, error],
            )
            .map_err(sql_error)?;
        Ok(())
    }

    /// Every vector for `model`, by fingerprint. Rows of the wrong length are
    /// skipped.
    pub fn vectors(&self, model: &str) -> Result<HashMap<String, Vec<f32>>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT fingerprint, vector FROM vectors WHERE model = ?1")
            .map_err(sql_error)?;
        let rows = stmt
            .query_map([model], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .map_err(sql_error)?;
        let mut vectors = HashMap::new();
        for row in rows {
            let (fingerprint, blob) = row.map_err(sql_error)?;
            if blob.len() != DIMENSIONS * 4 {
                continue;
            }
            let vector = blob
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            vectors.insert(fingerprint, vector);
        }
        Ok(vectors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(index: usize) -> Vec<f32> {
        let mut v = vec![0.0; DIMENSIONS];
        v[index] = 1.0;
        v
    }

    #[test]
    fn vectors_and_failures_are_kept_per_model() {
        let store = Store::open_in_memory().unwrap();
        store.put_vector("m1", "aa", &unit(0)).unwrap();
        store.put_failure("m1", "bb", "broken file").unwrap();
        store.put_vector("m2", "cc", &unit(1)).unwrap();
        assert!(store.put_vector("m1", "dd", &[1.0]).is_err());

        assert_eq!(
            store.done("m1").unwrap(),
            HashSet::from(["aa".to_string(), "bb".to_string()])
        );
        let vectors = store.vectors("m1").unwrap();
        assert_eq!(vectors.len(), 1);
        assert_eq!(vectors["aa"], unit(0));
        assert_eq!(store.failures("m1").unwrap()["bb"], "broken file");
        assert!(store.vectors("m3").unwrap().is_empty());
    }

    #[test]
    fn model_key_follows_the_files_and_preparation() {
        let dir = std::env::temp_dir().join(format!(
            "sharpr-vector-store-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let files = ModelFiles {
            model: dir.join("model.gguf"),
            projector: dir.join("projector.gguf"),
            server: dir.join("llama-server"),
        };
        std::fs::write(&files.model, b"GGUF one").unwrap();
        std::fs::write(&files.projector, b"GGUF two").unwrap();
        let path = dir.join("visual-search.sqlite");
        let store = Store::open(&path).unwrap();
        let first = store.model_key(&files).unwrap();
        assert!(
            first.starts_with("embeddinggemma-2:") && first.ends_with(":p1"),
            "{first}"
        );
        assert_eq!(store.model_key(&files).unwrap(), first);

        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(&files.model, b"GGUF new").unwrap();
        assert_ne!(store.model_key(&files).unwrap(), first);

        // Reopening keeps vectors; deleting the file loses only vectors.
        store.put_vector(&first, "aa", &unit(2)).unwrap();
        drop(store);
        assert_eq!(
            Store::open(&path).unwrap().vectors(&first).unwrap().len(),
            1
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
