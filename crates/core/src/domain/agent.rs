//! エージェント・CLI からの書き込みの、呼び出し元と権限（MCP・CLI 仕様書 2章・3.2）。
//!
//! ドメイン層の書き込みの関数は、エージェント・CLI から呼ばれるとき [`Agent`] を受け取り、
//! はじめに [`require_scope`] で権限を確かめる（アーキテクチャ設計書 4章 5。どの経路でも
//! 同じ判定を通す）。画面の状態（保存の失敗中か、利用者が入力中か。アーキテクチャ設計書 4.1）は
//! [`UiState`] で受け取る。

use rusqlite::Connection;

use crate::domain::logical_date::LogicalDate;
use crate::storage::table::agent_client::{self, Scope};
use crate::storage::table::text_doc::TextDoc;
use crate::storage::{StorageError, Ulid};

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
    /// いまは受け取る形だけで、`edit_note` の判定（守る行への書き換えと、版が古いときの
    /// `conflict`）は接続口の作業で使う。
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

    use std::collections::HashSet;
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
    }

    impl FakeUi {
        pub fn editing(item_id: Ulid, field: ItemField) -> Self {
            let ui = Self::default();
            ui.editing.lock().unwrap().insert((item_id, field));
            ui
        }
    }

    impl UiState for FakeUi {
        fn save_failing(&self) -> bool {
            self.save_failing
        }

        fn is_editing(&self, item_id: Ulid, field: ItemField) -> bool {
            self.editing.lock().unwrap().contains(&(item_id, field))
        }

        fn protected_lines(&self, _: TextDoc, _: LogicalDate) -> Option<ProtectedLines> {
            None
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
