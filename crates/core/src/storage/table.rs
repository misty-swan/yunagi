//! 表ごとの基本の読み書き（docs/design/02-data-model.md 3章）。
//!
//! ここにあるのは1つの表の行を入れる・読む・書き換えるだけの関数で、項目の操作の決まり
//! （持ち越し・分割・件数の数え方・変更履歴の記録など）は持たない。それらはドメイン層が、
//! これらの関数を組み合わせて作る。
//!
//! どの関数も `&Connection` を受け取る。`rusqlite::Transaction` も `Connection` として渡せるので、
//! 複数の表をまたぐ書き込みは、呼び出し側が1つのトランザクションにまとめられる。
//!
//! # 値の保存の形
//!
//! - 時刻（[`Timestamp`]）は UTC の ISO 8601 で、ミリ秒までの決まった桁数
//!   （`2026-11-03T09:15:00.000Z`）。文字列の大小が時刻の前後と一致する。
//!   ミリ秒より細かい部分は切り捨てる（行に入れる時刻を [`now`] や [`truncate_time`] で作れば、
//!   読み戻した値と一致する）。0 年より前の時刻は保存できない。
//! - 日付（[`LogicalDate`]）は `YYYY-MM-DD`。
//! - id（[`super::Ulid`]）は 26 文字の ULID。
//! - JSON の列（設定の値・変更前後のスナップショット）は、JSON の文字列のまま受け渡す。
//!   中身の形は使う側が決める。

pub mod agent_client;
pub mod change_log;
pub mod day;
pub mod item;
pub mod note_send;
pub mod setting;
pub mod text_doc;

use jiff::Timestamp;
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, Type, ValueRef};
use rusqlite::{Connection, Params, Row, ToSql};

use crate::domain::logical_date::LogicalDate;

/// 時刻を保存する形の文字列にする。0〜9999 年の外の時刻は桁数がそろわないので、
/// 保存するときは `Time` を通して範囲を確かめる。
pub fn format_time(t: Timestamp) -> String {
    t.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// ミリ秒より細かい部分を切り捨てた時刻。保存して読み戻した値と同じになる。
pub fn truncate_time(t: Timestamp) -> Timestamp {
    Timestamp::from_millisecond(t.as_millisecond()).expect("truncating keeps the time in range")
}

/// いまの時刻（ミリ秒まで）。行に入れる時刻はこれで作ると、手元の値と保存した値が一致する。
pub fn now() -> Timestamp {
    truncate_time(Timestamp::now())
}

/// 保存できる最も早い時刻（0000-01-01T00:00:00Z）の UNIX 時刻（ミリ秒）。これより前は年の桁数が変わる。
const EARLIEST_MS: i64 = -62_167_219_200_000;

/// 保存する時刻。SQL の引数に渡すと、保存する形の文字列になる。
/// 0 年より前（年が負になる）の時刻は、文字列の大小が時刻の前後と一致しなくなるので誤りにする
/// （jiff の時刻は 9999 年までなので、後ろの側は考えなくてよい）。
struct Time(Timestamp);

impl ToSql for Time {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        if self.0.as_millisecond() < EARLIEST_MS {
            return Err(rusqlite::Error::ToSqlConversionFailure(
                format!("time {} is before year 0 and cannot be stored", self.0).into(),
            ));
        }
        Ok(ToSqlOutput::from(format_time(self.0)))
    }
}

/// NULL になりうる時刻を、保存する形にする。
fn opt_time(t: Option<Timestamp>) -> Option<Time> {
    t.map(Time)
}

/// 行の `idx` 番目の列を時刻として読む。
fn get_time(row: &Row<'_>, idx: usize) -> rusqlite::Result<Timestamp> {
    let s: String = row.get(idx)?;
    parse_time(idx, &s)
}

/// 行の `idx` 番目の列を、NULL になりうる時刻として読む。
fn get_opt_time(row: &Row<'_>, idx: usize) -> rusqlite::Result<Option<Timestamp>> {
    let s: Option<String> = row.get(idx)?;
    s.map(|s| parse_time(idx, &s)).transpose()
}

fn parse_time(idx: usize, s: &str) -> rusqlite::Result<Timestamp> {
    s.parse()
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(idx, Type::Text, Box::new(e)))
}

/// SQL を実行する。解析した文は接続ごとに取っておき、同じ SQL では使い回す。
fn execute(conn: &Connection, sql: &str, params: impl Params) -> rusqlite::Result<usize> {
    conn.prepare_cached(sql)?.execute(params)
}

/// 1行を読む。解析した文は使い回す（[`execute`] と同じ）。
fn query_row<T>(
    conn: &Connection,
    sql: &str,
    params: impl Params,
    f: impl FnOnce(&Row<'_>) -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    conn.prepare_cached(sql)?.query_row(params, f)
}

impl ToSql for LogicalDate {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.to_string()))
    }
}

impl FromSql for LogicalDate {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value
            .as_str()?
            .parse()
            .map_err(|e| FromSqlError::Other(Box::new(e)))
    }
}

/// 決まった文字列のどれかを取る値（`kind` や `status` など）を、列挙型として定義する。
/// データベースには文字列で保存し、知らない文字列を読んだらエラーにする。
macro_rules! text_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident => $text:literal, )+
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $( $(#[$vmeta])* $variant, )+
        }

        impl $name {
            /// 保存する形の文字列。
            pub const fn as_str(self) -> &'static str {
                match self {
                    $( Self::$variant => $text, )+
                }
            }
        }

        impl std::str::FromStr for $name {
            type Err = $crate::storage::table::UnknownValue;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $( $text => Ok(Self::$variant), )+
                    _ => Err($crate::storage::table::UnknownValue {
                        kind: stringify!($name),
                        value: s.to_owned(),
                    }),
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl rusqlite::ToSql for $name {
            fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
                Ok(rusqlite::types::ToSqlOutput::from(self.as_str()))
            }
        }

        impl rusqlite::types::FromSql for $name {
            fn column_result(
                value: rusqlite::types::ValueRef<'_>,
            ) -> rusqlite::types::FromSqlResult<Self> {
                value
                    .as_str()?
                    .parse()
                    .map_err(|e| rusqlite::types::FromSqlError::Other(Box::new(e)))
            }
        }
    };
}
pub(crate) use text_enum;

/// `text_enum!` で定義した型が知らない文字列。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownValue {
    /// 型の名前。
    pub kind: &'static str,
    /// 読んだ文字列。
    pub value: String,
}

impl std::fmt::Display for UnknownValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown {} value: {:?}", self.kind, self.value)
    }
}

impl std::error::Error for UnknownValue {}

#[cfg(test)]
pub(crate) mod test_support {
    //! 表のテストで共通に使うもの。

    use jiff::Timestamp;

    use super::day;
    use crate::domain::logical_date::LogicalDate;
    use std::time::Duration;

    use crate::storage::data_dir::DefaultDir;
    use crate::storage::{Database, DbLock};

    /// 一時フォルダに、マイグレーションを適用したデータベースを作る。
    /// フォルダはデータベースより後に消えるよう、組にして返す。
    pub fn open() -> (Database, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let lock = DbLock::acquire(&DefaultDir::for_test(dir.path()), Duration::ZERO).unwrap();
        let db = Database::open(lock).unwrap();
        (db, dir)
    }

    /// 時刻（ミリ秒まで。保存しても変わらない値）。
    pub fn at(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    pub fn date(s: &str) -> LogicalDate {
        s.parse().unwrap()
    }

    /// その日の day の行を作る（項目などの外部キーのため）。
    pub fn ensure_day(db: &Database, d: &str) -> LogicalDate {
        let d = date(d);
        day::ensure(db.connection(), d, "Asia/Tokyo", at("2026-11-03T00:00:00Z")).unwrap();
        d
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{at, open};
    use super::*;

    #[test]
    fn times_are_stored_in_utc_with_fixed_millisecond_digits() {
        assert_eq!(
            format_time(at("2026-11-03T09:15:00Z")),
            "2026-11-03T09:15:00.000Z"
        );
        assert_eq!(
            format_time(at("2026-11-03T18:15:00.5+09:00")),
            "2026-11-03T09:15:00.500Z"
        );
        // ミリ秒より細かい部分は切り捨てる
        assert_eq!(
            format_time(at("2026-11-03T09:15:00.123987654Z")),
            "2026-11-03T09:15:00.123Z"
        );
    }

    #[test]
    fn now_and_truncate_time_keep_only_milliseconds() {
        let t = at("2026-11-03T09:15:00.123987654Z");
        assert_eq!(truncate_time(t), at("2026-11-03T09:15:00.123Z"));
        assert_eq!(now().subsec_nanosecond() % 1_000_000, 0);
    }

    #[test]
    fn times_before_year_0_are_not_stored() {
        let (db, _dir) = open();
        let conn = db.connection();
        let ok: String = conn
            .query_row("SELECT ?1", [Time(at("0000-01-01T00:00:00Z"))], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(ok, "0000-01-01T00:00:00.000Z");
        let too_early = at("0000-01-01T00:00:00Z") - jiff::SignedDuration::from_millis(1);
        assert!(
            conn.query_row("SELECT ?1", [Time(too_early)], |r| r.get::<_, String>(0))
                .is_err()
        );
    }

    #[test]
    fn sqlite_now_has_the_same_format() {
        // マイグレーションの SQL で入れる時刻（strftime）も、同じ形になる
        let (db, _dir) = open();
        let now: String = db
            .connection()
            .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |r| {
                r.get(0)
            })
            .unwrap();
        let parsed: Timestamp = now.parse().unwrap();
        assert_eq!(format_time(parsed), now);
    }

    /// 列の名前・型・NULL を許すか・主キーか（`PRAGMA table_info` の順）。
    fn columns(conn: &rusqlite::Connection, table: &str) -> Vec<(String, String, bool, bool)> {
        let mut stmt = conn
            .prepare("SELECT name, type, \"notnull\", pk FROM pragma_table_info(?1) ORDER BY cid")
            .unwrap();
        stmt.query_map([table], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                !r.get::<_, bool>(2)?,
                r.get::<_, i64>(3)? > 0,
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
    }

    /// 外部キー（列 → 参照先の表.列）。
    fn foreign_keys(conn: &rusqlite::Connection, table: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT \"from\", \"table\", \"to\" FROM pragma_foreign_key_list(?1) ORDER BY \"from\"")
            .unwrap();
        stmt.query_map([table], |r| {
            Ok(format!(
                "{} -> {}.{}",
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
    }

    /// 主キー以外の一意制約の列。
    fn unique_columns(conn: &rusqlite::Connection, table: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT ii.name FROM pragma_index_list(?1) AS il, pragma_index_info(il.name) AS ii \
                 WHERE il.\"unique\" AND il.origin = 'u' ORDER BY ii.name",
            )
            .unwrap();
        stmt.query_map([table], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    /// 期待する列を短く書く：`"名前 型"`、NULL を許すなら末尾に `?`、主キーなら末尾に `!`。
    fn expect(spec: &[&str]) -> Vec<(String, String, bool, bool)> {
        spec.iter()
            .map(|s| {
                let pk = s.ends_with('!');
                let nullable = s.ends_with('?');
                let s = s.trim_end_matches(['!', '?']);
                let (name, ty) = s.split_once(' ').unwrap();
                (name.to_owned(), ty.to_owned(), nullable, pk)
            })
            .collect()
    }

    #[test]
    fn schema_matches_the_design() {
        // docs/design/02-data-model.md 3章
        let (db, _dir) = open();
        let conn = db.connection();

        let tables: Vec<String> = conn
            // 全文検索の表（search_index と FTS5 が作る表）は storage::search のテストで確かめる
            .prepare(
                "SELECT name FROM sqlite_schema WHERE type = 'table' \
                 AND name NOT LIKE 'search\\_index%' ESCAPE '\\' ORDER BY name",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            tables,
            [
                "agent_client",
                "change_log",
                "day",
                "item",
                "note",
                "note_send",
                "reflection",
                "setting"
            ]
        );

        assert_eq!(
            columns(conn, "day"),
            expect(&[
                "date TEXT!",
                "tz TEXT",
                "carryover_done_at TEXT?",
                "carryover_postponed_at TEXT?",
                "carryover_reshown_at TEXT?",
                "created_at TEXT",
            ])
        );
        assert_eq!(
            columns(conn, "item"),
            expect(&[
                "id TEXT!",
                "date TEXT",
                "kind TEXT",
                "text TEXT",
                "status TEXT",
                "close_reason TEXT?",
                "sort_key TEXT",
                "carry_count INTEGER",
                "carried_from_id TEXT?",
                "split_from_id TEXT?",
                "resolution_note TEXT?",
                "origin TEXT",
                "agent_client_id TEXT?",
                "external_ref_id TEXT?",
                "completed_at TEXT?",
                "created_at TEXT",
                "updated_at TEXT",
                "deleted_at TEXT?",
            ])
        );
        for table in ["note", "reflection"] {
            assert_eq!(
                columns(conn, table),
                expect(&["date TEXT!", "body TEXT", "rev INTEGER", "updated_at TEXT"]),
                "{table}"
            );
        }
        assert_eq!(
            columns(conn, "note_send"),
            expect(&[
                "id TEXT!",
                "date TEXT",
                "line_text TEXT",
                "line_fingerprint TEXT",
                "item_id TEXT",
                "created_at TEXT",
            ])
        );
        assert_eq!(
            columns(conn, "change_log"),
            expect(&[
                "id TEXT!",
                "actor TEXT",
                "agent_client_id TEXT?",
                "operation TEXT",
                "target_type TEXT",
                "target_id TEXT",
                "before TEXT?",
                "after TEXT?",
                "created_at TEXT",
                "reverted_at TEXT?",
                "revert_of_id TEXT?",
                "seen_at TEXT?",
            ])
        );
        assert_eq!(
            columns(conn, "agent_client"),
            expect(&[
                "id TEXT!",
                "name TEXT",
                "scopes TEXT",
                "created_at TEXT",
                "last_seen_at TEXT?",
            ])
        );
        assert_eq!(
            columns(conn, "setting"),
            expect(&["key TEXT!", "value TEXT"])
        );

        assert!(foreign_keys(conn, "day").is_empty());
        assert_eq!(
            foreign_keys(conn, "item"),
            [
                "agent_client_id -> agent_client.id",
                "carried_from_id -> item.id",
                "date -> day.date",
                "split_from_id -> item.id",
            ]
        );
        assert_eq!(foreign_keys(conn, "note"), ["date -> day.date"]);
        assert_eq!(foreign_keys(conn, "reflection"), ["date -> day.date"]);
        assert_eq!(
            foreign_keys(conn, "note_send"),
            ["date -> day.date", "item_id -> item.id"]
        );
        assert_eq!(
            foreign_keys(conn, "change_log"),
            [
                "agent_client_id -> agent_client.id",
                "revert_of_id -> change_log.id",
            ]
        );
        assert!(foreign_keys(conn, "agent_client").is_empty());
        assert!(foreign_keys(conn, "setting").is_empty());

        assert_eq!(unique_columns(conn, "agent_client"), ["name"]);
        for table in [
            "day",
            "item",
            "note",
            "reflection",
            "note_send",
            "change_log",
            "setting",
        ] {
            assert!(unique_columns(conn, table).is_empty(), "{table}");
        }
    }

    #[test]
    fn tables_are_strict_about_types() {
        // STRICT の表は、列の型に直せない値（TEXT の列へのバイト列、INTEGER の列への文字など）を
        // 入れさせない
        let (db, _dir) = open();
        let conn = db.connection();
        assert!(
            conn.execute(
                "INSERT INTO day (date, tz, created_at) VALUES ('2026-11-03', X'00', 'x')",
                []
            )
            .is_err()
        );
        conn.execute(
            "INSERT INTO day (date, tz, created_at) VALUES ('2026-11-03', 'UTC', 'x')",
            [],
        )
        .unwrap();
        assert!(
            conn.execute(
                "INSERT INTO note (date, body, rev, updated_at) VALUES ('2026-11-03', '', 'one', 'x')",
                []
            )
            .is_err()
        );
    }
}
