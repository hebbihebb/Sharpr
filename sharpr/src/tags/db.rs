use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection};

pub struct TagDatabase {
    conn: Mutex<Connection>,
}

impl TagDatabase {
    pub fn open() -> rusqlite::Result<Self> {
        let dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("sharpr");
        std::fs::create_dir_all(&dir).ok();
        let conn = Connection::open(dir.join("tags.sqlite3"))?;
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
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Returns the stored sharpness variance for `path` if it was scored at
    /// the current file mtime, or `None` if not yet scored or the file changed.
    pub fn get_sharpness(&self, path: &Path) -> Option<f64> {
        let current_mtime = file_mtime_secs(path)?;
        let Ok(conn) = self.conn.lock() else { return None };
        let mut stmt = conn
            .prepare_cached(
                "SELECT sharpness, mtime FROM image_quality WHERE path = ?1",
            )
            .ok()?;
        let (score, stored_mtime) = stmt
            .query_row(
                rusqlite::params![path.to_string_lossy().as_ref()],
                |row| Ok((row.get::<_, f64>(0)?, row.get::<_, i64>(1)?)),
            )
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

    /// Insert auto-generated tags (format, resolution) without touching any
    /// user-created tags. Safe to call on every scan — uses INSERT OR IGNORE.
    pub fn insert_auto_tags(&self, path: &Path, tags: &[String]) {
        let Ok(conn) = self.conn.lock() else {
            return;
        };
        let path_str = path.to_string_lossy();
        for tag in tags {
            let _ = conn.execute(
                "INSERT OR IGNORE INTO file_tags (path, tag) VALUES (?1, ?2)",
                params![path_str.as_ref(), tag],
            );
        }
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
        prune_missing_paths(&conn, paths)
    }

    pub fn search_paths(&self, query: &str) -> Vec<PathBuf> {
        let terms = normalized_query_terms(query);
        if terms.is_empty() {
            return vec![];
        }
        let Ok(conn) = self.conn.lock() else {
            return vec![];
        };
        let mut matching_paths: Option<Vec<PathBuf>> = None;
        for term in &terms {
            let pattern = format!("%{term}%");
            let mut stmt =
                match conn.prepare("SELECT DISTINCT path FROM file_tags WHERE tag LIKE ?1") {
                    Ok(stmt) => stmt,
                    Err(_) => return vec![],
                };
            let rows = match stmt.query_map(params![pattern], |row| row.get::<_, String>(0)) {
                Ok(rows) => rows,
                Err(_) => return vec![],
            };
            let paths: Vec<PathBuf> = rows.filter_map(|r| r.ok()).map(PathBuf::from).collect();
            drop(stmt);
            let paths = prune_missing_paths(&conn, paths);
            matching_paths = Some(match matching_paths {
                Some(current) => current
                    .into_iter()
                    .filter(|path| paths.contains(path))
                    .collect(),
                None => paths,
            });
            if matching_paths.as_ref().is_some_and(Vec::is_empty) {
                return vec![];
            }
        }
        matching_paths.unwrap_or_default()
    }

    pub fn autocomplete(&self, prefix: &str, limit: usize) -> Vec<String> {
        let Ok(conn) = self.conn.lock() else {
            return vec![];
        };
        let pattern = format!("%{}%", prefix.to_lowercase());
        let mut stmt = match conn
            .prepare("SELECT DISTINCT tag FROM file_tags WHERE tag LIKE ?1 ORDER BY tag LIMIT ?2")
        {
            Ok(stmt) => stmt,
            Err(_) => return vec![],
        };
        let rows = match stmt.query_map(params![pattern, limit as i64], |row| {
            row.get::<_, String>(0)
        }) {
            Ok(rows) => rows,
            Err(_) => return vec![],
        };
        rows.filter_map(|r| r.ok()).collect()
    }

    pub fn all_tags(&self) -> Vec<(String, usize)> {
        let Ok(conn) = self.conn.lock() else {
            return vec![];
        };
        prune_all_missing_paths(&conn);
        let mut stmt = match conn
            .prepare("SELECT tag, COUNT(*) FROM file_tags GROUP BY tag ORDER BY tag COLLATE NOCASE")
        {
            Ok(stmt) => stmt,
            Err(_) => return vec![],
        };
        let rows = match stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
        }) {
            Ok(rows) => rows,
            Err(_) => return vec![],
        };
        rows.filter_map(|r| r.ok()).collect()
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

    pub fn remove_tag(&self, path: &Path, tag: &str) {
        let Ok(conn) = self.conn.lock() else {
            return;
        };
        let tag = normalize_tag(tag);
        if tag.is_empty() {
            return;
        }
        let _ = conn.execute(
            "DELETE FROM file_tags WHERE path = ?1 AND tag = ?2",
            params![path.to_string_lossy().as_ref(), tag],
        );
    }

    pub fn delete_tag_globally(&self, tag: &str) {
        let Ok(conn) = self.conn.lock() else {
            return;
        };
        let tag = tag.trim().to_lowercase();
        if tag.is_empty() {
            return;
        }
        let _ = conn.execute("DELETE FROM file_tags WHERE tag = ?1", params![tag]);
    }

    pub fn remove_path(&self, path: &Path) {
        let Ok(conn) = self.conn.lock() else { return };
        let _ = conn.execute(
            "DELETE FROM file_tags WHERE path = ?1",
            params![path.to_string_lossy().as_ref()],
        );
    }

    pub fn paths_for_all_tags(&self, tags: &[String]) -> Vec<PathBuf> {
        let tags = normalized_unique_tags(tags);
        if tags.is_empty() {
            return Vec::new();
        }
        let Ok(conn) = self.conn.lock() else {
            return vec![];
        };
        let placeholders = std::iter::repeat_n("?", tags.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT path
             FROM file_tags
             WHERE tag IN ({placeholders})
             GROUP BY path
             HAVING COUNT(DISTINCT tag) = ?
             ORDER BY path COLLATE NOCASE"
        );
        let mut stmt = match conn.prepare(&sql) {
            Ok(stmt) => stmt,
            Err(_) => return vec![],
        };
        let mut params_vec: Vec<&dyn rusqlite::ToSql> =
            tags.iter().map(|tag| tag as &dyn rusqlite::ToSql).collect();
        let tag_count = tags.len() as i64;
        params_vec.push(&tag_count);
        let rows = match stmt.query_map(params_vec.as_slice(), |row| row.get::<_, String>(0)) {
            Ok(rows) => rows,
            Err(_) => return vec![],
        };
        let paths: Vec<PathBuf> = rows.filter_map(|r| r.ok()).map(PathBuf::from).collect();
        drop(stmt);
        prune_missing_paths(&conn, paths)
    }

    pub fn add_tags_to_paths(&self, paths: &[PathBuf], tags: &[String]) -> usize {
        let tags = normalized_unique_tags(tags);
        if paths.is_empty() || tags.is_empty() {
            return 0;
        }
        let Ok(mut conn) = self.conn.lock() else {
            return 0;
        };
        let tx = match conn.transaction() {
            Ok(tx) => tx,
            Err(_) => return 0,
        };
        let mut changed = 0usize;
        for path in paths {
            let path_str = path.to_string_lossy();
            for tag in &tags {
                changed += tx
                    .execute(
                        "INSERT OR IGNORE INTO file_tags (path, tag) VALUES (?1, ?2)",
                        params![path_str.as_ref(), tag],
                    )
                    .unwrap_or(0);
            }
        }
        let _ = tx.commit();
        changed
    }

    pub fn remove_tags_from_paths(&self, paths: &[PathBuf], tags: &[String]) -> usize {
        let tags = normalized_unique_tags(tags);
        if paths.is_empty() || tags.is_empty() {
            return 0;
        }
        let Ok(mut conn) = self.conn.lock() else {
            return 0;
        };
        let tx = match conn.transaction() {
            Ok(tx) => tx,
            Err(_) => return 0,
        };
        let mut changed = 0usize;
        for path in paths {
            let path_str = path.to_string_lossy();
            for tag in &tags {
                changed += tx
                    .execute(
                        "DELETE FROM file_tags WHERE path = ?1 AND tag = ?2",
                        params![path_str.as_ref(), tag],
                    )
                    .unwrap_or(0);
            }
        }
        let _ = tx.commit();
        changed
    }

    pub fn replace_tag_in_paths(&self, paths: &[PathBuf], old_tag: &str, new_tag: &str) -> usize {
        let old_tag = normalize_tag(old_tag);
        let new_tag = normalize_tag(new_tag);
        if paths.is_empty() || old_tag.is_empty() || new_tag.is_empty() || old_tag == new_tag {
            return 0;
        }
        let Ok(mut conn) = self.conn.lock() else {
            return 0;
        };
        let tx = match conn.transaction() {
            Ok(tx) => tx,
            Err(_) => return 0,
        };
        let mut changed = 0usize;
        for path in paths {
            let path_str = path.to_string_lossy();
            let removed = tx
                .execute(
                    "DELETE FROM file_tags WHERE path = ?1 AND tag = ?2",
                    params![path_str.as_ref(), old_tag],
                )
                .unwrap_or(0);
            if removed > 0 {
                let _ = tx.execute(
                    "INSERT OR IGNORE INTO file_tags (path, tag) VALUES (?1, ?2)",
                    params![path_str.as_ref(), new_tag],
                );
                changed += removed;
            }
        }
        let _ = tx.commit();
        changed
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

fn prune_missing_paths(conn: &Connection, paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut existing = Vec::with_capacity(paths.len());
    for path in paths {
        if path.exists() {
            existing.push(path);
        } else {
            let _ = conn.execute(
                "DELETE FROM file_tags WHERE path = ?1",
                params![path.to_string_lossy().as_ref()],
            );
        }
    }
    existing
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

fn prune_all_missing_paths(conn: &Connection) {
    let mut stmt = match conn.prepare("SELECT DISTINCT path FROM file_tags") {
        Ok(stmt) => stmt,
        Err(_) => return,
    };
    let rows = match stmt.query_map([], |row| row.get::<_, String>(0)) {
        Ok(rows) => rows,
        Err(_) => return,
    };
    let paths: Vec<String> = rows.filter_map(Result::ok).collect();
    drop(stmt);
    for path in paths {
        if !Path::new(&path).exists() {
            let _ = conn.execute("DELETE FROM file_tags WHERE path = ?1", params![path]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn tag_queries_prune_missing_paths() {
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

        std::fs::remove_file(&existing).unwrap();
        assert!(db.paths_for_tag("cow").is_empty());
        assert!(db.all_tags().is_empty());

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
}
