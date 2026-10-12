//! Schema versions for Sharpr's SQLite databases, kept in `PRAGMA user_version`.
//!
//! Each database records the newest schema version that has written to it.
//! A build refuses to open a database stamped by a newer build, and checks
//! before any schema setup runs, so an older Sharpr never writes to data it
//! does not understand.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

/// Fails without writing anything when the database at `path` was stamped
/// with a schema version newer than `supported`. A missing file is fine.
pub fn ensure_not_newer(path: &Path, supported: i64, label: &str) -> rusqlite::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let found: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if found > supported {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
            Some(format!(
                "The {label} was written by a newer version of Sharpr (schema {found}, \
                 this build supports {supported}); it was left untouched"
            )),
        ));
    }
    Ok(())
}

/// Records `supported` as the schema version after setup, never lowering it.
pub fn stamp(conn: &Connection, supported: i64) -> rusqlite::Result<()> {
    let found: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if found < supported {
        conn.pragma_update(None, "user_version", supported)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sharpr-db-version-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("test.sqlite")
    }

    #[test]
    fn missing_older_and_current_databases_are_accepted() {
        let path = temp_db("accepted");
        assert!(ensure_not_newer(&path, 1, "test database").is_ok());
        let conn = Connection::open(&path).unwrap();
        assert!(ensure_not_newer(&path, 1, "test database").is_ok());
        stamp(&conn, 1).unwrap();
        assert!(ensure_not_newer(&path, 1, "test database").is_ok());
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn newer_database_is_refused_and_never_downgraded() {
        let path = temp_db("newer");
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 7).unwrap();
        let error = ensure_not_newer(&path, 1, "test database").unwrap_err();
        assert!(
            error.to_string().contains("newer version of Sharpr"),
            "{error}"
        );
        stamp(&conn, 1).unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 7);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
