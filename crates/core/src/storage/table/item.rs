//! item：TODO と障害（設計書 3章 item）。
//!
//! 種類と状態の組み合わせ（障害に `done` を付けないなど）や、持ち越し・分割の決まりは
//! ここでは確かめない。項目の操作（`crate::domain::item`）が確かめる。
//!
//! ただし、持ち越し先（[`carried_to`]）は、データモデル設計書 3章の派生列 `carried_to_id` の
//! 読み取りなので、ここに置く（ボードの読み取りと検索の結果の両方に付けるため）。

use std::collections::HashSet;

use jiff::Timestamp;
use rusqlite::types::ToSql;
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};

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

/// `from` から `to` まで（両端を含む）の日の項目を、日付・並び順（`sort_key`、同じなら id）で返す。
/// 論理削除したものは含めない。
pub fn list_between(
    conn: &Connection,
    from: LogicalDate,
    to: LogicalDate,
) -> Result<Vec<Item>, StorageError> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM item WHERE date BETWEEN ?1 AND ?2 AND deleted_at IS NULL \
         ORDER BY date, sort_key, id"
    ))?;
    let rows = stmt.query_map([from, to], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// `before` より前で、削除されていない項目がある日を、新しい順に `limit` 日まで返す。
pub fn dates_before(
    conn: &Connection,
    before: LogicalDate,
    limit: usize,
) -> Result<Vec<LogicalDate>, StorageError> {
    let mut stmt = conn.prepare_cached(
        "SELECT DISTINCT date FROM item WHERE date < ?1 AND deleted_at IS NULL \
         ORDER BY date DESC LIMIT ?2",
    )?;
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let rows = stmt.query_map(params![before, limit], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// `before` より前で、削除されていない項目がある最も新しい日。無ければ `None`。
pub fn latest_date_before(
    conn: &Connection,
    before: LogicalDate,
) -> Result<Option<LogicalDate>, StorageError> {
    Ok(query_row(
        conn,
        "SELECT MAX(date) FROM item WHERE date < ?1 AND deleted_at IS NULL",
        [before],
        |r| r.get(0),
    )?)
}

/// `carried_from_id` で `id` を指す項目（`id` を持ち越して作った項目）。論理削除したものも含める。
pub fn list_carried_from(conn: &Connection, id: Ulid) -> Result<Vec<Item>, StorageError> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM item WHERE carried_from_id = ?1 ORDER BY id"
    ))?;
    let rows = stmt.query_map([id], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// 項目の持ち越し先の id と日付（MCP・CLI の `carried_to_id`・`carried_to_date`。MCP・CLI 仕様書 3章）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CarriedTo {
    pub id: Ulid,
    pub date: LogicalDate,
}

impl From<&Item> for CarriedTo {
    fn from(item: &Item) -> Self {
        Self {
            id: item.id,
            date: item.date,
        }
    }
}

/// 項目 `id` の持ち越し先（データモデル設計書 3章 `carried_to_id`）：`carried_from_id` で `id` を
/// 指す項目をたどり、最初に行き着いた削除されていない項目。削除されている項目は、さらにそれを
/// 指す項目へたどる。行き着かなければ `None`（持ち越し済みでない）。`skip` の項目（とその先）は
/// たどらない（削除した持ち越し先を戻すときの確かめに使う。ふだんは `None`）。
pub fn carried_to(
    conn: &Connection,
    id: Ulid,
    skip: Option<Ulid>,
) -> Result<Option<Item>, StorageError> {
    let mut seen: HashSet<Ulid> = [Some(id), skip].into_iter().flatten().collect();
    let mut pending = vec![id];
    while let Some(from) = pending.pop() {
        for next in list_carried_from(conn, from)? {
            // 壊れたデータで輪になっていても止まるように、一度見た項目はたどらない
            if !seen.insert(next.id) {
                continue;
            }
            if next.deleted_at.is_none() {
                return Ok(Some(next));
            }
            pending.push(next.id);
        }
    }
    Ok(None)
}

/// 項目の、書き換える欄とその値（アーキテクチャ設計書 4.1「項目の保存」）。`None` の欄には
/// 触れない。NULL になりうる欄は、`Some(None)` で NULL にする。
///
/// 作った後に変わらない欄（`date`・`kind`・`carry_count`・持ち越し元・分割元・`origin`・
/// エージェント・外部課題・`created_at`）と `deleted_at`（削除は [`soft_delete`]、戻すのは
/// [`restore`]）は持たない。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemChanges {
    pub text: Option<String>,
    pub status: Option<ItemStatus>,
    pub close_reason: Option<Option<CloseReason>>,
    pub sort_key: Option<String>,
    pub resolution_note: Option<Option<String>>,
    pub completed_at: Option<Option<Timestamp>>,
}

impl ItemChanges {
    /// 書き換える欄が無いか。
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// [`update_fields`] で書いた後の行：`item` に書き換えを当て、`updated_at` を `now` にしたもの。
    /// 書き換える欄が無ければ `item` のまま。
    pub fn applied_to(&self, item: &Item, now: Timestamp) -> Item {
        if self.is_empty() {
            return item.clone();
        }
        let mut after = item.clone();
        if let Some(v) = &self.text {
            after.text.clone_from(v);
        }
        if let Some(v) = self.status {
            after.status = v;
        }
        if let Some(v) = self.close_reason {
            after.close_reason = v;
        }
        if let Some(v) = &self.sort_key {
            after.sort_key.clone_from(v);
        }
        if let Some(v) = &self.resolution_note {
            after.resolution_note.clone_from(v);
        }
        if let Some(v) = self.completed_at {
            after.completed_at = v;
        }
        after.updated_at = now;
        after
    }
}

/// 並び順のキーだけを書き換える。`updated_at` は変えない（その日のキーの付け直しに使う。並び順は
/// 変わらないので、項目の変更として扱わない。データモデル設計書 3章 item）。
/// 行が無いか、論理削除済みなら何もせず `false`。
pub fn renumber_sort_key(
    conn: &Connection,
    id: Ulid,
    sort_key: &str,
) -> Result<bool, StorageError> {
    let n = execute(
        conn,
        "UPDATE item SET sort_key = ?2 WHERE id = ?1 AND deleted_at IS NULL",
        params![id, sort_key],
    )?;
    Ok(n == 1)
}

/// 行の、`changes` の欄だけを書き換え、`updated_at` に `now` を入れる。ほかの欄には触れないので、
/// 別の欄への書き込み（エージェントの変更など）を、前に読んだ古い値で上書きしない。
///
/// 行が無いか、論理削除済みなら何もせず `false`（削除の前に読んだ値で書いても、削除は
/// 取り消されない）。書き換える欄が無ければ、何も書かずに `false`。
pub fn update_fields(
    conn: &Connection,
    id: Ulid,
    changes: &ItemChanges,
    now: Timestamp,
) -> Result<bool, StorageError> {
    if changes.is_empty() {
        return Ok(false);
    }
    let completed_at = changes.completed_at.map(opt_time);
    let now = Time(now);
    // 書く列と値。?1 は id、?2 は updated_at で、列は ?3 から順に
    let columns: [(&str, Option<&dyn ToSql>); 6] = [
        ("text", changes.text.as_ref().map(|v| v as &dyn ToSql)),
        ("status", changes.status.as_ref().map(|v| v as &dyn ToSql)),
        (
            "close_reason",
            changes.close_reason.as_ref().map(|v| v as &dyn ToSql),
        ),
        (
            "sort_key",
            changes.sort_key.as_ref().map(|v| v as &dyn ToSql),
        ),
        (
            "resolution_note",
            changes.resolution_note.as_ref().map(|v| v as &dyn ToSql),
        ),
        (
            "completed_at",
            completed_at.as_ref().map(|v| v as &dyn ToSql),
        ),
    ];
    let mut sets: Vec<&str> = Vec::new();
    let mut values: Vec<&dyn ToSql> = vec![&id, &now];
    for (column, value) in columns {
        if let Some(value) = value {
            sets.push(column);
            values.push(value);
        }
    }
    let assignments: String = sets
        .iter()
        .enumerate()
        .map(|(i, column)| format!(", {column} = ?{}", i + 3))
        .collect();
    let n = execute(
        conn,
        &format!(
            "UPDATE item SET updated_at = ?2{assignments} WHERE id = ?1 AND deleted_at IS NULL"
        ),
        params_from_iter(values),
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
    fn latest_date_before_skips_deleted_items_and_later_days() {
        let (db, _dir) = open();
        let conn = db.connection();
        let today = date("2026-11-03");
        assert_eq!(latest_date_before(conn, today).unwrap(), None);
        for d in ["2026-10-30", "2026-11-03"] {
            let d = ensure_day(&db, d);
            insert(conn, &todo(d, "x", "a")).unwrap();
        }
        let deleted = todo(ensure_day(&db, "2026-11-01"), "deleted", "a");
        insert(conn, &deleted).unwrap();
        soft_delete(conn, deleted.id, at("2026-11-03T05:00:00Z")).unwrap();
        assert_eq!(
            latest_date_before(conn, today).unwrap(),
            Some(date("2026-10-30"))
        );
        assert_eq!(latest_date_before(conn, date("2026-10-30")).unwrap(), None);
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
    fn update_fields_writes_only_the_given_fields() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = ensure_day(&db, "2026-11-03");
        let item = Item {
            resolution_note: Some("メモ".into()),
            ..todo(d, "before", "a")
        };
        insert(conn, &item).unwrap();

        let t1 = at("2026-11-03T04:00:00Z");
        let changes = ItemChanges {
            status: Some(ItemStatus::Done),
            completed_at: Some(Some(t1)),
            ..ItemChanges::default()
        };
        assert!(update_fields(conn, item.id, &changes, t1).unwrap());
        let after_status = Item {
            status: ItemStatus::Done,
            completed_at: Some(t1),
            updated_at: t1,
            ..item.clone()
        };
        assert_eq!(get(conn, item.id).unwrap(), Some(after_status.clone()));
        assert_eq!(changes.applied_to(&item, t1), after_status);

        // 別の欄だけを書くと、前に書いた欄は残る。NULL にもできる
        let t2 = at("2026-11-03T05:00:00Z");
        let changes = ItemChanges {
            text: Some("after".into()),
            sort_key: Some("b".into()),
            resolution_note: Some(None),
            close_reason: Some(Some(CloseReason::Dropped)),
            ..ItemChanges::default()
        };
        assert!(update_fields(conn, item.id, &changes, t2).unwrap());
        assert_eq!(
            Some(changes.applied_to(&after_status, t2)),
            get(conn, item.id).unwrap()
        );
        assert_eq!(
            get(conn, item.id).unwrap(),
            Some(Item {
                text: "after".into(),
                sort_key: "b".into(),
                resolution_note: None,
                close_reason: Some(CloseReason::Dropped),
                updated_at: t2,
                ..after_status.clone()
            })
        );
        let changes = ItemChanges {
            status: Some(ItemStatus::Open),
            completed_at: Some(None),
            ..ItemChanges::default()
        };
        assert!(update_fields(conn, item.id, &changes, t2).unwrap());
        assert_eq!(get(conn, item.id).unwrap().unwrap().completed_at, None);

        // 書く欄が無ければ何もしない。行が無ければ false
        assert!(!update_fields(conn, item.id, &ItemChanges::default(), t2).unwrap());
        assert!(!update_fields(conn, Ulid::generate(), &changes, t2).unwrap());
    }

    #[test]
    fn renumber_sort_key_keeps_updated_at() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = ensure_day(&db, "2026-11-03");
        let item = todo(d, "x", "a");
        insert(conn, &item).unwrap();
        assert!(renumber_sort_key(conn, item.id, "V").unwrap());
        assert_eq!(
            get(conn, item.id).unwrap(),
            Some(Item {
                sort_key: "V".into(),
                ..item.clone()
            })
        );
        soft_delete(conn, item.id, at("2026-11-03T05:00:00Z")).unwrap();
        assert!(!renumber_sort_key(conn, item.id, "W").unwrap());
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

        // 削除の前に読んだ項目へ書いても、何も変わらない
        let edited = ItemChanges {
            text: Some("after".into()),
            ..ItemChanges::default()
        };
        assert!(!update_fields(conn, stale.id, &edited, at("2026-11-03T06:00:00Z")).unwrap());
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
