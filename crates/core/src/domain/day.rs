//! 日付の行（day）を作る（docs/design/02-data-model.md 3章 day）。

use jiff::Timestamp;
use jiff::tz::TimeZone;
use rusqlite::Connection;

use crate::domain::logical_date::{LogicalDate, time_zone_record};
use crate::storage::StorageError;
use crate::storage::table::day;

/// 論理日付 `date` の day の行が無ければ、端末のタイムゾーン `tz` を記録して作る。作ったら `true`。
/// すでにあれば何も変えずに `false`（`tz` と作成時刻は最初に作ったときのまま）。
///
/// その日に最初の書き込みをする処理（項目・ノート・振り返りの作成、持ち越しの状態の記録）が、
/// 同じトランザクションの中で先に呼ぶ。表示しただけでは呼ばない。
///
/// - `tz` は `TimeZone::try_system()` で得た端末のタイムゾーン。記録する文字列は
///   [`time_zone_record`] で決める（IANA の名前が取れないときの形もそこにある）。
/// - `now` は行の作成時刻で、名前の無いタイムゾーンを UTC からのずれで記録するときの瞬間にも使う。
///   `crate::storage::table::now` で作る（ミリ秒まで）。
pub fn ensure(
    conn: &Connection,
    date: LogicalDate,
    tz: &TimeZone,
    now: Timestamp,
) -> Result<bool, StorageError> {
    day::ensure(conn, date, &time_zone_record(tz, now), now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::logical_date::{DayBoundary, parse_time_zone_record};
    use crate::storage::table::test_support::{at, open};

    #[test]
    fn records_the_time_zone_so_it_reads_back_to_the_same_date() {
        // 記録の形ごとの確認は logical_date のテストにある。ここでは、作った行の tz を読み戻すと、
        // 作った瞬間の論理日付が同じになることを、IANA の名前と、名前の無い POSIX 形式で確かめる
        let b = DayBoundary::DEFAULT;
        let now = at("2026-11-02T19:30:00Z"); // 東京の 11/3 4:30
        for (tz, expected) in [
            (TimeZone::get("Asia/Tokyo").unwrap(), "Asia/Tokyo"),
            (TimeZone::posix("JST-9").unwrap(), "JST-9"),
        ] {
            let (db, _dir) = open();
            let conn = db.connection();
            let today = LogicalDate::at(now, &tz, b);
            assert!(ensure(conn, today, &tz, now).unwrap());

            let row = day::get(conn, today).unwrap().unwrap();
            assert_eq!(row.tz, expected);
            assert_eq!(row.created_at, now);
            let restored = parse_time_zone_record(&row.tz).unwrap();
            assert_eq!(LogicalDate::at(row.created_at, &restored, b), today);
        }
    }

    #[test]
    fn keeps_the_time_zone_of_the_first_write() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d: LogicalDate = "2026-11-03".parse().unwrap();
        let tokyo = TimeZone::get("Asia/Tokyo").unwrap();
        let posix = TimeZone::posix("EST5EDT,M3.2.0,M11.1.0").unwrap();

        assert!(ensure(conn, d, &tokyo, at("2026-11-02T19:30:00Z")).unwrap());
        assert!(!ensure(conn, d, &posix, at("2026-11-03T05:00:00Z")).unwrap());

        let row = day::get(conn, d).unwrap().unwrap();
        assert_eq!(row.tz, "Asia/Tokyo");
        assert_eq!(row.created_at, at("2026-11-02T19:30:00Z"));
    }
}
