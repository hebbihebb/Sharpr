use std::borrow::Cow;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection, OptionalExtension};

use crate::model::library::SortOrder;
use crate::quality::QualityClass;

const SCHEMA_VERSION: &str = "2";

#[derive(Clone, Debug)]
pub struct Collection {
    pub id: i64,
    pub name: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub item_count: usize,
}

#[derive(Clone, Debug)]
pub struct BasicImageInfo {
    pub path: PathBuf,
    pub folder_path: PathBuf,
    pub filename: String,
    pub extension: String,
    pub file_size: u64,
    pub modified_secs: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct IndexedImage {
    pub path: PathBuf,
    pub filename: String,
    pub extension: String,
    pub file_size: u64,
    pub modified_secs: Option<i64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub metadata_status: String,
}

pub struct LibraryIndex {
    pool: Pool<SqliteConnectionManager>,
}

impl LibraryIndex {
    pub fn open() -> rusqlite::Result<Self> {
        let started = std::time::Instant::now();
        let dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("sharpr");
        std::fs::create_dir_all(&dir).ok();
        let manager = SqliteConnectionManager::file(dir.join("library-index.sqlite"))
            .with_init(|conn| {
                configure_connection(conn)?;
                Ok(())
            });
        let pool = Pool::new(manager).map_err(|_| rusqlite::Error::InvalidQuery)?;
        {
            let conn = pool.get().map_err(|_| rusqlite::Error::InvalidQuery)?;
            initialize_schema(&conn)?;
        }
        crate::bench_event!(
            "index.open",
            serde_json::json!({
                "duration_ms": crate::bench::duration_ms(started),
            }),
        );
        Ok(Self { pool })
    }

    pub fn upsert_folder(&self, path: &Path) -> rusqlite::Result<()> {
        let now = now_secs();
        let conn = self.conn()?;
        conn.execute(
            "
            INSERT INTO folders (path, ignored, discovered_at, updated_at)
            VALUES (?1, 0, ?2, ?2)
            ON CONFLICT(path) DO UPDATE SET updated_at = excluded.updated_at
            ",
            params![path_to_string(path), now],
        )?;
        Ok(())
    }

    pub fn set_folder_ignored(&self, path: &Path, ignored: bool) -> rusqlite::Result<()> {
        let now = now_secs();
        let conn = self.conn()?;
        conn.execute(
            "
            INSERT INTO folders (path, ignored, discovered_at, updated_at)
            VALUES (?1, ?2, ?3, ?3)
            ON CONFLICT(path) DO UPDATE SET
                ignored = excluded.ignored,
                updated_at = excluded.updated_at
            ",
            params![path_to_string(path), ignored as i64, now],
        )?;
        Ok(())
    }

    pub fn ignored_folders(&self) -> rusqlite::Result<Vec<PathBuf>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT path FROM folders WHERE ignored = 1 ORDER BY path")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows.filter_map(Result::ok).map(PathBuf::from).collect())
    }

    /// Reconcile `folder` against `entries` in a single SQLite transaction.
    /// Upserts all entries (preserving metadata/phash for unchanged files, invalidating for
    /// changed ones), removes rows whose paths are no longer present, then returns the final
    /// sorted rows, the stale-removal count, and the list of images still needing metadata.
    pub fn reconcile_folder(
        &self,
        folder: &Path,
        entries: &[BasicImageInfo],
        sort_order: SortOrder,
    ) -> rusqlite::Result<(Vec<IndexedImage>, usize, Vec<BasicImageInfo>)> {
        let now = now_secs();
        let mut conn = self.conn()?;

        let stale_count: usize;
        {
            let tx = conn.transaction()?;

            tx.execute(
                "INSERT INTO folders (path, ignored, discovered_at, updated_at)
                 VALUES (?1, 0, ?2, ?2)
                 ON CONFLICT(path) DO UPDATE SET updated_at = excluded.updated_at",
                params![path_to_string(folder), now],
            )?;

            for info in entries {
                tx.execute(
                    "INSERT INTO images (
                         path, folder_path, filename, extension, file_size, modified_secs,
                         width, height, quality_class, phash, phash_status, metadata_status,
                         indexed_at, error
                     )
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, NULL, NULL,
                             'missing', 'missing', ?7, NULL)
                     ON CONFLICT(path) DO UPDATE SET
                         folder_path      = excluded.folder_path,
                         filename         = excluded.filename,
                         extension        = excluded.extension,
                         file_size        = excluded.file_size,
                         modified_secs    = excluded.modified_secs,
                         indexed_at       = excluded.indexed_at,
                         error            = NULL,
                         width = CASE WHEN images.file_size = excluded.file_size
                                           AND images.modified_secs IS excluded.modified_secs
                                      THEN images.width        ELSE NULL END,
                         height = CASE WHEN images.file_size = excluded.file_size
                                            AND images.modified_secs IS excluded.modified_secs
                                       THEN images.height       ELSE NULL END,
                         quality_class = CASE WHEN images.file_size = excluded.file_size
                                                   AND images.modified_secs IS excluded.modified_secs
                                              THEN images.quality_class ELSE NULL END,
                         phash = CASE WHEN images.file_size = excluded.file_size
                                           AND images.modified_secs IS excluded.modified_secs
                                      THEN images.phash        ELSE NULL END,
                         phash_status = CASE WHEN images.file_size = excluded.file_size
                                                  AND images.modified_secs IS excluded.modified_secs
                                             THEN images.phash_status ELSE 'stale' END,
                         metadata_status = CASE WHEN images.file_size = excluded.file_size
                                                     AND images.modified_secs IS excluded.modified_secs
                                                THEN images.metadata_status ELSE 'missing' END",
                    params![
                        path_to_string(&info.path),
                        path_to_string(&info.folder_path),
                        info.filename,
                        info.extension,
                        info.file_size as i64,
                        info.modified_secs,
                        now,
                    ],
                )?;
            }

            let current_set: HashSet<String> =
                entries.iter().map(|e| path_to_string(&e.path)).collect();
            let db_paths: Vec<String> = {
                let mut stmt =
                    tx.prepare("SELECT path FROM images WHERE folder_path = ?1")?;
                let rows = stmt.query_map(params![path_to_string(folder)], |row| {
                    row.get::<_, String>(0)
                })?
                .filter_map(Result::ok)
                .collect();
                rows
            };
            let stale: Vec<_> = db_paths
                .iter()
                .filter(|p| !current_set.contains(*p))
                .collect();
            stale_count = stale.len();
            for path in stale {
                tx.execute("DELETE FROM images WHERE path = ?1", params![path])?;
            }

            tx.commit()?;
        }

        let order = match sort_order {
            SortOrder::Name => "filename COLLATE NOCASE ASC",
            SortOrder::DateModified => {
                "modified_secs DESC NULLS LAST, filename COLLATE NOCASE ASC"
            }
            SortOrder::FileType => {
                "extension COLLATE NOCASE ASC, filename COLLATE NOCASE ASC"
            }
        };
        let rows: Vec<IndexedImage> = {
            let sql = format!(
                "SELECT path, filename, extension, file_size, modified_secs,
                        width, height, metadata_status
                 FROM images WHERE folder_path = ?1 ORDER BY {order}"
            );
            let mut stmt = conn.prepare(&sql)?;
            let collected = stmt
                .query_map(params![path_to_string(folder)], indexed_image_from_row)?
                .filter_map(Result::ok)
                .collect();
            collected
        };
        let metadata_pending: Vec<BasicImageInfo> = {
            let mut stmt = conn.prepare(
                "SELECT path, folder_path, filename, extension, file_size, modified_secs
                 FROM images
                 WHERE folder_path = ?1
                   AND metadata_status IN ('missing', 'stale', 'failed')
                 ORDER BY filename COLLATE NOCASE",
            )?;
            let collected = stmt
                .query_map(params![path_to_string(folder)], basic_info_from_row)?
                .filter_map(Result::ok)
                .collect();
            collected
        };

        Ok((rows, stale_count, metadata_pending))
    }

    pub fn upsert_image_basic(&self, info: &BasicImageInfo) -> rusqlite::Result<()> {
        let now = now_secs();
        let conn = self.conn()?;
        let existing = conn
            .query_row(
                "SELECT file_size, modified_secs FROM images WHERE path = ?1",
                params![path_to_string(&info.path)],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?)),
            )
            .optional()?;
        let unchanged = existing
            .as_ref()
            .map(|(size, modified)| {
                *size == info.file_size as i64 && *modified == info.modified_secs
            })
            .unwrap_or(false);
        let (metadata_status, phash_status): (Cow<'static, str>, Cow<'static, str>) =
            match (existing, unchanged) {
            (Some(_), true) => (
                current_status(&conn, &info.path, "metadata_status")?
                    .map(Cow::Owned)
                    .unwrap_or(Cow::Borrowed("missing")),
                current_status(&conn, &info.path, "phash_status")?
                    .map(Cow::Owned)
                    .unwrap_or(Cow::Borrowed("missing")),
            ),
            (Some(_), false) => (Cow::Borrowed("missing"), Cow::Borrowed("stale")),
            (None, _) => (Cow::Borrowed("missing"), Cow::Borrowed("missing")),
        };
        conn.execute(
            "
            INSERT INTO images (
                path, folder_path, filename, extension, file_size, modified_secs,
                width, height, quality_class, phash, phash_status, metadata_status,
                indexed_at, error
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, NULL, NULL, ?7, ?8, ?9, NULL)
            ON CONFLICT(path) DO UPDATE SET
                folder_path = excluded.folder_path,
                filename = excluded.filename,
                extension = excluded.extension,
                file_size = excluded.file_size,
                modified_secs = excluded.modified_secs,
                width = CASE WHEN ?10 THEN images.width ELSE NULL END,
                height = CASE WHEN ?10 THEN images.height ELSE NULL END,
                quality_class = CASE WHEN ?10 THEN images.quality_class ELSE NULL END,
                phash = CASE WHEN ?10 THEN images.phash ELSE NULL END,
                phash_status = excluded.phash_status,
                metadata_status = excluded.metadata_status,
                indexed_at = excluded.indexed_at,
                error = NULL
            ",
            params![
                path_to_string(&info.path),
                path_to_string(&info.folder_path),
                info.filename,
                info.extension,
                info.file_size as i64,
                info.modified_secs,
                phash_status.as_ref(),
                metadata_status.as_ref(),
                now,
                unchanged,
            ],
        )?;
        Ok(())
    }

    pub fn update_image_metadata(
        &self,
        path: &Path,
        width: u32,
        height: u32,
        quality_class: QualityClass,
    ) -> rusqlite::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "
            UPDATE images
            SET width = ?2,
                height = ?3,
                quality_class = ?4,
                metadata_status = 'ready',
                error = NULL
            WHERE path = ?1
            ",
            params![
                path_to_string(path),
                width as i64,
                height as i64,
                quality_class.label(),
            ],
        )?;
        Ok(())
    }

    pub fn update_image_phash(&self, path: &Path, hash: u64) -> rusqlite::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE images SET phash = ?2, phash_status = 'ready', error = NULL WHERE path = ?1",
            params![path_to_string(path), hash as i64],
        )?;
        Ok(())
    }

    pub fn mark_image_error(&self, path: &Path, error: &str) -> rusqlite::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "
            UPDATE images
            SET metadata_status = 'failed',
                error = ?2
            WHERE path = ?1
            ",
            params![path_to_string(path), error],
        )?;
        Ok(())
    }

    pub fn remove_missing_in_folder(
        &self,
        folder: &Path,
        current_paths: &[PathBuf],
    ) -> rusqlite::Result<usize> {
        let current: HashSet<String> = current_paths
            .iter()
            .map(|path| path_to_string(path))
            .collect();
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT path FROM images WHERE folder_path = ?1")?;
        let rows = stmt.query_map(params![path_to_string(folder)], |row| {
            row.get::<_, String>(0)
        })?;
        let stale: Vec<String> = rows
            .filter_map(Result::ok)
            .filter(|path| !current.contains(path))
            .collect();
        drop(stmt);
        for path in &stale {
            conn.execute("DELETE FROM images WHERE path = ?1", params![path])?;
        }
        Ok(stale.len())
    }

    pub fn images_in_folder(
        &self,
        folder: &Path,
        sort_order: SortOrder,
    ) -> rusqlite::Result<Vec<IndexedImage>> {
        let order = match sort_order {
            SortOrder::Name => "filename COLLATE NOCASE ASC",
            SortOrder::DateModified => "modified_secs DESC NULLS LAST, filename COLLATE NOCASE ASC",
            SortOrder::FileType => "extension COLLATE NOCASE ASC, filename COLLATE NOCASE ASC",
        };
        let sql = format!(
            "
            SELECT path, filename, extension, file_size, modified_secs, width, height, metadata_status
            FROM images
            WHERE folder_path = ?1
            ORDER BY {order}
            "
        );
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![path_to_string(folder)], indexed_image_from_row)?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn images_by_quality(&self, class: QualityClass) -> rusqlite::Result<Vec<PathBuf>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "
            SELECT images.path
            FROM images
            LEFT JOIN folders ON folders.path = images.folder_path
            WHERE images.quality_class = ?1
              AND COALESCE(folders.ignored, 0) = 0
            ORDER BY images.path COLLATE NOCASE
            ",
        )?;
        let rows = stmt.query_map(params![class.label()], |row| row.get::<_, String>(0))?;
        Ok(rows.filter_map(Result::ok).map(PathBuf::from).collect())
    }

    pub fn images_with_phash(&self) -> rusqlite::Result<Vec<(PathBuf, u64)>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "
            SELECT path, phash
            FROM images
            WHERE phash_status = 'ready' AND phash IS NOT NULL
            ORDER BY path COLLATE NOCASE
            ",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                PathBuf::from(row.get::<_, String>(0)?),
                row.get::<_, i64>(1)? as u64,
            ))
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn duplicate_groups(&self) -> rusqlite::Result<Vec<Vec<PathBuf>>> {
        Ok(crate::duplicates::phash::group_duplicates(
            &self.images_with_phash()?,
        ))
    }

    pub fn all_indexed_paths(&self) -> rusqlite::Result<Vec<PathBuf>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT path FROM images ORDER BY path COLLATE NOCASE")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows.filter_map(Result::ok).map(PathBuf::from).collect())
    }

    pub fn images_needing_metadata(&self, folder: &Path) -> rusqlite::Result<Vec<BasicImageInfo>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "
            SELECT path, folder_path, filename, extension, file_size, modified_secs
            FROM images
            WHERE folder_path = ?1
              AND metadata_status IN ('missing', 'stale', 'failed')
            ORDER BY filename COLLATE NOCASE
            ",
        )?;
        let rows = stmt.query_map(params![path_to_string(folder)], basic_info_from_row)?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn list_collections(&self) -> rusqlite::Result<Vec<Collection>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT c.id, c.name, c.created_at, c.updated_at,
                    COUNT(ci.image_path) AS item_count
             FROM collections c
             LEFT JOIN collection_items ci ON ci.collection_id = c.id
             GROUP BY c.id
             ORDER BY c.name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Collection {
                id: row.get(0)?,
                name: row.get(1)?,
                created_at: row.get(2)?,
                updated_at: row.get(3)?,
                item_count: row.get::<_, i64>(4)? as usize,
            })
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn create_collection(&self, name: &str) -> rusqlite::Result<Collection> {
        let name = name.trim();
        if name.is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "collection name cannot be empty".into(),
            ));
        }
        let now = now_secs();
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO collections (name, created_at, updated_at) VALUES (?1, ?2, ?2)",
            params![name, now],
        )?;
        let id = conn.last_insert_rowid();
        Ok(Collection {
            id,
            name: name.to_string(),
            created_at: now,
            updated_at: now,
            item_count: 0,
        })
    }

    pub fn rename_collection(&self, id: i64, name: &str) -> rusqlite::Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "collection name cannot be empty".into(),
            ));
        }
        let now = now_secs();
        let conn = self.conn()?;
        conn.execute(
            "UPDATE collections SET name = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, name, now],
        )?;
        Ok(())
    }

    pub fn delete_collection(&self, id: i64) -> rusqlite::Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM collections WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn add_paths_to_collection(
        &self,
        collection_id: i64,
        paths: &[PathBuf],
    ) -> rusqlite::Result<usize> {
        if paths.is_empty() {
            return Ok(0);
        }
        let now = now_secs();
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let mut added = 0usize;
        for path in paths {
            let n = tx.execute(
                "INSERT OR IGNORE INTO collection_items (collection_id, image_path, added_at)
                 VALUES (?1, ?2, ?3)",
                params![collection_id, path_to_string(path), now],
            )?;
            added += n;
        }
        if added > 0 {
            tx.execute(
                "UPDATE collections SET updated_at = ?2 WHERE id = ?1",
                params![collection_id, now],
            )?;
        }
        tx.commit()?;
        Ok(added)
    }

    pub fn remove_paths_from_collection(
        &self,
        collection_id: i64,
        paths: &[PathBuf],
    ) -> rusqlite::Result<usize> {
        if paths.is_empty() {
            return Ok(0);
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let mut removed = 0usize;
        for path in paths {
            let n = tx.execute(
                "DELETE FROM collection_items
                 WHERE collection_id = ?1 AND image_path = ?2",
                params![collection_id, path_to_string(path)],
            )?;
            removed += n;
        }
        if removed > 0 {
            let now = now_secs();
            tx.execute(
                "UPDATE collections SET updated_at = ?2 WHERE id = ?1",
                params![collection_id, now],
            )?;
        }
        tx.commit()?;
        Ok(removed)
    }

    pub fn collection_paths(&self, collection_id: i64) -> rusqlite::Result<Vec<PathBuf>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT image_path FROM collection_items
             WHERE collection_id = ?1
             ORDER BY added_at ASC",
        )?;
        let rows = stmt.query_map(params![collection_id], |row| row.get::<_, String>(0))?;
        Ok(rows.filter_map(Result::ok).map(PathBuf::from).collect())
    }

    fn conn(&self) -> rusqlite::Result<PooledConnection<SqliteConnectionManager>> {
        self.pool.get().map_err(|_| rusqlite::Error::InvalidQuery)
    }
}

pub fn basic_info_from_path(folder: &Path, path: PathBuf) -> BasicImageInfo {
    let filename = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let meta = std::fs::metadata(&path).ok();
    let file_size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let modified_secs = meta
        .and_then(|m| m.modified().ok())
        .and_then(system_time_to_secs);
    BasicImageInfo {
        path,
        folder_path: folder.to_path_buf(),
        filename,
        extension,
        file_size,
        modified_secs,
    }
}

fn indexed_image_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<IndexedImage> {
    Ok(IndexedImage {
        path: PathBuf::from(row.get::<_, String>(0)?),
        filename: row.get(1)?,
        extension: row.get(2)?,
        file_size: row.get::<_, i64>(3)? as u64,
        modified_secs: row.get(4)?,
        width: row.get::<_, Option<i64>>(5)?.map(|v| v as u32),
        height: row.get::<_, Option<i64>>(6)?.map(|v| v as u32),
        metadata_status: row.get(7)?,
    })
}

fn basic_info_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<BasicImageInfo> {
    Ok(BasicImageInfo {
        path: PathBuf::from(row.get::<_, String>(0)?),
        folder_path: PathBuf::from(row.get::<_, String>(1)?),
        filename: row.get(2)?,
        extension: row.get(3)?,
        file_size: row.get::<_, i64>(4)? as u64,
        modified_secs: row.get(5)?,
    })
}

fn current_status(
    conn: &Connection,
    path: &Path,
    column: &str,
) -> rusqlite::Result<Option<String>> {
    let sql = format!("SELECT {column} FROM images WHERE path = ?1");
    conn.query_row(&sql, params![path_to_string(path)], |row| row.get(0))
        .optional()
}

fn now_secs() -> i64 {
    system_time_to_secs(SystemTime::now()).unwrap_or(0)
}

fn system_time_to_secs(time: SystemTime) -> Option<i64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn configure_connection(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}

fn initialize_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS folders (
            path TEXT PRIMARY KEY,
            ignored INTEGER NOT NULL DEFAULT 0,
            discovered_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS images (
            path TEXT PRIMARY KEY,
            folder_path TEXT NOT NULL,
            filename TEXT NOT NULL,
            extension TEXT NOT NULL,
            file_size INTEGER NOT NULL,
            modified_secs INTEGER,
            width INTEGER,
            height INTEGER,
            quality_class TEXT,
            phash INTEGER,
            phash_status TEXT NOT NULL DEFAULT 'missing',
            metadata_status TEXT NOT NULL DEFAULT 'missing',
            indexed_at INTEGER NOT NULL,
            error TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_images_folder ON images(folder_path);
        CREATE INDEX IF NOT EXISTS idx_images_quality ON images(quality_class);
        CREATE INDEX IF NOT EXISTS idx_images_phash ON images(phash);

        CREATE TABLE IF NOT EXISTS schema_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS collections (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS collection_items (
            collection_id INTEGER NOT NULL,
            image_path TEXT NOT NULL,
            added_at INTEGER NOT NULL,
            PRIMARY KEY (collection_id, image_path),
            FOREIGN KEY(collection_id) REFERENCES collections(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_collection_items_path
            ON collection_items(image_path);
        CREATE INDEX IF NOT EXISTS idx_collection_items_collection_added
            ON collection_items(collection_id, added_at);
        ",
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('schema_version', ?1)",
        params![SCHEMA_VERSION],
    )?;
    Ok(())
}

#[cfg(test)]
impl LibraryIndex {
    fn open_in_memory() -> rusqlite::Result<Self> {
        let manager = SqliteConnectionManager::memory().with_init(|conn| {
            configure_connection(conn)?;
            initialize_schema(conn)?;
            Ok(())
        });
        let pool = Pool::builder()
            .max_size(1)
            .build(manager)
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        Ok(Self { pool })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_info(folder: &str, name: &str, size: u64, mtime: Option<i64>) -> BasicImageInfo {
        let folder = PathBuf::from(folder);
        let path = folder.join(name);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        BasicImageInfo {
            path,
            folder_path: folder,
            filename: name.to_string(),
            extension: ext,
            file_size: size,
            modified_secs: mtime,
        }
    }

    #[test]
    fn upsert_new_image_creates_row() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let info = make_info("/photos", "a.jpg", 1000, Some(100));
        idx.upsert_image_basic(&info).unwrap();
        let rows = idx
            .images_in_folder(Path::new("/photos"), SortOrder::Name)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].filename, "a.jpg");
        assert_eq!(rows[0].file_size, 1000);
        assert_eq!(rows[0].metadata_status, "missing");
    }

    #[test]
    fn unchanged_file_preserves_metadata_and_phash_status() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let info = make_info("/photos", "a.jpg", 1000, Some(100));
        idx.upsert_image_basic(&info).unwrap();
        idx.update_image_metadata(Path::new("/photos/a.jpg"), 1920, 1080, QualityClass::Good)
            .unwrap();
        idx.update_image_phash(Path::new("/photos/a.jpg"), 0xdeadbeef)
            .unwrap();

        // Upsert same file again — size and mtime unchanged.
        idx.upsert_image_basic(&info).unwrap();
        let rows = idx
            .images_in_folder(Path::new("/photos"), SortOrder::Name)
            .unwrap();
        assert_eq!(rows[0].metadata_status, "ready");
        assert_eq!(rows[0].width, Some(1920));
        assert_eq!(rows[0].height, Some(1080));
    }

    #[test]
    fn changed_file_invalidates_metadata_and_phash() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let info = make_info("/photos", "a.jpg", 1000, Some(100));
        idx.upsert_image_basic(&info).unwrap();
        idx.update_image_metadata(Path::new("/photos/a.jpg"), 1920, 1080, QualityClass::Good)
            .unwrap();
        idx.update_image_phash(Path::new("/photos/a.jpg"), 0xdeadbeef)
            .unwrap();

        // Upsert with different size — file changed.
        let changed = make_info("/photos", "a.jpg", 2000, Some(200));
        idx.upsert_image_basic(&changed).unwrap();
        let rows = idx
            .images_in_folder(Path::new("/photos"), SortOrder::Name)
            .unwrap();
        assert_eq!(rows[0].metadata_status, "missing");
        assert_eq!(rows[0].width, None);
        assert_eq!(rows[0].height, None);
    }

    #[test]
    fn stale_row_removed_by_remove_missing_in_folder() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        idx.upsert_image_basic(&make_info("/photos", "a.jpg", 100, None))
            .unwrap();
        idx.upsert_image_basic(&make_info("/photos", "b.jpg", 200, None))
            .unwrap();
        let keep = vec![PathBuf::from("/photos/a.jpg")];
        let removed = idx
            .remove_missing_in_folder(Path::new("/photos"), &keep)
            .unwrap();
        assert_eq!(removed, 1);
        let rows = idx
            .images_in_folder(Path::new("/photos"), SortOrder::Name)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].filename, "a.jpg");
    }

    #[test]
    fn reconcile_folder_batch_upsert_and_stale_removal() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        // Pre-populate with two images.
        idx.upsert_image_basic(&make_info("/photos", "old.jpg", 100, None))
            .unwrap();
        idx.upsert_image_basic(&make_info("/photos", "keep.jpg", 200, None))
            .unwrap();
        idx.update_image_metadata(
            Path::new("/photos/keep.jpg"),
            800,
            600,
            QualityClass::Good,
        )
        .unwrap();

        // Reconcile with keep.jpg (unchanged) and new.jpg; old.jpg is gone.
        let entries = vec![
            make_info("/photos", "keep.jpg", 200, None),
            make_info("/photos", "new.jpg", 300, None),
        ];
        let (rows, stale, pending) = idx
            .reconcile_folder(Path::new("/photos"), &entries, SortOrder::Name)
            .unwrap();

        assert_eq!(stale, 1, "old.jpg should be removed");
        assert_eq!(rows.len(), 2);
        // keep.jpg metadata should be preserved (unchanged file).
        let keep_row = rows.iter().find(|r| r.filename == "keep.jpg").unwrap();
        assert_eq!(keep_row.metadata_status, "ready");
        assert_eq!(keep_row.width, Some(800));
        // new.jpg needs metadata.
        assert!(pending.iter().any(|p| p.filename == "new.jpg"));
    }

    #[test]
    fn images_by_quality_filters_correctly() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        idx.upsert_folder(Path::new("/photos")).unwrap();
        idx.upsert_image_basic(&make_info("/photos", "good.jpg", 100, None))
            .unwrap();
        idx.upsert_image_basic(&make_info("/photos", "small.jpg", 50, None))
            .unwrap();
        idx.update_image_metadata(
            Path::new("/photos/good.jpg"),
            1920,
            1080,
            QualityClass::Good,
        )
        .unwrap();
        idx.update_image_metadata(
            Path::new("/photos/small.jpg"),
            200,
            150,
            QualityClass::NeedsUpscale,
        )
        .unwrap();

        let good = idx.images_by_quality(QualityClass::Good).unwrap();
        assert_eq!(good, vec![PathBuf::from("/photos/good.jpg")]);

        let upscale = idx.images_by_quality(QualityClass::NeedsUpscale).unwrap();
        assert_eq!(upscale, vec![PathBuf::from("/photos/small.jpg")]);
    }

    #[test]
    fn images_by_quality_excludes_ignored_folders() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        idx.upsert_image_basic(&make_info("/ignored", "a.jpg", 100, None))
            .unwrap();
        idx.update_image_metadata(
            Path::new("/ignored/a.jpg"),
            1920,
            1080,
            QualityClass::Good,
        )
        .unwrap();
        idx.set_folder_ignored(Path::new("/ignored"), true).unwrap();

        let good = idx.images_by_quality(QualityClass::Good).unwrap();
        assert!(good.is_empty(), "ignored folder images should be excluded");
    }

    #[test]
    fn update_image_phash_persists_and_is_queryable() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        idx.upsert_image_basic(&make_info("/photos", "a.jpg", 100, None))
            .unwrap();
        idx.update_image_phash(Path::new("/photos/a.jpg"), 0xabcd1234)
            .unwrap();
        let hashes = idx.images_with_phash().unwrap();
        assert_eq!(hashes.len(), 1);
        assert_eq!(hashes[0].0, PathBuf::from("/photos/a.jpg"));
        assert_eq!(hashes[0].1, 0xabcd1234u64);
    }

    #[test]
    fn images_in_folder_sort_orders() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let mut b = make_info("/photos", "b.jpg", 100, Some(200));
        let mut a = make_info("/photos", "a.jpg", 100, Some(100));
        let mut c = make_info("/photos", "c.png", 100, Some(300));
        // Give a more recent mtime than b to test DateModified ordering.
        a.modified_secs = Some(300);
        b.modified_secs = Some(200);
        c.modified_secs = Some(100);
        idx.upsert_image_basic(&b).unwrap();
        idx.upsert_image_basic(&a).unwrap();
        idx.upsert_image_basic(&c).unwrap();

        let by_name = idx
            .images_in_folder(Path::new("/photos"), SortOrder::Name)
            .unwrap();
        assert_eq!(
            by_name.iter().map(|r| r.filename.as_str()).collect::<Vec<_>>(),
            vec!["a.jpg", "b.jpg", "c.png"]
        );

        let by_date = idx
            .images_in_folder(Path::new("/photos"), SortOrder::DateModified)
            .unwrap();
        // Descending mtime: a(300), b(200), c(100).
        assert_eq!(by_date[0].filename, "a.jpg");
        assert_eq!(by_date[2].filename, "c.png");

        let by_type = idx
            .images_in_folder(Path::new("/photos"), SortOrder::FileType)
            .unwrap();
        // jpg before png.
        assert_eq!(by_type[0].extension, "jpg");
        assert_eq!(by_type[2].extension, "png");
    }

    #[test]
    fn create_and_list_collections() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let c = idx.create_collection("Pinned").unwrap();
        assert_eq!(c.name, "Pinned");
        assert_eq!(c.item_count, 0);

        let list = idx.list_collections().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Pinned");
        assert_eq!(list[0].item_count, 0);
    }

    #[test]
    fn create_collection_rejects_empty_name() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        assert!(idx.create_collection("").is_err());
        assert!(idx.create_collection("   ").is_err());
    }

    #[test]
    fn create_collection_rejects_duplicate_name() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        idx.create_collection("Pinned").unwrap();
        assert!(idx.create_collection("Pinned").is_err());
    }

    #[test]
    fn rename_collection() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let c = idx.create_collection("Old").unwrap();
        idx.rename_collection(c.id, "New").unwrap();
        let list = idx.list_collections().unwrap();
        assert_eq!(list[0].name, "New");
    }

    #[test]
    fn rename_collection_rejects_empty_name() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let c = idx.create_collection("Name").unwrap();
        assert!(idx.rename_collection(c.id, "").is_err());
    }

    #[test]
    fn delete_collection_cascades_items() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let c = idx.create_collection("Pinned").unwrap();
        let paths = vec![PathBuf::from("/photos/a.jpg")];
        idx.add_paths_to_collection(c.id, &paths).unwrap();

        idx.delete_collection(c.id).unwrap();
        assert!(idx.list_collections().unwrap().is_empty());
        // Collection items should be gone via CASCADE.
        assert!(idx.collection_paths(c.id).unwrap().is_empty());
    }

    #[test]
    fn add_paths_idempotent() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let c = idx.create_collection("Pinned").unwrap();
        let paths = vec![
            PathBuf::from("/photos/a.jpg"),
            PathBuf::from("/photos/b.jpg"),
        ];
        let added = idx.add_paths_to_collection(c.id, &paths).unwrap();
        assert_eq!(added, 2);

        // Add again — should be no-ops.
        let added2 = idx.add_paths_to_collection(c.id, &paths).unwrap();
        assert_eq!(added2, 0);

        let list = idx.list_collections().unwrap();
        assert_eq!(list[0].item_count, 2);
    }

    #[test]
    fn remove_paths_from_collection() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let c = idx.create_collection("Batch").unwrap();
        let paths = vec![
            PathBuf::from("/photos/a.jpg"),
            PathBuf::from("/photos/b.jpg"),
            PathBuf::from("/photos/c.jpg"),
        ];
        idx.add_paths_to_collection(c.id, &paths).unwrap();

        let removed = idx
            .remove_paths_from_collection(c.id, &[PathBuf::from("/photos/b.jpg")])
            .unwrap();
        assert_eq!(removed, 1);

        let remaining = idx.collection_paths(c.id).unwrap();
        assert_eq!(remaining.len(), 2);
        assert!(!remaining.contains(&PathBuf::from("/photos/b.jpg")));
    }

    #[test]
    fn collection_paths_ordered_by_added_at() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let c = idx.create_collection("Order Test").unwrap();
        // Add one at a time so added_at timestamps differ (same second is fine; ordering is by
        // insertion order since we use now_secs() once per batch call).
        idx.add_paths_to_collection(c.id, &[PathBuf::from("/photos/a.jpg")])
            .unwrap();
        idx.add_paths_to_collection(c.id, &[PathBuf::from("/photos/b.jpg")])
            .unwrap();
        idx.add_paths_to_collection(c.id, &[PathBuf::from("/photos/c.jpg")])
            .unwrap();

        let paths = idx.collection_paths(c.id).unwrap();
        assert_eq!(paths.len(), 3);
        // All three paths are present (order within same second is not guaranteed).
        assert!(paths.contains(&PathBuf::from("/photos/a.jpg")));
        assert!(paths.contains(&PathBuf::from("/photos/b.jpg")));
        assert!(paths.contains(&PathBuf::from("/photos/c.jpg")));
    }
}
