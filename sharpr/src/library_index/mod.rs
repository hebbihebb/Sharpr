use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection, OptionalExtension};

use crate::model::library::SortOrder;
use crate::quality::QualityClass;

mod schema;
use schema::initialize_schema;

const SCHEMA_VERSION: &str = "3";
/// Version recorded in `PRAGMA user_version`; see `crate::db_version`. The
/// older `schema_meta.schema_version` value is still written but never read.
const USER_VERSION: i64 = 1;

#[derive(Clone, Debug)]
pub struct Collection {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub primary_tag: String,
    pub extra_tags: Vec<String>,
    pub color: Option<String>,
    pub icon_name: Option<String>,
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
    dimensions: std::sync::Mutex<crate::model::browse::DimensionCache>,
    /// Duplicate groups with a fingerprint of the hash list they came from.
    duplicate_groups: std::sync::Mutex<Option<(u64, Vec<Vec<PathBuf>>)>>,
}

fn sort_indexed_aspects(
    rows: &mut [IndexedImage],
    order: SortOrder,
    cache: &std::sync::Mutex<crate::model::browse::DimensionCache>,
) {
    if !matches!(
        order.field(),
        crate::model::library::SortField::Aspect | crate::model::library::SortField::WideScreen
    ) {
        return;
    }
    let mut cache = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for row in rows.iter_mut() {
        let dimensions = std::fs::metadata(&row.path)
            .ok()
            .and_then(|metadata| cache.get(&row.path, &metadata));
        row.width = dimensions.map(|d| d.0);
        row.height = dimensions.map(|d| d.1);
    }
    rows.sort_by(|a, b| {
        crate::model::browse::compare_aspects(
            a.width.zip(a.height),
            b.width.zip(b.height),
            order.field() == crate::model::library::SortField::WideScreen,
            order.descending(),
        )
        .then_with(|| a.path.cmp(&b.path))
    });
}

impl LibraryIndex {
    pub fn open() -> rusqlite::Result<(Self, usize)> {
        let dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("sharpr");
        std::fs::create_dir_all(&dir).ok();
        Self::open_at(&dir.join("library-index.sqlite"))
    }

    fn open_at(path: &Path) -> rusqlite::Result<(Self, usize)> {
        let started = std::time::Instant::now();
        crate::db_version::ensure_not_newer(path, USER_VERSION, "library index")?;
        let manager = SqliteConnectionManager::file(path).with_init(|conn| {
            configure_connection(conn)?;
            Ok(())
        });
        let pool = Pool::new(manager).map_err(|_| rusqlite::Error::InvalidQuery)?;
        let interrupted_count = {
            let conn = pool.get().map_err(|_| rusqlite::Error::InvalidQuery)?;
            initialize_schema(&conn)?;
            invalidate_legacy_quality_labels(&conn)?;
            ensure_collection_schema(&conn)?;
            crate::db_version::stamp(&conn, USER_VERSION)?;
            0
        };
        crate::bench_event!(
            "index.open",
            serde_json::json!({
                "duration_ms": crate::bench::duration_ms(started),
            }),
        );
        Ok((
            Self {
                pool,
                dimensions: Default::default(),
                duplicate_groups: Default::default(),
            },
            interrupted_count,
        ))
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
        if ignored {
            conn.execute(
                "
                INSERT INTO folders (path, ignored, discovered_at, updated_at)
                VALUES (?1, 1, ?2, ?2)
                ON CONFLICT(path) DO UPDATE SET
                    ignored = 1,
                    updated_at = excluded.updated_at
                ",
                params![path_to_string(path), now],
            )?;
            let prefix = format!("{}/%", path_to_string(path).trim_end_matches('/'));
            conn.execute(
                "DELETE FROM images WHERE folder_path = ?1 OR folder_path LIKE ?2",
                params![path_to_string(path), prefix],
            )?;
        } else {
            let ignored_folders = {
                let mut stmt = conn.prepare("SELECT path FROM folders WHERE ignored = 1")?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                rows.filter_map(Result::ok)
                    .map(PathBuf::from)
                    .collect::<Vec<_>>()
            };
            for folder in ignored_folders {
                if path.starts_with(&folder) {
                    conn.execute(
                        "UPDATE folders SET ignored = 0, updated_at = ?2 WHERE path = ?1",
                        params![path_to_string(&folder), now],
                    )?;
                }
            }
            conn.execute(
                "
                INSERT INTO folders (path, ignored, discovered_at, updated_at)
                VALUES (?1, 0, ?2, ?2)
                ON CONFLICT(path) DO UPDATE SET
                    ignored = 0,
                    updated_at = excluded.updated_at
                ",
                params![path_to_string(path), now],
            )?;
        }
        Ok(())
    }

    pub fn is_folder_ignored(&self, path: &Path) -> rusqlite::Result<bool> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT path FROM folders WHERE ignored = 1")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let ignored = rows
            .filter_map(Result::ok)
            .map(PathBuf::from)
            .any(|folder| path.starts_with(folder));
        Ok(ignored)
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
        if self.is_folder_ignored(folder)? {
            return Ok((Vec::new(), 0, Vec::new()));
        }
        let now = now_secs();
        let mut conn = self.conn()?;

        let stale_count: usize;
        {
            let tx = conn.transaction()?;
            let folder_path_str = path_to_string(folder);

            tx.execute(
                "INSERT INTO folders (path, ignored, discovered_at, updated_at)
                 VALUES (?1, 0, ?2, ?2)
                 ON CONFLICT(path) DO UPDATE SET updated_at = excluded.updated_at",
                params![&folder_path_str, now],
            )?;

            {
                let mut stmt = tx.prepare_cached(
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
                )?;

                for info in entries {
                    stmt.execute(params![
                        path_to_string(&info.path),
                        &folder_path_str,
                        info.filename,
                        info.extension,
                        info.file_size as i64,
                        info.modified_secs,
                        now,
                    ])?;
                }
            }

            let current_set: HashSet<String> =
                entries.iter().map(|e| path_to_string(&e.path)).collect();
            let db_paths: Vec<String> = {
                let mut stmt = tx.prepare("SELECT path FROM images WHERE folder_path = ?1")?;
                let rows = stmt
                    .query_map(params![&folder_path_str], |row| row.get::<_, String>(0))?
                    .filter_map(Result::ok)
                    .collect();
                rows
            };
            let stale: Vec<_> = db_paths
                .iter()
                .filter(|p| !current_set.contains(*p))
                .collect();
            stale_count = stale.len();
            {
                let mut stmt = tx.prepare_cached("DELETE FROM images WHERE path = ?1")?;
                for path in stale {
                    stmt.execute(params![path])?;
                }
            }

            tx.commit()?;
        }

        let order = match sort_order {
            SortOrder::NameAsc => "filename COLLATE NOCASE ASC",
            SortOrder::NameDesc => "filename COLLATE NOCASE DESC",
            SortOrder::DateModifiedAsc => {
                "modified_secs ASC NULLS LAST, filename COLLATE NOCASE ASC"
            }
            SortOrder::DateModifiedDesc => {
                "modified_secs DESC NULLS LAST, filename COLLATE NOCASE ASC"
            }
            SortOrder::FileTypeAsc => "extension COLLATE NOCASE ASC, filename COLLATE NOCASE ASC",
            SortOrder::FileTypeDesc => "extension COLLATE NOCASE DESC, filename COLLATE NOCASE ASC",
            SortOrder::AspectAsc => "CASE WHEN width > 0 AND height > 0 THEN 1.0*width/height END ASC NULLS LAST, filename COLLATE NOCASE ASC",
            SortOrder::AspectDesc => "CASE WHEN width > 0 AND height > 0 THEN 1.0*width/height END DESC NULLS LAST, filename COLLATE NOCASE ASC",
            SortOrder::WideScreenAsc => "CASE WHEN width > 0 AND height > 0 THEN ABS((1.0*width/height)/(16.0/9.0)-1.0) END ASC NULLS LAST, filename COLLATE NOCASE ASC",
            SortOrder::WideScreenDesc => "CASE WHEN width > 0 AND height > 0 THEN ABS((1.0*width/height)/(16.0/9.0)-1.0) END DESC NULLS LAST, filename COLLATE NOCASE ASC",
        };
        let mut rows: Vec<IndexedImage> = {
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

        sort_indexed_aspects(&mut rows, sort_order, &self.dimensions);
        Ok((rows, stale_count, metadata_pending))
    }

    pub fn upsert_image_basic(&self, info: &BasicImageInfo) -> rusqlite::Result<()> {
        if self.is_folder_ignored(&info.folder_path)? {
            return Ok(());
        }
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

    #[cfg(test)]
    pub fn update_image_phash(&self, path: &Path, hash: u64) -> rusqlite::Result<()> {
        self.update_image_phashes(&[(path.to_path_buf(), hash)])
    }

    /// Stores perceptual hashes in one transaction. Writing them one by one
    /// flooded the shared worker pool and delayed folder opens.
    pub fn update_image_phashes(&self, hashes: &[(PathBuf, u64)]) -> rusqlite::Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "UPDATE images SET phash = ?2, phash_status = 'ready', error = NULL WHERE path = ?1",
            )?;
            for (path, hash) in hashes {
                stmt.execute(params![path_to_string(path), *hash as i64])?;
            }
        }
        tx.commit()
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
        let mut conn = self.conn()?;
        let stale: Vec<String> = {
            let mut stmt = conn.prepare("SELECT path FROM images WHERE folder_path = ?1")?;
            let rows = stmt.query_map(params![path_to_string(folder)], |row| {
                row.get::<_, String>(0)
            })?;
            rows.filter_map(Result::ok)
                .filter(|path| !current.contains(path))
                .collect()
        };

        if stale.is_empty() {
            return Ok(0);
        }

        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached("DELETE FROM images WHERE path = ?1")?;
            for path in &stale {
                stmt.execute(params![path])?;
            }
        }
        tx.commit()?;
        Ok(stale.len())
    }

    pub fn images_in_folder(
        &self,
        folder: &Path,
        sort_order: SortOrder,
    ) -> rusqlite::Result<Vec<IndexedImage>> {
        if self.is_folder_ignored(folder)? {
            return Ok(Vec::new());
        }
        let order = match sort_order {
            SortOrder::NameAsc => "filename COLLATE NOCASE ASC",
            SortOrder::NameDesc => "filename COLLATE NOCASE DESC",
            SortOrder::DateModifiedAsc => {
                "modified_secs ASC NULLS LAST, filename COLLATE NOCASE ASC"
            }
            SortOrder::DateModifiedDesc => {
                "modified_secs DESC NULLS LAST, filename COLLATE NOCASE ASC"
            }
            SortOrder::FileTypeAsc => "extension COLLATE NOCASE ASC, filename COLLATE NOCASE ASC",
            SortOrder::FileTypeDesc => "extension COLLATE NOCASE DESC, filename COLLATE NOCASE ASC",
            SortOrder::AspectAsc => "CASE WHEN width > 0 AND height > 0 THEN 1.0*width/height END ASC NULLS LAST, filename COLLATE NOCASE ASC",
            SortOrder::AspectDesc => "CASE WHEN width > 0 AND height > 0 THEN 1.0*width/height END DESC NULLS LAST, filename COLLATE NOCASE ASC",
            SortOrder::WideScreenAsc => "CASE WHEN width > 0 AND height > 0 THEN ABS((1.0*width/height)/(16.0/9.0)-1.0) END ASC NULLS LAST, filename COLLATE NOCASE ASC",
            SortOrder::WideScreenDesc => "CASE WHEN width > 0 AND height > 0 THEN ABS((1.0*width/height)/(16.0/9.0)-1.0) END DESC NULLS LAST, filename COLLATE NOCASE ASC",
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
        let mut rows: Vec<_> = rows.collect::<rusqlite::Result<_>>()?;
        sort_indexed_aspects(&mut rows, sort_order, &self.dimensions);
        Ok(rows)
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
            SELECT images.path, images.phash
            FROM images
            LEFT JOIN folders ON folders.path = images.folder_path
            WHERE images.phash_status = 'ready'
              AND images.phash IS NOT NULL
              AND COALESCE(folders.ignored, 0) = 0
            ORDER BY images.path COLLATE NOCASE
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

    /// Duplicate groups over all stored hashes. Grouping compares every pair,
    /// so the result is reused until the hash list itself changes; the viewer
    /// asks on every image change.
    pub fn duplicate_groups(&self) -> rusqlite::Result<Vec<Vec<PathBuf>>> {
        use std::hash::{Hash, Hasher};
        let hashes = self.images_with_phash()?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hashes.hash(&mut hasher);
        let fingerprint = hasher.finish();
        if let Ok(cache) = self.duplicate_groups.lock() {
            if let Some((cached, groups)) = cache.as_ref() {
                if *cached == fingerprint {
                    return Ok(groups.clone());
                }
            }
        }
        let groups = crate::duplicates::phash::group_duplicates(&hashes);
        if let Ok(mut cache) = self.duplicate_groups.lock() {
            *cache = Some((fingerprint, groups.clone()));
        }
        Ok(groups)
    }

    pub fn all_indexed_paths(&self) -> rusqlite::Result<Vec<PathBuf>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "
            SELECT images.path
            FROM images
            LEFT JOIN folders ON folders.path = images.folder_path
            WHERE COALESCE(folders.ignored, 0) = 0
            ORDER BY images.path COLLATE NOCASE
            ",
        )?;
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
        self.list_collections_for_library(None)
    }

    pub fn assign_orphan_collections(&self, library_id: &str) -> rusqlite::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE collections SET library_id = ?1 WHERE library_id = ''",
            params![library_id],
        )?;
        Ok(())
    }

    pub fn list_collections_for_library(
        &self,
        library_id: Option<&str>,
    ) -> rusqlite::Result<Vec<Collection>> {
        let conn = self.conn()?;
        let make_row = |row: &rusqlite::Row<'_>| {
            Ok(Collection {
                id: row.get(0)?,
                parent_id: row.get(1)?,
                name: row.get(2)?,
                primary_tag: row.get(3)?,
                extra_tags: parse_tags_json(&row.get::<_, String>(4)?),
                color: row.get(5)?,
                icon_name: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
                item_count: 0,
            })
        };
        let rows: Vec<Collection> = if let Some(lid) = library_id {
            let mut stmt = conn.prepare(
                "SELECT c.id, c.parent_id, c.name, c.primary_tag, c.extra_tags_json,
                        c.color, c.icon_name, c.created_at, c.updated_at
                 FROM collections c
                 WHERE c.library_id = ?1
                 ORDER BY c.name COLLATE NOCASE",
            )?;
            let mapped = stmt.query_map(params![lid], make_row)?;
            mapped.filter_map(Result::ok).collect()
        } else {
            let mut stmt = conn.prepare(
                "SELECT c.id, c.parent_id, c.name, c.primary_tag, c.extra_tags_json,
                        c.color, c.icon_name, c.created_at, c.updated_at
                 FROM collections c
                 ORDER BY c.name COLLATE NOCASE",
            )?;
            let mapped = stmt.query_map([], make_row)?;
            mapped.filter_map(Result::ok).collect()
        };
        Ok(rows)
    }

    pub fn collection(&self, id: i64) -> rusqlite::Result<Option<Collection>> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT id, parent_id, name, primary_tag, extra_tags_json, color, icon_name,
                    created_at, updated_at
             FROM collections
             WHERE id = ?1",
            params![id],
            |row| {
                Ok(Collection {
                    id: row.get(0)?,
                    parent_id: row.get(1)?,
                    name: row.get(2)?,
                    primary_tag: row.get(3)?,
                    extra_tags: parse_tags_json(&row.get::<_, String>(4)?),
                    color: row.get(5)?,
                    icon_name: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                    item_count: 0,
                })
            },
        )
        .optional()
    }

    pub fn create_collection(
        &self,
        library_id: &str,
        parent_id: Option<i64>,
        name: &str,
        extra_tags: &[String],
        color: Option<&str>,
        icon_name: Option<&str>,
    ) -> rusqlite::Result<Collection> {
        let name = name.trim();
        if name.is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "collection name cannot be empty".into(),
            ));
        }
        if let Some(parent_id) = parent_id {
            if self.collection(parent_id)?.is_none() {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
        }
        let now = now_secs();
        let primary_tag = normalize_collection_tag(name);
        self.ensure_bucket_tag_free(&primary_tag, None)?;
        let mut extra_tags = normalize_collection_tags(extra_tags);
        extra_tags.retain(|tag| tag != &primary_tag);
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO collections (
                 library_id, parent_id, name, primary_tag, extra_tags_json, color, icon_name,
                 created_at, updated_at, tag_migrated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?8)",
            params![
                library_id,
                parent_id,
                name,
                primary_tag,
                tags_to_json(&extra_tags),
                color,
                icon_name,
                now
            ],
        )?;
        let id = conn.last_insert_rowid();
        Ok(Collection {
            id,
            parent_id,
            name: name.to_string(),
            primary_tag,
            extra_tags,
            color: color.map(str::to_string),
            icon_name: icon_name.map(str::to_string),
            created_at: now,
            updated_at: now,
            item_count: 0,
        })
    }

    pub fn ensure_output_collection(&self, name: &str) -> rusqlite::Result<i64> {
        let name = name.trim();
        if name.is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "collection name cannot be empty".into(),
            ));
        }

        {
            let conn = self.conn()?;
            if let Some(id) = conn
                .query_row(
                    "SELECT id FROM collections WHERE name = ?1",
                    params![name],
                    |row| row.get(0),
                )
                .optional()?
            {
                return Ok(id);
            }
        }

        self.create_collection("", None, name, &[], None, None)
            .map(|collection| collection.id)
    }

    pub fn add_to_collection_by_id(&self, path: &Path, collection_id: i64) -> rusqlite::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT OR IGNORE INTO collection_items (collection_id, image_path, added_at)
             VALUES (?1, ?2, ?3)",
            params![collection_id, path_to_string(path), now_secs()],
        )?;
        conn.execute(
            "UPDATE collections SET updated_at = ?2 WHERE id = ?1",
            params![collection_id, now_secs()],
        )?;
        Ok(())
    }

    pub fn copy_collection_memberships(
        &self,
        source: &Path,
        output: &Path,
    ) -> rusqlite::Result<()> {
        if source == output {
            return Ok(());
        }

        let conn = self.conn()?;
        conn.execute(
            "INSERT OR IGNORE INTO collection_items (collection_id, image_path, added_at)
             SELECT collection_id, ?2, ?3
             FROM collection_items
             WHERE image_path = ?1",
            params![path_to_string(source), path_to_string(output), now_secs()],
        )?;
        Ok(())
    }

    /// Two buckets with the same primary tag would silently share members, so
    /// names that differ only by case (or surrounding space) are rejected.
    fn ensure_bucket_tag_free(
        &self,
        primary_tag: &str,
        except: Option<i64>,
    ) -> rusqlite::Result<()> {
        let existing: Option<String> = self
            .conn()?
            .query_row(
                "SELECT name FROM collections WHERE primary_tag = ?1 AND id IS NOT ?2 LIMIT 1",
                params![primary_tag, except],
                |row| row.get(0),
            )
            .optional()?;
        match existing {
            Some(name) => Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
                Some(format!("A bucket named “{name}” already exists")),
            )),
            None => Ok(()),
        }
    }

    pub fn update_collection(
        &self,
        id: i64,
        name: &str,
        extra_tags: &[String],
        color: Option<&str>,
        icon_name: Option<&str>,
    ) -> rusqlite::Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "collection name cannot be empty".into(),
            ));
        }
        let now = now_secs();
        let primary_tag = normalize_collection_tag(name);
        self.ensure_bucket_tag_free(&primary_tag, Some(id))?;
        let mut extra_tags = normalize_collection_tags(extra_tags);
        extra_tags.retain(|tag| tag != &primary_tag);
        let conn = self.conn()?;
        conn.execute(
            "UPDATE collections
             SET name = ?2,
                 primary_tag = ?3,
                 extra_tags_json = ?4,
                 color = ?5,
                 icon_name = ?6,
                 updated_at = ?7
             WHERE id = ?1",
            params![
                id,
                name,
                primary_tag,
                tags_to_json(&extra_tags),
                color,
                icon_name,
                now
            ],
        )?;
        Ok(())
    }

    pub fn delete_collection(&self, id: i64) -> rusqlite::Result<()> {
        let ids = self.collection_subtree_ids(id)?;
        if ids.is_empty() {
            return Ok(());
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached("DELETE FROM collections WHERE id = ?1")?;
            for id in ids {
                stmt.execute(params![id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn reparent_collection(&self, id: i64, new_parent_id: Option<i64>) -> rusqlite::Result<()> {
        let Some(collection) = self.collection(id)? else {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        };
        if let Some(parent_id) = new_parent_id {
            if id == parent_id {
                return Err(rusqlite::Error::InvalidParameterName(
                    "collection cannot be its own parent".into(),
                ));
            }
            if self.collection(parent_id)?.is_none() {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
        }
        let collections = self.list_collections()?;
        if let Some(parent_id) = new_parent_id {
            let mut current = Some(parent_id);
            while let Some(current_id) = current {
                if current_id == id {
                    return Err(rusqlite::Error::InvalidParameterName(
                        "collection cannot be moved under its own child".into(),
                    ));
                }
                current = collections
                    .iter()
                    .find(|collection| collection.id == current_id)
                    .and_then(|collection| collection.parent_id);
            }
        }
        if collection.parent_id == new_parent_id {
            return Ok(());
        }
        let now = now_secs();
        let conn = self.conn()?;
        conn.execute(
            "UPDATE collections
             SET parent_id = ?2, updated_at = ?3
             WHERE id = ?1",
            params![id, new_parent_id, now],
        )?;
        Ok(())
    }

    pub fn collection_effective_tags(&self, collection_id: i64) -> rusqlite::Result<Vec<String>> {
        Ok(effective_tags_in(&self.list_collections()?, collection_id))
    }

    pub fn touch_collection(&self, collection_id: i64) -> rusqlite::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE collections SET updated_at = ?2 WHERE id = ?1",
            params![collection_id, now_secs()],
        )?;
        Ok(())
    }

    pub fn ignored_folders_snapshot(&self) -> rusqlite::Result<Vec<PathBuf>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT path FROM folders WHERE ignored = 1 ORDER BY path")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows.filter_map(Result::ok).map(PathBuf::from).collect())
    }

    pub fn filter_ignored_paths(&self, paths: Vec<PathBuf>) -> rusqlite::Result<Vec<PathBuf>> {
        let ignored = self.ignored_folders_snapshot()?;
        Ok(paths
            .into_iter()
            .filter(|path| !path_is_ignored(path, &ignored))
            .collect())
    }

    pub fn migrate_legacy_collections_to_tags(
        &self,
        tags: &crate::tags::TagDatabase,
    ) -> rusqlite::Result<()> {
        let legacy_collections = self.legacy_collections_pending_tag_migration()?;
        if legacy_collections.is_empty() {
            return Ok(());
        }
        let conn = self.conn()?;
        for collection in legacy_collections {
            let paths = {
                let mut stmt = conn.prepare(
                    "SELECT image_path FROM collection_items
                     WHERE collection_id = ?1
                     ORDER BY added_at ASC",
                )?;
                let rows = stmt.query_map(params![collection.id], |row| row.get::<_, String>(0))?;
                rows.filter_map(Result::ok)
                    .map(PathBuf::from)
                    .collect::<Vec<_>>()
            };
            if !paths.is_empty() {
                tags.change_tags_checked(
                    &paths,
                    std::slice::from_ref(&collection.primary_tag),
                    false,
                )
                .map_err(rusqlite::Error::InvalidParameterName)?;
            }
            conn.execute(
                "UPDATE collections
                 SET tag_migrated_at = ?2,
                     updated_at = ?2
                 WHERE id = ?1",
                params![collection.id, now_secs()],
            )?;
        }
        Ok(())
    }

    fn legacy_collections_pending_tag_migration(&self) -> rusqlite::Result<Vec<Collection>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, parent_id, name, primary_tag, extra_tags_json, color, icon_name,
                    created_at, updated_at
             FROM collections
             WHERE tag_migrated_at IS NULL
             ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Collection {
                id: row.get(0)?,
                parent_id: row.get(1)?,
                name: row.get(2)?,
                primary_tag: row.get(3)?,
                extra_tags: parse_tags_json(&row.get::<_, String>(4)?),
                color: row.get(5)?,
                icon_name: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
                item_count: 0,
            })
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    fn collection_subtree_ids(&self, id: i64) -> rusqlite::Result<Vec<i64>> {
        let collections = self.list_collections()?;
        let mut by_parent: HashMap<Option<i64>, Vec<i64>> = HashMap::new();
        for collection in collections {
            by_parent
                .entry(collection.parent_id)
                .or_default()
                .push(collection.id);
        }
        let mut ids = Vec::new();
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            ids.push(current);
            if let Some(children) = by_parent.get(&Some(current)) {
                stack.extend(children.iter().copied());
            }
        }
        Ok(ids)
    }

    // ---- Pipeline CRUD ----

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

fn path_is_ignored(path: &Path, ignored_folders: &[PathBuf]) -> bool {
    ignored_folders
        .iter()
        .any(|folder| path.starts_with(folder))
}

fn configure_connection(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}

fn invalidate_legacy_quality_labels(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE images
         SET quality_class = NULL,
             metadata_status = 'missing'
         WHERE quality_class IN ('Excellent', 'Good', 'Fair', 'Poor', 'Needs Upscale')",
        [],
    )?;
    Ok(())
}

fn ensure_collection_schema(conn: &Connection) -> rusqlite::Result<()> {
    let columns = table_columns(conn, "collections")?;
    if !columns.iter().any(|column| column == "parent_id") {
        conn.execute("ALTER TABLE collections ADD COLUMN parent_id INTEGER", [])?;
    }
    if !columns.iter().any(|column| column == "primary_tag") {
        conn.execute(
            "ALTER TABLE collections ADD COLUMN primary_tag TEXT NOT NULL DEFAULT ''",
            [],
        )?;
        conn.execute(
            "UPDATE collections
             SET primary_tag = lower(trim(name))
             WHERE primary_tag = ''",
            [],
        )?;
    }
    if !columns.iter().any(|column| column == "extra_tags_json") {
        conn.execute(
            "ALTER TABLE collections ADD COLUMN extra_tags_json TEXT NOT NULL DEFAULT '[]'",
            [],
        )?;
    }
    if !columns.iter().any(|column| column == "tag_migrated_at") {
        conn.execute(
            "ALTER TABLE collections ADD COLUMN tag_migrated_at INTEGER",
            [],
        )?;
    }
    if !columns.iter().any(|column| column == "color") {
        conn.execute("ALTER TABLE collections ADD COLUMN color TEXT", [])?;
    }
    if !columns.iter().any(|column| column == "icon_name") {
        conn.execute("ALTER TABLE collections ADD COLUMN icon_name TEXT", [])?;
    }
    if !columns.iter().any(|column| column == "library_id") {
        conn.execute(
            "ALTER TABLE collections ADD COLUMN library_id TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }
    conn.execute(
        "UPDATE collections
         SET primary_tag = lower(trim(name))
         WHERE trim(primary_tag) = ''",
        [],
    )?;
    Ok(())
}

fn table_columns(conn: &Connection, table: &str) -> rusqlite::Result<Vec<String>> {
    let sql = format!("PRAGMA table_info({table})");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    Ok(rows.filter_map(Result::ok).collect())
}

fn parse_tags_json(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw)
        .map(|tags| normalize_collection_tags(&tags))
        .unwrap_or_default()
}

fn tags_to_json(tags: &[String]) -> String {
    serde_json::to_string(&normalize_collection_tags(tags)).unwrap_or_else(|_| "[]".to_string())
}

pub fn normalize_collection_tag(tag: &str) -> String {
    tag.trim().to_lowercase()
}

fn normalize_collection_tags(tags: &[String]) -> Vec<String> {
    let mut normalized = Vec::new();
    for tag in tags {
        let tag = normalize_collection_tag(tag);
        if !tag.is_empty() && !normalized.contains(&tag) {
            normalized.push(tag);
        }
    }
    normalized
}

/// The tags an image needs to belong to `collection_id`: its own tags plus all
/// of its ancestors' tags, from the root down.
pub fn effective_tags_in(collections: &[Collection], collection_id: i64) -> Vec<String> {
    let by_id: HashMap<i64, &Collection> = collections.iter().map(|c| (c.id, c)).collect();
    let mut current_id = Some(collection_id);
    let mut lineage = Vec::new();
    while let Some(id) = current_id {
        let Some(collection) = by_id.get(&id) else {
            break;
        };
        // A corrupt parent cycle must not loop forever.
        if lineage.iter().any(|c: &&Collection| c.id == id) {
            break;
        }
        lineage.push(*collection);
        current_id = collection.parent_id;
    }
    lineage.reverse();
    let mut tags = Vec::new();
    for collection in lineage {
        push_unique_tag(&mut tags, &collection.primary_tag);
        for tag in &collection.extra_tags {
            push_unique_tag(&mut tags, tag);
        }
    }
    tags
}

fn push_unique_tag(tags: &mut Vec<String>, tag: &str) {
    let tag = normalize_collection_tag(tag);
    if !tag.is_empty() && !tags.contains(&tag) {
        tags.push(tag);
    }
}

#[cfg(test)]
impl LibraryIndex {
    pub(crate) fn open_in_memory() -> rusqlite::Result<Self> {
        let manager = SqliteConnectionManager::memory().with_init(|conn| {
            configure_connection(conn)?;
            initialize_schema(conn)?;
            invalidate_legacy_quality_labels(conn)?;
            ensure_collection_schema(conn)?;
            // Legacy task tables are intentionally left untouched.
            Ok(())
        });
        let pool = Pool::builder()
            .max_size(1)
            .build(manager)
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        Ok(Self {
            pool,
            dimensions: Default::default(),
            duplicate_groups: Default::default(),
        })
    }

    fn open_bare_db() -> rusqlite::Result<Self> {
        let manager = SqliteConnectionManager::memory().with_init(|conn| {
            configure_connection(conn)?;
            initialize_schema(conn)?;
            Ok(())
        });
        let pool = Pool::builder()
            .max_size(1)
            .build(manager)
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        Ok(Self {
            pool,
            dimensions: Default::default(),
            duplicate_groups: Default::default(),
        })
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
            .images_in_folder(Path::new("/photos"), SortOrder::NameAsc)
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
        idx.update_image_metadata(Path::new("/photos/a.jpg"), 1920, 1080, QualityClass::Fair)
            .unwrap();
        idx.update_image_phash(Path::new("/photos/a.jpg"), 0xdeadbeef)
            .unwrap();

        // Upsert same file again — size and mtime unchanged.
        idx.upsert_image_basic(&info).unwrap();
        let rows = idx
            .images_in_folder(Path::new("/photos"), SortOrder::NameAsc)
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
        idx.update_image_metadata(Path::new("/photos/a.jpg"), 1920, 1080, QualityClass::Fair)
            .unwrap();
        idx.update_image_phash(Path::new("/photos/a.jpg"), 0xdeadbeef)
            .unwrap();

        // Upsert with different size — file changed.
        let changed = make_info("/photos", "a.jpg", 2000, Some(200));
        idx.upsert_image_basic(&changed).unwrap();
        let rows = idx
            .images_in_folder(Path::new("/photos"), SortOrder::NameAsc)
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
            .images_in_folder(Path::new("/photos"), SortOrder::NameAsc)
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
        idx.update_image_metadata(Path::new("/photos/keep.jpg"), 1600, 900, QualityClass::Poor)
            .unwrap();

        // Reconcile with keep.jpg (unchanged) and new.jpg; old.jpg is gone.
        let entries = vec![
            make_info("/photos", "keep.jpg", 200, None),
            make_info("/photos", "new.jpg", 300, None),
        ];
        let (rows, stale, pending) = idx
            .reconcile_folder(Path::new("/photos"), &entries, SortOrder::NameAsc)
            .unwrap();

        assert_eq!(stale, 1, "old.jpg should be removed");
        assert_eq!(rows.len(), 2);
        // keep.jpg metadata should be preserved (unchanged file).
        let keep_row = rows.iter().find(|r| r.filename == "keep.jpg").unwrap();
        assert_eq!(keep_row.metadata_status, "ready");
        assert_eq!(keep_row.width, Some(1600));
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
            QualityClass::Fair,
        )
        .unwrap();
        idx.update_image_metadata(
            Path::new("/photos/small.jpg"),
            200,
            150,
            QualityClass::NeedsUpscale,
        )
        .unwrap();

        let fair = idx.images_by_quality(QualityClass::Fair).unwrap();
        assert_eq!(fair, vec![PathBuf::from("/photos/good.jpg")]);

        let upscale = idx.images_by_quality(QualityClass::NeedsUpscale).unwrap();
        assert_eq!(upscale, vec![PathBuf::from("/photos/small.jpg")]);
    }

    #[test]
    fn images_by_quality_excludes_ignored_folders() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        idx.upsert_image_basic(&make_info("/ignored", "a.jpg", 100, None))
            .unwrap();
        idx.update_image_metadata(Path::new("/ignored/a.jpg"), 1920, 1080, QualityClass::Fair)
            .unwrap();
        idx.set_folder_ignored(Path::new("/ignored"), true).unwrap();

        let fair = idx.images_by_quality(QualityClass::Fair).unwrap();
        assert!(fair.is_empty(), "ignored folder images should be excluded");
    }

    #[test]
    fn legacy_quality_labels_are_marked_for_recompute() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        idx.upsert_image_basic(&make_info("/photos", "a.jpg", 100, None))
            .unwrap();
        let conn = idx.pool.get().unwrap();
        conn.execute(
            "UPDATE images
             SET quality_class = 'Good',
                 metadata_status = 'ready'
             WHERE path = '/photos/a.jpg'",
            [],
        )
        .unwrap();

        invalidate_legacy_quality_labels(&conn).unwrap();

        let (quality_class, metadata_status): (Option<String>, String) = conn
            .query_row(
                "SELECT quality_class, metadata_status
                 FROM images
                 WHERE path = '/photos/a.jpg'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(quality_class, None);
        assert_eq!(metadata_status, "missing");
    }

    #[test]
    fn ignored_folder_is_removed_and_not_reindexed() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let info = make_info("/ignored", "a.jpg", 100, None);
        idx.upsert_image_basic(&info).unwrap();
        idx.set_folder_ignored(Path::new("/ignored"), true).unwrap();

        assert!(idx.is_folder_ignored(Path::new("/ignored")).unwrap());
        assert!(idx
            .images_in_folder(Path::new("/ignored"), SortOrder::NameAsc)
            .unwrap()
            .is_empty());

        idx.upsert_image_basic(&info).unwrap();
        assert!(idx.all_indexed_paths().unwrap().is_empty());
    }

    #[test]
    fn enabling_child_clears_ignored_ancestor() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        idx.set_folder_ignored(Path::new("/library"), true).unwrap();

        assert!(idx.is_folder_ignored(Path::new("/library/child")).unwrap());
        idx.set_folder_ignored(Path::new("/library/child"), false)
            .unwrap();

        assert!(!idx.is_folder_ignored(Path::new("/library")).unwrap());
        assert!(!idx.is_folder_ignored(Path::new("/library/child")).unwrap());
    }

    #[test]
    fn duplicate_groups_follow_hash_changes() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        for name in ["a.jpg", "b.jpg", "c.jpg"] {
            idx.upsert_image_basic(&make_info("/photos", name, 100, None))
                .unwrap();
        }
        idx.update_image_phashes(&[
            (PathBuf::from("/photos/a.jpg"), 0xF0F0),
            (PathBuf::from("/photos/b.jpg"), 0xF0F0),
            (PathBuf::from("/photos/c.jpg"), 0xFFFF_0000_FFFF_0000),
        ])
        .unwrap();
        let first = idx.duplicate_groups().unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].len(), 2);
        assert_eq!(
            idx.duplicate_groups().unwrap(),
            first,
            "cached result reused"
        );

        idx.update_image_phash(Path::new("/photos/c.jpg"), 0xF0F0)
            .unwrap();
        let after = idx.duplicate_groups().unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].len(), 3, "a changed hash regroups");
    }

    #[test]
    fn phashes_are_stored_in_one_batch() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        for name in ["a.jpg", "b.jpg"] {
            idx.upsert_image_basic(&make_info("/photos", name, 100, None))
                .unwrap();
        }
        idx.update_image_phashes(&[
            (PathBuf::from("/photos/a.jpg"), 1),
            (PathBuf::from("/photos/b.jpg"), 2),
            (PathBuf::from("/photos/not-indexed.jpg"), 3),
        ])
        .unwrap();
        let mut hashes = idx.images_with_phash().unwrap();
        hashes.sort();
        assert_eq!(
            hashes,
            vec![
                (PathBuf::from("/photos/a.jpg"), 1),
                (PathBuf::from("/photos/b.jpg"), 2)
            ]
        );
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
            .images_in_folder(Path::new("/photos"), SortOrder::NameAsc)
            .unwrap();
        assert_eq!(
            by_name
                .iter()
                .map(|r| r.filename.as_str())
                .collect::<Vec<_>>(),
            vec!["a.jpg", "b.jpg", "c.png"]
        );

        let by_date = idx
            .images_in_folder(Path::new("/photos"), SortOrder::DateModifiedDesc)
            .unwrap();
        // Descending mtime: a(300), b(200), c(100).
        assert_eq!(by_date[0].filename, "a.jpg");
        assert_eq!(by_date[2].filename, "c.png");

        let by_type = idx
            .images_in_folder(Path::new("/photos"), SortOrder::FileTypeAsc)
            .unwrap();
        // jpg before png.
        assert_eq!(by_type[0].extension, "jpg");
        assert_eq!(by_type[2].extension, "png");

        let by_oldest = idx
            .images_in_folder(Path::new("/photos"), SortOrder::DateModifiedAsc)
            .unwrap();
        assert_eq!(by_oldest[0].filename, "c.png");

        let by_name_desc = idx
            .images_in_folder(Path::new("/photos"), SortOrder::NameDesc)
            .unwrap();
        assert_eq!(by_name_desc[0].filename, "c.png");
    }

    #[test]
    fn create_and_list_collections() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let c = idx
            .create_collection("", None, "Pinned", &[], None, None)
            .unwrap();
        assert_eq!(c.name, "Pinned");
        assert_eq!(c.item_count, 0);
        assert_eq!(c.primary_tag, "pinned");
        assert_eq!(c.color, None);
        assert_eq!(c.icon_name, None);

        let list = idx.list_collections().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Pinned");
        assert_eq!(list[0].item_count, 0);
    }

    #[test]
    fn create_collection_rejects_empty_name() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        assert!(idx
            .create_collection("", None, "", &[], None, None)
            .is_err());
        assert!(idx
            .create_collection("", None, "   ", &[], None, None)
            .is_err());
    }

    #[test]
    fn create_collection_rejects_duplicate_name() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        idx.create_collection("", None, "Pinned", &[], None, None)
            .unwrap();
        assert!(idx
            .create_collection("", None, "Pinned", &[], None, None)
            .is_err());
    }

    #[test]
    fn output_collection_helpers_are_idempotent() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let collection_id = idx.ensure_output_collection("Upscaled").unwrap();
        assert_eq!(
            idx.ensure_output_collection("Upscaled").unwrap(),
            collection_id
        );

        let output = Path::new("/photos/source-upscaled.png");
        idx.add_to_collection_by_id(output, collection_id).unwrap();
        idx.add_to_collection_by_id(output, collection_id).unwrap();

        let conn = idx.conn().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM collection_items
                 WHERE collection_id = ?1 AND image_path = ?2",
                params![collection_id, path_to_string(output)],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn copy_collection_memberships_adds_output_to_source_collections() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let source = Path::new("/photos/source.jpg");
        let output = Path::new("/photos/source.jxl");
        let first = idx
            .create_collection("", None, "People", &[], None, None)
            .unwrap();
        let second = idx
            .create_collection("", None, "Favorites", &[], None, None)
            .unwrap();

        idx.add_to_collection_by_id(source, first.id).unwrap();
        idx.add_to_collection_by_id(source, second.id).unwrap();
        idx.copy_collection_memberships(source, output).unwrap();
        idx.copy_collection_memberships(source, output).unwrap();

        let conn = idx.conn().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT collection_id FROM collection_items
                 WHERE image_path = ?1
                 ORDER BY collection_id",
            )
            .unwrap();
        let rows = stmt
            .query_map(params![path_to_string(output)], |row| row.get::<_, i64>(0))
            .unwrap();
        let collection_ids: Vec<i64> = rows.filter_map(Result::ok).collect();
        assert_eq!(collection_ids, vec![first.id, second.id]);
    }

    #[test]
    fn open_stamps_the_schema_version_and_refuses_newer_databases() {
        let dir = std::env::temp_dir().join(format!(
            "sharpr-index-version-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("library-index.sqlite");
        {
            let (index, _) = LibraryIndex::open_at(&path).unwrap();
            index
                .create_collection("test", None, "Art", &[], None, None)
                .unwrap();
            let conn = index.pool.get().unwrap();
            let version: i64 = conn
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .unwrap();
            assert_eq!(version, USER_VERSION);
        }
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", USER_VERSION + 1)
            .unwrap();
        drop(conn);
        let before = std::fs::read(&path).unwrap();

        assert!(LibraryIndex::open_at(&path).is_err());
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "a refused index is untouched"
        );

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parent_rename_preserves_descendant_members_with_inherited_extra_tags() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let tags = crate::tags::TagDatabase::open_in_memory().unwrap();
        let path =
            std::env::temp_dir().join(format!("sharpr-parent-rename-{}.png", std::process::id()));
        std::fs::write(&path, b"fixture").unwrap();
        let parent = idx
            .create_collection("test", None, "Trips", &["outdoors".into()], None, None)
            .unwrap();
        let child = idx
            .create_collection(
                "test",
                Some(parent.id),
                "Iceland",
                &["moss".into()],
                None,
                None,
            )
            .unwrap();
        let grandchild = idx
            .create_collection("test", Some(child.id), "Summer", &[], None, None)
            .unwrap();
        let before = idx.collection_effective_tags(grandchild.id).unwrap();
        tags.change_tags_checked(std::slice::from_ref(&path), &before, false)
            .unwrap();
        let paths = tags
            .paths_for_all_tags_checked(&idx.collection_effective_tags(parent.id).unwrap())
            .unwrap();
        assert_eq!(paths, vec![path.clone()]);
        idx.update_collection(parent.id, "Travel", &parent.extra_tags, None, None)
            .unwrap();
        tags.replace_tag_in_paths_checked(&paths, &parent.primary_tag, "travel")
            .unwrap();
        assert_eq!(
            tags.paths_for_all_tags_checked(&idx.collection_effective_tags(child.id).unwrap())
                .unwrap(),
            vec![path.clone()]
        );
        assert_eq!(
            tags.paths_for_all_tags_checked(&idx.collection_effective_tags(grandchild.id).unwrap())
                .unwrap(),
            vec![path.clone()]
        );
        assert!(tags.tags_for_path(&path).contains(&"outdoors".into()));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn rename_collection() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let c = idx
            .create_collection("", None, "Old", &["People".into()], None, None)
            .unwrap();
        idx.update_collection(c.id, "New", &["Model".into()], None, None)
            .unwrap();
        let list = idx.list_collections().unwrap();
        assert_eq!(list[0].name, "New");
        assert_eq!(list[0].primary_tag, "new");
        assert_eq!(list[0].extra_tags, vec!["model".to_string()]);
    }

    #[test]
    fn collection_color_and_icon_name_round_trip() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let collection = idx
            .create_collection(
                "",
                None,
                "Pinned",
                &[],
                Some("#57e389"),
                Some("folder-symbolic"),
            )
            .unwrap();
        assert_eq!(collection.color.as_deref(), Some("#57e389"));
        assert_eq!(collection.icon_name.as_deref(), Some("folder-symbolic"));

        idx.update_collection(
            collection.id,
            "Pinned",
            &[],
            Some("#62a0ea"),
            Some("starred-symbolic"),
        )
        .unwrap();

        let stored = idx.collection(collection.id).unwrap().unwrap();
        assert_eq!(stored.color.as_deref(), Some("#62a0ea"));
        assert_eq!(stored.icon_name.as_deref(), Some("starred-symbolic"));
    }

    #[test]
    fn bucket_names_differing_only_by_case_are_rejected() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let art = idx
            .create_collection("", None, "Art", &[], None, None)
            .unwrap();
        let photos = idx
            .create_collection("", None, "Photos", &[], None, None)
            .unwrap();
        let error = idx
            .create_collection("", None, " art ", &[], None, None)
            .unwrap_err();
        assert!(
            error.to_string().contains("“Art” already exists"),
            "{error}"
        );
        assert!(idx
            .update_collection(photos.id, "ART", &[], None, None)
            .is_err());
        // Renaming a bucket to a different case of its own name is fine.
        idx.update_collection(art.id, "ART", &[], None, None)
            .unwrap();
    }

    #[test]
    fn rename_collection_rejects_empty_name() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let c = idx
            .create_collection("", None, "Name", &[], None, None)
            .unwrap();
        assert!(idx.update_collection(c.id, "", &[], None, None).is_err());
    }

    #[test]
    fn delete_collection_removes_subtree() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let root = idx
            .create_collection("", None, "People", &[], None, None)
            .unwrap();
        idx.create_collection("", Some(root.id), "Model", &[], None, None)
            .unwrap();

        idx.delete_collection(root.id).unwrap();
        assert!(idx.list_collections().unwrap().is_empty());
    }

    #[test]
    fn delete_empty_collection_removes_collection() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let collection = idx
            .create_collection("", None, "Empty", &[], None, None)
            .unwrap();

        idx.delete_collection(collection.id).unwrap();

        assert!(idx.collection(collection.id).unwrap().is_none());
        assert!(idx.list_collections().unwrap().is_empty());
    }

    #[test]
    fn collection_effective_tags_include_ancestors_and_extra_tags() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let people = idx
            .create_collection("", None, "People", &["portrait".into()], None, None)
            .unwrap();
        let model = idx
            .create_collection("", Some(people.id), "Model", &["studio".into()], None, None)
            .unwrap();
        let blonde = idx
            .create_collection("", Some(model.id), "Blonde", &[], None, None)
            .unwrap();

        let tags = idx.collection_effective_tags(blonde.id).unwrap();
        assert_eq!(
            tags,
            vec![
                "people".to_string(),
                "portrait".to_string(),
                "model".to_string(),
                "studio".to_string(),
                "blonde".to_string()
            ]
        );
    }

    #[test]
    fn reparent_leaf_collection_updates_parent() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let art = idx
            .create_collection("", None, "Art", &[], None, None)
            .unwrap();
        let diffusion = idx
            .create_collection("", None, "Diffusion", &[], None, None)
            .unwrap();

        idx.reparent_collection(diffusion.id, Some(art.id)).unwrap();
        let moved = idx.collection(diffusion.id).unwrap().unwrap();
        assert_eq!(moved.parent_id, Some(art.id));
    }

    #[test]
    fn reparent_collection_allows_subtrees_but_rejects_cycles() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let art = idx
            .create_collection("", None, "Art", &[], None, None)
            .unwrap();
        let root = idx
            .create_collection("", None, "People", &[], None, None)
            .unwrap();
        let model = idx
            .create_collection("", Some(root.id), "Model", &[], None, None)
            .unwrap();

        idx.reparent_collection(root.id, Some(art.id)).unwrap();
        assert_eq!(
            idx.collection(root.id).unwrap().unwrap().parent_id,
            Some(art.id)
        );
        assert!(idx.reparent_collection(root.id, Some(model.id)).is_err());
        idx.reparent_collection(root.id, None).unwrap();
        assert_eq!(idx.collection(root.id).unwrap().unwrap().parent_id, None);
    }

    #[test]
    fn migrate_legacy_collection_items_into_tags() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let tags = crate::tags::TagDatabase::open_in_memory().unwrap();
        let root = std::env::temp_dir().join(format!("sharpr-legacy-migrate-{}", now_secs()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("a.jpg");
        std::fs::write(&path, b"a").unwrap();

        let collection = idx
            .create_collection("", None, "Pinned", &[], None, None)
            .unwrap();
        let conn = idx.conn().unwrap();
        conn.execute(
            "UPDATE collections SET tag_migrated_at = NULL WHERE id = ?1",
            params![collection.id],
        )
        .unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO collection_items (collection_id, image_path, added_at)
             VALUES (?1, ?2, ?3)",
            params![collection.id, path.to_string_lossy().as_ref(), now_secs()],
        )
        .unwrap();
        drop(conn);

        idx.migrate_legacy_collections_to_tags(&tags).unwrap();
        assert_eq!(tags.tags_for_path(&path), vec!["pinned".to_string()]);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn basic_info_from_path_handles_edge_cases() {
        let temp_dir = std::env::temp_dir().join(format!("sharpr-test-{}", now_secs()));
        std::fs::create_dir_all(&temp_dir).unwrap();

        // 1. Valid file with extension
        let file_path = temp_dir.join("test.jpg");
        std::fs::write(&file_path, b"test data").unwrap();
        let info = basic_info_from_path(&temp_dir, file_path.clone());
        assert_eq!(info.path, file_path);
        assert_eq!(info.folder_path, temp_dir);
        assert_eq!(info.filename, "test.jpg");
        assert_eq!(info.extension, "jpg");
        assert_eq!(info.file_size, 9);
        assert!(info.modified_secs.is_some());

        // 2. File without extension
        let file_no_ext = temp_dir.join("no_extension");
        std::fs::write(&file_no_ext, b"more data").unwrap();
        let info = basic_info_from_path(&temp_dir, file_no_ext.clone());
        assert_eq!(info.filename, "no_extension");
        assert_eq!(info.extension, "");
        assert_eq!(info.file_size, 9);

        // 3. Non-existent file
        let non_existent = temp_dir.join("does_not_exist.png");
        let info = basic_info_from_path(&temp_dir, non_existent.clone());
        assert_eq!(info.path, non_existent);
        assert_eq!(info.filename, "does_not_exist.png");
        assert_eq!(info.extension, "png");
        assert_eq!(info.file_size, 0);
        assert!(info.modified_secs.is_none());

        std::fs::remove_dir_all(temp_dir).unwrap();
    }

    #[test]
    fn ensure_collection_schema_adds_missing_columns() {
        let idx = LibraryIndex::open_bare_db().unwrap();
        let conn = idx.conn().unwrap();

        // Verify columns before migration
        let columns = table_columns(&conn, "collections").unwrap();
        assert!(!columns.iter().any(|c| c == "parent_id"));

        ensure_collection_schema(&conn).unwrap();

        let columns = table_columns(&conn, "collections").unwrap();
        assert!(columns.iter().any(|c| c == "parent_id"));
        assert!(columns.iter().any(|c| c == "primary_tag"));
        assert!(columns.iter().any(|c| c == "extra_tags_json"));
        assert!(columns.iter().any(|c| c == "tag_migrated_at"));
        assert!(columns.iter().any(|c| c == "color"));
        assert!(columns.iter().any(|c| c == "icon_name"));
        assert!(columns.iter().any(|c| c == "library_id"));
    }

    #[test]
    fn ensure_collection_schema_preserves_existing_rows() {
        let idx = LibraryIndex::open_bare_db().unwrap();
        let conn = idx.conn().unwrap();

        let now = now_secs();
        conn.execute(
            "INSERT INTO collections (id, name, created_at, updated_at) VALUES (1, 'Travel', ?1, ?1)",
            params![now],
        )
        .unwrap();

        ensure_collection_schema(&conn).unwrap();

        let mut stmt = conn
            .prepare(
                "SELECT name, primary_tag, extra_tags_json, parent_id, color FROM collections WHERE id = 1",
            )
            .unwrap();
        let (name, primary_tag, extra_tags_json, parent_id, color): (
            String,
            String,
            String,
            Option<i64>,
            Option<String>,
        ) = stmt
            .query_row([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .unwrap();

        assert_eq!(name, "Travel");
        assert_eq!(primary_tag, "travel");
        assert_eq!(extra_tags_json, "[]");
        assert_eq!(parent_id, None);
        assert_eq!(color, None);
    }

    #[test]
    fn ensure_collection_schema_is_idempotent() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let conn = idx.conn().unwrap();

        // Should succeed first time (it already ran in open_in_memory, so this is technically the second time)
        ensure_collection_schema(&conn).unwrap();
        // Should succeed third time
        ensure_collection_schema(&conn).unwrap();
    }

    #[test]
    fn fresh_schema_does_not_create_task_tables() {
        let idx = LibraryIndex::open_in_memory().unwrap();
        let conn = idx.conn().unwrap();
        let task_table_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name IN ('pipelines', 'pipeline_steps')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(task_table_count, 0);
    }

    #[test]
    fn schema_initialization_leaves_legacy_task_data_unchanged() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE pipelines (id INTEGER PRIMARY KEY, status TEXT NOT NULL);
             CREATE TABLE pipeline_steps (id INTEGER PRIMARY KEY, pipeline_id INTEGER NOT NULL, status TEXT NOT NULL);
             INSERT INTO pipelines VALUES (7, 'in_progress');
             INSERT INTO pipeline_steps VALUES (9, 7, 'in_progress');",
        )
        .unwrap();
        initialize_schema(&conn).unwrap();
        let pipeline: (i64, String) = conn
            .query_row("SELECT id, status FROM pipelines", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        let step: (i64, i64, String) = conn
            .query_row(
                "SELECT id, pipeline_id, status FROM pipeline_steps",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(pipeline, (7, "in_progress".to_string()));
        assert_eq!(step, (9, 7, "in_progress".to_string()));
    }
}
