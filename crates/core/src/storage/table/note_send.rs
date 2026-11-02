//! note_send：ノートの行を TODO・障害へ送った記録（設計書 3章 note_send）。

use jiff::Timestamp;
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{Time, execute, get_time, query_row};
use crate::domain::logical_date::LogicalDate;
use crate::storage::{StorageError, Ulid};

/// note_send の1行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteSend {
    pub id: Ulid,
    /// ノートの日付。
    pub date: LogicalDate,
    /// 送った時点の行の本文。
    pub line_text: String,
    /// 行の位置が変わっても追えるようにするための指紋。
    pub line_fingerprint: String,
    /// 送り先の項目。
    pub item_id: Ulid,
    pub created_at: Timestamp,
}

const COLUMNS: &str = "id, date, line_text, line_fingerprint, item_id, created_at";

fn from_row(row: &Row<'_>) -> rusqlite::Result<NoteSend> {
    Ok(NoteSend {
        id: row.get(0)?,
        date: row.get(1)?,
        line_text: row.get(2)?,
        line_fingerprint: row.get(3)?,
        item_id: row.get(4)?,
        created_at: get_time(row, 5)?,
    })
}

/// 行を入れる。その日の day の行と、送り先の項目が先に要る（外部キー）。
pub fn insert(conn: &Connection, send: &NoteSend) -> Result<(), StorageError> {
    execute(
        conn,
        &format!("INSERT INTO note_send ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6)"),
        params![
            send.id,
            send.date,
            send.line_text,
            send.line_fingerprint,
            send.item_id,
            Time(send.created_at),
        ],
    )?;
    Ok(())
}

/// id の行。無ければ `None`。
pub fn get(conn: &Connection, id: Ulid) -> Result<Option<NoteSend>, StorageError> {
    Ok(query_row(
        conn,
        &format!("SELECT {COLUMNS} FROM note_send WHERE id = ?1"),
        [id],
        from_row,
    )
    .optional()?)
}

/// その日のノートから送った記録を、id の順（おおよそ送った順。[`Ulid`] を参照）で返す。
pub fn list_by_date(conn: &Connection, date: LogicalDate) -> Result<Vec<NoteSend>, StorageError> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM note_send WHERE date = ?1 ORDER BY id"
    ))?;
    let rows = stmt.query_map([date], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// その項目へ送った記録を、id の順（おおよそ送った順。[`Ulid`] を参照）で返す。
pub fn list_by_item(conn: &Connection, item_id: Ulid) -> Result<Vec<NoteSend>, StorageError> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM note_send WHERE item_id = ?1 ORDER BY id"
    ))?;
    let rows = stmt.query_map([item_id], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::table::item::{self, Item, ItemKind, ItemStatus, Origin};
    use crate::storage::table::test_support::{at, ensure_day, open};

    fn insert_item(db: &crate::storage::Database, date: LogicalDate) -> Ulid {
        let now = at("2026-11-03T01:00:00Z");
        let item = Item {
            id: Ulid::generate(),
            date,
            kind: ItemKind::Todo,
            text: "x".into(),
            status: ItemStatus::Open,
            close_reason: None,
            sort_key: "a".into(),
            carry_count: 0,
            carried_from_id: None,
            split_from_id: None,
            resolution_note: None,
            origin: Origin::Note,
            agent_client_id: None,
            external_ref_id: None,
            completed_at: None,
            created_at: now,
            updated_at: now,
            deleted_at: None,
        };
        item::insert(db.connection(), &item).unwrap();
        item.id
    }

    fn send(date: LogicalDate, item_id: Ulid, line: &str) -> NoteSend {
        NoteSend {
            id: Ulid::generate(),
            date,
            line_text: line.into(),
            line_fingerprint: format!("fp:{line}"),
            item_id,
            created_at: at("2026-11-03T01:00:00.001Z"),
        }
    }

    #[test]
    fn insert_get_and_list() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = ensure_day(&db, "2026-11-03");
        let other = ensure_day(&db, "2026-11-02");
        let first_item = insert_item(&db, d);
        let second_item = insert_item(&db, d);

        let first = send(d, first_item, "- 一行目");
        let second = send(d, second_item, "- 二行目");
        let third = send(d, first_item, "- 一行目をもう一度");
        let elsewhere = send(other, second_item, "- 別の日");
        for s in [&first, &second, &third, &elsewhere] {
            insert(conn, s).unwrap();
        }

        assert_eq!(get(conn, first.id).unwrap(), Some(first.clone()));
        assert_eq!(get(conn, Ulid::generate()).unwrap(), None);
        assert_eq!(
            list_by_date(conn, d).unwrap(),
            [first.clone(), second.clone(), third.clone()]
        );
        assert_eq!(list_by_item(conn, first_item).unwrap(), [first, third]);
        assert_eq!(
            list_by_item(conn, second_item).unwrap(),
            [second, elsewhere]
        );
    }

    #[test]
    fn insert_requires_the_item() {
        let (db, _dir) = open();
        let d = ensure_day(&db, "2026-11-03");
        assert!(insert(db.connection(), &send(d, Ulid::generate(), "x")).is_err());
    }
}
