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
            ",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
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
        rows.filter_map(|r| r.ok()).map(PathBuf::from).collect()
    }

    pub fn search_paths(&self, query: &str) -> Vec<PathBuf> {
        let Ok(conn) = self.conn.lock() else {
            return vec![];
        };
        let pattern = format!("%{}%", query.to_lowercase());
        let mut stmt = match conn.prepare("SELECT DISTINCT path FROM file_tags WHERE tag LIKE ?1") {
            Ok(stmt) => stmt,
            Err(_) => return vec![],
        };
        let rows = match stmt.query_map(params![pattern], |row| row.get::<_, String>(0)) {
            Ok(rows) => rows,
            Err(_) => return vec![],
        };
        rows.filter_map(|r| r.ok()).map(PathBuf::from).collect()
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
        let tag = tag.trim().to_lowercase();
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
        let _ = conn.execute(
            "DELETE FROM file_tags WHERE path = ?1 AND tag = ?2",
            params![path.to_string_lossy().as_ref(), tag.to_lowercase()],
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
}
