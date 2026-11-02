//! change_log：エージェント・CLI による変更の履歴と取り消し（設計書 3章 change_log）。
//!
//! 何を記録するか（実際に何かを変えた操作だけ）や、取り消しの手順は、変更履歴の課題で
//! ドメイン層に作る。ここでは行を入れる・読む・印を付けるだけ。

use jiff::Timestamp;
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{format_time, get_opt_time, get_time, opt_time, text_enum};
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

/// change_log の1行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeLog {
    pub id: Ulid,
    pub actor: Actor,
    /// 操作したエージェント（CLI は `cli` の行）。`actor = human` のときは `None`。
    pub agent_client_id: Option<Ulid>,
    /// `<名詞>.<動詞>` 形式（例：`todo.add`）。
    pub operation: String,
    pub target_type: TargetType,
    /// item なら項目の id、note・reflection なら日付（`YYYY-MM-DD`）。
    pub target_id: String,
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
        target_type: row.get(4)?,
        target_id: row.get(5)?,
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
    conn.execute(
        &format!(
            "INSERT INTO change_log ({COLUMNS}) VALUES \
             (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"
        ),
        params![
            log.id,
            log.actor,
            log.agent_client_id,
            log.operation,
            log.target_type,
            log.target_id,
            log.before,
            log.after,
            format_time(log.created_at),
            opt_time(log.reverted_at),
            log.revert_of_id,
            opt_time(log.seen_at),
        ],
    )?;
    Ok(())
}

/// id の行。無ければ `None`。
pub fn get(conn: &Connection, id: Ulid) -> Result<Option<ChangeLog>, StorageError> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM change_log WHERE id = ?1"),
            [id],
            from_row,
        )
        .optional()?)
}

/// 同じ対象の記録を、記録した順（id の順）で返す。
pub fn list_by_target(
    conn: &Connection,
    target_type: TargetType,
    target_id: &str,
) -> Result<Vec<ChangeLog>, StorageError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM change_log WHERE target_type = ?1 AND target_id = ?2 ORDER BY id"
    ))?;
    let rows = stmt.query_map(params![target_type, target_id], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// 取り消した時刻を記録する。行が無ければ `false`。
pub fn set_reverted_at(conn: &Connection, id: Ulid, at: Timestamp) -> Result<bool, StorageError> {
    let n = conn.execute(
        "UPDATE change_log SET reverted_at = ?2 WHERE id = ?1",
        params![id, format_time(at)],
    )?;
    Ok(n == 1)
}

/// 利用者が確認した時刻を記録する。行が無ければ `false`。
pub fn set_seen_at(conn: &Connection, id: Ulid, at: Timestamp) -> Result<bool, StorageError> {
    let n = conn.execute(
        "UPDATE change_log SET seen_at = ?2 WHERE id = ?1",
        params![id, format_time(at)],
    )?;
    Ok(n == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::table::agent_client::CLI_ID;
    use crate::storage::table::test_support::{at, open};

    fn log(target_id: &str) -> ChangeLog {
        ChangeLog {
            id: Ulid::new(),
            actor: Actor::Agent,
            agent_client_id: Some(CLI_ID),
            operation: "todo.add".into(),
            target_type: TargetType::Item,
            target_id: target_id.into(),
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
        let first = log("A");
        let other = log("B");
        let revert = ChangeLog {
            actor: Actor::Human,
            agent_client_id: None,
            operation: "agent.revert".into(),
            before: Some(r#"{"text":"x"}"#.into()),
            after: Some(r#"{"deleted_at":"2026-11-03T02:00:00.000Z"}"#.into()),
            revert_of_id: Some(first.id),
            reverted_at: Some(at("2026-11-03T03:00:00Z")),
            seen_at: Some(at("2026-11-03T04:00:00Z")),
            ..log("A")
        };
        for l in [&first, &other, &revert] {
            insert(conn, l).unwrap();
        }
        assert_eq!(get(conn, revert.id).unwrap(), Some(revert.clone()));
        assert_eq!(get(conn, Ulid::new()).unwrap(), None);
        assert_eq!(
            list_by_target(conn, TargetType::Item, "A").unwrap(),
            [first, revert]
        );
        assert!(
            list_by_target(conn, TargetType::Note, "A")
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
            target_type: TargetType::Note,
            before: Some(r#""old body""#.into()),
            after: Some(r#""new body""#.into()),
            ..log("2026-11-03")
        };
        insert(conn, &edit).unwrap();
        assert_eq!(
            list_by_target(conn, TargetType::Note, "2026-11-03").unwrap(),
            [edit]
        );
    }

    #[test]
    fn marks_reverted_and_seen() {
        let (db, _dir) = open();
        let conn = db.connection();
        let l = log("A");
        insert(conn, &l).unwrap();
        assert!(set_reverted_at(conn, l.id, at("2026-11-03T02:00:00Z")).unwrap());
        assert!(set_seen_at(conn, l.id, at("2026-11-03T03:00:00Z")).unwrap());
        let saved = get(conn, l.id).unwrap().unwrap();
        assert_eq!(saved.reverted_at, Some(at("2026-11-03T02:00:00Z")));
        assert_eq!(saved.seen_at, Some(at("2026-11-03T03:00:00Z")));
        assert!(!set_reverted_at(conn, Ulid::new(), at("2026-11-03T02:00:00Z")).unwrap());
        assert!(!set_seen_at(conn, Ulid::new(), at("2026-11-03T02:00:00Z")).unwrap());
    }

    #[test]
    fn rejects_invalid_json_and_unknown_agent_clients() {
        let (db, _dir) = open();
        let conn = db.connection();
        let bad_json = ChangeLog {
            after: Some("{not json".into()),
            ..log("A")
        };
        assert!(insert(conn, &bad_json).is_err());
        let unknown_client = ChangeLog {
            agent_client_id: Some(Ulid::new()),
            ..log("A")
        };
        assert!(insert(conn, &unknown_client).is_err());
    }
}
