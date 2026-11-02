// SPDX-License-Identifier: MIT OR Apache-2.0

//! Schema and migrations (DESIGN §7.2).

use rusqlite::Connection;

use crate::StoreError;

/// `'decl'`.
pub const APPLICATION_ID: i64 = 0x6465_636C;

/// Migrations, indexed by the `user_version` they upgrade *from*.
const MIGRATIONS: &[&str] = &[
    // 0 -> 1: initial schema.
    r#"
    CREATE TABLE workbook   (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    CREATE TABLE sheets     (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, position INTEGER NOT NULL);

    CREATE TABLE metatables (hash BLOB PRIMARY KEY, canonical TEXT NOT NULL, created_txn INTEGER NOT NULL)
      WITHOUT ROWID;

    CREATE TABLE cells (
      sheet_id INTEGER NOT NULL, row INTEGER NOT NULL, col INTEGER NOT NULL,
      vtype INTEGER,
      value,
      meta BLOB REFERENCES metatables(hash),
      PRIMARY KEY (sheet_id, row, col)
    ) WITHOUT ROWID;
    CREATE INDEX cells_by_meta ON cells(meta);

    CREATE TABLE layout (sheet_id INTEGER, axis INTEGER, idx INTEGER, size REAL,
                         PRIMARY KEY (sheet_id, axis, idx)) WITHOUT ROWID;
    CREATE TABLE charts (id INTEGER PRIMARY KEY, sheet_id INTEGER NOT NULL, spec TEXT NOT NULL);

    CREATE TABLE txns (
      id INTEGER PRIMARY KEY, ts INTEGER NOT NULL,
      label TEXT NOT NULL,
      kind INTEGER NOT NULL,
      target INTEGER REFERENCES txns(id)
    );
    CREATE TABLE cell_changes (
      txn INTEGER NOT NULL, sheet_id INTEGER NOT NULL, row INTEGER NOT NULL, col INTEGER NOT NULL,
      b_vtype INTEGER, b_value, b_meta BLOB,
      a_vtype INTEGER, a_value, a_meta BLOB,
      PRIMARY KEY (sheet_id, row, col, txn)
    ) WITHOUT ROWID;
    CREATE INDEX cell_changes_by_txn ON cell_changes(txn);
    CREATE TABLE other_changes (
      txn INTEGER NOT NULL, seq INTEGER NOT NULL, entity TEXT NOT NULL,
      before TEXT, after TEXT,
      PRIMARY KEY (txn, seq)
    ) WITHOUT ROWID;
    "#,
];

pub const CURRENT_VERSION: i64 = MIGRATIONS.len() as i64;

pub fn configure(conn: &Connection) -> Result<(), StoreError> {
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    // 64 MiB page cache: history inserts are scattered across the
    // cell_changes primary key, so a small cache thrashes.
    conn.pragma_update(None, "cache_size", -65_536)?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    // Checkpointing a large WAL costs ~200 ms, so it is kept off the commit
    // path: the engine calls `Store::checkpoint` when idle. This high
    // threshold is only a safety net (~400 MiB of WAL).
    conn.pragma_update(None, "wal_autocheckpoint", 100_000)?;
    Ok(())
}

/// Checks the file identity and applies any pending migrations.
pub fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
    let app_id: i64 = conn.pragma_query_value(None, "application_id", |r| r.get(0))?;
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if app_id != APPLICATION_ID {
        let empty: i64 = conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))?;
        if app_id != 0 || empty != 0 {
            return Err(StoreError::NotADecelFile);
        }
    }
    if version > CURRENT_VERSION {
        return Err(StoreError::TooNew {
            version,
            supported: CURRENT_VERSION,
        });
    }
    if version == CURRENT_VERSION && app_id == APPLICATION_ID {
        return Ok(());
    }
    conn.pragma_update(None, "journal_mode", "WAL")?;
    let tx = conn.transaction()?;
    for (from, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", from as i64 + 1)?;
    }
    tx.pragma_update(None, "application_id", APPLICATION_ID)?;
    tx.commit()?;
    Ok(())
}

/// Validates identity without migrating (read-only opens).
pub fn check_readable(conn: &Connection) -> Result<(), StoreError> {
    let app_id: i64 = conn.pragma_query_value(None, "application_id", |r| r.get(0))?;
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if app_id != APPLICATION_ID {
        return Err(StoreError::NotADecelFile);
    }
    if version != CURRENT_VERSION {
        return Err(StoreError::TooNew {
            version,
            supported: CURRENT_VERSION,
        });
    }
    Ok(())
}
