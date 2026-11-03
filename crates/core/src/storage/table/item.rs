//! item：TODO と障害（設計書 3章 item）。
//!
//! 種類と状態の組み合わせ（障害に `done` を付けないなど）や、持ち越し・分割の決まりは
//! ここでは確かめない。項目の操作（YUN-23 以降）がドメイン層で確かめる。

use jiff::Timestamp;
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{Time, execute, get_opt_time, get_time, opt_time, query_row, text_enum};
use crate::domain::logical_date::LogicalDate;
use crate::storage::{StorageError, Ulid};

text_enum! {
    /// 項目の種類（`kind`）。
    pub enum ItemKind {
        Todo => "todo",
        Blocker => "blocker",
    }
}

text_enum! {
    /// 項目の状態（`status`）。TODO は `open` / `done` / `dropped`、障害は `open` / `resolved`。
    pub enum ItemStatus {
        Open => "open",
        Done => "done",
        Dropped => "dropped",
        Resolved => "resolved",
    }
}

text_enum! {
    /// `status = dropped` の理由（`close_reason`）。
    pub enum CloseReason {
        /// やめる。
        Dropped => "dropped",
        /// 分割した。
        Split => "split",
        /// 外部ツールへ戻した。
        ReturnedExternal => "returned_external",
    }
}

text_enum! {
    /// 項目を作ったもの（`origin`）。
    pub enum Origin {
        Human => "human",
        Agent => "agent",
        /// ノートから送った。
        Note => "note",
        External => "external",
    }
}

/// item の1行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: Ulid,
    pub date: LogicalDate,
    pub kind: ItemKind,
    pub text: String,
    pub status: ItemStatus,
    pub close_reason: Option<CloseReason>,
    /// 並び順（分数インデックス）。文字列の大小で並べる。
    pub sort_key: String,
    pub carry_count: i64,
    pub carried_from_id: Option<Ulid>,
    pub split_from_id: Option<Ulid>,
    pub resolution_note: Option<String>,
    pub origin: Origin,
    pub agent_client_id: Option<Ulid>,
    /// 外部課題との紐付け（MVP 後。いまは参照先の表が無い）。
    pub external_ref_id: Option<Ulid>,
    pub completed_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub deleted_at: Option<Timestamp>,
}

const COLUMNS: &str = "id, date, kind, text, status, close_reason, sort_key, carry_count, \
     carried_from_id, split_from_id, resolution_note, origin, agent_client_id, external_ref_id, \
     completed_at, created_at, updated_at, deleted_at";

fn from_row(row: &Row<'_>) -> rusqlite::Result<Item> {
    Ok(Item {
        id: row.get(0)?,
        date: row.get(1)?,
        kind: row.get(2)?,
        text: row.get(3)?,
        status: row.get(4)?,
        close_reason: row.get(5)?,
        sort_key: row.get(6)?,
        carry_count: row.get(7)?,
        carried_from_id: row.get(8)?,
        split_from_id: row.get(9)?,
        resolution_note: row.get(10)?,
        origin: row.get(11)?,
        agent_client_id: row.get(12)?,
        external_ref_id: row.get(13)?,
        completed_at: get_opt_time(row, 14)?,
        created_at: get_time(row, 15)?,
        updated_at: get_time(row, 16)?,
        deleted_at: get_opt_time(row, 17)?,
    })
}

/// 行を入れる。その日の day の行が先に要る（外部キー）。
pub fn insert(conn: &Connection, item: &Item) -> Result<(), StorageError> {
    execute(
        conn,
        &format!(
            "INSERT INTO item ({COLUMNS}) VALUES \
             (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)"
        ),
        params![
            item.id,
            item.date,
            item.kind,
            item.text,
            item.status,
            item.close_reason,
            item.sort_key,
            item.carry_count,
            item.carried_from_id,
            item.split_from_id,
            item.resolution_note,
            item.origin,
            item.agent_client_id,
            item.external_ref_id,
            opt_time(item.completed_at),
            Time(item.created_at),
            Time(item.updated_at),
            opt_time(item.deleted_at),
        ],
    )?;
    Ok(())
}

/// id の行。論理削除したものも返す。無ければ `None`。
pub fn get(conn: &Connection, id: Ulid) -> Result<Option<Item>, StorageError> {
    Ok(query_row(
        conn,
        &format!("SELECT {COLUMNS} FROM item WHERE id = ?1"),
        [id],
        from_row,
    )
    .optional()?)
}

/// その日の項目を並び順（`sort_key`、同じなら id）で返す。論理削除したものは含めない。
pub fn list_by_date(conn: &Connection, date: LogicalDate) -> Result<Vec<Item>, StorageError> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM item WHERE date = ?1 AND deleted_at IS NULL ORDER BY sort_key, id"
    ))?;
    let rows = stmt.query_map([date], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// `carried_from_id` で `id` を指す項目（`id` を持ち越して作った項目）。論理削除したものも含める。
pub fn list_carried_from(conn: &Connection, id: Ulid) -> Result<Vec<Item>, StorageError> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM item WHERE carried_from_id = ?1 ORDER BY id"
    ))?;
    let rows = stmt.query_map([id], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// 行の `id`・`date`・`created_at`・`deleted_at` 以外の列を `item` の値で書き換える。
/// 行が無いか、論理削除済みなら何もせず `false`。
///
/// `deleted_at` は書かない（削除は [`soft_delete`]、戻すのは [`restore`]）。削除の前に読んだ
/// 古い値で書いても、削除が取り消されないようにするため。欄ごとの書き込み
/// （アーキテクチャ設計書 4.1）は項目の操作の課題で作る。
pub fn update(conn: &Connection, item: &Item) -> Result<bool, StorageError> {
    let n = execute(
        conn,
        "UPDATE item SET kind = ?2, text = ?3, status = ?4, close_reason = ?5, sort_key = ?6, \
         carry_count = ?7, carried_from_id = ?8, split_from_id = ?9, resolution_note = ?10, \
         origin = ?11, agent_client_id = ?12, external_ref_id = ?13, completed_at = ?14, \
         updated_at = ?15 \
         WHERE id = ?1 AND deleted_at IS NULL",
        params![
            item.id,
            item.kind,
            item.text,
            item.status,
            item.close_reason,
            item.sort_key,
            item.carry_count,
            item.carried_from_id,
            item.split_from_id,
            item.resolution_note,
            item.origin,
            item.agent_client_id,
            item.external_ref_id,
            opt_time(item.completed_at),
            Time(item.updated_at),
        ],
    )?;
    Ok(n == 1)
}

/// 論理削除する（`deleted_at` と `updated_at` に `now` を入れる）。
/// 行が無いか、すでに削除済みなら何もせず `false`。
pub fn soft_delete(conn: &Connection, id: Ulid, now: Timestamp) -> Result<bool, StorageError> {
    let n = execute(
        conn,
        "UPDATE item SET deleted_at = ?2, updated_at = ?2 WHERE id = ?1 AND deleted_at IS NULL",
        params![id, Time(now)],
    )?;
    Ok(n == 1)
}

/// 論理削除を戻す（`deleted_at` を NULL にし、`updated_at` に `now` を入れる）。削除の取り消しに使う。
/// 行が無いか、削除されていなければ何もせず `false`。
pub fn restore(conn: &Connection, id: Ulid, now: Timestamp) -> Result<bool, StorageError> {
    let n = execute(
        conn,
        "UPDATE item SET deleted_at = NULL, updated_at = ?2 WHERE id = ?1 AND deleted_at IS NOT NULL",
        params![id, Time(now)],
    )?;
    Ok(n == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::table::agent_client;
    use crate::storage::table::test_support::{at, date, ensure_day, open};

    fn todo(date: LogicalDate, text: &str, sort_key: &str) -> Item {
        let now = at("2026-11-03T01:00:00Z");
        Item {
            id: Ulid::generate(),
            date,
            kind: ItemKind::Todo,
            text: text.into(),
            status: ItemStatus::Open,
            close_reason: None,
            sort_key: sort_key.into(),
            carry_count: 0,
            carried_from_id: None,
            split_from_id: None,
            resolution_note: None,
            origin: Origin::Human,
            agent_client_id: None,
            external_ref_id: None,
            completed_at: None,
            created_at: now,
            updated_at: now,
            deleted_at: None,
        }
    }

    #[test]
    fn insert_and_get_round_trip_every_column() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = ensure_day(&db, "2026-11-03");
        let source = todo(d, "元", "a0");
        insert(conn, &source).unwrap();

        let item = Item {
            kind: ItemKind::Blocker,
            status: ItemStatus::Dropped,
            close_reason: Some(CloseReason::ReturnedExternal),
            carry_count: 2,
            carried_from_id: Some(source.id),
            split_from_id: Some(source.id),
            resolution_note: Some("解消メモ".into()),
            origin: Origin::Agent,
            agent_client_id: Some(agent_client::CLI_ID),
            external_ref_id: Some(Ulid::generate()),
            completed_at: Some(at("2026-11-03T02:00:00.250Z")),
            updated_at: at("2026-11-03T02:00:00.250Z"),
            deleted_at: Some(at("2026-11-03T03:00:00Z")),
            ..todo(d, "ステージング環境の権限待ち", "a1")
        };
        insert(conn, &item).unwrap();
        assert_eq!(get(conn, item.id).unwrap(), Some(item));
        assert_eq!(get(conn, Ulid::generate()).unwrap(), None);
    }

    #[test]
    fn insert_requires_the_day_row() {
        let (db, _dir) = open();
        let err = insert(db.connection(), &todo(date("2026-11-03"), "x", "a0")).unwrap_err();
        assert!(
            matches!(&err, StorageError::Sqlite(e) if e.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation)),
            "{err:?}"
        );
    }

    #[test]
    fn insert_requires_a_known_agent_client() {
        let (db, _dir) = open();
        let d = ensure_day(&db, "2026-11-03");
        let item = Item {
            agent_client_id: Some(Ulid::generate()),
            ..todo(d, "x", "a0")
        };
        assert!(insert(db.connection(), &item).is_err());
    }

    #[test]
    fn list_by_date_is_sorted_and_skips_deleted_items_and_other_days() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = ensure_day(&db, "2026-11-03");
        let other = ensure_day(&db, "2026-11-02");
        let b = todo(d, "b", "b");
        let a = todo(d, "a", "a");
        let deleted = todo(d, "deleted", "a5");
        insert(conn, &b).unwrap();
        insert(conn, &a).unwrap();
        insert(conn, &deleted).unwrap();
        insert(conn, &todo(other, "other day", "a")).unwrap();
        assert!(soft_delete(conn, deleted.id, at("2026-11-03T05:00:00Z")).unwrap());

        let texts: Vec<String> = list_by_date(conn, d)
            .unwrap()
            .into_iter()
            .map(|i| i.text)
            .collect();
        assert_eq!(texts, ["a", "b"]);
    }

    #[test]
    fn list_carried_from_finds_the_copies() {
        let (db, _dir) = open();
        let conn = db.connection();
        let yesterday = ensure_day(&db, "2026-11-02");
        let today = ensure_day(&db, "2026-11-03");
        let source = todo(yesterday, "元", "a");
        insert(conn, &source).unwrap();
        assert!(list_carried_from(conn, source.id).unwrap().is_empty());

        let carried = Item {
            carry_count: 1,
            carried_from_id: Some(source.id),
            ..todo(today, "元", "a")
        };
        insert(conn, &carried).unwrap();
        assert_eq!(list_carried_from(conn, source.id).unwrap(), [carried]);
    }

    #[test]
    fn update_rewrites_the_row_but_keeps_date_and_created_at() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = ensure_day(&db, "2026-11-03");
        let other = ensure_day(&db, "2026-11-04");
        let item = todo(d, "before", "a");
        insert(conn, &item).unwrap();

        let changed = Item {
            text: "after".into(),
            status: ItemStatus::Done,
            completed_at: Some(at("2026-11-03T04:00:00Z")),
            updated_at: at("2026-11-03T04:00:00Z"),
            date: other,
            created_at: at("2026-11-04T00:00:00Z"),
            deleted_at: Some(at("2026-11-04T00:00:00Z")),
            ..item.clone()
        };
        assert!(update(conn, &changed).unwrap());
        assert_eq!(
            get(conn, item.id).unwrap(),
            Some(Item {
                date: item.date,
                created_at: item.created_at,
                deleted_at: None,
                ..changed
            })
        );
        assert!(!update(conn, &todo(d, "missing", "a")).unwrap());
    }

    #[test]
    fn soft_delete_keeps_the_row_and_happens_once() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = ensure_day(&db, "2026-11-03");
        let item = todo(d, "x", "a");
        insert(conn, &item).unwrap();
        let now = at("2026-11-03T05:00:00Z");
        assert!(soft_delete(conn, item.id, now).unwrap());
        assert!(!soft_delete(conn, item.id, at("2026-11-03T06:00:00Z")).unwrap());

        let deleted = get(conn, item.id).unwrap().unwrap();
        assert_eq!(deleted.deleted_at, Some(now));
        assert_eq!(deleted.updated_at, now);
        assert!(!soft_delete(conn, Ulid::generate(), now).unwrap());
    }

    #[test]
    fn unknown_stored_values_are_errors() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = ensure_day(&db, "2026-11-03");
        let item = todo(d, "x", "a");
        insert(conn, &item).unwrap();
        conn.execute("UPDATE item SET status = 'paused'", [])
            .unwrap();
        assert!(get(conn, item.id).is_err());
    }

    #[test]
    fn enum_texts_match_the_design() {
        let all = [
            ItemKind::Todo.as_str(),
            ItemKind::Blocker.as_str(),
            ItemStatus::Open.as_str(),
            ItemStatus::Done.as_str(),
            ItemStatus::Dropped.as_str(),
            ItemStatus::Resolved.as_str(),
            CloseReason::Dropped.as_str(),
            CloseReason::Split.as_str(),
            CloseReason::ReturnedExternal.as_str(),
            Origin::Human.as_str(),
            Origin::Agent.as_str(),
            Origin::Note.as_str(),
            Origin::External.as_str(),
        ];
        assert_eq!(
            all,
            [
                "todo",
                "blocker",
                "open",
                "done",
                "dropped",
                "resolved",
                "dropped",
                "split",
                "returned_external",
                "human",
                "agent",
                "note",
                "external",
            ]
        );
        assert_eq!(
            "returned_external".parse(),
            Ok(CloseReason::ReturnedExternal)
        );
        assert!("Open".parse::<ItemStatus>().is_err());
    }

    #[test]
    fn update_of_a_deleted_item_does_not_undo_the_delete() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = ensure_day(&db, "2026-11-03");
        let stale = todo(d, "before", "a");
        insert(conn, &stale).unwrap();
        let deleted_at = at("2026-11-03T05:00:00Z");
        assert!(soft_delete(conn, stale.id, deleted_at).unwrap());

        // 削除の前に読んだ値で書いても、何も変わらない
        let edited = Item {
            text: "after".into(),
            ..stale.clone()
        };
        assert!(!update(conn, &edited).unwrap());
        let saved = get(conn, stale.id).unwrap().unwrap();
        assert_eq!(saved.deleted_at, Some(deleted_at));
        assert_eq!(saved.text, "before");
    }

    #[test]
    fn restore_undoes_a_soft_delete() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = ensure_day(&db, "2026-11-03");
        let item = todo(d, "x", "a");
        insert(conn, &item).unwrap();
        assert!(!restore(conn, item.id, at("2026-11-03T05:00:00Z")).unwrap());
        soft_delete(conn, item.id, at("2026-11-03T05:00:00Z")).unwrap();

        let now = at("2026-11-03T06:00:00Z");
        assert!(restore(conn, item.id, now).unwrap());
        let restored = get(conn, item.id).unwrap().unwrap();
        assert_eq!(restored.deleted_at, None);
        assert_eq!(restored.updated_at, now);
        assert_eq!(list_by_date(conn, d).unwrap(), [restored]);
        assert!(!restore(conn, Ulid::generate(), now).unwrap());
    }
}
