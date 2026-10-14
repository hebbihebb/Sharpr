//! File identity in the library index: cached content fingerprints and
//! bucket membership (`ROADMAP.md` Phase 4).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{params, params_from_iter, Connection, OptionalExtension};

use super::{now_secs, path_to_string, LibraryIndex};

const MIGRATED_KEY: &str = "bucket_membership_migrated_at";

/// Whether bucket membership has moved from tags to `bucket_members`.
pub(super) fn membership_migrated(conn: &Connection) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_meta WHERE key = ?1)",
        params![MIGRATED_KEY],
        |row| row.get(0),
    )
}

/// One stored bucket member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberRow {
    pub collection_id: i64,
    pub last_path: PathBuf,
    pub fingerprint: Option<String>,
}

/// Files cached with a fingerprint: `hash → [(path, size, mtime_ns)]`.
pub type FilesByFingerprint = HashMap<String, Vec<(PathBuf, u64, i64)>>;

fn placeholders(n: usize) -> String {
    std::iter::repeat_n("?", n).collect::<Vec<_>>().join(",")
}

/// Adds `(path, fingerprint)` members to bucket `id` inside `tx`; returns
/// how many were new. A known fingerprint fills in one that was missing.
fn insert_members(
    tx: &rusqlite::Transaction<'_>,
    id: i64,
    members: &[(PathBuf, Option<String>)],
) -> rusqlite::Result<usize> {
    let mut insert = tx.prepare(
        "INSERT OR IGNORE INTO bucket_members (collection_id, last_path, fingerprint, added_at)
         VALUES (?1, ?2, ?3, ?4)",
    )?;
    let mut fill = tx.prepare(
        "UPDATE bucket_members SET fingerprint = ?3
         WHERE collection_id = ?1 AND last_path = ?2 AND fingerprint IS NULL",
    )?;
    let now = now_secs();
    let mut added = 0;
    for (path, fingerprint) in members {
        let path = path_to_string(path);
        added += insert.execute(params![id, path, fingerprint, now])?;
        if fingerprint.is_some() {
            fill.execute(params![id, path, fingerprint])?;
        }
    }
    Ok(added)
}

impl LibraryIndex {
    /// Every image size the index has read, by path. Some older rows may be
    /// unrotated, so use them only where orientation doesn't matter.
    pub fn stored_dimensions(&self) -> rusqlite::Result<HashMap<String, (u32, u32)>> {
        let conn = self.conn()?;
        let mut stmt =
            conn.prepare("SELECT path, width, height FROM images WHERE width > 0 AND height > 0")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, (row.get(1)?, row.get(2)?))))?;
        rows.collect()
    }

    /// Whether the one-time notice `key` was shown (kept in `schema_meta`).
    pub fn notice_shown(&self, key: &str) -> rusqlite::Result<bool> {
        self.conn()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_meta WHERE key = ?1)",
            params![key],
            |row| row.get(0),
        )
    }

    pub fn mark_notice_shown(&self, key: &str) -> rusqlite::Result<()> {
        self.conn()?.execute(
            "INSERT OR IGNORE INTO schema_meta (key, value) VALUES (?1, ?2)",
            params![key, now_secs().to_string()],
        )?;
        Ok(())
    }

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

    /// Whether bucket membership has been migrated from tags.
    pub fn is_membership_migrated(&self) -> rusqlite::Result<bool> {
        let conn = self.conn()?;
        membership_migrated(&conn)
    }

    /// Moves bucket membership out of tags in one transaction: `members`
    /// holds each bucket's stored members, `fingerprints` what is known of
    /// them. Fails, writing nothing, unless every bucket ends up with exactly
    /// as many members as it had; then stamps schema version 2.
    pub fn migrate_bucket_membership(
        &self,
        members: &[(i64, Vec<PathBuf>)],
        fingerprints: &HashMap<PathBuf, String>,
    ) -> Result<(), String> {
        let mut conn = self.conn().map_err(|e| e.to_string())?;
        if membership_migrated(&conn).map_err(|e| e.to_string())? {
            return Ok(());
        }
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let now = now_secs();
        {
            let mut insert = tx
                .prepare(
                    "INSERT OR IGNORE INTO bucket_members (collection_id, last_path, fingerprint, added_at)
                     VALUES (?1, ?2, ?3, ?4)",
                )
                .map_err(|e| e.to_string())?;
            let mut count = tx
                .prepare("SELECT COUNT(*) FROM bucket_members WHERE collection_id = ?1")
                .map_err(|e| e.to_string())?;
            for (id, paths) in members {
                for path in paths {
                    insert
                        .execute(params![
                            id,
                            path_to_string(path),
                            fingerprints.get(path),
                            now
                        ])
                        .map_err(|e| e.to_string())?;
                }
                let expected = paths.iter().collect::<std::collections::HashSet<_>>().len();
                let stored: i64 = count
                    .query_row(params![id], |row| row.get(0))
                    .map_err(|e| e.to_string())?;
                if stored as usize != expected {
                    return Err(format!(
                        "bucket {id} would have {stored} members instead of {expected}"
                    ));
                }
            }
        }
        tx.execute(
            "INSERT INTO schema_meta (key, value) VALUES (?1, ?2)",
            params![MIGRATED_KEY, now.to_string()],
        )
        .map_err(|e| e.to_string())?;
        crate::db_version::stamp(&tx, super::USER_VERSION).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }

    /// Stored members of the buckets `ids`.
    pub fn bucket_members(&self, ids: &[i64]) -> rusqlite::Result<Vec<MemberRow>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT collection_id, last_path, fingerprint FROM bucket_members
             WHERE collection_id IN ({}) ORDER BY last_path",
            placeholders(ids.len())
        ))?;
        let rows = stmt.query_map(params_from_iter(ids), |row| {
            Ok(MemberRow {
                collection_id: row.get(0)?,
                last_path: PathBuf::from(row.get::<_, String>(1)?),
                fingerprint: row.get(2)?,
            })
        })?;
        rows.collect()
    }

    /// Adds `(path, fingerprint)` members to bucket `id`; returns how many
    /// were new. A known fingerprint fills in one that was missing.
    pub fn add_bucket_members(
        &self,
        id: i64,
        members: &[(PathBuf, Option<String>)],
    ) -> rusqlite::Result<usize> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let added = insert_members(&tx, id, members)?;
        tx.commit()?;
        Ok(added)
    }

    /// Creates a root bucket named `name` with `members` in one
    /// transaction: on any failure, or a crash, neither exists.
    pub fn create_bucket_with_members(
        &self,
        library_id: &str,
        name: &str,
        members: &[(PathBuf, Option<String>)],
    ) -> Result<super::Collection, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("A bucket needs a name".into());
        }
        let mut conn = self.conn().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let primary_tag = super::normalize_collection_tag(name);
        let taken: Option<String> = tx
            .query_row(
                "SELECT name FROM collections WHERE primary_tag = ?1 LIMIT 1",
                params![primary_tag],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(existing) = taken {
            return Err(format!("A bucket named “{existing}” already exists"));
        }
        let mut bucket = super::insert_collection(&tx, library_id, None, name, &[], None, None)
            .map_err(|e| e.to_string())?;
        insert_members(&tx, bucket.id, members).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        bucket.item_count = members.len();
        Ok(bucket)
    }

    /// Removes from the buckets `ids` every member stored at one of `paths`
    /// or carrying one of `fingerprints`; returns how many rows went.
    pub fn remove_bucket_members(
        &self,
        ids: &[i64],
        paths: &[PathBuf],
        fingerprints: &[String],
    ) -> rusqlite::Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let mut removed = 0;
        {
            let ids_sql = placeholders(ids.len());
            let mut by_path = tx.prepare(&format!(
                "DELETE FROM bucket_members WHERE last_path = ? AND collection_id IN ({ids_sql})"
            ))?;
            let mut by_fingerprint = tx.prepare(&format!(
                "DELETE FROM bucket_members WHERE fingerprint = ? AND collection_id IN ({ids_sql})"
            ))?;
            for path in paths {
                let mut args: Vec<rusqlite::types::Value> = vec![path_to_string(path).into()];
                args.extend(ids.iter().map(|id| (*id).into()));
                removed += by_path.execute(params_from_iter(args))?;
            }
            for fingerprint in fingerprints {
                let mut args: Vec<rusqlite::types::Value> = vec![fingerprint.clone().into()];
                args.extend(ids.iter().map(|id| (*id).into()));
                removed += by_fingerprint.execute(params_from_iter(args))?;
            }
        }
        tx.commit()?;
        Ok(removed)
    }

    /// Every stored bucket member, in every library.
    pub fn all_bucket_members(&self) -> rusqlite::Result<Vec<MemberRow>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT collection_id, last_path, fingerprint FROM bucket_members ORDER BY last_path",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(MemberRow {
                collection_id: row.get(0)?,
                last_path: PathBuf::from(row.get::<_, String>(1)?),
                fingerprint: row.get(2)?,
            })
        })?;
        rows.collect()
    }

    /// Records the current fingerprint of a member's file.
    pub fn set_member_fingerprint(
        &self,
        collection_id: i64,
        last_path: &Path,
        fingerprint: &str,
    ) -> rusqlite::Result<()> {
        self.conn()?.execute(
            "UPDATE bucket_members SET fingerprint = ?3 WHERE collection_id = ?1 AND last_path = ?2",
            params![collection_id, path_to_string(last_path), fingerprint],
        )?;
        Ok(())
    }

    /// Points a member at the file's new path. If the bucket already has a
    /// member there (an identical copy), the two merge into that one.
    pub fn move_member(&self, collection_id: i64, from: &Path, to: &Path) -> rusqlite::Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let moved = tx.execute(
            "UPDATE OR IGNORE bucket_members SET last_path = ?3
             WHERE collection_id = ?1 AND last_path = ?2",
            params![collection_id, path_to_string(from), path_to_string(to)],
        )?;
        if moved == 0 {
            tx.execute(
                "DELETE FROM bucket_members WHERE collection_id = ?1 AND last_path = ?2
                 AND EXISTS (SELECT 1 FROM bucket_members WHERE collection_id = ?1 AND last_path = ?3)",
                params![collection_id, path_to_string(from), path_to_string(to)],
            )?;
        }
        tx.commit()
    }

    /// Cached files with each of `hashes`: `hash → [(path, size, mtime_ns)]`.
    /// Callers check that a file still matches before trusting it.
    pub fn files_with_fingerprints(
        &self,
        hashes: &[String],
    ) -> rusqlite::Result<FilesByFingerprint> {
        let mut found = FilesByFingerprint::new();
        let conn = self.conn()?;
        for chunk in hashes.chunks(500) {
            let mut stmt = conn.prepare(&format!(
                "SELECT hash, path, size, mtime_ns FROM file_fingerprints WHERE hash IN ({})",
                placeholders(chunk.len())
            ))?;
            let rows = stmt.query_map(params_from_iter(chunk), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    (
                        PathBuf::from(row.get::<_, String>(1)?),
                        row.get::<_, i64>(2)? as u64,
                        row.get::<_, i64>(3)?,
                    ),
                ))
            })?;
            for row in rows {
                let (hash, file) = row?;
                found.entry(hash).or_default().push(file);
            }
        }
        Ok(found)
    }

    /// Copies the whole index to `dest` (which must not exist), for a
    /// backup before a migration.
    pub fn backup_to(&self, dest: &Path) -> Result<(), String> {
        if dest.exists() {
            return Err(format!("{} already exists", dest.display()));
        }
        self.conn()
            .map_err(|e| e.to_string())?
            .execute("VACUUM INTO ?1", params![path_to_string(dest)])
            .map_err(|e| e.to_string())?;
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_member_write_leaves_no_bucket_behind() {
        let index = LibraryIndex::open_in_memory().unwrap();
        index
            .conn()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_members BEFORE INSERT ON bucket_members
                 BEGIN SELECT RAISE(ABORT, 'member write failure'); END;",
            )
            .unwrap();
        let members = vec![(PathBuf::from("/library/a.jpg"), None)];
        let error = index
            .create_bucket_with_members("test", "nature", &members)
            .unwrap_err();
        assert!(error.contains("member write failure"), "{error}");
        assert!(index.list_collections().unwrap().is_empty());

        index
            .conn()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_members;")
            .unwrap();
        let bucket = index
            .create_bucket_with_members("test", "nature", &members)
            .unwrap();
        assert_eq!(index.bucket_members(&[bucket.id]).unwrap().len(), 1);
        assert!(index
            .create_bucket_with_members("test", "Nature", &members)
            .is_err());
    }
}
