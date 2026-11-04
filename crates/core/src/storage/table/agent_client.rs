//! agent_client：接続したエージェントの識別と権限（設計書 3章 agent_client）。
//!
//! `cli` と `unknown` の行は、最初のマイグレーションで固定の id（[`CLI_ID`]・[`UNKNOWN_ID`]）で
//! 作る。そのほかの行は、新しい識別子で初めて接続したときに [`insert`] で作る。

use jiff::Timestamp;
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{Time, execute, get_opt_time, get_time, opt_time, query_row, text_enum};
use crate::storage::{StorageError, Ulid};

/// `cli` クライアントの id（migrations/0001_initial.sql と同じ値）。
pub const CLI_ID: Ulid = Ulid::from_u128(1);
/// `unknown` クライアント（識別子のない MCP 接続）の id（migrations/0001_initial.sql と同じ値）。
pub const UNKNOWN_ID: Ulid = Ulid::from_u128(2);

/// `unknown` クライアントの識別子（migrations/0001_initial.sql と同じ値）。
pub const UNKNOWN_NAME: &str = "unknown";

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

impl Scope {
    /// すべてのスコープ。表示と保存で並べるときの順（MCP・CLI 仕様書 2章の表の順）。
    pub const ALL: [Scope; 5] = [
        Scope::Read,
        Scope::Todo,
        Scope::Blocker,
        Scope::Append,
        Scope::NoteEdit,
    ];
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

/// 読むときの列。`scopes` は SQLite の JSON 関数で読み、要素を区切り文字（U+001F）でつないだ
/// 文字列と、文字列でない要素の数にする。JSON のエスケープなども SQLite が読むので、
/// 表の検査（正しい JSON の配列）を通ったものはすべて読める。1回の問い合わせで済む。
const SELECT_COLUMNS: &str = "id, name, \
     (SELECT group_concat(value, char(31) ORDER BY key) FROM json_each(scopes)), \
     (SELECT count(*) FROM json_each(scopes) WHERE type != 'text'), \
     created_at, last_seen_at";

fn from_row(row: &Row<'_>) -> rusqlite::Result<AgentClient> {
    let joined: Option<String> = row.get(2)?;
    let non_text: i64 = row.get(3)?;
    Ok(AgentClient {
        id: row.get(0)?,
        name: row.get(1)?,
        scopes: decode(joined.as_deref(), non_text).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, e.into())
        })?,
        created_at: get_time(row, 4)?,
        last_seen_at: get_opt_time(row, 5)?,
    })
}

/// [`SELECT_COLUMNS`] で読んだ `scopes` をスコープの並びにする。空の配列なら `joined` は `None`。
/// 文字列でない要素や知らない名前は誤りとする（値の種類を足すときは版の番号を上げるので、
/// 知らない名前は壊れたデータのときにしか現れない。設計書 1.1）。
fn decode(joined: Option<&str>, non_text: i64) -> Result<Vec<Scope>, String> {
    if non_text > 0 {
        return Err("scopes must be an array of strings".into());
    }
    joined
        .into_iter()
        .flat_map(|j| j.split('\u{1f}'))
        .map(|name| name.parse::<Scope>().map_err(|e| e.to_string()))
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
    execute(
        conn,
        &format!("INSERT INTO agent_client ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5)"),
        params![
            client.id,
            client.name,
            encode(&client.scopes),
            Time(client.created_at),
            opt_time(client.last_seen_at),
        ],
    )?;
    Ok(())
}

/// id の行。無ければ `None`。
pub fn get(conn: &Connection, id: Ulid) -> Result<Option<AgentClient>, StorageError> {
    query_row(
        conn,
        &format!("SELECT {SELECT_COLUMNS} FROM agent_client WHERE id = ?1"),
        [id],
        from_row,
    )
    .optional()
    .map_err(Into::into)
}

/// 識別子（`name`）の行。無ければ `None`。
pub fn get_by_name(conn: &Connection, name: &str) -> Result<Option<AgentClient>, StorageError> {
    query_row(
        conn,
        &format!("SELECT {SELECT_COLUMNS} FROM agent_client WHERE name = ?1"),
        [name],
        from_row,
    )
    .optional()
    .map_err(Into::into)
}

/// すべての行を、登録した順（id の順。固定の `cli`・`unknown` が先）で返す。
pub fn list(conn: &Connection) -> Result<Vec<AgentClient>, StorageError> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {SELECT_COLUMNS} FROM agent_client ORDER BY id"
    ))?;
    let rows = stmt.query_map([], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// 許可するスコープを書き換える。行が無ければ `false`。
pub fn update_scopes(conn: &Connection, id: Ulid, scopes: &[Scope]) -> Result<bool, StorageError> {
    let n = execute(
        conn,
        "UPDATE agent_client SET scopes = ?2 WHERE id = ?1",
        params![id, encode(scopes)],
    )?;
    Ok(n == 1)
}

/// 最後に接続した時刻を記録する。行が無ければ `false`。
pub fn update_last_seen(conn: &Connection, id: Ulid, now: Timestamp) -> Result<bool, StorageError> {
    let n = execute(
        conn,
        "UPDATE agent_client SET last_seen_at = ?2 WHERE id = ?1",
        params![id, Time(now)],
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
        for bad in [
            r#"["read","admin"]"#,
            r#"["read",1]"#,
            r#"["read",["todo"]]"#,
        ] {
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
    fn reads_every_valid_json_array_of_scope_names() {
        let (db, _dir) = open();
        let conn = db.connection();
        let all = [
            Scope::Read,
            Scope::Todo,
            Scope::Blocker,
            Scope::Append,
            Scope::NoteEdit,
        ];
        for (json, expected) in [
            (encode(&all), all.to_vec()),
            (encode(&[]), Vec::new()),
            // 空白やエスケープを含む書き方も、SQLite の JSON 関数が読む
            (
                r#" [ "read" , "todo" ] "#.to_owned(),
                vec![Scope::Read, Scope::Todo],
            ),
            (r#"["re\u0061d"]"#.to_owned(), vec![Scope::Read]),
        ] {
            conn.execute(
                "UPDATE agent_client SET scopes = ?1 WHERE id = ?2",
                params![json, CLI_ID],
            )
            .unwrap();
            assert_eq!(
                get(conn, CLI_ID).unwrap().unwrap().scopes,
                expected,
                "{json}"
            );
        }
    }
}
