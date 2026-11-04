//! エージェント・CLI からの書き込みの、呼び出し元と権限（MCP・CLI 仕様書 2章・3.2）。
//!
//! ドメイン層の書き込みの関数は、エージェント・CLI から呼ばれるとき [`Agent`] を受け取り、
//! はじめに [`require_scope`] で権限を確かめる（アーキテクチャ設計書 4章 5。どの経路でも
//! 同じ判定を通す）。画面の状態（保存の失敗中か、利用者が入力中か。アーキテクチャ設計書 4.1）は
//! [`UiState`] で受け取る。
//!
//! 読み取り（`get_board`・`open_board`・`get_note`）も同じく、呼び出し元のクライアントを受け取り、
//! はじめに `read` を確かめる（[`read_as`]、失敗は [`ReadError`]。MCP・CLI 仕様書 2章）。

use rusqlite::Connection;

use crate::domain::logical_date::LogicalDate;
use crate::storage::table::agent_client::{self, Scope};
use crate::storage::table::text_doc::TextDoc;
use crate::storage::{StorageError, Ulid};

/// クライアントの識別子（`yunagi mcp --client <name>`）の長さの上限（バイト数）。設定画面と
/// change_log に出るので、長すぎるものは断る。
pub const MAX_CLIENT_NAME_LEN: usize = 64;

/// 識別子 `name` が使える形か：1〜[`MAX_CLIENT_NAME_LEN`] 文字の、英小文字・数字で始まり、英小文字・
/// 数字・`-`・`_`・`.` だけでできたもの（`claude-code`・`codex` など）。
pub fn is_valid_client_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= MAX_CLIENT_NAME_LEN
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'))
}

/// 識別子 `name` の呼び出し元（agent_client）の id を求める。
///
/// - `name` が無い（識別子のない接続）なら `unknown`（[`agent_client::UNKNOWN_ID`]）。
/// - すでに登録された識別子（固定の `cli`・`unknown` を含む）ならその行。
/// - 初めての識別子は `read` だけの権限で登録する（MCP・CLI 仕様書 2章。権限を足すのは利用者）。
///
/// 識別子は名乗るだけのもので、本人であることは確かめない（SECURITY.md 4.2）。使えない形の識別子は
/// `Ok(None)`（何も登録しない）。最後に接続した時刻の更新は [`touch_client`]（失敗しても依頼を
/// 失敗させないよう、別にしてある）。
pub fn client_for(
    conn: &Connection,
    name: Option<&str>,
    now: jiff::Timestamp,
) -> Result<Option<Ulid>, StorageError> {
    let Some(name) = name else {
        return Ok(Some(agent_client::UNKNOWN_ID));
    };
    if !is_valid_client_name(name) {
        return Ok(None);
    }
    if let Some(client) = agent_client::get_by_name(conn, name)? {
        return Ok(Some(client.id));
    }
    let id = Ulid::generate();
    agent_client::insert(
        conn,
        &agent_client::AgentClient {
            id,
            name: name.to_owned(),
            scopes: vec![Scope::Read],
            created_at: now,
            last_seen_at: Some(now),
        },
    )?;
    Ok(Some(id))
}

/// 最後に接続した時刻を、これより新しければ更新しない間隔（依頼のたびに書き込みが増えないように）。
pub const TOUCH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// クライアント `client_id` の最後に接続した時刻を `now` にする。前の時刻から [`TOUCH_INTERVAL`] たって
/// いなければ書かない（設定画面に出す「最後の接続」は分の精度で足り、読み取りのたびに書き込みを
/// 増やさないため）。
pub fn touch_client(
    conn: &Connection,
    client_id: Ulid,
    now: jiff::Timestamp,
) -> Result<(), StorageError> {
    let recent = agent_client::get(conn, client_id)?
        .and_then(|c| c.last_seen_at)
        .is_some_and(|seen| {
            now.duration_since(seen)
                .try_into()
                .is_ok_and(|d: std::time::Duration| d < TOUCH_INTERVAL)
        });
    if !recent {
        agent_client::update_last_seen(conn, client_id, now)?;
    }
    Ok(())
}

/// エージェント・CLI の呼び出し元。
#[derive(Clone, Copy)]
pub struct Agent<'a> {
    /// 操作したエージェント（CLI は `agent_client::CLI_ID`）。
    pub client_id: Ulid,
    /// 画面の状態。アプリの停止中（直接モード）は [`NO_UI`]。
    pub ui: &'a (dyn UiState + Sync),
}

impl std::fmt::Debug for Agent<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Agent")
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

impl Agent<'static> {
    /// 画面の無いとき（直接モード、テスト）の呼び出し元。
    pub fn without_ui(client_id: Ulid) -> Self {
        Self {
            client_id,
            ui: &NO_UI,
        }
    }
}

/// 項目の欄のうち、利用者が画面で入力するもの（アーキテクチャ設計書 4.1「項目の入力中」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemField {
    /// 本文。
    Text,
    /// 障害の解消メモ。
    ResolutionNote,
}

/// ノート・振り返りの守る行（アーキテクチャ設計書 4.1「ノート・振り返りの守る行」）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectedLines {
    /// 行番号を数えた本文（画面が最後に保存した本文）の版の番号。
    pub rev: i64,
    /// 守る行の番号（1始まり。`rev` の本文で数える）。昇順で重なりは無い。
    pub lines: Vec<usize>,
}

/// エージェントの書き込みの判定に使う、画面の状態（アーキテクチャ設計書 4.1）。
///
/// アプリの起動中は、画面から知らされた状態を返すものをアプリ（src-tauri の `ScreenState`）が渡す。
pub trait UiState {
    /// 画面からの保存の書き込みに失敗して、再試行している間か。`true` なら、エージェントの
    /// 書き込みはすべて `storage_unavailable` になる。
    fn save_failing(&self) -> bool;

    /// 利用者が項目 `item_id` の欄 `field` を入力中か（フォーカスがあり、最後の入力から 10 秒以内か
    /// IME の変換中、またはまだ保存していない入力の書き込みを待っている間）。
    fn is_editing(&self, item_id: Ulid, field: ItemField) -> bool;

    /// 日付 `date` のノート・振り返り `doc` の守る行。エディタにフォーカスが無く、まだ保存していない
    /// 入力も無いとき（画面が知らせていないとき）は `None`。
    ///
    /// `edit_note` の判定（守る行への書き換えと、版が古いときの `conflict`）に使う
    /// （`crate::domain::text_doc::edit_note`）。
    fn protected_lines(&self, doc: TextDoc, date: LogicalDate) -> Option<ProtectedLines>;
}

/// 画面の無いときの状態：保存の失敗も入力中も無い。
#[derive(Debug, Clone, Copy, Default)]
pub struct NoUi;

/// [`NoUi`] の値。
pub static NO_UI: NoUi = NoUi;

impl UiState for NoUi {
    fn save_failing(&self) -> bool {
        false
    }

    fn is_editing(&self, _: Ulid, _: ItemField) -> bool {
        false
    }

    fn protected_lines(&self, _: TextDoc, _: LogicalDate) -> Option<ProtectedLines> {
        None
    }
}

/// 許可されていない（`permission_denied`）。何も読み書きしていない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionDenied {
    /// 操作に必要なスコープ（エラーの `required_scope`）。
    pub required_scope: Scope,
}

impl std::fmt::Display for PermissionDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the agent is not allowed to use the {} scope",
            self.required_scope
        )
    }
}

impl std::error::Error for PermissionDenied {}

/// 画面の保存の書き込みが失敗して再試行している（`storage_unavailable`。アーキテクチャ設計書 4.1）。
/// 何も書いていない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageUnavailable;

impl std::fmt::Display for StorageUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the app is retrying a failed save, so writes are not accepted")
    }
}

impl std::error::Error for StorageUnavailable {}

/// エージェント `client_id` に `scope` が許可されているかを確かめる。許可されていなければ
/// [`PermissionDenied`]。登録されていないエージェントは、何も許可されていないものとして扱う。
///
/// 失敗は、呼び出し側のエラーの型 `E` にして返す（ノートと項目の操作で、同じ変換を書かないため）。
pub fn require_scope<E>(conn: &Connection, client_id: Ulid, scope: Scope) -> Result<(), E>
where
    E: From<PermissionDenied> + From<StorageError>,
{
    let client = agent_client::get(conn, client_id)?;
    if client.is_some_and(|c| c.scopes.contains(&scope)) {
        Ok(())
    } else {
        Err(PermissionDenied {
            required_scope: scope,
        }
        .into())
    }
}

/// エージェント・CLI の読み取りに共通の失敗：権限が無いか、読み取りに失敗した。読み取りは画面の
/// 保存の失敗中でも受け付けるので、`storage_unavailable` は無い（MCP・CLI 仕様書 4.3）。引数の
/// 誤りや対象なしがある読み取りは、これを含む自分のエラーの型を持つ（`get_note` の
/// [`NoteReadError`](crate::domain::text_doc::NoteReadError)）。
#[derive(Debug)]
pub enum ReadError {
    /// `read` スコープが許可されていない（`permission_denied`）。何も読んでいない。
    PermissionDenied(PermissionDenied),
    /// 読み取りに失敗した。
    Storage(StorageError),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PermissionDenied(e) => e.fmt(f),
            Self::Storage(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::PermissionDenied(_) => None,
            Self::Storage(e) => Some(e),
        }
    }
}

impl From<PermissionDenied> for ReadError {
    fn from(e: PermissionDenied) -> Self {
        Self::PermissionDenied(e)
    }
}

impl From<StorageError> for ReadError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

impl From<rusqlite::Error> for ReadError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Storage(e.into())
    }
}

/// エージェント・CLI の読み取りの共通の手順：読み取りのトランザクションの中で、`client_id` に
/// `read` が許可されているかをはじめに確かめてから `f` を行う。読み取りの入り口は、これを通す
/// （確かめ忘れを防ぐため）。
pub fn read_as<T, E>(
    conn: &Connection,
    client_id: Ulid,
    f: impl FnOnce() -> Result<T, E>,
) -> Result<T, E>
where
    E: From<PermissionDenied> + From<StorageError> + From<rusqlite::Error>,
{
    crate::storage::in_read_transaction(conn, || {
        require_scope::<E>(conn, client_id, Scope::Read)?;
        f()
    })
}

/// 画面の保存の失敗中なら [`StorageUnavailable`]（アーキテクチャ設計書 4.1）。
pub fn require_save_working(agent: &Agent<'_>) -> Result<(), StorageUnavailable> {
    if agent.ui.save_failing() {
        Err(StorageUnavailable)
    } else {
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! エージェントの書き込みのテストで共通に使うもの。

    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;

    use super::*;

    /// `client_id` にスコープを許可する（既定は `read` だけ）。
    pub fn grant(conn: &Connection, client_id: Ulid, scopes: &[Scope]) {
        let mut all = vec![Scope::Read];
        all.extend_from_slice(scopes);
        assert!(agent_client::update_scopes(conn, client_id, &all).unwrap());
    }

    /// テスト用の画面の状態。
    #[derive(Default)]
    pub struct FakeUi {
        pub save_failing: bool,
        pub editing: Mutex<HashSet<(Ulid, ItemField)>>,
        pub protected: Mutex<HashMap<(TextDoc, LogicalDate), ProtectedLines>>,
    }

    impl FakeUi {
        pub fn editing(item_id: Ulid, field: ItemField) -> Self {
            let ui = Self::default();
            ui.editing.lock().unwrap().insert((item_id, field));
            ui
        }

        /// `date` の `doc` の守る行を、版 `rev` の `lines` にする（空なら守る行が無い知らせ）。
        pub fn protect(&self, doc: TextDoc, date: LogicalDate, rev: i64, lines: &[usize]) {
            self.protected.lock().unwrap().insert(
                (doc, date),
                ProtectedLines {
                    rev,
                    lines: lines.to_vec(),
                },
            );
        }
    }

    impl UiState for FakeUi {
        fn save_failing(&self) -> bool {
            self.save_failing
        }

        fn is_editing(&self, item_id: Ulid, field: ItemField) -> bool {
            self.editing.lock().unwrap().contains(&(item_id, field))
        }

        fn protected_lines(&self, doc: TextDoc, date: LogicalDate) -> Option<ProtectedLines> {
            self.protected.lock().unwrap().get(&(doc, date)).cloned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{FakeUi, grant};
    use super::*;
    use crate::storage::table::agent_client::CLI_ID;
    use crate::storage::table::test_support::open;

    /// テスト用の、require_scope の失敗の型。
    #[derive(Debug)]
    enum Failure {
        Denied(PermissionDenied),
        Storage,
    }

    impl From<PermissionDenied> for Failure {
        fn from(e: PermissionDenied) -> Self {
            Self::Denied(e)
        }
    }

    impl From<StorageError> for Failure {
        fn from(_: StorageError) -> Self {
            Self::Storage
        }
    }

    fn check(conn: &Connection, client_id: Ulid, scope: Scope) -> Result<(), Failure> {
        require_scope(conn, client_id, scope)
    }

    #[test]
    fn scopes_must_be_granted() {
        let (db, _dir) = open();
        let conn = db.connection();
        assert!(check(conn, CLI_ID, Scope::Read).is_ok());
        assert!(matches!(
            check(conn, CLI_ID, Scope::Todo),
            Err(Failure::Denied(PermissionDenied {
                required_scope: Scope::Todo
            }))
        ));
        grant(conn, CLI_ID, &[Scope::Todo]);
        assert!(check(conn, CLI_ID, Scope::Todo).is_ok());
        assert!(check(conn, CLI_ID, Scope::Blocker).is_err());
        // 登録されていないエージェントには何も許可しない
        assert!(matches!(
            check(conn, Ulid::from_u128(99), Scope::Read),
            Err(Failure::Denied(_))
        ));
        assert!(!matches!(
            check(conn, CLI_ID, Scope::Read),
            Err(Failure::Storage)
        ));
    }

    #[test]
    fn clients_are_looked_up_and_registered_with_read_only() {
        let (db, _dir) = open();
        let conn = db.connection();
        let now = jiff::Timestamp::from_second(1_000_000).unwrap();
        // 識別子なしは unknown、固定の cli は既存の行、使えない形は登録しない
        assert_eq!(
            client_for(conn, None, now).unwrap(),
            Some(agent_client::UNKNOWN_ID)
        );
        assert_eq!(client_for(conn, Some("cli"), now).unwrap(), Some(CLI_ID));
        assert_eq!(client_for(conn, Some("Bad Name"), now).unwrap(), None);
        assert_eq!(agent_client::get_by_name(conn, "Bad Name").unwrap(), None);
        // 初めての識別子は read だけで登録され、2 回目は同じ id
        let id = client_for(conn, Some("claude-code"), now).unwrap().unwrap();
        let row = agent_client::get(conn, id).unwrap().unwrap();
        assert_eq!(row.scopes, vec![Scope::Read]);
        assert_eq!(
            client_for(conn, Some("claude-code"), now).unwrap(),
            Some(id)
        );
    }

    #[test]
    fn touch_client_writes_only_when_the_last_time_is_old() {
        let (db, _dir) = open();
        let conn = db.connection();
        let t0 = jiff::Timestamp::from_second(1_000_000).unwrap();
        let seen = |conn: &Connection| {
            agent_client::get(conn, CLI_ID)
                .unwrap()
                .unwrap()
                .last_seen_at
        };
        assert_eq!(seen(conn), None);
        touch_client(conn, CLI_ID, t0).unwrap();
        assert_eq!(seen(conn), Some(t0));
        // 間隔の中なら書かない
        let soon = t0 + TOUCH_INTERVAL - std::time::Duration::from_secs(1);
        touch_client(conn, CLI_ID, soon).unwrap();
        assert_eq!(seen(conn), Some(t0));
        // 間隔を過ぎたら書く
        let later = t0 + TOUCH_INTERVAL + std::time::Duration::from_secs(1);
        touch_client(conn, CLI_ID, later).unwrap();
        assert_eq!(seen(conn), Some(later));
    }

    #[test]
    fn save_failures_refuse_agent_writes() {
        let failing = FakeUi {
            save_failing: true,
            ..FakeUi::default()
        };
        let agent = Agent {
            client_id: CLI_ID,
            ui: &failing,
        };
        assert_eq!(require_save_working(&agent), Err(StorageUnavailable));
        assert_eq!(require_save_working(&Agent::without_ui(CLI_ID)), Ok(()));
    }
}
