//! agent_client：接続したエージェントの識別と権限（設計書 3章 agent_client）。
//!
//! `cli` と `unknown` の行は、最初のマイグレーションで固定の id（[`CLI_ID`]・[`UNKNOWN_ID`]）で
//! 作る。そのほかの行は、新しい識別子で初めて接続したときに [`insert`] で作る。

use jiff::Timestamp;
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{format_time, get_opt_time, get_time, opt_time, text_enum};
use crate::storage::{StorageError, Ulid};

/// `cli` クライアントの id（migrations/0001_initial.sql と同じ値）。
pub const CLI_ID: Ulid = Ulid::from_u128(1);
/// `unknown` クライアント（識別子のない MCP 接続）の id（migrations/0001_initial.sql と同じ値）。
pub const UNKNOWN_ID: Ulid = Ulid::from_u128(2);

text_enum! {
    /// 権限（スコープ。MCP・CLI 仕様書 2章）。
    pub enum Scope {
        Read => "read",
        Todo => "todo",
        Blocker => "blocker",
        Append => "append",
        NoteEdit => "note_edit",
    }
}

/// agent_client の1行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentClient {
    pub id: Ulid,
    /// 接続時の識別子（`yunagi mcp --client <name>`）。表の中で重ならない。
    pub name: String,
    /// 許可したスコープ。保存した順に並ぶ。
    pub scopes: Vec<Scope>,
    pub created_at: Timestamp,
    /// 最後に接続した時刻。まだ接続していなければ `None`。
    pub last_seen_at: Option<Timestamp>,
}

const COLUMNS: &str = "id, name, scopes, created_at, last_seen_at";

fn from_row(row: &Row<'_>) -> rusqlite::Result<AgentClient> {
    let scopes: String = row.get(2)?;
    Ok(AgentClient {
        id: row.get(0)?,
        name: row.get(1)?,
        scopes: decode(&scopes).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, e.into())
        })?,
        created_at: get_time(row, 3)?,
        last_seen_at: get_opt_time(row, 4)?,
    })
}

/// `scopes` の JSON（スコープの名前の文字列の配列）を読む。[`encode`] で書いた形のほか、
/// 区切りの前後の空白も受け付ける。スコープの名前にはエスケープの要る文字が無いので、
/// エスケープを含む文字列や、配列でないもの、知らない名前は誤りとする。
fn decode(json: &str) -> Result<Vec<Scope>, String> {
    let invalid = || format!("scopes must be a JSON array of scope names: {json}");
    let inner = json
        .trim()
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or_else(invalid)?
        .trim();
    if inner.is_empty() {
        return Ok(Vec::new());
    }
    inner
        .split(',')
        .map(|item| {
            let name = item
                .trim()
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .filter(|s| !s.contains(['"', '\\']))
                .ok_or_else(invalid)?;
            name.parse().map_err(|e: super::UnknownValue| e.to_string())
        })
        .collect()
}

/// スコープを保存する形（JSON の文字列の配列）にする。スコープの名前は英小文字と `_` だけなので、
/// そのまま引用符で囲めばよい。
fn encode(scopes: &[Scope]) -> String {
    let items: Vec<String> = scopes
        .iter()
        .map(|s| format!("\"{}\"", s.as_str()))
        .collect();
    format!("[{}]", items.join(","))
}

/// 行を入れる。`name` がすでにあれば失敗する（一意制約）。
pub fn insert(conn: &Connection, client: &AgentClient) -> Result<(), StorageError> {
    conn.execute(
        &format!("INSERT INTO agent_client ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5)"),
        params![
            client.id,
            client.name,
            encode(&client.scopes),
            format_time(client.created_at),
            opt_time(client.last_seen_at),
        ],
    )?;
    Ok(())
}

/// id の行。無ければ `None`。
pub fn get(conn: &Connection, id: Ulid) -> Result<Option<AgentClient>, StorageError> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM agent_client WHERE id = ?1"),
        [id],
        from_row,
    )
    .optional()
    .map_err(Into::into)
}

/// 識別子（`name`）の行。無ければ `None`。
pub fn get_by_name(conn: &Connection, name: &str) -> Result<Option<AgentClient>, StorageError> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM agent_client WHERE name = ?1"),
        [name],
        from_row,
    )
    .optional()
    .map_err(Into::into)
}

/// すべての行を、登録した順（id の順。固定の `cli`・`unknown` が先）で返す。
pub fn list(conn: &Connection) -> Result<Vec<AgentClient>, StorageError> {
    let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM agent_client ORDER BY id"))?;
    let rows = stmt.query_map([], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// 許可するスコープを書き換える。行が無ければ `false`。
pub fn update_scopes(conn: &Connection, id: Ulid, scopes: &[Scope]) -> Result<bool, StorageError> {
    let n = conn.execute(
        "UPDATE agent_client SET scopes = ?2 WHERE id = ?1",
        params![id, encode(scopes)],
    )?;
    Ok(n == 1)
}

/// 最後に接続した時刻を記録する。行が無ければ `false`。
pub fn update_last_seen(conn: &Connection, id: Ulid, now: Timestamp) -> Result<bool, StorageError> {
    let n = conn.execute(
        "UPDATE agent_client SET last_seen_at = ?2 WHERE id = ?1",
        params![id, format_time(now)],
    )?;
    Ok(n == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::table::test_support::{at, open};

    #[test]
    fn cli_and_unknown_exist_from_the_start_with_read_only() {
        let (db, _dir) = open();
        let conn = db.connection();
        let clients = list(conn).unwrap();
        let summary: Vec<(Ulid, &str, &[Scope], Option<Timestamp>)> = clients
            .iter()
            .map(|c| (c.id, c.name.as_str(), c.scopes.as_slice(), c.last_seen_at))
            .collect();
        assert_eq!(
            summary,
            [
                (CLI_ID, "cli", &[Scope::Read][..], None),
                (UNKNOWN_ID, "unknown", &[Scope::Read][..], None),
            ]
        );
        assert_eq!(CLI_ID.to_string(), "00000000000000000000000001");
        assert_eq!(UNKNOWN_ID.to_string(), "00000000000000000000000002");
        assert_eq!(get_by_name(conn, "cli").unwrap().unwrap().id, CLI_ID);
        assert_eq!(get(conn, UNKNOWN_ID).unwrap().unwrap().name, "unknown");
    }

    #[test]
    fn fixed_rows_get_the_creation_time_of_the_database() {
        let before = Timestamp::now();
        let (db, _dir) = open();
        let after = Timestamp::now();
        let created = get(db.connection(), CLI_ID).unwrap().unwrap().created_at;
        // SQLite の時刻はミリ秒までなので、前の時刻もミリ秒に切り捨てて比べる
        let before = Timestamp::from_millisecond(before.as_millisecond()).unwrap();
        assert!(
            before <= created && created <= after,
            "{before} {created} {after}"
        );
    }

    #[test]
    fn insert_get_and_update() {
        let (db, _dir) = open();
        let conn = db.connection();
        let client = AgentClient {
            id: Ulid::generate(),
            name: "claude-code".into(),
            scopes: vec![Scope::Read],
            created_at: at("2026-11-03T01:00:00Z"),
            last_seen_at: Some(at("2026-11-03T01:00:00Z")),
        };
        insert(conn, &client).unwrap();
        assert_eq!(
            get_by_name(conn, "claude-code").unwrap(),
            Some(client.clone())
        );
        assert_eq!(get_by_name(conn, "codex").unwrap(), None);
        assert_eq!(get(conn, Ulid::generate()).unwrap(), None);

        let scopes = [
            Scope::Read,
            Scope::Todo,
            Scope::Blocker,
            Scope::Append,
            Scope::NoteEdit,
        ];
        assert!(update_scopes(conn, client.id, &scopes).unwrap());
        assert!(update_last_seen(conn, client.id, at("2026-11-03T02:00:00Z")).unwrap());
        assert_eq!(
            get(conn, client.id).unwrap(),
            Some(AgentClient {
                scopes: scopes.to_vec(),
                last_seen_at: Some(at("2026-11-03T02:00:00Z")),
                ..client.clone()
            })
        );

        // スコープを空にもできる
        assert!(update_scopes(conn, client.id, &[]).unwrap());
        assert!(get(conn, client.id).unwrap().unwrap().scopes.is_empty());

        assert!(!update_scopes(conn, Ulid::generate(), &scopes).unwrap());
        assert!(!update_last_seen(conn, Ulid::generate(), at("2026-11-03T02:00:00Z")).unwrap());
        assert_eq!(list(conn).unwrap().len(), 3);
    }

    #[test]
    fn names_are_unique() {
        let (db, _dir) = open();
        let client = AgentClient {
            id: Ulid::generate(),
            name: "cli".into(),
            scopes: vec![Scope::Read],
            created_at: at("2026-11-03T01:00:00Z"),
            last_seen_at: None,
        };
        assert!(insert(db.connection(), &client).is_err());
    }

    #[test]
    fn scopes_that_are_not_known_strings_are_errors() {
        let (db, _dir) = open();
        let conn = db.connection();
        for bad in [r#"["read","admin"]"#, r#"["read",1]"#, r#"["re\u0061d"]"#] {
            conn.execute(
                "UPDATE agent_client SET scopes = ?1 WHERE id = ?2",
                params![bad, CLI_ID],
            )
            .unwrap();
            assert!(get(conn, CLI_ID).is_err(), "{bad}");
        }
        // JSON の配列でない値は表に入らない
        for bad in ["read", r#""todo""#, r#"{"x":"note_edit"}"#] {
            assert!(
                conn.execute(
                    "UPDATE agent_client SET scopes = ?1 WHERE id = ?2",
                    params![bad, CLI_ID]
                )
                .is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn decode_reads_what_encode_writes_and_tolerates_spaces() {
        let all = [
            Scope::Read,
            Scope::Todo,
            Scope::Blocker,
            Scope::Append,
            Scope::NoteEdit,
        ];
        assert_eq!(decode(&encode(&all)), Ok(all.to_vec()));
        assert_eq!(decode(&encode(&[])), Ok(Vec::new()));
        assert_eq!(
            decode(r#" [ "read" , "todo" ] "#),
            Ok(vec![Scope::Read, Scope::Todo])
        );
        for bad in [
            "",
            "[",
            r#"["read",]"#,
            r#"["read""todo"]"#,
            r#"[read]"#,
            r#""read""#,
        ] {
            assert!(decode(bad).is_err(), "{bad}");
        }
    }
}
