use rusqlite::{params, Connection};

pub(super) fn initialize_schema(conn: &Connection) -> rusqlite::Result<()> {
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

        -- Content fingerprints (ROADMAP Phase 4), reused while size and
        -- modification time match. Rebuildable, like every cache here.
        CREATE TABLE IF NOT EXISTS file_fingerprints (
            path TEXT PRIMARY KEY,
            size INTEGER NOT NULL,
            mtime_ns INTEGER NOT NULL,
            hash TEXT NOT NULL,
            hashed_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_file_fingerprints_hash
            ON file_fingerprints(hash);

        -- Bucket membership (ROADMAP Phase 4): user data. A member is the
        -- file at last_path or any file with the same fingerprint. Rows go
        -- with their bucket when the user deletes it.
        CREATE TABLE IF NOT EXISTS bucket_members (
            collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
            last_path TEXT NOT NULL,
            fingerprint TEXT,
            added_at INTEGER NOT NULL,
            PRIMARY KEY (collection_id, last_path)
        );
        CREATE INDEX IF NOT EXISTS idx_bucket_members_fingerprint
            ON bucket_members(fingerprint);
        CREATE INDEX IF NOT EXISTS idx_bucket_members_path
            ON bucket_members(last_path);
        ",
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('schema_version', ?1)",
        params![super::SCHEMA_VERSION],
    )?;
    Ok(())
}
