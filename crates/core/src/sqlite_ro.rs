//! One read-only way to open another application's SQLite database: never
//! create, write, checkpoint or take a write lock on a file Aieyes does not own.
use anyhow::Result;
use rusqlite::{Connection, OpenFlags};
use std::{path::Path, time::Duration};

const BUSY: Duration = Duration::from_millis(500);

fn uri(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file:");
    if raw.as_bytes().get(1) == Some(&b':') {
        // Windows drive paths need an empty authority: file:///C:/...
        out.push_str("///");
    }
    for byte in raw.bytes() {
        match byte {
            b'?' | b'#' | b'%' | 128..=255 => out.push_str(&format!("%{byte:02X}")),
            _ => out.push(byte as char),
        }
    }
    // `immutable=0` keeps WAL and lock checks; the file may still be written by its owner.
    out.push_str("?mode=ro&immutable=0");
    out
}

/// Open `path` read-only with `query_only`, a bounded busy wait and no checkpoint.
pub fn open_ro(path: &Path) -> Result<Connection> {
    let db = Connection::open_with_flags(
        uri(path),
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(BUSY)?;
    db.execute_batch("PRAGMA query_only=ON")?;
    Ok(db)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_without_writing_and_escapes_uri_characters() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("账户?a#b%c.sqlite");
        let writer = Connection::open(&path).unwrap();
        writer
            .execute_batch("PRAGMA journal_mode=WAL;CREATE TABLE t(v);INSERT INTO t VALUES(1)")
            .unwrap();
        let db = open_ro(&path).unwrap();
        let v: i64 = db.query_row("SELECT v FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(v, 1);
        assert!(db.execute("INSERT INTO t VALUES(2)", []).is_err());
        assert!(open_ro(&dir.path().join("missing.sqlite")).is_err());
        assert!(!dir.path().join("missing.sqlite").exists());
    }
}
