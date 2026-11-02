//! setting：設定値（設計書 3章 setting）。キーと値（JSON）の組で持つ。
//!
//! キーごとの値の形と既定値は、設定を使う側が決める。ここでは JSON の文字列のまま読み書きする。

use rusqlite::{Connection, OptionalExtension, params};

use super::{execute, query_row};
use crate::storage::StorageError;

/// キーの値（JSON）。無ければ `None`（使う側が既定値を使う）。
pub fn get(conn: &Connection, key: &str) -> Result<Option<String>, StorageError> {
    Ok(query_row(
        conn,
        "SELECT value FROM setting WHERE key = ?1",
        [key],
        |r| r.get(0),
    )
    .optional()?)
}

/// キーの値（JSON）を書く。すでにあれば置き換える。JSON として正しくなければ失敗する。
pub fn set(conn: &Connection, key: &str, value_json: &str) -> Result<(), StorageError> {
    execute(
        conn,
        "INSERT INTO setting (key, value) VALUES (?1, ?2) \
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![key, value_json],
    )?;
    Ok(())
}

/// キーを消す（既定値に戻す）。消したら `true`、無ければ `false`。
pub fn remove(conn: &Connection, key: &str) -> Result<bool, StorageError> {
    let n = execute(conn, "DELETE FROM setting WHERE key = ?1", [key])?;
    Ok(n == 1)
}

/// すべてのキーと値を、キーの順で返す。
pub fn list(conn: &Connection) -> Result<Vec<(String, String)>, StorageError> {
    let mut stmt = conn.prepare_cached("SELECT key, value FROM setting ORDER BY key")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::table::test_support::open;

    #[test]
    fn set_get_replace_remove_and_list() {
        let (db, _dir) = open();
        let conn = db.connection();
        assert_eq!(get(conn, "day_boundary").unwrap(), None);
        assert!(list(conn).unwrap().is_empty());

        set(conn, "day_boundary", r#""04:00""#).unwrap();
        set(conn, "carryover_review_threshold", "3").unwrap();
        set(conn, "day_boundary", r#""05:30""#).unwrap();
        assert_eq!(
            get(conn, "day_boundary").unwrap().as_deref(),
            Some(r#""05:30""#)
        );
        assert_eq!(
            list(conn).unwrap(),
            [
                ("carryover_review_threshold".to_owned(), "3".to_owned()),
                ("day_boundary".to_owned(), r#""05:30""#.to_owned()),
            ]
        );

        assert!(remove(conn, "day_boundary").unwrap());
        assert!(!remove(conn, "day_boundary").unwrap());
        assert_eq!(get(conn, "day_boundary").unwrap(), None);
    }

    #[test]
    fn values_must_be_json() {
        let (db, _dir) = open();
        let conn = db.connection();
        assert!(set(conn, "theme", "dark").is_err());
        set(conn, "theme", r#""dark""#).unwrap();
        // 失敗した書き込みは、前の値を変えない
        assert!(set(conn, "theme", "{").is_err());
        assert_eq!(get(conn, "theme").unwrap().as_deref(), Some(r#""dark""#));
    }
}
