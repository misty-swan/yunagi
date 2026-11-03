//! note と reflection：その日のノート・振り返りの本文（設計書 3章 note / reflection）。
//!
//! 2つの表は同じ列を持つので、どちらの表かを [`TextDoc`] で選んで同じ関数を使う。

use jiff::Timestamp;
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{Time, get_time, query_row};
use crate::domain::logical_date::LogicalDate;
use crate::storage::StorageError;

/// どちらの表か。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextDoc {
    Note,
    Reflection,
}

impl TextDoc {
    /// 表の名前。SQL に埋め込むので、決まった文字列だけを返す。
    const fn table(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Reflection => "reflection",
        }
    }
}

/// note・reflection の1行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextDocRow {
    pub date: LogicalDate,
    pub body: String,
    /// 版の番号。1 から始め、書き込みのたびに 1 増える。
    pub rev: i64,
    pub updated_at: Timestamp,
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<TextDocRow> {
    Ok(TextDocRow {
        date: row.get(0)?,
        body: row.get(1)?,
        rev: row.get(2)?,
        updated_at: get_time(row, 3)?,
    })
}

/// その日の行。無ければ `None`。
pub fn get(
    conn: &Connection,
    doc: TextDoc,
    date: LogicalDate,
) -> Result<Option<TextDocRow>, StorageError> {
    Ok(query_row(
        conn,
        &format!(
            "SELECT date, body, rev, updated_at FROM {} WHERE date = ?1",
            doc.table()
        ),
        [date],
        from_row,
    )
    .optional()?)
}

/// 本文を書き、書いた後の版の番号を返す。行が無ければ版 1 で作り、あれば版を 1 増やす。
/// その日の day の行が先に要る（外部キー）。
///
/// 版の番号による保存の行き違いの確かめ（アーキテクチャ設計書 4.1）と変更履歴の記録は、
/// ドメイン層（`crate::domain::text_doc`）が行う。
pub fn save(
    conn: &Connection,
    doc: TextDoc,
    date: LogicalDate,
    body: &str,
    now: Timestamp,
) -> Result<i64, StorageError> {
    let rev = query_row(
        conn,
        &format!(
            "INSERT INTO {table} (date, body, rev, updated_at) VALUES (?1, ?2, 1, ?3) \
             ON CONFLICT (date) DO UPDATE SET body = excluded.body, rev = {table}.rev + 1, \
             updated_at = excluded.updated_at \
             RETURNING rev",
            table = doc.table()
        ),
        params![date, body, Time(now)],
        |r| r.get(0),
    )?;
    Ok(rev)
}

/// `before` より前で `after` より後（`None` なら下限なし）の日のうち、本文が空白（Unicode の空白文字）
/// 以外の文字を含む、最も新しい日。無ければ `None`。新しい日から順に読み、見つかったら止める。
pub fn latest_nonblank_date(
    conn: &Connection,
    doc: TextDoc,
    before: LogicalDate,
    after: Option<LogicalDate>,
) -> Result<Option<LogicalDate>, StorageError> {
    Ok(nonblank_dates(conn, doc, before, after, 1)?.pop())
}

/// `before` より前で `after` より後（`None` なら下限なし）の日のうち、本文が空白（Unicode の空白文字）
/// 以外の文字を含む日を、新しい順に `limit` 日まで返す。新しい日から順に読み、`limit` 日見つかったら止める。
pub fn nonblank_dates(
    conn: &Connection,
    doc: TextDoc,
    before: LogicalDate,
    after: Option<LogicalDate>,
    limit: usize,
) -> Result<Vec<LogicalDate>, StorageError> {
    let mut found = Vec::new();
    if limit == 0 {
        return Ok(found);
    }
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT date, body FROM {} WHERE date < ?1 AND date > ?2 ORDER BY date DESC",
        doc.table()
    ))?;
    // 下限なしは空の文字列（どの日付より小さい）にして、主キーの範囲で読めるようにする
    let after = after.map_or_else(String::new, |d| d.to_string());
    let mut rows = stmt.query(params![before, after])?;
    while let Some(row) = rows.next()? {
        let body: String = row.get(1)?;
        if !body.trim().is_empty() {
            found.push(row.get(0)?);
            if found.len() == limit {
                break;
            }
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::table::test_support::{at, date, ensure_day, open};

    #[test]
    fn save_creates_at_rev_1_and_then_increments() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = ensure_day(&db, "2026-11-03");
        for doc in [TextDoc::Note, TextDoc::Reflection] {
            assert_eq!(get(conn, doc, d).unwrap(), None);
            assert_eq!(
                save(conn, doc, d, "## 10:00 定例", at("2026-11-03T01:00:00Z")).unwrap(),
                1
            );
            assert_eq!(
                save(conn, doc, d, "二回目", at("2026-11-03T02:00:00Z")).unwrap(),
                2
            );
            assert_eq!(
                get(conn, doc, d).unwrap(),
                Some(TextDocRow {
                    date: d,
                    body: "二回目".into(),
                    rev: 2,
                    updated_at: at("2026-11-03T02:00:00Z"),
                })
            );
        }
    }

    #[test]
    fn note_and_reflection_are_separate() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = ensure_day(&db, "2026-11-03");
        save(conn, TextDoc::Note, d, "note", at("2026-11-03T01:00:00Z")).unwrap();
        assert_eq!(get(conn, TextDoc::Reflection, d).unwrap(), None);
        save(
            conn,
            TextDoc::Reflection,
            d,
            "reflection",
            at("2026-11-03T01:00:00Z"),
        )
        .unwrap();
        assert_eq!(get(conn, TextDoc::Note, d).unwrap().unwrap().body, "note");
        assert_eq!(
            get(conn, TextDoc::Reflection, d).unwrap().unwrap().body,
            "reflection"
        );
    }

    #[test]
    fn latest_nonblank_date_skips_blank_bodies_and_respects_the_range() {
        let (db, _dir) = open();
        let conn = db.connection();
        let t = at("2026-11-03T01:00:00Z");
        for (d, body) in [
            ("2026-10-28", "古い"),
            ("2026-10-30", "メモ"),
            ("2026-10-31", " \n　\t"),
            ("2026-11-03", "今日"),
        ] {
            let d = ensure_day(&db, d);
            save(conn, TextDoc::Note, d, body, t).unwrap();
        }
        let today = date("2026-11-03");
        let latest = |after| latest_nonblank_date(conn, TextDoc::Note, today, after).unwrap();
        assert_eq!(latest(None), Some(date("2026-10-30")));
        assert_eq!(latest(Some(date("2026-10-29"))), Some(date("2026-10-30")));
        assert_eq!(latest(Some(date("2026-10-30"))), None);
        assert_eq!(
            latest_nonblank_date(conn, TextDoc::Reflection, today, None).unwrap(),
            None
        );
    }

    #[test]
    fn save_requires_the_day_row() {
        let (db, _dir) = open();
        let conn = db.connection();
        for doc in [TextDoc::Note, TextDoc::Reflection] {
            assert!(
                save(
                    conn,
                    doc,
                    date("2026-11-03"),
                    "x",
                    at("2026-11-03T01:00:00Z")
                )
                .is_err()
            );
        }
    }
}
