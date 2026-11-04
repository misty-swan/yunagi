//! change_log：エージェント・CLI による変更の履歴と取り消し（設計書 3章 change_log）。
//!
//! 何を記録するか（実際に何かを変えた操作だけ）や、取り消しの手順は、変更履歴の課題で
//! ドメイン層に作る。ここでは行を入れる・読む・印を付けるだけ。

use std::collections::HashSet;

use jiff::Timestamp;
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{Time, execute, get_opt_time, get_time, opt_time, query_row, text_enum};
use crate::domain::logical_date::LogicalDate;
use crate::storage::{StorageError, Ulid};

text_enum! {
    /// 操作したもの（`actor`）。
    pub enum Actor {
        Agent => "agent",
        Human => "human",
    }
}

text_enum! {
    /// 対象の種類（`target_type`）。
    pub enum TargetType {
        Item => "item",
        Note => "note",
        Reflection => "reflection",
    }
}

/// 記録の対象（`target_type` と `target_id` の組）。id や日付を型で持つので、
/// 保存する `target_id` は必ず決まった書き方（ULID は大文字、日付は `YYYY-MM-DD`）になる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    /// 項目の id。
    Item(Ulid),
    /// その日のノート。
    Note(LogicalDate),
    /// その日の振り返り。
    Reflection(LogicalDate),
}

impl Target {
    pub const fn target_type(self) -> TargetType {
        match self {
            Self::Item(_) => TargetType::Item,
            Self::Note(_) => TargetType::Note,
            Self::Reflection(_) => TargetType::Reflection,
        }
    }

    /// 保存する `target_id`。
    fn target_id(self) -> String {
        match self {
            Self::Item(id) => id.to_string(),
            Self::Note(date) | Self::Reflection(date) => date.to_string(),
        }
    }

    /// 読んだ `target_type`・`target_id` から作る。`target_id` の列の番号 `idx` は誤りの表示に使う。
    fn from_columns(ty: TargetType, id: &str, idx: usize) -> rusqlite::Result<Self> {
        let invalid = |e: Box<dyn std::error::Error + Send + Sync>| {
            rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, e)
        };
        Ok(match ty {
            TargetType::Item => Self::Item(id.parse().map_err(|e| invalid(Box::new(e)))?),
            TargetType::Note => Self::Note(id.parse().map_err(|e| invalid(Box::new(e)))?),
            TargetType::Reflection => {
                Self::Reflection(id.parse().map_err(|e| invalid(Box::new(e)))?)
            }
        })
    }
}

/// change_log の1行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeLog {
    pub id: Ulid,
    pub actor: Actor,
    /// 操作したエージェント（CLI は `cli` の行）。`actor = human` のときは `None`。
    pub agent_client_id: Option<Ulid>,
    /// `<名詞>.<動詞>` 形式（例：`todo.add`）。
    pub operation: String,
    pub target: Target,
    /// 変更前のスナップショット（JSON）。追加のように変更前が無いときは `None`。
    pub before: Option<String>,
    /// 変更後のスナップショット（JSON）。変更後が無いときは `None`。
    pub after: Option<String>,
    pub created_at: Timestamp,
    pub reverted_at: Option<Timestamp>,
    /// この記録が取り消し操作なら、取り消した元の記録の id。
    pub revert_of_id: Option<Ulid>,
    /// 利用者が変更履歴画面で確認した時刻。
    pub seen_at: Option<Timestamp>,
}

const COLUMNS: &str = "id, actor, agent_client_id, operation, target_type, target_id, before, \
     after, created_at, reverted_at, revert_of_id, seen_at";

fn from_row(row: &Row<'_>) -> rusqlite::Result<ChangeLog> {
    Ok(ChangeLog {
        id: row.get(0)?,
        actor: row.get(1)?,
        agent_client_id: row.get(2)?,
        operation: row.get(3)?,
        target: Target::from_columns(row.get(4)?, &row.get::<_, String>(5)?, 5)?,
        before: row.get(6)?,
        after: row.get(7)?,
        created_at: get_time(row, 8)?,
        reverted_at: get_opt_time(row, 9)?,
        revert_of_id: row.get(10)?,
        seen_at: get_opt_time(row, 11)?,
    })
}

/// 行を入れる。`before`・`after` が JSON として正しくなければ失敗する。
pub fn insert(conn: &Connection, log: &ChangeLog) -> Result<(), StorageError> {
    execute(
        conn,
        &format!(
            "INSERT INTO change_log ({COLUMNS}) VALUES \
             (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"
        ),
        params![
            log.id,
            log.actor,
            log.agent_client_id,
            log.operation,
            log.target.target_type(),
            log.target.target_id(),
            log.before,
            log.after,
            Time(log.created_at),
            opt_time(log.reverted_at),
            log.revert_of_id,
            opt_time(log.seen_at),
        ],
    )?;
    Ok(())
}

/// id の行。無ければ `None`。
pub fn get(conn: &Connection, id: Ulid) -> Result<Option<ChangeLog>, StorageError> {
    Ok(query_row(
        conn,
        &format!("SELECT {COLUMNS} FROM change_log WHERE id = ?1"),
        [id],
        from_row,
    )
    .optional()?)
}

/// 同じ対象の記録を、id の順（おおよそ記録した順。[`Ulid`] を参照）で返す。
pub fn list_by_target(conn: &Connection, target: Target) -> Result<Vec<ChangeLog>, StorageError> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM change_log WHERE target_type = ?1 AND target_id = ?2 ORDER BY id"
    ))?;
    let rows = stmt.query_map(params![target.target_type(), target.target_id()], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// 履歴の一覧の絞り込み（[`list_recent`]）。
#[derive(Debug, Clone, Copy, Default)]
pub struct RecentFilter {
    /// このクライアントの記録だけ（人の取り消しの記録は `agent_client_id` が無いので入らない）。
    pub client: Option<Ulid>,
    /// エージェントによる未確認の変更（`actor = agent` で `seen_at` が NULL）だけ。
    pub unseen_only: bool,
    /// この id より前（古い）の記録だけ（続きを読むときの目印）。
    pub before: Option<Ulid>,
}

/// 一覧の1件。ノート・振り返りの記録は本文が大きいので、`before`・`after` を読まずに空にし、
/// 本文の文字数だけを SQLite に数えさせる。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentRow {
    pub log: ChangeLog,
    /// ノート・振り返りの記録の、変更前・変更後の本文の文字数（項目の記録は `None`）。
    pub before_chars: Option<i64>,
    pub after_chars: Option<i64>,
}

/// 新しい順（id の降順）に、最大 `limit` 件。エージェントの記録と、人の取り消しの記録の両方を含む。
pub fn list_recent(
    conn: &Connection,
    filter: &RecentFilter,
    limit: usize,
) -> Result<Vec<RecentRow>, StorageError> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, actor, agent_client_id, operation, target_type, target_id, \
         CASE WHEN target_type = 'item' THEN before END, \
         CASE WHEN target_type = 'item' THEN after END, \
         created_at, reverted_at, revert_of_id, seen_at, \
         CASE WHEN target_type <> 'item' THEN length(json_extract(before, '$')) END, \
         CASE WHEN target_type <> 'item' THEN length(json_extract(after, '$')) END \
         FROM change_log \
         WHERE (?1 IS NULL OR agent_client_id = ?1) \
         AND (?2 = 0 OR (actor = 'agent' AND seen_at IS NULL)) \
         AND (?3 IS NULL OR id < ?3) \
         ORDER BY id DESC LIMIT ?4",
    )?;
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let rows = stmt.query_map(
        params![
            filter.client,
            i64::from(filter.unseen_only),
            filter.before,
            limit
        ],
        |row| {
            Ok(RecentRow {
                log: from_row(row)?,
                before_chars: row.get(12)?,
                after_chars: row.get(13)?,
            })
        },
    )?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// 取り消した時刻を記録する。行が無いか、すでに取り消し済みなら何もせず `false`
/// （同じ記録を2回取り消したことに気づけるようにするため）。
pub fn set_reverted_at(conn: &Connection, id: Ulid, at: Timestamp) -> Result<bool, StorageError> {
    let n = execute(
        conn,
        "UPDATE change_log SET reverted_at = ?2 WHERE id = ?1 AND reverted_at IS NULL",
        params![id, Time(at)],
    )?;
    Ok(n == 1)
}

/// 記録の `before`・`after` を書き換える（利用者とエージェントが項目を同じ状態にした行き違いで、
/// 記録から欄を外すとき。アーキテクチャ設計書 4.1）。行が無ければ `false`。JSON として正しく
/// なければ失敗する。
pub fn update_snapshots(
    conn: &Connection,
    id: Ulid,
    before: Option<&str>,
    after: Option<&str>,
) -> Result<bool, StorageError> {
    let n = execute(
        conn,
        "UPDATE change_log SET before = ?2, after = ?3 WHERE id = ?1",
        params![id, before, after],
    )?;
    Ok(n == 1)
}

/// 記録を消す（行き違いで、記録から欄を外して何も残らなかったとき。アーキテクチャ設計書 4.1）。
/// 行が無ければ `false`。取り消しの記録（`revert_of_id`）が指している記録は、外部キーのため
/// 消せずに失敗する（消す前に [`is_reverted_by_any`] で確かめる）。
pub fn delete(conn: &Connection, id: Ulid) -> Result<bool, StorageError> {
    let n = execute(conn, "DELETE FROM change_log WHERE id = ?1", [id])?;
    Ok(n == 1)
}

/// 記録 `id` を取り消した記録（`revert_of_id` が `id`）があるか。
pub fn is_reverted_by_any(conn: &Connection, id: Ulid) -> Result<bool, StorageError> {
    Ok(query_row(
        conn,
        "SELECT EXISTS (SELECT 1 FROM change_log WHERE revert_of_id = ?1)",
        [id],
        |r| r.get(0),
    )?)
}

/// 利用者が確認した時刻を記録する。行が無いか、すでに確認済みなら何もせず `false`
/// （最初に確認した時刻を残す）。
pub fn set_seen_at(conn: &Connection, id: Ulid, at: Timestamp) -> Result<bool, StorageError> {
    let n = execute(
        conn,
        "UPDATE change_log SET seen_at = ?2 WHERE id = ?1 AND seen_at IS NULL",
        params![id, Time(at)],
    )?;
    Ok(n == 1)
}

/// エージェントによる未確認の変更の数：`actor = agent` で、`seen_at` が NULL の記録（データモデル設計書
/// 3章 change_log。上部のバーの「◆ 件数」とサイドバーの Agents の件数）。
pub fn count_unseen_by_agents(conn: &Connection) -> Result<u64, StorageError> {
    let n: i64 = query_row(
        conn,
        "SELECT COUNT(*) FROM change_log WHERE actor = ?1 AND seen_at IS NULL",
        [Actor::Agent],
        |r| r.get(0),
    )?;
    // COUNT(*) は 0 以上
    Ok(n.unsigned_abs())
}

/// その日 `date` の項目のうち、エージェントによる未確認の変更（`actor = agent` で `seen_at` が NULL の
/// 記録）がある項目の id（データモデル設計書 3章 change_log。項目の行頭の菱形）。削除した項目も入りうる。
pub fn unseen_items_by_agents(
    conn: &Connection,
    date: LogicalDate,
) -> Result<HashSet<Ulid>, StorageError> {
    let mut stmt = conn.prepare_cached(
        "SELECT DISTINCT change_log.target_id FROM change_log \
         JOIN item ON item.id = change_log.target_id \
         WHERE change_log.target_type = ?1 AND change_log.actor = ?2 \
         AND change_log.seen_at IS NULL AND item.date = ?3",
    )?;
    let rows = stmt.query_map(params![TargetType::Item, Actor::Agent, date], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::table::agent_client::CLI_ID;
    use crate::storage::table::test_support::{at, date, open};

    fn item(n: u128) -> Target {
        Target::Item(Ulid::from_u128(n))
    }

    fn log(target: Target) -> ChangeLog {
        ChangeLog {
            id: Ulid::generate(),
            actor: Actor::Agent,
            agent_client_id: Some(CLI_ID),
            operation: "todo.add".into(),
            target,
            before: None,
            after: Some(r#"{"text":"x"}"#.into()),
            created_at: at("2026-11-03T01:00:00Z"),
            reverted_at: None,
            revert_of_id: None,
            seen_at: None,
        }
    }

    #[test]
    fn insert_get_and_list_by_target() {
        let (db, _dir) = open();
        let conn = db.connection();
        let first = log(item(10));
        let other = log(item(11));
        let revert = ChangeLog {
            actor: Actor::Human,
            agent_client_id: None,
            operation: "agent.revert".into(),
            before: Some(r#"{"text":"x"}"#.into()),
            after: Some(r#"{"deleted_at":"2026-11-03T02:00:00.000Z"}"#.into()),
            revert_of_id: Some(first.id),
            reverted_at: Some(at("2026-11-03T03:00:00Z")),
            seen_at: Some(at("2026-11-03T04:00:00Z")),
            ..log(item(10))
        };
        for l in [&first, &other, &revert] {
            insert(conn, l).unwrap();
        }
        assert_eq!(get(conn, revert.id).unwrap(), Some(revert.clone()));
        assert_eq!(get(conn, Ulid::generate()).unwrap(), None);
        assert_eq!(list_by_target(conn, item(10)).unwrap(), [first, revert]);
        assert_eq!(list_by_target(conn, item(11)).unwrap(), [other]);
        assert!(
            list_by_target(conn, Target::Note(date("2026-11-03")))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn note_targets_use_the_date() {
        let (db, _dir) = open();
        let conn = db.connection();
        let edit = ChangeLog {
            operation: "note.edit".into(),
            before: Some(r#""old body""#.into()),
            after: Some(r#""new body""#.into()),
            ..log(Target::Note(date("2026-11-03")))
        };
        insert(conn, &edit).unwrap();
        assert_eq!(
            list_by_target(conn, Target::Note(date("2026-11-03"))).unwrap(),
            [edit]
        );
    }

    #[test]
    fn marks_reverted_and_seen() {
        let (db, _dir) = open();
        let conn = db.connection();
        let l = log(item(10));
        insert(conn, &l).unwrap();
        assert!(set_reverted_at(conn, l.id, at("2026-11-03T02:00:00Z")).unwrap());
        assert!(set_seen_at(conn, l.id, at("2026-11-03T03:00:00Z")).unwrap());
        // 2回目は何もせず false（最初の時刻が残る）
        assert!(!set_reverted_at(conn, l.id, at("2026-11-03T05:00:00Z")).unwrap());
        assert!(!set_seen_at(conn, l.id, at("2026-11-03T05:00:00Z")).unwrap());
        let saved = get(conn, l.id).unwrap().unwrap();
        assert_eq!(saved.reverted_at, Some(at("2026-11-03T02:00:00Z")));
        assert_eq!(saved.seen_at, Some(at("2026-11-03T03:00:00Z")));
        assert!(!set_reverted_at(conn, Ulid::generate(), at("2026-11-03T02:00:00Z")).unwrap());
        assert!(!set_seen_at(conn, Ulid::generate(), at("2026-11-03T02:00:00Z")).unwrap());
    }

    #[test]
    fn counts_unseen_changes_by_agents_only() {
        let (db, _dir) = open();
        let conn = db.connection();
        assert_eq!(count_unseen_by_agents(conn).unwrap(), 0);
        let seen = log(item(10));
        let unseen = log(item(11));
        // 人の記録（取り消しの操作）は、確認していなくても数えない
        let by_human = ChangeLog {
            actor: Actor::Human,
            agent_client_id: None,
            operation: "agent.revert".into(),
            revert_of_id: Some(seen.id),
            ..log(item(10))
        };
        for l in [
            &seen,
            &unseen,
            &log(Target::Note(date("2026-11-03"))),
            &by_human,
        ] {
            insert(conn, l).unwrap();
        }
        assert_eq!(count_unseen_by_agents(conn).unwrap(), 3);
        set_seen_at(conn, seen.id, at("2026-11-03T03:00:00Z")).unwrap();
        assert_eq!(count_unseen_by_agents(conn).unwrap(), 2);
    }

    #[test]
    fn rejects_invalid_json_and_unknown_agent_clients() {
        let (db, _dir) = open();
        let conn = db.connection();
        let bad_json = ChangeLog {
            after: Some("{not json".into()),
            ..log(item(10))
        };
        assert!(insert(conn, &bad_json).is_err());
        let unknown_client = ChangeLog {
            agent_client_id: Some(Ulid::generate()),
            ..log(item(10))
        };
        assert!(insert(conn, &unknown_client).is_err());
    }

    #[test]
    fn target_ids_are_stored_in_the_canonical_form() {
        let (db, _dir) = open();
        let conn = db.connection();
        let id: Ulid = "01jbz3k8q0abcdefghjkmnpqrs".parse().unwrap();
        let l = log(Target::Item(id));
        insert(conn, &l).unwrap();
        let stored: String = conn
            .query_row(
                "SELECT target_id FROM change_log WHERE id = ?1",
                [l.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, "01JBZ3K8Q0ABCDEFGHJKMNPQRS");
        assert_eq!(list_by_target(conn, Target::Item(id)).unwrap(), [l]);
    }

    #[test]
    fn snapshots_can_be_rewritten_and_records_deleted() {
        let (db, _dir) = open();
        let conn = db.connection();
        let l = log(item(10));
        insert(conn, &l).unwrap();
        assert!(update_snapshots(conn, l.id, Some(r#"{"a":1}"#), None).unwrap());
        let saved = get(conn, l.id).unwrap().unwrap();
        assert_eq!(saved.before.as_deref(), Some(r#"{"a":1}"#));
        assert_eq!(saved.after, None);
        assert!(update_snapshots(conn, l.id, Some("{bad"), None).is_err());
        assert!(!update_snapshots(conn, Ulid::generate(), None, None).unwrap());

        // 取り消しの記録が指している記録は消せない
        let revert = ChangeLog {
            revert_of_id: Some(l.id),
            ..log(item(10))
        };
        insert(conn, &revert).unwrap();
        assert!(is_reverted_by_any(conn, l.id).unwrap());
        assert!(!is_reverted_by_any(conn, revert.id).unwrap());
        assert!(delete(conn, l.id).is_err());
        assert!(delete(conn, revert.id).unwrap());
        assert!(!is_reverted_by_any(conn, l.id).unwrap());
        assert!(delete(conn, l.id).unwrap());
        assert!(!delete(conn, l.id).unwrap());
        assert_eq!(get(conn, l.id).unwrap(), None);
    }

    #[test]
    fn revert_of_id_must_point_at_a_record() {
        let (db, _dir) = open();
        let revert = ChangeLog {
            revert_of_id: Some(Ulid::generate()),
            ..log(item(10))
        };
        assert!(insert(db.connection(), &revert).is_err());
    }
}
