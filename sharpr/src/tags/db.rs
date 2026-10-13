#[cfg(test)]
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection};

pub struct TagChoices {
    pub active: Vec<String>,
    pub available: Vec<(String, usize)>,
}

pub struct TagChange {
    pub paths: Vec<PathBuf>,
    pub tags: Vec<String>,
    pub remove: bool,
}

pub struct TagDatabase {
    conn: Mutex<Connection>,
}

/// Version recorded in `PRAGMA user_version`; see `crate::db_version`.
const SCHEMA_VERSION: i64 = 1;

/// Tags the removed automatic tagger added: the file's own extension and a
/// resolution class. See `TagDatabase::automatic_tag_rows`.
const AUTOMATIC_FORMAT_TAGS: &[&str] = &[
    "jpg", "jpeg", "png", "webp", "jxl", "gif", "tif", "tiff", "bmp", "ico",
];
const AUTOMATIC_RESOLUTION_TAGS: &[&str] = &["720p", "1080p", "4k"];

impl TagDatabase {
    pub fn open() -> rusqlite::Result<Self> {
        let dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("sharpr");
        std::fs::create_dir_all(&dir).ok();
        Self::open_at(&dir.join("tags.sqlite3"))
    }

    fn open_at(path: &Path) -> rusqlite::Result<Self> {
        crate::db_version::ensure_not_newer(path, SCHEMA_VERSION, "tags database")?;
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS file_tags (
                path TEXT NOT NULL,
                tag  TEXT NOT NULL,
                PRIMARY KEY (path, tag)
            );
            CREATE INDEX IF NOT EXISTS idx_tag ON file_tags(tag);

            CREATE TABLE IF NOT EXISTS image_quality (
                path      TEXT    PRIMARY KEY,
                sharpness REAL    NOT NULL,
                mtime     INTEGER NOT NULL
            );
            ",
        )?;
        crate::db_version::stamp(&conn, SCHEMA_VERSION)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Returns the stored sharpness variance for `path` if it was scored at
    /// the current file mtime, or `None` if not yet scored or the file changed.
    pub fn get_sharpness(&self, path: &Path) -> Option<f64> {
        let current_mtime = file_mtime_secs(path)?;
        let Ok(conn) = self.conn.lock() else {
            return None;
        };
        let mut stmt = conn
            .prepare_cached("SELECT sharpness, mtime FROM image_quality WHERE path = ?1")
            .ok()?;
        let (score, stored_mtime) = stmt
            .query_row(rusqlite::params![path.to_string_lossy().as_ref()], |row| {
                Ok((row.get::<_, f64>(0)?, row.get::<_, i64>(1)?))
            })
            .ok()?;
        if stored_mtime as u64 == current_mtime {
            Some(score)
        } else {
            None
        }
    }

    /// Persist `score` (raw Laplacian variance) for `path` at the given file mtime.
    pub fn upsert_sharpness(&self, path: &Path, score: f64, mtime: u64) {
        let Ok(conn) = self.conn.lock() else { return };
        let _ = conn.execute(
            "INSERT OR REPLACE INTO image_quality (path, sharpness, mtime) VALUES (?1, ?2, ?3)",
            rusqlite::params![path.to_string_lossy().as_ref(), score, mtime as i64],
        );
    }

    pub fn insert_tags(&self, path: &Path, tags: &[String]) {
        let Ok(mut conn) = self.conn.lock() else {
            return;
        };
        let path_str = path.to_string_lossy();
        let tx = match conn.transaction() {
            Ok(tx) => tx,
            Err(_) => return,
        };
        let _ = tx.execute(
            "DELETE FROM file_tags WHERE path = ?1",
            params![path_str.as_ref()],
        );
        for tag in tags {
            let _ = tx.execute(
                "INSERT OR IGNORE INTO file_tags (path, tag) VALUES (?1, ?2)",
                params![path_str.as_ref(), tag],
            );
        }
        let _ = tx.commit();
    }

    pub fn paths_for_tag(&self, tag: &str) -> Vec<PathBuf> {
        let Ok(conn) = self.conn.lock() else {
            return vec![];
        };
        let mut stmt = match conn.prepare("SELECT path FROM file_tags WHERE tag = ?1") {
            Ok(stmt) => stmt,
            Err(_) => return vec![],
        };
        let rows = match stmt.query_map(params![tag.to_lowercase()], |row| row.get::<_, String>(0))
        {
            Ok(rows) => rows,
            Err(_) => return vec![],
        };
        let paths: Vec<PathBuf> = rows.filter_map(|r| r.ok()).map(PathBuf::from).collect();
        drop(stmt);
        drop(conn);
        // Missing files are hidden, never deleted: they may be on an unmounted
        // drive or moved back later, and their tags back bucket membership.
        paths.into_iter().filter(|p| p.is_file()).collect()
    }

    #[cfg(test)]
    pub fn search_paths(&self, query: &str) -> Vec<PathBuf> {
        self.search_paths_checked(query).unwrap_or_default()
    }

    pub fn search_paths_checked(&self, query: &str) -> Result<Vec<PathBuf>, String> {
        let terms = normalized_query_terms(query);
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let conditions = std::iter::repeat_n(
            "EXISTS (SELECT 1 FROM file_tags t WHERE t.path = f.path AND t.tag LIKE ? ESCAPE '\\')",
            terms.len(),
        )
        .collect::<Vec<_>>()
        .join(" AND ");
        let sql = format!("SELECT DISTINCT f.path FROM file_tags f WHERE {conditions} ORDER BY f.path COLLATE NOCASE");
        let patterns: Vec<_> = terms
            .iter()
            .map(|t| {
                format!(
                    "%{}%",
                    t.replace('\\', "\\\\")
                        .replace('%', "\\%")
                        .replace('_', "\\_")
                )
            })
            .collect();
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(patterns), |r| {
                r.get::<_, String>(0)
            })
            .map_err(|e| e.to_string())?;
        let paths = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        Ok(paths
            .into_iter()
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .collect())
    }

    /// Tags with the number of their paths that exist on disk. Rows for
    /// missing paths are kept but not counted; tags with no existing path
    /// are omitted.
    #[cfg(test)]
    pub fn all_tags(&self) -> Vec<(String, usize)> {
        let rows: Vec<(String, String)> = {
            let Ok(conn) = self.conn.lock() else {
                return vec![];
            };
            let mut stmt = match conn.prepare("SELECT tag, path FROM file_tags") {
                Ok(stmt) => stmt,
                Err(_) => return vec![],
            };
            let rows = match stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?))) {
                Ok(rows) => rows,
                Err(_) => return vec![],
            };
            rows.filter_map(|r| r.ok()).collect()
        };
        let mut exists: HashMap<String, bool> = HashMap::new();
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for (tag, path) in rows {
            let present = *exists
                .entry(path)
                .or_insert_with_key(|p| Path::new(p).is_file());
            if present {
                *counts.entry(tag).or_default() += 1;
            }
        }
        let mut tags: Vec<_> = counts.into_iter().collect();
        tags.sort_by_cached_key(|(tag, _)| tag.to_lowercase());
        tags
    }

    pub fn tag_choices_checked(&self, path: &Path) -> Result<TagChoices, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut active = conn
            .prepare("SELECT tag FROM file_tags WHERE path = ?1 ORDER BY tag COLLATE NOCASE")
            .map_err(|e| e.to_string())?;
        let tags = active
            .query_map(params![path.to_string_lossy().as_ref()], |r| {
                r.get::<_, String>(0)
            })
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        let mut all = conn
            .prepare("SELECT tag, COUNT(*) FROM file_tags GROUP BY tag ORDER BY tag COLLATE NOCASE")
            .map_err(|e| e.to_string())?;
        let choices = all
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, usize>(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        Ok(TagChoices {
            active: tags,
            available: choices,
        })
    }

    pub fn tags_for_path(&self, path: &Path) -> Vec<String> {
        let Ok(conn) = self.conn.lock() else {
            return vec![];
        };
        let mut stmt = match conn.prepare("SELECT tag FROM file_tags WHERE path = ?1 ORDER BY tag")
        {
            Ok(stmt) => stmt,
            Err(_) => return vec![],
        };
        let rows = match stmt.query_map(params![path.to_string_lossy().as_ref()], |row| {
            row.get::<_, String>(0)
        }) {
            Ok(rows) => rows,
            Err(_) => return vec![],
        };
        rows.filter_map(|r| r.ok()).collect()
    }

    pub fn tags_for_path_checked(&self, path: &Path) -> Result<Vec<String>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT tag FROM file_tags WHERE path=?1 ORDER BY tag")
            .map_err(|e| e.to_string())?;
        let result = stmt
            .query_map(params![path.to_string_lossy().as_ref()], |r| {
                r.get::<_, String>(0)
            })
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string());
        result
    }

    pub fn copy_tags(&self, source: &Path, output: &Path) {
        if source == output {
            return;
        }
        let Ok(conn) = self.conn.lock() else {
            return;
        };
        let _ = conn.execute(
            "INSERT OR IGNORE INTO file_tags (path, tag)
             SELECT ?2, tag FROM file_tags WHERE path = ?1",
            params![
                source.to_string_lossy().as_ref(),
                output.to_string_lossy().as_ref()
            ],
        );
    }

    #[cfg(test)]
    pub fn add_tag(&self, path: &Path, tag: &str) {
        let Ok(conn) = self.conn.lock() else {
            return;
        };
        let tag = normalize_tag(tag);
        if tag.is_empty() {
            return;
        }
        let _ = conn.execute(
            "INSERT OR IGNORE INTO file_tags (path, tag) VALUES (?1, ?2)",
            params![path.to_string_lossy().as_ref(), tag],
        );
    }

    /// `(path, tag)` rows the old automatic tagger created (decision D17).
    /// A format tag only counts when it equals the file's own extension, so a
    /// hand-made "png" tag on a JPEG is kept. Tags in `protected` (those used
    /// by buckets) are never returned.
    pub fn automatic_tag_rows(
        &self,
        protected: &std::collections::HashSet<String>,
    ) -> Result<Vec<(String, String)>, String> {
        let candidates: Vec<&str> = AUTOMATIC_FORMAT_TAGS
            .iter()
            .chain(AUTOMATIC_RESOLUTION_TAGS)
            .copied()
            .filter(|tag| !protected.contains(*tag))
            .collect();
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        let rows: Vec<(String, String)> = {
            let conn = self.conn.lock().map_err(|e| e.to_string())?;
            let placeholders = std::iter::repeat_n("?", candidates.len())
                .collect::<Vec<_>>()
                .join(",");
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT path, tag FROM file_tags WHERE tag IN ({placeholders})"
                ))
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(&candidates), |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .map_err(|e| e.to_string())?;
            rows.collect::<rusqlite::Result<_>>()
                .map_err(|e| e.to_string())?
        };
        Ok(rows
            .into_iter()
            .filter(|(path, tag)| {
                AUTOMATIC_RESOLUTION_TAGS.contains(&tag.as_str())
                    || Path::new(path)
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|ext| ext.eq_ignore_ascii_case(tag))
            })
            .collect())
    }

    /// Deletes exactly these `(path, tag)` rows in one transaction.
    pub fn delete_rows(&self, rows: &[(String, String)]) -> Result<usize, String> {
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let mut deleted = 0;
        {
            let mut stmt = tx
                .prepare("DELETE FROM file_tags WHERE path = ?1 AND tag = ?2")
                .map_err(|e| e.to_string())?;
            for (path, tag) in rows {
                deleted += stmt
                    .execute(params![path, tag])
                    .map_err(|e| e.to_string())?;
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(deleted)
    }

    /// Writes a consistent copy of the whole tags database to `dest`, which
    /// must not exist yet.
    pub fn backup_to(&self, dest: &Path) -> Result<(), String> {
        if dest.exists() {
            return Err(format!("{} already exists", dest.display()));
        }
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute("VACUUM INTO ?1", params![dest.to_string_lossy().as_ref()])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Moves tags and sharpness from `old_path` to `new_path` after a file
    /// rename or move. Callers guarantee `new_path` did not exist before, so
    /// any rows already stored there belong to a vanished file and are
    /// replaced rather than merged into the moved image.
    pub fn rename_path(&self, old_path: &Path, new_path: &Path) -> Result<(), String> {
        if old_path == new_path {
            return Ok(());
        }
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let old_path_str = old_path.to_string_lossy();
        let new_path_str = new_path.to_string_lossy();
        let old = old_path_str.as_ref();
        let new = new_path_str.as_ref();
        // Rows at the new path belong to a file that vanished from there; they
        // are kept user data, so never merge into or overwrite them.
        let retained: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM file_tags WHERE path = ?1)",
                params![new],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if retained {
            return Err(format!(
                "tags are still kept for a missing file at {new}; they were left in place"
            ));
        }
        for (sql, args) in [
            (
                "INSERT INTO file_tags (path, tag) SELECT ?2, tag FROM file_tags WHERE path = ?1",
                vec![old, new],
            ),
            ("DELETE FROM file_tags WHERE path = ?1", vec![old]),
            (
                "INSERT OR REPLACE INTO image_quality (path, sharpness, mtime)
                 SELECT ?2, sharpness, mtime FROM image_quality WHERE path = ?1",
                vec![old, new],
            ),
            ("DELETE FROM image_quality WHERE path = ?1", vec![old]),
        ] {
            tx.execute(sql, rusqlite::params_from_iter(args))
                .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }

    /// Every `(path, tag)` row, including rows for missing files.
    pub fn stored_rows(&self) -> Result<Vec<(String, String)>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT path, tag FROM file_tags")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<_>>()
            .map_err(|e| e.to_string())
    }

    pub fn paths_for_all_tags(&self, tags: &[String]) -> Vec<PathBuf> {
        self.paths_for_all_tags_checked(tags)
            .unwrap_or_else(|error| {
                crate::bench_error!("tags.query.fail", serde_json::json!({"error": error}));
                Vec::new()
            })
    }
    /// Paths carrying every tag in `tags` whose files currently exist.
    pub fn paths_for_all_tags_checked(&self, tags: &[String]) -> Result<Vec<PathBuf>, String> {
        Ok(self
            .stored_paths_for_all_tags_checked(tags)?
            .into_iter()
            .filter(|p| p.is_file())
            .collect())
    }

    /// Paths carrying every tag in `tags`, including files that are missing.
    pub fn stored_paths_for_all_tags_checked(
        &self,
        tags: &[String],
    ) -> Result<Vec<PathBuf>, String> {
        let tags = normalized_unique_tags(tags);
        if tags.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let placeholders = std::iter::repeat_n("?", tags.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!("SELECT path FROM file_tags WHERE tag IN ({placeholders}) GROUP BY path HAVING COUNT(DISTINCT tag) = ? ORDER BY path COLLATE NOCASE");
        let mut parameters: Vec<&dyn rusqlite::ToSql> =
            tags.iter().map(|t| t as &dyn rusqlite::ToSql).collect();
        let count = tags.len() as i64;
        parameters.push(&count);
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(parameters.as_slice(), |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        let paths = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        Ok(paths.into_iter().map(PathBuf::from).collect())
    }

    #[cfg(test)]
    pub fn add_tags_to_paths(&self, paths: &[PathBuf], tags: &[String]) -> usize {
        self.change_tags_checked(paths, tags, false)
            .unwrap_or_else(|error| {
                crate::bench_error!("tags.write.fail", serde_json::json!({"error": error}));
                0
            })
    }
    #[cfg(test)]
    pub fn remove_tags_from_paths(&self, paths: &[PathBuf], tags: &[String]) -> usize {
        self.change_tags_checked(paths, tags, true)
            .unwrap_or_else(|error| {
                crate::bench_error!("tags.write.fail", serde_json::json!({"error": error}));
                0
            })
    }
    pub fn change_tags_checked(
        &self,
        paths: &[PathBuf],
        tags: &[String],
        remove: bool,
    ) -> Result<usize, String> {
        self.change_tag_batches_checked(&[TagChange {
            paths: paths.to_vec(),
            tags: tags.to_vec(),
            remove,
        }])
    }

    /// Apply a hierarchy's membership changes together or roll all of them back.
    pub fn change_tag_batches_checked(&self, batches: &[TagChange]) -> Result<usize, String> {
        let result = (|| -> rusqlite::Result<usize> {
            let mut conn = self
                .conn
                .lock()
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
            let tx = conn.transaction()?;
            let mut changed = 0;
            for batch in batches {
                let tags = normalized_unique_tags(&batch.tags);
                let sql = if batch.remove {
                    "DELETE FROM file_tags WHERE path = ?1 AND tag = ?2"
                } else {
                    "INSERT OR IGNORE INTO file_tags (path,tag) VALUES (?1,?2)"
                };
                for path in &batch.paths {
                    for tag in &tags {
                        changed +=
                            tx.execute(sql, params![path.to_string_lossy().as_ref(), tag])?;
                    }
                }
            }
            tx.commit()?;
            Ok(changed)
        })()
        .map_err(|e| e.to_string());
        if let Err(error) = &result {
            crate::bench_error!("tags.write.fail", serde_json::json!({"error": error}));
        }
        result
    }

    #[cfg(test)]
    pub fn replace_tag_in_paths(&self, paths: &[PathBuf], old_tag: &str, new_tag: &str) -> usize {
        self.replace_tag_in_paths_checked(paths, old_tag, new_tag)
            .unwrap_or(0)
    }

    pub fn replace_tag_in_paths_checked(
        &self,
        paths: &[PathBuf],
        old_tag: &str,
        new_tag: &str,
    ) -> Result<usize, String> {
        let result = (|| -> rusqlite::Result<usize> {
            let old = normalize_tag(old_tag);
            let new = normalize_tag(new_tag);
            if old.is_empty() || new.is_empty() || old == new {
                return Ok(0);
            }
            let mut conn = self
                .conn
                .lock()
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
            let tx = conn.transaction()?;
            let mut changed = 0;
            for path in paths {
                let removed = tx.execute(
                    "DELETE FROM file_tags WHERE path=?1 AND tag=?2",
                    params![path.to_string_lossy().as_ref(), old],
                )?;
                if removed > 0 {
                    tx.execute(
                        "INSERT OR IGNORE INTO file_tags(path,tag) VALUES (?1,?2)",
                        params![path.to_string_lossy().as_ref(), new],
                    )?;
                    changed += removed;
                }
            }
            tx.commit()?;
            Ok(changed)
        })()
        .map_err(|e| e.to_string());
        if let Err(error) = &result {
            crate::bench_error!("tags.rename.fail", serde_json::json!({"error":error}));
        }
        result
    }
}

#[cfg(test)]
impl TagDatabase {
    pub(crate) fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS file_tags (
                path TEXT NOT NULL,
                tag  TEXT NOT NULL,
                PRIMARY KEY (path, tag)
            );
            CREATE INDEX IF NOT EXISTS idx_tag ON file_tags(tag);

            CREATE TABLE IF NOT EXISTS image_quality (
                path      TEXT    PRIMARY KEY,
                sharpness REAL    NOT NULL,
                mtime     INTEGER NOT NULL
            );
            ",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

pub(crate) fn file_mtime_secs(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
}

fn normalize_tag(tag: &str) -> String {
    tag.trim().to_lowercase()
}

fn normalized_query_terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(normalize_tag)
        .filter(|term| !term.is_empty())
        .collect()
}

fn normalized_unique_tags(tags: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for tag in tags {
        let normalized = normalize_tag(tag);
        if !normalized.is_empty() && !out.contains(&normalized) {
            out.push(normalized);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hierarchy_tag_batches_roll_back_removals_when_addition_fails() {
        let db = TagDatabase::open_in_memory().unwrap();
        let path = PathBuf::from("/image.png");
        db.change_tags_checked(std::slice::from_ref(&path), &["old-parent".into()], false)
            .unwrap();
        db.conn.lock().unwrap().execute_batch("CREATE TRIGGER reject_tag BEFORE INSERT ON file_tags WHEN NEW.tag='reject' BEGIN SELECT RAISE(ABORT,'forced failure'); END;").unwrap();
        let changes = [
            TagChange {
                paths: vec![path.clone()],
                tags: vec!["old-parent".into()],
                remove: true,
            },
            TagChange {
                paths: vec![path.clone()],
                tags: vec!["reject".into()],
                remove: false,
            },
        ];
        assert!(db.change_tag_batches_checked(&changes).is_err());
        assert_eq!(db.tags_for_path(&path), vec!["old-parent"]);
    }

    #[test]
    fn failed_tag_rename_preserves_old_membership() {
        let db = TagDatabase::open_in_memory().unwrap();
        let path = PathBuf::from("/image.png");
        db.change_tags_checked(
            std::slice::from_ref(&path),
            &["keeper".into(), "portrait".into()],
            false,
        )
        .unwrap();
        db.conn.lock().unwrap().execute_batch("CREATE TRIGGER reject_tag BEFORE INSERT ON file_tags WHEN NEW.tag='reject' BEGIN SELECT RAISE(ABORT,'forced failure'); END;").unwrap();
        assert!(db
            .replace_tag_in_paths_checked(std::slice::from_ref(&path), "keeper", "reject")
            .is_err());
        assert_eq!(
            db.tags_for_path(&path),
            vec!["keeper".to_string(), "portrait".to_string()]
        );
    }

    #[test]
    fn bulk_tag_failure_does_not_commit_partial_membership() {
        let db = TagDatabase::open_in_memory().unwrap();
        db.conn.lock().unwrap().execute_batch("CREATE TRIGGER reject_tag BEFORE INSERT ON file_tags WHEN NEW.tag = 'reject' BEGIN SELECT RAISE(ABORT, 'forced write failure'); END;").unwrap();
        assert_eq!(
            db.add_tags_to_paths(
                &[PathBuf::from("/image.png")],
                &["keeper".into(), "reject".into()]
            ),
            0
        );
        assert!(db.tags_for_path(Path::new("/image.png")).is_empty());
    }

    fn temp_path(name: &str) -> PathBuf {
        let unique = format!(
            "sharpr-tags-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        std::env::temp_dir().join(unique).join(name)
    }

    fn stored_rows(db: &TagDatabase, tag: &str) -> i64 {
        db.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM file_tags WHERE tag = ?1",
                params![tag],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn open_stamps_the_schema_version_and_refuses_newer_databases() {
        let root = temp_path("versioned");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("tags.sqlite3");
        let photo = root.join("photo.jpg");
        std::fs::write(&photo, b"image").unwrap();
        {
            let db = TagDatabase::open_at(&path).unwrap();
            db.add_tag(&photo, "art");
            let version: i64 = db
                .conn
                .lock()
                .unwrap()
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .unwrap();
            assert_eq!(version, SCHEMA_VERSION);
        }
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        let before = std::fs::read(&path).unwrap();

        assert!(TagDatabase::open_at(&path).is_err());
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "a refused database is untouched"
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn automatic_tag_cleanup_takes_only_what_the_old_tagger_added() {
        let root = temp_path("auto-cleanup");
        std::fs::create_dir_all(&root).unwrap();
        let db = TagDatabase::open_in_memory().unwrap();
        let png = root.join("sea.PNG");
        let jpg = root.join("road.jpg");
        db.add_tag(&png, "png");
        db.add_tag(&png, "4k");
        db.add_tag(&png, "art");
        db.add_tag(&jpg, "png"); // hand-made: not this file's format
        db.add_tag(&jpg, "1080p");
        db.add_tag(&jpg, "jpg");

        let protected = std::collections::HashSet::from(["1080p".to_string()]);
        let mut rows = db.automatic_tag_rows(&protected).unwrap();
        rows.sort();
        let p = |path: &PathBuf, tag: &str| (path.to_string_lossy().into_owned(), tag.to_string());
        assert_eq!(rows, vec![p(&jpg, "jpg"), p(&png, "4k"), p(&png, "png")]);

        let backup = root.join("backup.sqlite3");
        db.backup_to(&backup).unwrap();
        assert!(db.backup_to(&backup).is_err(), "never overwrites a backup");
        assert_eq!(db.delete_rows(&rows).unwrap(), 3);

        assert_eq!(db.tags_for_path(&png), vec!["art".to_string()]);
        let mut left = db.tags_for_path(&jpg);
        left.sort();
        assert_eq!(left, vec!["1080p".to_string(), "png".to_string()]);
        let restored = TagDatabase::open_at(&backup).unwrap();
        assert_eq!(
            restored.tags_for_path(&png).len(),
            3,
            "backup holds the old rows"
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tag_queries_hide_missing_paths_without_deleting_them() {
        let db = TagDatabase::open_in_memory().unwrap();
        let root = temp_path("root");
        std::fs::create_dir_all(&root).unwrap();
        let existing = root.join("cow.jpg");
        let missing = root.join("missing.jpg");
        std::fs::write(&existing, b"not really an image").unwrap();

        db.add_tag(&existing, "cow");
        db.add_tag(&missing, "cow");

        assert_eq!(db.paths_for_tag("cow"), vec![existing.clone()]);
        assert_eq!(db.search_paths("cow"), vec![existing.clone()]);
        assert_eq!(db.all_tags(), vec![("cow".to_string(), 1)]);
        assert_eq!(stored_rows(&db, "cow"), 2);

        // The whole library root disappears, as with an unmounted drive or a
        // folder moved away outside Sharpr.
        std::fs::remove_dir_all(&root).unwrap();
        assert!(db.paths_for_tag("cow").is_empty());
        assert!(db.search_paths("cow").is_empty());
        assert!(db.all_tags().is_empty());
        assert_eq!(
            stored_rows(&db, "cow"),
            2,
            "reads must never delete tag rows"
        );

        // When the file returns, so do its tags.
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&existing, b"not really an image").unwrap();
        assert_eq!(db.paths_for_tag("cow"), vec![existing.clone()]);
        assert_eq!(db.all_tags(), vec![("cow".to_string(), 1)]);
        assert_eq!(db.tags_for_path(&existing), vec!["cow".to_string()]);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn all_tags_counts_existing_paths_in_case_insensitive_order() {
        let db = TagDatabase::open_in_memory().unwrap();
        let root = temp_path("counts");
        std::fs::create_dir_all(&root).unwrap();
        let a = root.join("a.jpg");
        let b = root.join("b.jpg");
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();

        db.add_tag(&a, "Zebra");
        db.add_tag(&a, "art");
        db.add_tag(&b, "art");
        db.add_tag(&root.join("gone.jpg"), "Gone");

        assert_eq!(
            db.all_tags(),
            vec![("art".to_string(), 2), ("zebra".to_string(), 1)]
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn search_paths_matches_all_query_terms_across_tags() {
        let db = TagDatabase::open_in_memory().unwrap();
        let root = temp_path("search-terms");
        std::fs::create_dir_all(&root).unwrap();
        let a = root.join("a.jpg");
        let b = root.join("b.jpg");
        let c = root.join("c.jpg");
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        std::fs::write(&c, b"c").unwrap();

        db.add_tag(&a, "people");
        db.add_tag(&a, "model");
        db.add_tag(&b, "people");
        db.add_tag(&b, "portrait");
        db.add_tag(&c, "people model");

        assert_eq!(db.search_paths("people model"), vec![a.clone(), c.clone()]);
        assert_eq!(db.search_paths("people portrait"), vec![b.clone()]);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn paths_for_all_tags_requires_full_intersection() {
        let db = TagDatabase::open_in_memory().unwrap();
        let root = temp_path("all-tags");
        std::fs::create_dir_all(&root).unwrap();
        let a = root.join("a.jpg");
        let b = root.join("b.jpg");
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();

        db.add_tag(&a, "people");
        db.add_tag(&a, "model");
        db.add_tag(&b, "people");

        let paths = db.paths_for_all_tags(&["people".into(), "model".into()]);
        assert_eq!(paths, vec![a.clone()]);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn upsert_and_get_sharpness_round_trip() {
        let db = TagDatabase::open_in_memory().unwrap();
        let root = temp_path("sharpness-rt");
        std::fs::create_dir_all(&root).unwrap();
        let img = root.join("sharp.jpg");
        std::fs::write(&img, b"fake image data").unwrap();

        let actual_mtime = file_mtime_secs(&img).expect("mtime must be readable");
        db.upsert_sharpness(&img, 42.5, actual_mtime);

        let got = db.get_sharpness(&img);
        assert_eq!(got, Some(42.5), "should return the stored sharpness score");

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn get_sharpness_returns_none_when_stored_mtime_is_stale() {
        let db = TagDatabase::open_in_memory().unwrap();
        let root = temp_path("sharpness-stale");
        std::fs::create_dir_all(&root).unwrap();
        let img = root.join("img.jpg");
        std::fs::write(&img, b"data").unwrap();

        // Store with mtime 0 — intentionally wrong.
        db.upsert_sharpness(&img, 99.0, 0);

        // get_sharpness checks the *actual* mtime vs stored; 0 won't match.
        assert!(
            db.get_sharpness(&img).is_none(),
            "stale mtime must invalidate the cached score"
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn add_tag_normalizes_case_and_whitespace() {
        let db = TagDatabase::open_in_memory().unwrap();
        let root = temp_path("normalize");
        std::fs::create_dir_all(&root).unwrap();
        let img = root.join("img.jpg");
        std::fs::write(&img, b"data").unwrap();

        db.add_tag(&img, "  Portrait  ");
        db.add_tag(&img, "PORTRAIT"); // should be deduped after normalization

        let tags = db.tags_for_path(&img);
        assert_eq!(
            tags,
            vec!["portrait"],
            "tags must be normalised to lowercase trimmed form"
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn copy_tags_adds_source_tags_to_output() {
        let db = TagDatabase::open_in_memory().unwrap();
        let source = PathBuf::from("/photos/source.jpg");
        let output = PathBuf::from("/photos/source-upscaled.png");

        db.add_tag(&source, "people");
        db.add_tag(&source, "model");
        db.copy_tags(&source, &output);
        db.copy_tags(&source, &output);

        assert_eq!(
            db.tags_for_path(&output),
            vec!["model".to_string(), "people".to_string()]
        );
    }

    #[test]
    fn bulk_tag_operations_update_multiple_paths() {
        let db = TagDatabase::open_in_memory().unwrap();
        let root = temp_path("bulk-tags");
        std::fs::create_dir_all(&root).unwrap();
        let a = root.join("a.jpg");
        let b = root.join("b.jpg");
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();

        db.add_tags_to_paths(&[a.clone(), b.clone()], &["people".into(), "model".into()]);
        assert_eq!(
            db.paths_for_all_tags(&["people".into(), "model".into()]),
            vec![a.clone(), b.clone()]
        );

        db.replace_tag_in_paths(&[a.clone(), b.clone()], "model", "portrait");
        assert!(db
            .paths_for_all_tags(&["people".into(), "model".into()])
            .is_empty());
        assert_eq!(
            db.paths_for_all_tags(&["people".into(), "portrait".into()]),
            vec![a.clone(), b.clone()]
        );

        db.remove_tags_from_paths(std::slice::from_ref(&a), &["portrait".into()]);
        assert_eq!(db.tags_for_path(&a), vec!["people".to_string()]);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rename_path_keeps_retained_rows_of_a_vanished_file() {
        let db = TagDatabase::open_in_memory().unwrap();
        let root = temp_path("rename-stale");
        std::fs::create_dir_all(&root).unwrap();
        let old = root.join("keep.jpg");
        let new = root.join("reused-name.jpg");
        std::fs::write(&old, b"data").unwrap();
        db.add_tag(&old, "art");
        // A file that used to live at the new name left its tags behind.
        db.add_tag(&new, "nsfw");

        std::fs::rename(&old, &new).unwrap();
        assert!(db.rename_path(&old, &new).is_err());

        // Nothing is lost: both files' tags stay where they were.
        assert_eq!(db.tags_for_path(&new), vec!["nsfw".to_string()]);
        assert_eq!(db.tags_for_path(&old), vec!["art".to_string()]);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rename_path_moves_tags_and_sharpness() {
        let db = TagDatabase::open_in_memory().unwrap();
        let root = temp_path("rename-path");
        std::fs::create_dir_all(&root).unwrap();
        let old = root.join("photo.jpg");
        let new = root.join("photo.jxl");
        std::fs::write(&old, b"data").unwrap();
        std::fs::write(&new, b"data").unwrap();

        db.add_tag(&old, "portrait");
        db.upsert_sharpness(&old, 7.5, 123);

        db.rename_path(&old, &new).unwrap();

        assert!(db.tags_for_path(&old).is_empty());
        assert_eq!(db.tags_for_path(&new), vec!["portrait".to_string()]);

        let conn = db.conn.lock().unwrap();
        let new_quality: Option<(f64, i64)> = conn
            .query_row(
                "SELECT sharpness, mtime FROM image_quality WHERE path = ?1",
                params![new.to_string_lossy().as_ref()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        assert_eq!(new_quality, Some((7.5, 123)));
        let old_quality: Option<i64> = conn
            .query_row(
                "SELECT COUNT(*) FROM image_quality WHERE path = ?1",
                params![old.to_string_lossy().as_ref()],
                |row| row.get(0),
            )
            .ok();
        assert_eq!(old_quality, Some(0));

        std::fs::remove_dir_all(root).unwrap();
    }
}
