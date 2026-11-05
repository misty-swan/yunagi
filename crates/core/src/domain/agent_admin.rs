//! エージェント（クライアント）の一覧と、権限（スコープ）の変更（要件 A-08。MCP・CLI 仕様書 2章・
//! 4.4・7章）。
//!
//! 利用者が行う管理の操作で、MCP には出さない（エージェントが自分の権限を変えられないように
//! するため）。エージェントの書き込みと違い、呼び出し元のスコープは確かめない（`read` を外した
//! クライアントがあっても、管理の操作は使える）。確認（アプリの設定画面か、対話端末）は呼び出し側の
//! 責務で、ここは確認を済ませた後の変更を行う（確認は防御の境界ではない。MCP・CLI 仕様書 2章）。
//!
//! 権限の変更は人の操作なので、change_log には記録しない（データモデル設計書 change_log）。

use rusqlite::Connection;

use crate::storage::table::agent_client::{self, AgentClient, Scope};
use crate::storage::{StorageError, in_read_transaction, in_transaction};

/// 登録されているクライアントを、登録した順（固定の `cli`・`unknown` が先）で返す。
pub fn list_clients(conn: &Connection) -> Result<Vec<AgentClient>, StorageError> {
    agent_client::list(conn)
}

/// 登録されているクライアントと、消せるか。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientEntry {
    pub client: AgentClient,
    /// 消せるか：固定の `cli`・`unknown` ではなく、変更履歴にも項目にも指されていない。
    pub removable: bool,
}

/// 登録されているクライアントを、消せるかを添えて返す（登録した順）。
pub fn list_entries(conn: &Connection) -> Result<Vec<ClientEntry>, StorageError> {
    in_read_transaction(conn, || {
        let used = agent_client::referenced_ids(conn)?;
        Ok(agent_client::list(conn)?
            .into_iter()
            .map(|client| ClientEntry {
                removable: is_removable(&client, &used),
                client,
            })
            .collect())
    })
}

fn is_removable(client: &AgentClient, used: &[crate::storage::Ulid]) -> bool {
    client.id != agent_client::CLI_ID
        && client.id != agent_client::UNKNOWN_ID
        && !used.contains(&client.id)
}

/// 登録を消せない理由。どれも何も書いていない。
#[derive(Debug)]
pub enum RemoveError {
    /// その識別子のクライアントが無い。
    NotFound,
    /// 固定の `cli`・`unknown`。
    Fixed,
    /// 変更履歴か項目が指している（履歴の表示に名前が要る）。
    InUse,
    Storage(StorageError),
}

impl std::fmt::Display for RemoveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("the client was not found"),
            Self::Fixed => f.write_str("the fixed client cannot be removed"),
            Self::InUse => f.write_str("the client is referenced by the change history"),
            Self::Storage(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for RemoveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(e) => Some(e),
            _ => None,
        }
    }
}

impl From<StorageError> for RemoveError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

impl From<rusqlite::Error> for RemoveError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Storage(e.into())
    }
}

/// 登録を消す（登録の数が上限に達したときの整理。MCP・CLI 仕様書 9章の決定）。消せるのは、固定の
/// `cli`・`unknown` ではなく、変更履歴にも項目にも指されていない登録だけ。指す記録の確認と削除は
/// 同じトランザクションで行う。消した識別子が再び接続すると、`read` だけで登録し直される。
///
/// 画面（設定のエージェント区分）だけの口で、ローカル API・CLI には出さない（エージェントが自分の
/// 登録を消して、権限を `read` に戻させないため）。
pub fn remove_client(conn: &Connection, name: &str) -> Result<(), RemoveError> {
    in_transaction(conn, || {
        let Some(client) = agent_client::get_by_name(conn, name)? else {
            return Err(RemoveError::NotFound);
        };
        if client.id == agent_client::CLI_ID || client.id == agent_client::UNKNOWN_ID {
            return Err(RemoveError::Fixed);
        }
        if agent_client::is_referenced(conn, client.id)? {
            return Err(RemoveError::InUse);
        }
        agent_client::delete(conn, client.id)?;
        Ok(())
    })
}

/// 権限の変更の引数の誤り（`invalid_argument`）。何も書いていない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeArgument {
    /// 足すスコープも外すスコープも無い。
    Nothing,
    /// 同じスコープを、足すことと外すことの両方に指定した。
    Both(Scope),
    /// 識別子のない接続（`unknown`）に、`read` より多くのスコープを足そうとした。名乗らないすべての
    /// 接続が書けるようになってしまうため、`read` 以外は足せない（MCP・CLI 仕様書 2章）。
    UnknownWrite(Scope),
}

impl std::fmt::Display for ScopeArgument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Nothing => f.write_str("no scope to allow or deny was given"),
            Self::Both(scope) => write!(f, "the scope {scope} is both allowed and denied"),
            Self::UnknownWrite(scope) => write!(
                f,
                "the unknown client can only have the read scope, not {scope}"
            ),
        }
    }
}

impl std::error::Error for ScopeArgument {}

/// 権限の変更が失敗した理由。どれも何も書いていない。
#[derive(Debug)]
pub enum ScopeError {
    /// その識別子のクライアントが無い（初めての識別子は、接続したときに登録される。権限の変更では
    /// 登録しない）。
    NotFound,
    InvalidArgument(ScopeArgument),
    /// 確認の画面を見せた後に、そのクライアントのスコープが変わった。
    Changed,
    Storage(StorageError),
}

impl std::fmt::Display for ScopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("the client was not found"),
            Self::Changed => f.write_str("the scopes changed after they were shown"),
            Self::InvalidArgument(e) => e.fmt(f),
            Self::Storage(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ScopeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidArgument(e) => Some(e),
            Self::Storage(e) => Some(e),
            Self::NotFound | Self::Changed => None,
        }
    }
}

impl From<StorageError> for ScopeError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

impl From<rusqlite::Error> for ScopeError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Storage(e.into())
    }
}

/// 権限の変更の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeChange {
    /// 変えたクライアント（変えた後）。
    pub client: AgentClient,
    /// 変える前のスコープ（[`Scope::ALL`] の順）。
    pub before: Vec<Scope>,
    /// 実際に変えたか。足すスコープがすでにあり、外すスコープがすでに無いときは `false`（何も書かない）。
    pub updated: bool,
}

/// 識別子 `name` のクライアントの権限を、`allow` を足し、`deny` を外して変える。結果の並びは
/// [`Scope::ALL`] の順。何も変わらなければ書かない。
///
/// `read` も外せる（外すと、そのクライアントは何も読めない。MCP・CLI 仕様書 2章の既定は `read` のみで、
/// 利用者が決める）。`unknown` に足せるのは `read` だけ（[`ScopeArgument::UnknownWrite`]）。
///
/// `expect` は、確認の画面で見せた変更前のスコープ。いまのスコープと違えば（確認を待つ間に別の
/// ところで変わった）、書かずに [`ScopeError::Changed`]。
pub fn change_scopes(
    conn: &Connection,
    name: &str,
    allow: &[Scope],
    deny: &[Scope],
    expect: Option<&[Scope]>,
) -> Result<ScopeChange, ScopeError> {
    if allow.is_empty() && deny.is_empty() {
        return Err(ScopeError::InvalidArgument(ScopeArgument::Nothing));
    }
    if let Some(&both) = allow.iter().find(|s| deny.contains(s)) {
        return Err(ScopeError::InvalidArgument(ScopeArgument::Both(both)));
    }
    in_transaction(conn, || {
        let Some(mut client) = agent_client::get_by_name(conn, name)? else {
            return Err(ScopeError::NotFound);
        };
        if client.id == agent_client::UNKNOWN_ID
            && let Some(scope) = forbidden_scope(&client.name, allow)
        {
            return Err(ScopeError::InvalidArgument(ScopeArgument::UnknownWrite(
                scope,
            )));
        }
        let before = ordered(&client.scopes);
        if expect.is_some_and(|e| ordered(e) != before) {
            return Err(ScopeError::Changed);
        }
        let after = apply_scopes(&before, allow, deny);
        let updated = after != before;
        if updated {
            agent_client::update_scopes(conn, client.id, &after)?;
            client.scopes.clone_from(&after);
        }
        Ok(ScopeChange {
            client,
            before,
            updated,
        })
    })
}

/// `before` のスコープに `allow` を足し、`deny` を外した結果（[`Scope::ALL`] の順）。確認の前に
/// 見せる「変えた後」を、書き込みと同じ計算で求めるために公開している。
pub fn apply_scopes(before: &[Scope], allow: &[Scope], deny: &[Scope]) -> Vec<Scope> {
    Scope::ALL
        .into_iter()
        .filter(|s| (before.contains(s) || allow.contains(s)) && !deny.contains(s))
        .collect()
}

/// 識別子 `name` のクライアントに、`allow` の中で足せないスコープ（`unknown` に `read` 以外）があれば、
/// それ。確認の前に断るために公開している（[`change_scopes`] も同じ判定）。
pub fn forbidden_scope(name: &str, allow: &[Scope]) -> Option<Scope> {
    (name == agent_client::UNKNOWN_NAME)
        .then(|| allow.iter().copied().find(|s| *s != Scope::Read))
        .flatten()
}

/// スコープを [`Scope::ALL`] の順に並べる。
fn ordered(scopes: &[Scope]) -> Vec<Scope> {
    Scope::ALL
        .into_iter()
        .filter(|s| scopes.contains(s))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::{ClientLookup, client_for};
    use crate::storage::table::agent_client::{CLI_ID, UNKNOWN_ID};
    use crate::storage::table::test_support::{at, open};

    fn change(
        conn: &Connection,
        name: &str,
        allow: &[Scope],
        deny: &[Scope],
    ) -> Result<ScopeChange, ScopeError> {
        change_scopes(conn, name, allow, deny, None)
    }

    #[test]
    fn a_change_is_refused_when_the_scopes_moved_after_they_were_shown() {
        let (db, _dir) = open();
        let conn = db.connection();
        let shown = [Scope::Read];
        change_scopes(conn, "cli", &[Scope::Blocker], &[], None).unwrap();
        assert!(matches!(
            change_scopes(conn, "cli", &[Scope::Todo], &[], Some(&shown)),
            Err(ScopeError::Changed)
        ));
        assert_eq!(
            agent_client::get(conn, CLI_ID).unwrap().unwrap().scopes,
            [Scope::Read, Scope::Blocker]
        );
        let now = [Scope::Blocker, Scope::Read];
        assert!(change_scopes(conn, "cli", &[Scope::Todo], &[], Some(&now)).is_ok());
    }

    #[test]
    fn scopes_are_added_and_removed_in_a_fixed_order() {
        let (db, _dir) = open();
        let conn = db.connection();
        let c = change(conn, "cli", &[Scope::NoteEdit, Scope::Todo], &[]).unwrap();
        assert!(c.updated);
        assert_eq!(c.before, [Scope::Read]);
        assert_eq!(c.client.scopes, [Scope::Read, Scope::Todo, Scope::NoteEdit]);
        assert_eq!(
            agent_client::get(conn, CLI_ID).unwrap().unwrap().scopes,
            [Scope::Read, Scope::Todo, Scope::NoteEdit]
        );
        let c = change(conn, "cli", &[Scope::Blocker], &[Scope::Todo]).unwrap();
        assert_eq!(c.before, [Scope::Read, Scope::Todo, Scope::NoteEdit]);
        assert_eq!(
            c.client.scopes,
            [Scope::Read, Scope::Blocker, Scope::NoteEdit]
        );
        // read も外せる
        let c = change(conn, "cli", &[], &[Scope::Read]).unwrap();
        assert_eq!(c.client.scopes, [Scope::Blocker, Scope::NoteEdit]);
    }

    #[test]
    fn a_change_that_changes_nothing_is_not_written() {
        let (db, _dir) = open();
        let conn = db.connection();
        let c = change(conn, "cli", &[Scope::Read], &[Scope::Todo]).unwrap();
        assert!(!c.updated);
        assert_eq!(c.before, c.client.scopes);
    }

    #[test]
    fn mistakes_are_refused_without_writing() {
        let (db, _dir) = open();
        let conn = db.connection();
        assert!(matches!(
            change(conn, "cli", &[], &[]),
            Err(ScopeError::InvalidArgument(ScopeArgument::Nothing))
        ));
        assert!(matches!(
            change(conn, "cli", &[Scope::Todo], &[Scope::Todo]),
            Err(ScopeError::InvalidArgument(ScopeArgument::Both(
                Scope::Todo
            )))
        ));
        assert!(matches!(
            change(conn, "nobody", &[Scope::Todo], &[]),
            Err(ScopeError::NotFound)
        ));
        // 識別子のない接続には、書き込みのスコープを足せない（read は足せる）
        assert!(matches!(
            change(conn, "unknown", &[Scope::Todo], &[]),
            Err(ScopeError::InvalidArgument(ScopeArgument::UnknownWrite(
                Scope::Todo
            )))
        ));
        assert!(change(conn, "unknown", &[Scope::Read], &[Scope::Todo]).is_ok());
        assert!(change(conn, "unknown", &[], &[Scope::Read]).is_ok());
        assert_eq!(
            agent_client::get(conn, UNKNOWN_ID).unwrap().unwrap().scopes,
            []
        );
        assert_eq!(
            agent_client::get(conn, CLI_ID).unwrap().unwrap().scopes,
            [Scope::Read]
        );
    }

    #[test]
    fn only_registrations_nothing_refers_to_can_be_removed() {
        use crate::domain::agent::test_support::grant;
        use crate::domain::agent::{Agent, NO_UI};
        use crate::domain::item;
        use crate::storage::table::test_support::date;
        let (db, _dir) = open();
        let conn = db.connection();
        let now = at("2026-11-04T00:00:00Z");
        let id_of = |name: &str| match client_for(conn, Some(name), now).unwrap() {
            ClientLookup::Registered(id) => id,
            other => panic!("{other:?}"),
        };
        let idle = id_of("idle");
        let worker = id_of("worker");
        // 履歴のある登録（項目を足した）は消せない
        grant(conn, worker, &[Scope::Todo]);
        let agent = Agent {
            client_id: worker,
            ui: &NO_UI,
        };
        let tz = jiff::tz::TimeZone::get("Asia/Tokyo").unwrap();
        item::add_todo(conn, &agent, date("2026-11-04"), "調べる", &tz, now).unwrap();

        let entries = list_entries(conn).unwrap();
        let flags: Vec<(&str, bool)> = entries
            .iter()
            .map(|e| (e.client.name.as_str(), e.removable))
            .collect();
        assert_eq!(
            flags,
            [
                ("cli", false),
                ("unknown", false),
                ("idle", true),
                ("worker", false)
            ]
        );

        assert!(matches!(
            remove_client(conn, "cli"),
            Err(RemoveError::Fixed)
        ));
        assert!(matches!(
            remove_client(conn, "unknown"),
            Err(RemoveError::Fixed)
        ));
        assert!(matches!(
            remove_client(conn, "worker"),
            Err(RemoveError::InUse)
        ));
        assert!(matches!(
            remove_client(conn, "nobody"),
            Err(RemoveError::NotFound)
        ));
        remove_client(conn, "idle").unwrap();
        assert!(agent_client::get(conn, idle).unwrap().is_none());
        assert!(matches!(
            remove_client(conn, "idle"),
            Err(RemoveError::NotFound)
        ));
        // 消した識別子は、次に接続したとき read だけで登録し直される
        assert!(matches!(
            client_for(conn, Some("idle"), now).unwrap(),
            ClientLookup::Registered(_)
        ));
        let again = agent_client::get_by_name(conn, "idle").unwrap().unwrap();
        assert_eq!(again.scopes, [Scope::Read]);
    }

    #[test]
    fn registered_clients_are_listed_and_can_be_changed() {
        let (db, _dir) = open();
        let conn = db.connection();
        let now = at("2026-11-04T00:00:00Z");
        assert!(matches!(
            client_for(conn, Some("claude-code"), now).unwrap(),
            ClientLookup::Registered(_)
        ));
        let names: Vec<String> = list_clients(conn)
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, ["cli", "unknown", "claude-code"]);
        let c = change(conn, "claude-code", &[Scope::Todo, Scope::Append], &[]).unwrap();
        assert_eq!(c.client.scopes, [Scope::Read, Scope::Todo, Scope::Append]);
    }
}
