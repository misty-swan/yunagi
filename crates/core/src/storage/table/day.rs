//! day：論理日付ごとのボード（設計書 3章 day）。

use jiff::Timestamp;
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{format_time, get_opt_time, get_time, opt_time};
use crate::domain::logical_date::LogicalDate;
use crate::storage::StorageError;

/// day の1行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Day {
    pub date: LogicalDate,
    /// 行を作ったときの端末のタイムゾーン（IANA の名前。例：`Asia/Tokyo`）。
    pub tz: String,
    pub carryover_done_at: Option<Timestamp>,
    pub carryover_postponed_at: Option<Timestamp>,
    pub carryover_reshown_at: Option<Timestamp>,
    pub created_at: Timestamp,
}

const COLUMNS: &str =
    "date, tz, carryover_done_at, carryover_postponed_at, carryover_reshown_at, created_at";

fn from_row(row: &Row<'_>) -> rusqlite::Result<Day> {
    Ok(Day {
        date: row.get(0)?,
        tz: row.get(1)?,
        carryover_done_at: get_opt_time(row, 2)?,
        carryover_postponed_at: get_opt_time(row, 3)?,
        carryover_reshown_at: get_opt_time(row, 4)?,
        created_at: get_time(row, 5)?,
    })
}

/// その日の行が無ければ作る（持ち越しの状態はすべて未設定）。作ったら `true`。
/// すでにあれば何も変えずに `false`（`tz` と `created_at` は最初に作ったときのまま）。
pub fn ensure(
    conn: &Connection,
    date: LogicalDate,
    tz: &str,
    now: Timestamp,
) -> Result<bool, StorageError> {
    let n = conn.execute(
        "INSERT INTO day (date, tz, created_at) VALUES (?1, ?2, ?3) ON CONFLICT (date) DO NOTHING",
        params![date, tz, format_time(now)],
    )?;
    Ok(n == 1)
}

/// その日の行。無ければ `None`。
pub fn get(conn: &Connection, date: LogicalDate) -> Result<Option<Day>, StorageError> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM day WHERE date = ?1"),
            [date],
            from_row,
        )
        .optional()?)
}

/// 持ち越しの状態（3つの時刻）を書き換える。行が無ければ `false`。
pub fn update_carryover(
    conn: &Connection,
    date: LogicalDate,
    done_at: Option<Timestamp>,
    postponed_at: Option<Timestamp>,
    reshown_at: Option<Timestamp>,
) -> Result<bool, StorageError> {
    let n = conn.execute(
        "UPDATE day SET carryover_done_at = ?2, carryover_postponed_at = ?3, carryover_reshown_at = ?4
         WHERE date = ?1",
        params![
            date,
            opt_time(done_at),
            opt_time(postponed_at),
            opt_time(reshown_at)
        ],
    )?;
    Ok(n == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::table::test_support::{at, date, open};

    #[test]
    fn ensure_creates_once_and_keeps_the_first_values() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        assert_eq!(get(conn, d).unwrap(), None);

        assert!(ensure(conn, d, "Asia/Tokyo", at("2026-11-03T00:00:00.001Z")).unwrap());
        assert!(!ensure(conn, d, "UTC", at("2026-11-03T05:00:00Z")).unwrap());

        assert_eq!(
            get(conn, d).unwrap(),
            Some(Day {
                date: d,
                tz: "Asia/Tokyo".into(),
                carryover_done_at: None,
                carryover_postponed_at: None,
                carryover_reshown_at: None,
                created_at: at("2026-11-03T00:00:00.001Z"),
            })
        );
    }

    #[test]
    fn update_carryover_writes_the_three_times() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        ensure(conn, d, "Asia/Tokyo", at("2026-11-03T00:00:00Z")).unwrap();

        let postponed = at("2026-11-03T01:00:00Z");
        let reshown = at("2026-11-03T02:00:00Z");
        assert!(update_carryover(conn, d, None, Some(postponed), Some(reshown)).unwrap());
        let day = get(conn, d).unwrap().unwrap();
        assert_eq!(day.carryover_done_at, None);
        assert_eq!(day.carryover_postponed_at, Some(postponed));
        assert_eq!(day.carryover_reshown_at, Some(reshown));

        // NULL に戻せる
        let done = at("2026-11-03T03:00:00Z");
        assert!(update_carryover(conn, d, Some(done), None, None).unwrap());
        let day = get(conn, d).unwrap().unwrap();
        assert_eq!(day.carryover_done_at, Some(done));
        assert_eq!(day.carryover_postponed_at, None);
        assert_eq!(day.carryover_reshown_at, None);
    }

    #[test]
    fn update_carryover_of_a_missing_day_changes_nothing() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        assert!(!update_carryover(conn, d, Some(at("2026-11-03T03:00:00Z")), None, None).unwrap());
        assert_eq!(get(conn, d).unwrap(), None);
    }
}
