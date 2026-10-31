//! One read-only way to open another application's SQLite database: never
//! create, write, checkpoint or take a write lock on a file Aieyes does not own.
use anyhow::Result;
use rusqlite::{Connection, OpenFlags};
use std::{path::Path, time::Duration};

const BUSY: Duration = Duration::from_millis(500);

/// Open `path` read-only with `query_only`, a bounded busy wait and no checkpoint.
pub fn open_ro(path: &Path) -> Result<Connection> {
    // Use a native path, including Windows canonical/UNC paths, rather than a
    // URI. READ_ONLY prevents creation and writes; normal WAL/lock checks remain
    // enabled because the owner's database is not marked immutable.
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(BUSY)?;
    db.execute_batch("PRAGMA query_only=ON")?;
    Ok(db)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_native_and_canonical_paths_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "账户 a#b%c.sqlite",
            #[cfg(not(windows))] // '?' is not a legal Windows filename.
            "账户?a#b%c.sqlite",
        ] {
            let path = dir.path().join(name);
            let writer = Connection::open(&path).unwrap();
            writer
                .execute_batch("PRAGMA journal_mode=WAL;CREATE TABLE t(v);INSERT INTO t VALUES(1)")
                .unwrap();
            for path in [&path, &path.canonicalize().unwrap()] {
                let db = open_ro(path).unwrap();
                let v: i64 = db.query_row("SELECT v FROM t", [], |r| r.get(0)).unwrap();
                assert_eq!(v, 1);
                assert!(db.execute("INSERT INTO t VALUES(2)", []).is_err());
            }
        }
        assert!(open_ro(&dir.path().join("missing.sqlite")).is_err());
        assert!(!dir.path().join("missing.sqlite").exists());
    }
}
