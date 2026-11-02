//! day：論理日付ごとのボード（設計書 3章 day）。

use jiff::Timestamp;
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{Time, execute, get_opt_time, get_time, opt_time, query_row};
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
    let n = execute(
        conn,
        "INSERT INTO day (date, tz, created_at) VALUES (?1, ?2, ?3) ON CONFLICT (date) DO NOTHING",
        params![date, tz, Time(now)],
    )?;
    Ok(n == 1)
}

/// その日の行。無ければ `None`。
pub fn get(conn: &Connection, date: LogicalDate) -> Result<Option<Day>, StorageError> {
    Ok(query_row(
        conn,
        &format!("SELECT {COLUMNS} FROM day WHERE date = ?1"),
        [date],
        from_row,
    )
    .optional()?)
}

/// 持ち越しの状態を表す時刻のどれか。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CarryoverMark {
    /// 持ち越し処理を終えた（`carryover_done_at`）。
    Done,
    /// 後回しにした（`carryover_postponed_at`）。
    Postponed,
    /// 後回しにしたものを自動で再表示した（`carryover_reshown_at`）。
    Reshown,
}

impl CarryoverMark {
    /// 列の名前。SQL に埋め込むので、決まった文字列だけを返す。
    const fn column(self) -> &'static str {
        match self {
            Self::Done => "carryover_done_at",
            Self::Postponed => "carryover_postponed_at",
            Self::Reshown => "carryover_reshown_at",
        }
    }
}

/// 持ち越しの状態の時刻を1つだけ書く（`None` なら NULL に戻す）。ほかの2つには触れない。
/// 行が無ければ `false`。
pub fn set_carryover(
    conn: &Connection,
    date: LogicalDate,
    mark: CarryoverMark,
    at: Option<Timestamp>,
) -> Result<bool, StorageError> {
    let n = execute(
        conn,
        &format!("UPDATE day SET {} = ?2 WHERE date = ?1", mark.column()),
        params![date, opt_time(at)],
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
    fn set_carryover_writes_only_the_given_time() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        ensure(conn, d, "Asia/Tokyo", at("2026-11-03T00:00:00Z")).unwrap();

        let postponed = at("2026-11-03T01:00:00Z");
        let reshown = at("2026-11-03T02:00:00Z");
        let done = at("2026-11-03T03:00:00Z");
        assert!(set_carryover(conn, d, CarryoverMark::Postponed, Some(postponed)).unwrap());
        assert!(set_carryover(conn, d, CarryoverMark::Reshown, Some(reshown)).unwrap());
        assert!(set_carryover(conn, d, CarryoverMark::Done, Some(done)).unwrap());
        let day = get(conn, d).unwrap().unwrap();
        assert_eq!(day.carryover_done_at, Some(done));
        assert_eq!(day.carryover_postponed_at, Some(postponed));
        assert_eq!(day.carryover_reshown_at, Some(reshown));

        // NULL に戻せる。ほかの時刻は残る
        assert!(set_carryover(conn, d, CarryoverMark::Postponed, None).unwrap());
        let day = get(conn, d).unwrap().unwrap();
        assert_eq!(day.carryover_done_at, Some(done));
        assert_eq!(day.carryover_postponed_at, None);
        assert_eq!(day.carryover_reshown_at, Some(reshown));
    }

    #[test]
    fn set_carryover_of_a_missing_day_changes_nothing() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        let done = Some(at("2026-11-03T03:00:00Z"));
        assert!(!set_carryover(conn, d, CarryoverMark::Done, done).unwrap());
        assert_eq!(get(conn, d).unwrap(), None);
    }
}
