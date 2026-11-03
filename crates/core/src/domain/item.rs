//! TODO と障害の項目の操作（データモデル設計書 3章 item、MCP・CLI 仕様書 3章・3.2・7章、
//! アーキテクチャ設計書 4.1「項目の保存」、操作仕様書 2章）。
//!
//! 画面・CLI・MCP の項目の書き込みは、すべてここを通る。
//!
//! - エージェント・CLI の操作（[`add_todo`]・[`add_blocker`]・[`edit_todo`]・[`edit_blocker`]・
//!   [`complete_todo`]・[`reopen_todo`]・[`resolve_blocker`]）は、MCP・CLI 仕様書 3.2 の判定の順で
//!   確かめ、実際に何かを変えたときだけ書き込んで change_log に記録する。何も変えない操作は
//!   `updated: false` で成功し、`updated_at` も変えない。
//! - 画面の操作（[`add`]・[`save_text`]・[`set_closed`]・[`save_resolution_note`]・[`delete`]・
//!   [`move_item`]・[`split`]）は、人の編集なので記録しない。
//! - どちらも、変えた欄だけを書く（item は版の番号を持たない。アーキテクチャ設計書 4.1）。
//! - 項目を作るときは、その日の day の行が無ければ作る（[`day::ensure`]）。
//!
//! ここに無いもの（画面がつながってから作る）：利用者とエージェントが項目を同じ状態にした
//! 行き違いの判定（アーキテクチャ設計書 4.1 の後半。画面の保存に添える「変える前の値」と操作の時刻を
//! 使う）。画面の状態（保存の失敗中・入力中）は [`Agent::ui`] で受け取って判定するが、画面から
//! それを知らせる仕組みはまだ無い。

use std::collections::HashSet;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use rusqlite::Connection;
use serde_json::{Map, Value, json};

use super::agent::{self, Agent, ItemField, PermissionDenied, ScopeError};
use super::sort_key;
use crate::domain::day;
use crate::domain::logical_date::LogicalDate;
use crate::storage::table::agent_client::Scope;
use crate::storage::table::change_log::{self, Actor, ChangeLog, Target};
use crate::storage::table::format_time;
use crate::storage::table::item::{self as item_table, ItemChanges};
use crate::storage::{StorageError, Ulid, in_transaction};

pub use crate::storage::table::item::{CloseReason, Item, ItemKind, ItemStatus, Origin};

/// エージェント・CLI の項目の操作（MCP ツール名。MCP・CLI 仕様書 3章）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemOperation {
    AddTodo,
    EditTodo,
    CompleteTodo,
    ReopenTodo,
    AddBlocker,
    EditBlocker,
    ResolveBlocker,
}

impl ItemOperation {
    /// MCP ツール名（エラーの `use_operation` にも使う）。
    pub const fn name(self) -> &'static str {
        match self {
            Self::AddTodo => "add_todo",
            Self::EditTodo => "edit_todo",
            Self::CompleteTodo => "complete_todo",
            Self::ReopenTodo => "reopen_todo",
            Self::AddBlocker => "add_blocker",
            Self::EditBlocker => "edit_blocker",
            Self::ResolveBlocker => "resolve_blocker",
        }
    }

    /// change_log の `operation`（命名規則 `<名詞>.<動詞>`）。
    pub const fn log_operation(self) -> &'static str {
        match self {
            Self::AddTodo => "todo.add",
            Self::EditTodo => "todo.edit",
            Self::CompleteTodo => "todo.complete",
            Self::ReopenTodo => "todo.reopen",
            Self::AddBlocker => "blocker.add",
            Self::EditBlocker => "blocker.edit",
            Self::ResolveBlocker => "blocker.resolve",
        }
    }

    /// 操作に要るスコープ（MCP・CLI 仕様書 2章）。
    pub const fn scope(self) -> Scope {
        match self.kind() {
            ItemKind::Todo => Scope::Todo,
            ItemKind::Blocker => Scope::Blocker,
        }
    }

    /// 操作の名詞の、項目の種類。
    pub const fn kind(self) -> ItemKind {
        match self {
            Self::AddTodo | Self::EditTodo | Self::CompleteTodo | Self::ReopenTodo => {
                ItemKind::Todo
            }
            Self::AddBlocker | Self::EditBlocker | Self::ResolveBlocker => ItemKind::Blocker,
        }
    }

    /// 種類の違う項目に届いたときに、代わりに使う操作（エラーの `use_operation`）。
    /// 解消済みの障害を未解消に戻す操作はエージェントに無いので、`reopen_todo` は `None`。
    const fn counterpart(self) -> Option<Self> {
        match self {
            Self::EditTodo => Some(Self::EditBlocker),
            Self::EditBlocker => Some(Self::EditTodo),
            Self::CompleteTodo => Some(Self::ResolveBlocker),
            Self::ResolveBlocker => Some(Self::CompleteTodo),
            Self::ReopenTodo | Self::AddTodo | Self::AddBlocker => None,
        }
    }
}

/// 引数が誤っている（`invalid_argument`）。何も書いていない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidArgument {
    /// ID が、操作と違う種類の項目を指す（`edit_todo` に障害など）。`use_operation` は、その項目に
    /// 使う操作（無ければ `None`）。
    WrongKind {
        item_kind: ItemKind,
        use_operation: Option<ItemOperation>,
    },
    /// ID が取りやめた TODO（`dropped`）を指す。状態と本文は変えられない。
    Dropped { close_reason: CloseReason },
    /// 本文が空（前後の空白を除くと何も無い）。
    EmptyText,
    /// 本文の途中に改行（LF か CR）がある。本文は1行。
    MultilineText,
    /// 解消メモが空（前後の空白を除くと何も無い）。
    EmptyNote,
    /// 解消メモの途中に改行（LF か CR）がある。解消メモは1行。
    MultilineNote,
    /// 位置の基準の項目が、同じ日の（削除されていない、動かす項目とは別の）項目でない。
    InvalidPosition,
}

impl std::fmt::Display for InvalidArgument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongKind { item_kind, .. } => {
                write!(
                    f,
                    "the item is a {item_kind}, not the kind of the operation"
                )
            }
            Self::Dropped { close_reason } => {
                write!(
                    f,
                    "the TODO was closed ({close_reason}) and cannot be changed"
                )
            }
            Self::EmptyText => f.write_str("the text is empty"),
            Self::MultilineText => f.write_str("the text must be a single line"),
            Self::EmptyNote => f.write_str("the resolution note is empty"),
            Self::MultilineNote => f.write_str("the resolution note must be a single line"),
            Self::InvalidPosition => {
                f.write_str("the position does not refer to another item of the same day")
            }
        }
    }
}

impl std::error::Error for InvalidArgument {}

/// 受け付けられない状態にある（`conflict`）。何も書いていない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conflict {
    /// 項目は持ち越し済み（`reason: carried_over`）。持ち越し先の項目とその日付。
    CarriedOver {
        carried_to_id: Ulid,
        carried_to_date: LogicalDate,
    },
    /// 利用者がその欄を入力中（`reason: user_editing`）。
    UserEditing,
    /// 障害に、送られたものと違う解消メモがすでにある（`reason: note_exists`）。
    NoteExists { resolution_note: String },
}

/// 項目の操作が失敗した理由。どれも、何も書いていない（トランザクションは取り消してある）。
#[derive(Debug)]
pub enum ItemError {
    /// エージェントに操作のスコープが許可されていない（`permission_denied`）。
    PermissionDenied(PermissionDenied),
    /// ID の項目が無いか、削除されている（`not_found`）。
    NotFound,
    InvalidArgument(InvalidArgument),
    /// 画面の保存の書き込みが失敗して再試行している（`storage_unavailable`。アーキテクチャ設計書 4.1）。
    StorageUnavailable,
    Conflict(Conflict),
    /// 読み書きに失敗した。
    Storage(StorageError),
}

impl std::fmt::Display for ItemError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PermissionDenied(e) => e.fmt(f),
            Self::NotFound => f.write_str("the item was not found"),
            Self::InvalidArgument(e) => e.fmt(f),
            Self::StorageUnavailable => {
                f.write_str("the app is retrying a failed save, so writes are not accepted")
            }
            Self::Conflict(Conflict::CarriedOver {
                carried_to_date, ..
            }) => write!(f, "the item was carried over to {carried_to_date}"),
            Self::Conflict(Conflict::UserEditing) => f.write_str("the user is editing the item"),
            Self::Conflict(Conflict::NoteExists { .. }) => {
                f.write_str("the blocker already has a different resolution note")
            }
            Self::Storage(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ItemError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidArgument(e) => Some(e),
            Self::Storage(e) => Some(e),
            Self::PermissionDenied(_)
            | Self::NotFound
            | Self::StorageUnavailable
            | Self::Conflict(_) => None,
        }
    }
}

impl From<StorageError> for ItemError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

impl From<rusqlite::Error> for ItemError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Storage(e.into())
    }
}

impl From<InvalidArgument> for ItemError {
    fn from(e: InvalidArgument) -> Self {
        Self::InvalidArgument(e)
    }
}

impl From<Conflict> for ItemError {
    fn from(e: Conflict) -> Self {
        Self::Conflict(e)
    }
}

impl From<ScopeError> for ItemError {
    fn from(e: ScopeError) -> Self {
        match e {
            ScopeError::Denied(e) => Self::PermissionDenied(e),
            ScopeError::Storage(e) => Self::Storage(e),
        }
    }
}

/// 書き込みの結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// 実際に何かを変えたか（MCP の成功の応答の `updated`）。`false` なら何も書いておらず、
    /// change_log にも記録していない。
    pub updated: bool,
    /// 操作の後の項目（応答の `text` などに使う）。
    pub item: Item,
}

/// [`resolve_blocker`] の結果。CLI の表示（`Resolved.`・`Note added.`・
/// `Already resolved. Nothing changed.`）を分けるため、何を変えたかを持つ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// 未解消の障害を解消にした。
    pub resolved: bool,
    /// 解消メモを書いた。
    pub note_written: bool,
    /// 操作の後の項目（応答の `resolution_note` に使う）。
    pub item: Item,
}

impl Resolution {
    /// 実際に何かを変えたか（`updated`）。
    pub fn updated(&self) -> bool {
        self.resolved || self.note_written
    }
}

/// 画面で、項目を入れる・動かす位置（同じ日の並びの中）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    /// その日の先頭。
    First,
    /// その日の末尾。
    Last,
    /// その項目のすぐ前。
    Before(Ulid),
    /// その項目のすぐ後。
    After(Ulid),
}

// ---- エージェント・CLI の操作 ----

/// エージェント・CLI が TODO を足す（`add_todo`）。その日の末尾に、`origin = agent` で作る。
/// 本文は前後の空白を除いて保存する。change_log に `todo.add` として項目全体を記録する。
pub fn add_todo(
    conn: &Connection,
    agent: &Agent<'_>,
    date: LogicalDate,
    text: &str,
    tz: &TimeZone,
    now: Timestamp,
) -> Result<Item, ItemError> {
    add_by_agent(conn, agent, ItemOperation::AddTodo, date, text, tz, now)
}

/// エージェント・CLI が障害を足す（`add_blocker`）。ほかは [`add_todo`] と同じで、記録は
/// `blocker.add`。
pub fn add_blocker(
    conn: &Connection,
    agent: &Agent<'_>,
    date: LogicalDate,
    text: &str,
    tz: &TimeZone,
    now: Timestamp,
) -> Result<Item, ItemError> {
    add_by_agent(conn, agent, ItemOperation::AddBlocker, date, text, tz, now)
}

/// エージェント・CLI が TODO の本文を変える（`edit_todo`）。完了済みの TODO も受け付ける。
/// 前後の空白を除いた本文が今と同じなら `updated: false`。記録は `todo.edit`（本文だけ）。
pub fn edit_todo(
    conn: &Connection,
    agent: &Agent<'_>,
    id: Ulid,
    text: &str,
    now: Timestamp,
) -> Result<Outcome, ItemError> {
    edit_by_agent(conn, agent, ItemOperation::EditTodo, id, text, now)
}

/// エージェント・CLI が障害の本文を変える（`edit_blocker`）。解消済みの障害も受け付ける。
/// 扱いは [`edit_todo`] と同じで、記録は `blocker.edit`。
pub fn edit_blocker(
    conn: &Connection,
    agent: &Agent<'_>,
    id: Ulid,
    text: &str,
    now: Timestamp,
) -> Result<Outcome, ItemError> {
    edit_by_agent(conn, agent, ItemOperation::EditBlocker, id, text, now)
}

/// エージェント・CLI が TODO を完了にする（`complete_todo`）。完了時刻は `now`。すでに完了なら
/// 状態と完了時刻を変えずに `updated: false`。記録は `todo.complete`（状態と完了時刻）。
pub fn complete_todo(
    conn: &Connection,
    agent: &Agent<'_>,
    id: Ulid,
    now: Timestamp,
) -> Result<Outcome, ItemError> {
    set_done_by_agent(conn, agent, ItemOperation::CompleteTodo, id, now)
}

/// エージェント・CLI が TODO を未完了に戻す（`reopen_todo`）。完了時刻を空にする。すでに未完了なら
/// `updated: false`。記録は `todo.reopen`（状態と完了時刻）。
pub fn reopen_todo(
    conn: &Connection,
    agent: &Agent<'_>,
    id: Ulid,
    now: Timestamp,
) -> Result<Outcome, ItemError> {
    set_done_by_agent(conn, agent, ItemOperation::ReopenTodo, id, now)
}

/// エージェント・CLI が障害を解消する（`resolve_blocker`。MCP・CLI 仕様書 3章）。
///
/// - 未解消なら解消にし、解消時刻を `now` にする。解消済みなら状態と解消時刻を変えない。
/// - `note` は前後の空白を除いて扱う。障害の解消メモが空（NULL か空白だけ）なら書き、同じ本文なら
///   書かず、別の本文があれば [`Conflict::NoteExists`]（解消にもしない）。
/// - 記録は `blocker.resolve` で、実際に変えた欄だけを入れる。
pub fn resolve_blocker(
    conn: &Connection,
    agent: &Agent<'_>,
    id: Ulid,
    note: Option<&str>,
    now: Timestamp,
) -> Result<Resolution, ItemError> {
    let op = ItemOperation::ResolveBlocker;
    in_transaction(conn, || {
        let item = target_for_agent(conn, agent, op, id)?;
        let note = note
            .map(|n| {
                one_line(
                    n,
                    InvalidArgument::EmptyNote,
                    InvalidArgument::MultilineNote,
                )
            })
            .transpose()?;
        ready_for_agent(
            conn,
            agent,
            &item,
            note.as_ref().map(|_| ItemField::ResolutionNote),
        )?;
        let mut changes = ItemChanges::default();
        if let Some(note) = note {
            match item.resolution_note.as_deref() {
                Some(existing) if existing == note => {}
                Some(existing) if !existing.trim().is_empty() => {
                    return Err(Conflict::NoteExists {
                        resolution_note: existing.to_owned(),
                    }
                    .into());
                }
                _ => changes.resolution_note = Some(Some(note)),
            }
        }
        let resolved = item.status != ItemStatus::Resolved;
        if resolved {
            changes.status = Some(ItemStatus::Resolved);
            changes.completed_at = Some(Some(now));
        }
        let note_written = changes.resolution_note.is_some();
        let outcome = apply(conn, Some((agent, op)), item, changes, now)?;
        Ok(Resolution {
            resolved,
            note_written,
            item: outcome.item,
        })
    })
}

fn add_by_agent(
    conn: &Connection,
    agent: &Agent<'_>,
    op: ItemOperation,
    date: LogicalDate,
    text: &str,
    tz: &TimeZone,
    now: Timestamp,
) -> Result<Item, ItemError> {
    in_transaction(conn, || {
        agent::require_scope(conn, agent.client_id, op.scope())?;
        let text = one_line(
            text,
            InvalidArgument::EmptyText,
            InvalidArgument::MultilineText,
        )?;
        if agent.ui.save_failing() {
            return Err(ItemError::StorageUnavailable);
        }
        let item = insert_new(
            conn,
            NewItem {
                date,
                kind: op.kind(),
                text,
                position: Position::Last,
                origin: Origin::Agent,
                agent_client_id: Some(agent.client_id),
            },
            tz,
            now,
        )?;
        record(conn, agent, op, item.id, None, Some(item_json(&item)), now)?;
        Ok(item)
    })
}

fn edit_by_agent(
    conn: &Connection,
    agent: &Agent<'_>,
    op: ItemOperation,
    id: Ulid,
    text: &str,
    now: Timestamp,
) -> Result<Outcome, ItemError> {
    in_transaction(conn, || {
        let item = target_for_agent(conn, agent, op, id)?;
        let text = one_line(
            text,
            InvalidArgument::EmptyText,
            InvalidArgument::MultilineText,
        )?;
        ready_for_agent(conn, agent, &item, Some(ItemField::Text))?;
        let changes = ItemChanges {
            text: (text != item.text).then_some(text),
            ..ItemChanges::default()
        };
        apply(conn, Some((agent, op)), item, changes, now)
    })
}

fn set_done_by_agent(
    conn: &Connection,
    agent: &Agent<'_>,
    op: ItemOperation,
    id: Ulid,
    now: Timestamp,
) -> Result<Outcome, ItemError> {
    in_transaction(conn, || {
        let item = target_for_agent(conn, agent, op, id)?;
        // 状態の欄は入力中でも受け付ける（アーキテクチャ設計書 4.1）
        ready_for_agent(conn, agent, &item, None)?;
        let done = op == ItemOperation::CompleteTodo;
        let changes = done_changes(&item, done.then_some(now), ItemStatus::Done, false);
        apply(conn, Some((agent, op)), item, changes, now)
    })
}

/// エージェントの操作の対象を読み、判定の順（MCP・CLI 仕様書 3.2）の前半を確かめる：
/// 権限、項目があるか（`not_found`）、取りやめた TODO と種類の違い（`invalid_argument` の最初）。
fn target_for_agent(
    conn: &Connection,
    agent: &Agent<'_>,
    op: ItemOperation,
    id: Ulid,
) -> Result<Item, ItemError> {
    agent::require_scope(conn, agent.client_id, op.scope())?;
    let item = load(conn, id)?;
    // 取りやめた TODO は、種類の違う操作（edit_blocker・resolve_blocker）に届いても、送り直した
    // 先でも成功しないので、種類の違いより先に返す
    check_not_dropped(&item)?;
    if item.kind != op.kind() {
        return Err(InvalidArgument::WrongKind {
            item_kind: item.kind,
            use_operation: op.counterpart(),
        }
        .into());
    }
    Ok(item)
}

/// エージェントの操作の、引数を確かめた後の判定（MCP・CLI 仕様書 3.2）：保存の失敗中
/// （`storage_unavailable`）、持ち越し済み（`carried_over`）、入力中（`user_editing`）。
/// `field` は操作が書く、利用者が入力する欄（状態だけを変える操作は `None`）。
fn ready_for_agent(
    conn: &Connection,
    agent: &Agent<'_>,
    item: &Item,
    field: Option<ItemField>,
) -> Result<(), ItemError> {
    if agent.ui.save_failing() {
        return Err(ItemError::StorageUnavailable);
    }
    check_not_carried(conn, item)?;
    if field.is_some_and(|f| agent.ui.is_editing(item.id, f)) {
        return Err(Conflict::UserEditing.into());
    }
    Ok(())
}

// ---- 画面の操作 ----

/// 画面で項目を足す（`origin = human`）。本文は前後の空白を除いて保存する（空の本文は
/// [`InvalidArgument::EmptyText`]。画面は、本文が入るまで項目を保存しない）。
pub fn add(
    conn: &Connection,
    date: LogicalDate,
    kind: ItemKind,
    text: &str,
    position: Position,
    tz: &TimeZone,
    now: Timestamp,
) -> Result<Item, ItemError> {
    in_transaction(conn, || {
        let text = one_line(
            text,
            InvalidArgument::EmptyText,
            InvalidArgument::MultilineText,
        )?;
        insert_new(
            conn,
            NewItem {
                date,
                kind,
                text,
                position,
                origin: Origin::Human,
                agent_client_id: None,
            },
            tz,
            now,
        )
    })
}

/// 画面で項目の本文を保存する。前後の空白を除いて今と同じなら書かない。取りやめた TODO と
/// 持ち越し済みの項目は変えられない（操作仕様書 2章）。
pub fn save_text(
    conn: &Connection,
    id: Ulid,
    text: &str,
    now: Timestamp,
) -> Result<Outcome, ItemError> {
    in_transaction(conn, || {
        let item = load(conn, id)?;
        check_not_dropped(&item)?;
        let text = one_line(
            text,
            InvalidArgument::EmptyText,
            InvalidArgument::MultilineText,
        )?;
        check_not_carried(conn, &item)?;
        let changes = ItemChanges {
            text: (text != item.text).then_some(text),
            ..ItemChanges::default()
        };
        apply(conn, None, item, changes, now)
    })
}

/// 画面で項目の完了・解消を切り替える。`closed_at` が `Some` なら完了（TODO）・解消（障害）にして
/// その時刻を完了時刻に、`None` なら未完了・未解消に戻して完了時刻を空にする。完了時刻は画面が
/// 決める（ふつうは操作の時刻。Mod+Z で戻す操作を取り消したときは元の時刻。操作仕様書 2章）。
///
/// 障害を未解消に戻しても、解消メモは残す。取りやめた TODO と持ち越し済みの項目は変えられない。
pub fn set_closed(
    conn: &Connection,
    id: Ulid,
    closed_at: Option<Timestamp>,
    now: Timestamp,
) -> Result<Outcome, ItemError> {
    in_transaction(conn, || {
        let item = load(conn, id)?;
        check_not_dropped(&item)?;
        check_not_carried(conn, &item)?;
        let closed = match item.kind {
            ItemKind::Todo => ItemStatus::Done,
            ItemKind::Blocker => ItemStatus::Resolved,
        };
        let changes = done_changes(&item, closed_at, closed, true);
        apply(conn, None, item, changes, now)
    })
}

/// 画面で障害の解消メモを保存する。前後の空白を除いて保存し、空なら解消メモを消す（NULL）。
/// TODO は [`InvalidArgument::WrongKind`]、持ち越し済みの障害は変えられない。
pub fn save_resolution_note(
    conn: &Connection,
    id: Ulid,
    note: &str,
    now: Timestamp,
) -> Result<Outcome, ItemError> {
    in_transaction(conn, || {
        let item = load(conn, id)?;
        if item.kind != ItemKind::Blocker {
            return Err(InvalidArgument::WrongKind {
                item_kind: item.kind,
                use_operation: None,
            }
            .into());
        }
        let note = match one_line(
            note,
            InvalidArgument::EmptyNote,
            InvalidArgument::MultilineNote,
        ) {
            Ok(note) => Some(note),
            Err(InvalidArgument::EmptyNote) => None,
            Err(e) => return Err(e.into()),
        };
        check_not_carried(conn, &item)?;
        let changes = ItemChanges {
            resolution_note: (note != item.resolution_note).then_some(note),
            ..ItemChanges::default()
        };
        apply(conn, None, item, changes, now)
    })
}

/// 画面で項目を削除する（論理削除）。取りやめた TODO と持ち越し済みの項目も削除できる。
pub fn delete(conn: &Connection, id: Ulid, now: Timestamp) -> Result<(), ItemError> {
    in_transaction(conn, || {
        if item_table::soft_delete(conn, id, now)? {
            Ok(())
        } else {
            Err(ItemError::NotFound)
        }
    })
}

/// 画面で項目を同じ日の並びの中で動かす（Alt+↑ / Alt+↓）。動かす項目の `sort_key` だけを書く
/// （キーを付け直せないときだけ、その日のキーをまとめて付け直す）。すでにその位置なら書かない。
/// 取りやめた TODO と持ち越し済みの項目も動かせる。
pub fn move_item(
    conn: &Connection,
    id: Ulid,
    position: Position,
    now: Timestamp,
) -> Result<Outcome, ItemError> {
    in_transaction(conn, || {
        let item = load(conn, id)?;
        let key = key_at(conn, item.date, position, Some(&item), now)?;
        let changes = ItemChanges {
            sort_key: (key != item.sort_key).then_some(key),
            ..ItemChanges::default()
        };
        apply(conn, None, item, changes, now)
    })
}

/// 画面で項目を2つに分ける（行の途中の Enter。操作仕様書 2章）。`first` を元の項目の本文にし、
/// `second` を本文にした新しい項目（同じ種類で未完了・未解消、`origin = human`）を、そのすぐ後に作る。
/// どちらも前後の空白を除いて空なら [`InvalidArgument::EmptyText`]（行頭・行末の Enter は、分けずに
/// 項目を足す）。取りやめた TODO と持ち越し済みの項目は分けられない（本文を変えるため）。
///
/// 返すのは、操作の後の元の項目と、新しい項目。
pub fn split(
    conn: &Connection,
    id: Ulid,
    first: &str,
    second: &str,
    now: Timestamp,
) -> Result<(Item, Item), ItemError> {
    in_transaction(conn, || {
        let item = load(conn, id)?;
        check_not_dropped(&item)?;
        let first = one_line(
            first,
            InvalidArgument::EmptyText,
            InvalidArgument::MultilineText,
        )?;
        let second = one_line(
            second,
            InvalidArgument::EmptyText,
            InvalidArgument::MultilineText,
        )?;
        check_not_carried(conn, &item)?;
        // 項目があるので day の行もある。tz は使われない
        let new = insert_new(
            conn,
            NewItem {
                date: item.date,
                kind: item.kind,
                text: second,
                position: Position::After(item.id),
                origin: Origin::Human,
                agent_client_id: None,
            },
            &TimeZone::UTC,
            now,
        )?;
        let changes = ItemChanges {
            text: (first != item.text).then_some(first),
            ..ItemChanges::default()
        };
        let outcome = apply(conn, None, item, changes, now)?;
        Ok((outcome.item, new))
    })
}

// ---- 持ち越し ----

/// 項目 `id` の持ち越し先（データモデル設計書 3章 `carried_to_id`）：`carried_from_id` で `id` を
/// 指す項目をたどり、最初に行き着いた削除されていない項目。削除されている項目は、さらにそれを
/// 指す項目へたどる。行き着かなければ `None`（持ち越し済みでない）。
pub fn carried_to(conn: &Connection, id: Ulid) -> Result<Option<Item>, StorageError> {
    let mut seen = HashSet::from([id]);
    let mut pending = vec![id];
    while let Some(from) = pending.pop() {
        for next in item_table::list_carried_from(conn, from)? {
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

// ---- 共通の部品 ----

/// 削除されていない項目を読む。無ければ [`ItemError::NotFound`]。
fn load(conn: &Connection, id: Ulid) -> Result<Item, ItemError> {
    match item_table::get(conn, id)? {
        Some(item) if item.deleted_at.is_none() => Ok(item),
        _ => Err(ItemError::NotFound),
    }
}

/// 取りやめた TODO なら [`InvalidArgument::Dropped`]。
fn check_not_dropped(item: &Item) -> Result<(), ItemError> {
    if item.status == ItemStatus::Dropped {
        return Err(InvalidArgument::Dropped {
            // 理由の無い dropped は壊れたデータだが、「やめた」として扱う
            close_reason: item.close_reason.unwrap_or(CloseReason::Dropped),
        }
        .into());
    }
    Ok(())
}

/// 持ち越し済みの項目なら [`Conflict::CarriedOver`]。
fn check_not_carried(conn: &Connection, item: &Item) -> Result<(), ItemError> {
    match carried_to(conn, item.id)? {
        Some(to) => Err(Conflict::CarriedOver {
            carried_to_id: to.id,
            carried_to_date: to.date,
        }
        .into()),
        None => Ok(()),
    }
}

/// 1行の本文（項目の本文・解消メモ）を整える：前後の空白（改行・全角空白を含む Unicode の
/// 空白文字）を除く。除いて空なら `empty`、途中に改行（LF か CR）があれば `multiline`。
fn one_line(
    text: &str,
    empty: InvalidArgument,
    multiline: InvalidArgument,
) -> Result<String, InvalidArgument> {
    let text = text.trim();
    if text.is_empty() {
        Err(empty)
    } else if text.contains(['\n', '\r']) {
        Err(multiline)
    } else {
        Ok(text.to_owned())
    }
}

/// 完了・解消（`closed_at` が `Some`。状態は `closed`）か未完了・未解消（`None`）にする書き換え。
/// 今と違う欄だけを入れる。
///
/// すでに目的の状態なら、エージェントの操作（`write_time_if_same` が `false`）は完了時刻も変えない
/// （最初に完了した時刻を残す。MCP・CLI 仕様書 3章）。画面の保存（`true`）は、画面が決めて添えた
/// 完了時刻を書く（アーキテクチャ設計書 4.1）。
fn done_changes(
    item: &Item,
    closed_at: Option<Timestamp>,
    closed: ItemStatus,
    write_time_if_same: bool,
) -> ItemChanges {
    let status = if closed_at.is_some() {
        closed
    } else {
        ItemStatus::Open
    };
    let same = item.status == status;
    ItemChanges {
        status: (!same).then_some(status),
        completed_at: ((!same || write_time_if_same) && item.completed_at != closed_at)
            .then_some(closed_at),
        ..ItemChanges::default()
    }
}

/// 書き換えを行い、操作の後の項目を返す。何も変えないなら書かない（`updated: false`）。
/// エージェントの操作（`by` が `Some`）なら、変えた欄だけを change_log に記録する。
fn apply(
    conn: &Connection,
    by: Option<(&Agent<'_>, ItemOperation)>,
    item: Item,
    changes: ItemChanges,
    now: Timestamp,
) -> Result<Outcome, ItemError> {
    if changes.is_empty() {
        return Ok(Outcome {
            updated: false,
            item,
        });
    }
    if !item_table::update_fields(conn, item.id, &changes, now)? {
        // 同じトランザクションで読んだばかりなので、ふつうは起きない
        return Err(ItemError::NotFound);
    }
    let after = load(conn, item.id)?;
    if let Some((agent, op)) = by {
        record(
            conn,
            agent,
            op,
            item.id,
            Some(fields_json(&item, &changes)),
            Some(fields_json(&after, &changes)),
            now,
        )?;
    }
    Ok(Outcome {
        updated: true,
        item: after,
    })
}

/// 作る項目の、操作ごとに違う値。
struct NewItem {
    date: LogicalDate,
    kind: ItemKind,
    /// 整えた本文。
    text: String,
    position: Position,
    origin: Origin,
    agent_client_id: Option<Ulid>,
}

/// 項目を作る。その日の day の行が無ければ端末のタイムゾーン `tz` で作る。
fn insert_new(
    conn: &Connection,
    new: NewItem,
    tz: &TimeZone,
    now: Timestamp,
) -> Result<Item, ItemError> {
    let sort_key = key_at(conn, new.date, new.position, None, now)?;
    day::ensure(conn, new.date, tz, now)?;
    let item = Item {
        id: Ulid::generate(),
        date: new.date,
        kind: new.kind,
        text: new.text,
        status: ItemStatus::Open,
        close_reason: None,
        sort_key,
        carry_count: 0,
        carried_from_id: None,
        split_from_id: None,
        resolution_note: None,
        origin: new.origin,
        agent_client_id: new.agent_client_id,
        external_ref_id: None,
        completed_at: None,
        created_at: now,
        updated_at: now,
        deleted_at: None,
    };
    item_table::insert(conn, &item)?;
    Ok(item)
}

/// その日の並びの `position` に置くキー。`moving` は動かす項目（並びから除いて数える）で、
/// すでにその位置にあれば、そのキーをそのまま返す。
///
/// 隣のキーの間にキーを作れない（形に当てはまらない、同じキーが並ぶ）ときは、その日のほかの項目の
/// キーを付け直してから作る（並び順は変えない）。
fn key_at(
    conn: &Connection,
    date: LogicalDate,
    position: Position,
    moving: Option<&Item>,
    now: Timestamp,
) -> Result<String, ItemError> {
    let items: Vec<Item> = item_table::list_by_date(conn, date)?
        .into_iter()
        .filter(|i| Some(i.id) != moving.map(|m| m.id))
        .collect();
    let index_of = |id: Ulid| {
        items
            .iter()
            .position(|i| i.id == id)
            .ok_or(InvalidArgument::InvalidPosition)
    };
    let at = match position {
        Position::First => 0,
        Position::Last => items.len(),
        Position::Before(id) => index_of(id)?,
        Position::After(id) => index_of(id)? + 1,
    };
    let before = at.checked_sub(1).map(|i| items[i].sort_key.as_str());
    let after = items.get(at).map(|i| i.sort_key.as_str());
    if let Some(m) = moving
        && sort_key::is_valid(&m.sort_key)
        && before.is_none_or(|b| b < m.sort_key.as_str())
        && after.is_none_or(|a| m.sort_key.as_str() < a)
    {
        return Ok(m.sort_key.clone());
    }
    if let Some(key) = sort_key::between(before, after) {
        return Ok(key);
    }
    // 付け直す。`at` の位置を空けておく
    let keys = sort_key::spread(items.len() + 1);
    for (i, item) in items.iter().enumerate() {
        let key = &keys[if i < at { i } else { i + 1 }];
        if &item.sort_key != key {
            let changes = ItemChanges {
                sort_key: Some(key.clone()),
                ..ItemChanges::default()
            };
            item_table::update_fields(conn, item.id, &changes, now)?;
        }
    }
    Ok(keys[at].clone())
}

/// change_log に、エージェントの項目の操作を記録する。
fn record(
    conn: &Connection,
    agent: &Agent<'_>,
    op: ItemOperation,
    id: Ulid,
    before: Option<Value>,
    after: Option<Value>,
    now: Timestamp,
) -> Result<(), StorageError> {
    change_log::insert(
        conn,
        &ChangeLog {
            id: Ulid::generate(),
            actor: Actor::Agent,
            agent_client_id: Some(agent.client_id),
            operation: op.log_operation().to_owned(),
            target: Target::Item(id),
            before: before.map(|v| v.to_string()),
            after: after.map(|v| v.to_string()),
            created_at: now,
            reverted_at: None,
            revert_of_id: None,
            seen_at: None,
        },
    )
}

fn time_json(t: Option<Timestamp>) -> Value {
    t.map_or(Value::Null, |t| Value::String(format_time(t)))
}

fn id_json(id: Option<Ulid>) -> Value {
    id.map_or(Value::Null, |id| Value::String(id.to_string()))
}

/// 項目全体のスナップショット（追加の `after`）。キーは列の名前、値は保存する形。
fn item_json(item: &Item) -> Value {
    json!({
        "id": item.id.to_string(),
        "date": item.date.to_string(),
        "kind": item.kind.as_str(),
        "text": item.text,
        "status": item.status.as_str(),
        "close_reason": item.close_reason.map(CloseReason::as_str),
        "sort_key": item.sort_key,
        "carry_count": item.carry_count,
        "carried_from_id": id_json(item.carried_from_id),
        "split_from_id": id_json(item.split_from_id),
        "resolution_note": item.resolution_note,
        "origin": item.origin.as_str(),
        "agent_client_id": id_json(item.agent_client_id),
        "external_ref_id": id_json(item.external_ref_id),
        "completed_at": time_json(item.completed_at),
        "created_at": format_time(item.created_at),
        "updated_at": format_time(item.updated_at),
        "deleted_at": time_json(item.deleted_at),
    })
}

/// `changes` が書き換える欄だけの、`item` での値（書き換えの `before` / `after`）。
fn fields_json(item: &Item, changes: &ItemChanges) -> Value {
    let all = item_json(item);
    let mut fields = Map::new();
    for (name, written) in [
        ("text", changes.text.is_some()),
        ("status", changes.status.is_some()),
        ("close_reason", changes.close_reason.is_some()),
        ("sort_key", changes.sort_key.is_some()),
        ("resolution_note", changes.resolution_note.is_some()),
        ("completed_at", changes.completed_at.is_some()),
    ] {
        if written {
            fields.insert(name.to_owned(), all[name].clone());
        }
    }
    Value::Object(fields)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::ItemField;
    use crate::domain::agent::test_support::{FakeUi, grant};
    use crate::storage::Database;
    use crate::storage::table::agent_client::CLI_ID;
    use crate::storage::table::day as day_table;
    use crate::storage::table::test_support::{at, date, fail_change_log_inserts};

    fn tokyo() -> TimeZone {
        TimeZone::get("Asia/Tokyo").unwrap()
    }

    const T0: &str = "2026-11-03T01:00:00Z";
    const T1: &str = "2026-11-03T02:00:00Z";
    const T2: &str = "2026-11-03T03:00:00Z";

    /// データベースを開き、`cli` に `todo`・`blocker` を許可する。
    fn open() -> (Database, tempfile::TempDir) {
        let (db, dir) = crate::storage::table::test_support::open();
        grant(db.connection(), CLI_ID, &[Scope::Todo, Scope::Blocker]);
        (db, dir)
    }

    fn cli() -> Agent<'static> {
        Agent::without_ui(CLI_ID)
    }

    fn with_ui(ui: &FakeUi) -> Agent<'_> {
        Agent {
            client_id: CLI_ID,
            ui,
        }
    }

    fn logs(conn: &Connection, id: Ulid) -> Vec<ChangeLog> {
        change_log::list_by_target(conn, Target::Item(id)).unwrap()
    }

    /// 記録の `before` / `after`。無ければ null。
    fn json(s: &Option<String>) -> Value {
        s.as_deref()
            .map_or(Value::Null, |s| serde_json::from_str(s).unwrap())
    }

    fn get(conn: &Connection, id: Ulid) -> Item {
        item_table::get(conn, id).unwrap().unwrap()
    }

    fn texts(conn: &Connection, d: &str) -> Vec<String> {
        item_table::list_by_date(conn, date(d))
            .unwrap()
            .into_iter()
            .map(|i| i.text)
            .collect()
    }

    fn todo(conn: &Connection, text: &str) -> Item {
        add_todo(conn, &cli(), date("2026-11-03"), text, &tokyo(), at(T0)).unwrap()
    }

    fn blocker(conn: &Connection, text: &str) -> Item {
        add_blocker(conn, &cli(), date("2026-11-03"), text, &tokyo(), at(T0)).unwrap()
    }

    /// `item` を取りやめた TODO にする（持ち越しの見直しの代わり）。
    fn drop_todo(conn: &Connection, item: &Item, reason: CloseReason) {
        let changes = ItemChanges {
            status: Some(ItemStatus::Dropped),
            close_reason: Some(Some(reason)),
            ..ItemChanges::default()
        };
        assert!(item_table::update_fields(conn, item.id, &changes, at(T0)).unwrap());
    }

    /// `from` を `d` へ持ち越した項目を作る（持ち越しの処理の代わり）。
    fn carry(conn: &Connection, from: &Item, d: &str) -> Item {
        let d = date(d);
        day::ensure(conn, d, &tokyo(), at(T0)).unwrap();
        let item = Item {
            id: Ulid::generate(),
            date: d,
            carry_count: from.carry_count + 1,
            carried_from_id: Some(from.id),
            resolution_note: None,
            status: ItemStatus::Open,
            completed_at: None,
            ..from.clone()
        };
        item_table::insert(conn, &item).unwrap();
        item
    }

    fn invalid(r: Result<impl std::fmt::Debug, ItemError>) -> InvalidArgument {
        match r {
            Err(ItemError::InvalidArgument(e)) => e,
            other => panic!("{other:?}"),
        }
    }

    fn conflict(r: Result<impl std::fmt::Debug, ItemError>) -> Conflict {
        match r {
            Err(ItemError::Conflict(e)) => e,
            other => panic!("{other:?}"),
        }
    }

    // ---- エージェントの追加 ----

    #[test]
    fn add_creates_the_day_and_records_the_whole_item() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        assert_eq!(day_table::get(conn, d).unwrap(), None);
        let a = add_todo(conn, &cli(), d, "　API の調査 \n", &tokyo(), at(T0)).unwrap();
        let b = add_blocker(conn, &cli(), d, "権限待ち", &tokyo(), at(T1)).unwrap();
        let c = add_todo(conn, &cli(), d, "c", &tokyo(), at(T1)).unwrap();

        assert_eq!(day_table::get(conn, d).unwrap().unwrap().tz, "Asia/Tokyo");
        // 前後の空白を除いて保存し、その日の末尾に足す
        assert_eq!(texts(conn, "2026-11-03"), ["API の調査", "権限待ち", "c"]);
        assert_eq!(a.kind, ItemKind::Todo);
        assert_eq!(b.kind, ItemKind::Blocker);
        for item in [&a, &b, &c] {
            assert_eq!(get(conn, item.id), *item);
            assert_eq!(item.origin, Origin::Agent);
            assert_eq!(item.agent_client_id, Some(CLI_ID));
            assert_eq!(item.status, ItemStatus::Open);
            assert!(sort_key::is_valid(&item.sort_key));
        }

        let log = &logs(conn, a.id)[0];
        assert_eq!(log.operation, "todo.add");
        assert_eq!(log.actor, Actor::Agent);
        assert_eq!(log.agent_client_id, Some(CLI_ID));
        assert_eq!(log.before, None);
        assert_eq!(json(&log.after), item_json(&a));
        assert_eq!(json(&log.after)["text"], "API の調査");
        assert_eq!(json(&log.after)["created_at"], "2026-11-03T01:00:00.000Z");
        assert_eq!(logs(conn, b.id)[0].operation, "blocker.add");
    }

    #[test]
    fn add_checks_scope_then_text_then_the_save_state() {
        let (db, _dir) = crate::storage::table::test_support::open();
        let conn = db.connection();
        let d = date("2026-11-03");
        // 権限は本文より先
        assert!(matches!(
            add_todo(conn, &cli(), d, "", &tokyo(), at(T0)),
            Err(ItemError::PermissionDenied(PermissionDenied {
                required_scope: Scope::Todo
            }))
        ));
        grant(conn, CLI_ID, &[Scope::Todo]);
        assert!(matches!(
            add_blocker(conn, &cli(), d, "x", &tokyo(), at(T0)),
            Err(ItemError::PermissionDenied(PermissionDenied {
                required_scope: Scope::Blocker
            }))
        ));
        let failing = FakeUi {
            save_failing: true,
            ..FakeUi::default()
        };
        // 本文の誤りは保存の失敗中より先
        for (text, e) in [
            (" \u{3000}\n", InvalidArgument::EmptyText),
            ("a\nb", InvalidArgument::MultilineText),
            ("a\rb", InvalidArgument::MultilineText),
        ] {
            assert_eq!(
                invalid(add_todo(
                    conn,
                    &with_ui(&failing),
                    d,
                    text,
                    &tokyo(),
                    at(T0)
                )),
                e
            );
        }
        assert!(matches!(
            add_todo(conn, &with_ui(&failing), d, "x", &tokyo(), at(T0)),
            Err(ItemError::StorageUnavailable)
        ));
        // どれも何も作らない
        assert_eq!(day_table::get(conn, d).unwrap(), None);
    }

    #[test]
    fn a_failed_write_leaves_nothing_behind() {
        let (db, _dir) = open();
        let conn = db.connection();
        let item = todo(conn, "a");
        fail_change_log_inserts(conn);
        let d = date("2026-11-04");
        assert!(matches!(
            add_todo(conn, &cli(), d, "x", &tokyo(), at(T1)),
            Err(ItemError::Storage(_))
        ));
        assert_eq!(day_table::get(conn, d).unwrap(), None);
        assert!(matches!(
            complete_todo(conn, &cli(), item.id, at(T1)),
            Err(ItemError::Storage(_))
        ));
        assert_eq!(get(conn, item.id), item);
        assert!(conn.is_autocommit());
    }

    // ---- 本文の編集 ----

    #[test]
    fn edit_writes_only_the_text_and_records_it() {
        let (db, _dir) = open();
        let conn = db.connection();
        let item = todo(conn, "before");
        complete_todo(conn, &cli(), item.id, at(T0)).unwrap();
        // 完了済みの TODO も受け付ける
        let out = edit_todo(conn, &cli(), item.id, " after ", at(T1)).unwrap();
        assert!(out.updated);
        assert_eq!(out.item.text, "after");
        assert_eq!(out.item.status, ItemStatus::Done);
        assert_eq!(out.item.updated_at, at(T1));
        assert_eq!(get(conn, item.id), out.item);

        let log = logs(conn, item.id).pop().unwrap();
        assert_eq!(log.operation, "todo.edit");
        assert_eq!(json(&log.before), json!({"text": "before"}));
        assert_eq!(json(&log.after), json!({"text": "after"}));
        assert_eq!(log.created_at, at(T1));

        // 解消済みの障害も受け付け、記録は blocker.edit
        let b = blocker(conn, "b");
        resolve_blocker(conn, &cli(), b.id, None, at(T0)).unwrap();
        let out = edit_blocker(conn, &cli(), b.id, "B", at(T1)).unwrap();
        assert!(out.updated);
        assert_eq!(out.item.status, ItemStatus::Resolved);
        assert_eq!(logs(conn, b.id).pop().unwrap().operation, "blocker.edit");
    }

    #[test]
    fn the_same_text_changes_nothing() {
        let (db, _dir) = open();
        let conn = db.connection();
        let item = todo(conn, "same");
        for text in ["same", "  same\n", "\u{3000}same"] {
            let out = edit_todo(conn, &cli(), item.id, text, at(T1)).unwrap();
            assert_eq!(
                out,
                Outcome {
                    updated: false,
                    item: item.clone()
                }
            );
        }
        // updated_at も変えず、記録もしない
        assert_eq!(get(conn, item.id).updated_at, at(T0));
        assert_eq!(logs(conn, item.id).len(), 1); // todo.add だけ
    }

    #[test]
    fn edit_of_the_other_kind_is_an_error_even_with_the_same_text() {
        let (db, _dir) = open();
        let conn = db.connection();
        let t = todo(conn, "t");
        let b = blocker(conn, "b");
        let failing = FakeUi {
            save_failing: true,
            ..FakeUi::default()
        };
        // 同じ本文・空の本文・保存の失敗中・入力中でも、種類の違いを返す
        for text in ["b", ""] {
            for agent in [cli(), with_ui(&failing)] {
                assert_eq!(
                    invalid(edit_todo(conn, &agent, b.id, text, at(T1))),
                    InvalidArgument::WrongKind {
                        item_kind: ItemKind::Blocker,
                        use_operation: Some(ItemOperation::EditBlocker),
                    }
                );
            }
        }
        let editing = FakeUi::editing(t.id, ItemField::Text);
        assert_eq!(
            invalid(edit_blocker(conn, &with_ui(&editing), t.id, "t", at(T1))),
            InvalidArgument::WrongKind {
                item_kind: ItemKind::Todo,
                use_operation: Some(ItemOperation::EditTodo),
            }
        );
        assert_eq!(get(conn, t.id), t);
        assert_eq!(get(conn, b.id), b);
    }

    #[test]
    fn edit_checks_text_then_save_state_then_carry_then_editing() {
        let (db, _dir) = open();
        let conn = db.connection();
        let item = todo(conn, "a");
        let failing = FakeUi {
            save_failing: true,
            ..FakeUi::default()
        };
        assert_eq!(
            invalid(edit_todo(conn, &with_ui(&failing), item.id, "a\nb", at(T1))),
            InvalidArgument::MultilineText
        );
        // 同じ本文でも、保存の失敗中は storage_unavailable
        assert!(matches!(
            edit_todo(conn, &with_ui(&failing), item.id, "a", at(T1)),
            Err(ItemError::StorageUnavailable)
        ));
        // 同じ本文でも、入力中なら user_editing（解消メモの入力中は関係ない）
        let editing = FakeUi::editing(item.id, ItemField::Text);
        assert_eq!(
            conflict(edit_todo(conn, &with_ui(&editing), item.id, "a", at(T1))),
            Conflict::UserEditing
        );
        let note_editing = FakeUi::editing(item.id, ItemField::ResolutionNote);
        assert!(
            edit_todo(conn, &with_ui(&note_editing), item.id, "x", at(T1))
                .unwrap()
                .updated
        );
        // 持ち越し済みなら、入力中でも carried_over
        let carried = carry(conn, &get(conn, item.id), "2026-11-04");
        let editing = FakeUi::editing(item.id, ItemField::Text);
        assert_eq!(
            conflict(edit_todo(conn, &with_ui(&editing), item.id, "x", at(T1))),
            Conflict::CarriedOver {
                carried_to_id: carried.id,
                carried_to_date: date("2026-11-04"),
            }
        );
    }

    // ---- 対象なし・権限 ----

    #[test]
    fn scope_comes_first_then_not_found() {
        let (db, _dir) = open();
        let conn = db.connection();
        let b = blocker(conn, "b");
        grant(conn, CLI_ID, &[Scope::Blocker]);
        // 障害の ID でも、edit_todo は todo スコープで確かめる
        assert!(matches!(
            edit_todo(conn, &cli(), b.id, "", at(T1)),
            Err(ItemError::PermissionDenied(PermissionDenied {
                required_scope: Scope::Todo
            }))
        ));
        grant(conn, CLI_ID, &[Scope::Todo, Scope::Blocker]);
        // 無い ID は、本文の誤りより先に not_found
        let unknown = Ulid::generate();
        assert!(matches!(
            edit_todo(conn, &cli(), unknown, "", at(T1)),
            Err(ItemError::NotFound)
        ));
        // 削除した項目も not_found
        delete(conn, b.id, at(T1)).unwrap();
        for r in [
            edit_blocker(conn, &cli(), b.id, "x", at(T1)).map(|_| ()),
            resolve_blocker(conn, &cli(), b.id, None, at(T1)).map(|_| ()),
            complete_todo(conn, &cli(), b.id, at(T1)).map(|_| ()),
        ] {
            assert!(matches!(r, Err(ItemError::NotFound)), "{r:?}");
        }
    }

    // ---- 取りやめた TODO ----

    #[test]
    fn dropped_todos_are_rejected_before_anything_else() {
        let (db, _dir) = open();
        let conn = db.connection();
        let item = todo(conn, "a");
        drop_todo(conn, &item, CloseReason::Split);
        let dropped = get(conn, item.id);
        carry(conn, &dropped, "2026-11-04");
        let ui = FakeUi {
            save_failing: true,
            ..FakeUi::editing(item.id, ItemField::Text)
        };
        let expected = InvalidArgument::Dropped {
            close_reason: CloseReason::Split,
        };
        for agent in [cli(), with_ui(&ui)] {
            // 同じ本文・空の本文でも同じエラー。edit_blocker・resolve_blocker でも同じ
            for text in ["a", "", "b"] {
                assert_eq!(
                    invalid(edit_todo(conn, &agent, item.id, text, at(T1))),
                    expected
                );
                assert_eq!(
                    invalid(edit_blocker(conn, &agent, item.id, text, at(T1))),
                    expected
                );
            }
            assert_eq!(
                invalid(complete_todo(conn, &agent, item.id, at(T1))),
                expected
            );
            assert_eq!(
                invalid(reopen_todo(conn, &agent, item.id, at(T1))),
                expected
            );
            assert_eq!(
                invalid(resolve_blocker(conn, &agent, item.id, Some(""), at(T1))),
                expected
            );
        }
        // 画面でも本文と状態は変えられない
        assert_eq!(invalid(save_text(conn, item.id, "b", at(T1))), expected);
        assert_eq!(
            invalid(set_closed(conn, item.id, Some(at(T1)), at(T1))),
            expected
        );
        assert_eq!(invalid(split(conn, item.id, "a", "b", at(T1))), expected);
        assert_eq!(get(conn, item.id), dropped);
        // 理由の無い dropped は「やめた」として扱う
        conn.execute("UPDATE item SET close_reason = NULL", [])
            .unwrap();
        assert_eq!(
            invalid(complete_todo(conn, &cli(), item.id, at(T1))),
            InvalidArgument::Dropped {
                close_reason: CloseReason::Dropped
            }
        );
        // 並べ替えと削除はできる
        move_item(conn, item.id, Position::First, at(T1)).unwrap();
        delete(conn, item.id, at(T1)).unwrap();
    }

    // ---- 完了・未完了 ----

    #[test]
    fn complete_and_reopen_are_idempotent() {
        let (db, _dir) = open();
        let conn = db.connection();
        let item = todo(conn, "a");
        let out = complete_todo(conn, &cli(), item.id, at(T1)).unwrap();
        assert!(out.updated);
        assert_eq!(out.item.status, ItemStatus::Done);
        assert_eq!(out.item.completed_at, Some(at(T1)));
        // やり直しは完了時刻も変えない
        let again = complete_todo(conn, &cli(), item.id, at(T2)).unwrap();
        assert_eq!(
            again,
            Outcome {
                updated: false,
                item: out.item.clone()
            }
        );
        let reopened = reopen_todo(conn, &cli(), item.id, at(T2)).unwrap();
        assert!(reopened.updated);
        assert_eq!(reopened.item.status, ItemStatus::Open);
        assert_eq!(reopened.item.completed_at, None);
        assert!(!reopen_todo(conn, &cli(), item.id, at(T2)).unwrap().updated);

        let ops: Vec<_> = logs(conn, item.id)
            .into_iter()
            .map(|l| (l.operation, json(&l.before), json(&l.after)))
            .collect();
        assert_eq!(
            ops,
            [
                ("todo.add".into(), Value::Null, item_json(&item)),
                (
                    "todo.complete".into(),
                    json!({"status": "open", "completed_at": null}),
                    json!({"status": "done", "completed_at": "2026-11-03T02:00:00.000Z"}),
                ),
                (
                    "todo.reopen".into(),
                    json!({"status": "done", "completed_at": "2026-11-03T02:00:00.000Z"}),
                    json!({"status": "open", "completed_at": null}),
                ),
            ]
        );
    }

    #[test]
    fn complete_ignores_editing_but_not_carry_or_save_state() {
        let (db, _dir) = open();
        let conn = db.connection();
        let item = todo(conn, "a");
        let b = blocker(conn, "b");
        assert_eq!(
            invalid(complete_todo(conn, &cli(), b.id, at(T1))),
            InvalidArgument::WrongKind {
                item_kind: ItemKind::Blocker,
                use_operation: Some(ItemOperation::ResolveBlocker),
            }
        );
        assert_eq!(
            invalid(reopen_todo(conn, &cli(), b.id, at(T1))),
            InvalidArgument::WrongKind {
                item_kind: ItemKind::Blocker,
                use_operation: None,
            }
        );
        // 本文の入力中でも受け付ける
        let editing = FakeUi::editing(item.id, ItemField::Text);
        assert!(
            complete_todo(conn, &with_ui(&editing), item.id, at(T1))
                .unwrap()
                .updated
        );
        // 何も変えない操作でも、保存の失敗中は storage_unavailable
        let failing = FakeUi {
            save_failing: true,
            ..FakeUi::default()
        };
        assert!(matches!(
            complete_todo(conn, &with_ui(&failing), item.id, at(T1)),
            Err(ItemError::StorageUnavailable)
        ));
        // 持ち越し済みなら、すでに目的の状態でも carried_over
        let carried = carry(conn, &get(conn, item.id), "2026-11-04");
        assert!(matches!(
            conflict(complete_todo(conn, &cli(), item.id, at(T1))),
            Conflict::CarriedOver { carried_to_id, .. } if carried_to_id == carried.id
        ));
        assert!(matches!(
            conflict(reopen_todo(conn, &cli(), item.id, at(T1))),
            Conflict::CarriedOver { .. }
        ));
    }

    // ---- 解消 ----

    #[test]
    fn resolve_without_a_note() {
        let (db, _dir) = open();
        let conn = db.connection();
        let b = blocker(conn, "b");
        let r = resolve_blocker(conn, &cli(), b.id, None, at(T1)).unwrap();
        assert!(r.resolved && !r.note_written && r.updated());
        assert_eq!(r.item.status, ItemStatus::Resolved);
        assert_eq!(r.item.completed_at, Some(at(T1)));
        let again = resolve_blocker(conn, &cli(), b.id, None, at(T2)).unwrap();
        assert!(!again.updated());
        assert_eq!(again.item, r.item);
        let log = logs(conn, b.id).pop().unwrap();
        assert_eq!(log.operation, "blocker.resolve");
        assert_eq!(
            json(&log.after),
            json!({"status": "resolved", "completed_at": "2026-11-03T02:00:00.000Z"})
        );
        assert_eq!(logs(conn, b.id).len(), 2);

        let t = todo(conn, "t");
        assert_eq!(
            invalid(resolve_blocker(conn, &cli(), t.id, None, at(T1))),
            InvalidArgument::WrongKind {
                item_kind: ItemKind::Todo,
                use_operation: Some(ItemOperation::CompleteTodo),
            }
        );
    }

    #[test]
    fn resolve_with_a_note() {
        let (db, _dir) = open();
        let conn = db.connection();
        let b = blocker(conn, "b");
        // 空の解消メモ（空白だけを含む）なら書く
        conn.execute(
            "UPDATE item SET resolution_note = ' \u{3000}' WHERE id = ?1",
            [b.id],
        )
        .unwrap();
        let r = resolve_blocker(conn, &cli(), b.id, Some(" DNS が原因\n"), at(T1)).unwrap();
        assert!(r.resolved && r.note_written);
        assert_eq!(r.item.resolution_note.as_deref(), Some("DNS が原因"));
        let log = logs(conn, b.id).pop().unwrap();
        assert_eq!(
            json(&log.before),
            json!({"status": "open", "completed_at": null, "resolution_note": " \u{3000}"})
        );
        assert_eq!(json(&log.after)["resolution_note"], "DNS が原因");

        // 同じ本文なら何も変えない
        let again = resolve_blocker(conn, &cli(), b.id, Some("DNS が原因"), at(T2)).unwrap();
        assert!(!again.updated());
        // 違う本文なら note_exists
        assert_eq!(
            conflict(resolve_blocker(conn, &cli(), b.id, Some("別"), at(T2))),
            Conflict::NoteExists {
                resolution_note: "DNS が原因".into()
            }
        );

        // 未解消に戻しても解消メモは残る。同じ本文なら、メモは書かずに解消にする
        set_closed(conn, b.id, None, at(T2)).unwrap();
        let r = resolve_blocker(conn, &cli(), b.id, Some("DNS が原因"), at(T2)).unwrap();
        assert!(r.resolved && !r.note_written);
        assert!(
            !json(&logs(conn, b.id).pop().unwrap().after)
                .as_object()
                .unwrap()
                .contains_key("resolution_note")
        );
        // 違う本文なら、解消にもしない
        set_closed(conn, b.id, None, at(T2)).unwrap();
        assert!(matches!(
            conflict(resolve_blocker(conn, &cli(), b.id, Some("別"), at(T2))),
            Conflict::NoteExists { .. }
        ));
        assert_eq!(get(conn, b.id).status, ItemStatus::Open);

        // 解消済みの障害に、解消メモだけを書く
        let c = blocker(conn, "c");
        resolve_blocker(conn, &cli(), c.id, None, at(T1)).unwrap();
        let r = resolve_blocker(conn, &cli(), c.id, Some("後から"), at(T2)).unwrap();
        assert!(!r.resolved && r.note_written);
        assert_eq!(r.item.completed_at, Some(at(T1)));
        assert_eq!(
            json(&logs(conn, c.id).pop().unwrap().after),
            json!({"resolution_note": "後から"})
        );

        for (note, e) in [
            ("  ", InvalidArgument::EmptyNote),
            ("a\nb", InvalidArgument::MultilineNote),
        ] {
            assert_eq!(
                invalid(resolve_blocker(conn, &cli(), c.id, Some(note), at(T2))),
                e
            );
        }
    }

    #[test]
    fn resolve_checks_carry_then_editing_then_the_note() {
        let (db, _dir) = open();
        let conn = db.connection();
        let b = blocker(conn, "b");
        resolve_blocker(conn, &cli(), b.id, Some("メモ"), at(T1)).unwrap();
        let editing = FakeUi::editing(b.id, ItemField::ResolutionNote);
        // 解消メモの入力中は、note 付きなら user_editing（note_exists より先）
        assert_eq!(
            conflict(resolve_blocker(
                conn,
                &with_ui(&editing),
                b.id,
                Some("別"),
                at(T2)
            )),
            Conflict::UserEditing
        );
        // note を付けなければ入力中でも受け付ける。本文の入力中も関係ない
        assert!(resolve_blocker(conn, &with_ui(&editing), b.id, None, at(T2)).is_ok());
        let text_editing = FakeUi::editing(b.id, ItemField::Text);
        assert!(matches!(
            conflict(resolve_blocker(
                conn,
                &with_ui(&text_editing),
                b.id,
                Some("別"),
                at(T2)
            )),
            Conflict::NoteExists { .. }
        ));
        // 持ち越し済みなら、入力中より先に carried_over
        set_closed(conn, b.id, None, at(T2)).unwrap();
        carry(conn, &get(conn, b.id), "2026-11-04");
        assert!(matches!(
            conflict(resolve_blocker(
                conn,
                &with_ui(&editing),
                b.id,
                Some("別"),
                at(T2)
            )),
            Conflict::CarriedOver { .. }
        ));
    }

    // ---- 持ち越し ----

    #[test]
    fn carried_to_follows_deleted_items() {
        let (db, _dir) = open();
        let conn = db.connection();
        let a = todo(conn, "a");
        assert_eq!(carried_to(conn, a.id).unwrap(), None);
        let b = carry(conn, &a, "2026-11-04");
        let c = carry(conn, &b, "2026-11-05");
        assert_eq!(carried_to(conn, a.id).unwrap().unwrap().id, b.id);
        // B を削除しても、A は C へ持ち越し済み
        delete(conn, b.id, at(T1)).unwrap();
        assert_eq!(carried_to(conn, a.id).unwrap().unwrap().id, c.id);
        assert!(matches!(
            conflict(edit_todo(conn, &cli(), a.id, "x", at(T1))),
            Conflict::CarriedOver { carried_to_id, carried_to_date }
                if carried_to_id == c.id && carried_to_date == date("2026-11-05")
        ));
        // 画面でも本文・状態・解消メモは変えられない
        assert!(matches!(
            conflict(save_text(conn, a.id, "x", at(T1))),
            Conflict::CarriedOver { .. }
        ));
        assert!(matches!(
            conflict(set_closed(conn, a.id, Some(at(T1)), at(T1))),
            Conflict::CarriedOver { .. }
        ));
        // 行き着く項目が無くなれば、通常どおり受け付ける
        delete(conn, c.id, at(T1)).unwrap();
        assert_eq!(carried_to(conn, a.id).unwrap(), None);
        assert!(edit_todo(conn, &cli(), a.id, "x", at(T1)).unwrap().updated);
    }

    #[test]
    fn carried_blockers_keep_their_note() {
        let (db, _dir) = open();
        let conn = db.connection();
        let b = blocker(conn, "b");
        carry(conn, &b, "2026-11-04");
        assert!(matches!(
            conflict(save_resolution_note(conn, b.id, "x", at(T1))),
            Conflict::CarriedOver { .. }
        ));
        assert!(matches!(
            conflict(edit_blocker(conn, &cli(), b.id, "b", at(T1))),
            Conflict::CarriedOver { .. }
        ));
        // 並べ替えと削除はできる
        move_item(conn, b.id, Position::First, at(T1)).unwrap();
        delete(conn, b.id, at(T1)).unwrap();
    }

    // ---- 画面の操作 ----

    #[test]
    fn screen_add_places_items_and_records_nothing() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        let add_at = |text: &str, position| {
            add(conn, d, ItemKind::Todo, text, position, &tokyo(), at(T0)).unwrap()
        };
        let b = add_at("b", Position::Last);
        let d_item = add_at("d", Position::Last);
        add_at("a", Position::First);
        add_at("c", Position::After(b.id));
        add_at("c2", Position::Before(d_item.id));
        assert_eq!(texts(conn, "2026-11-03"), ["a", "b", "c", "c2", "d"]);
        assert_eq!(b.origin, Origin::Human);
        assert_eq!(b.agent_client_id, None);
        assert!(logs(conn, b.id).is_empty());
        assert_eq!(day_table::get(conn, d).unwrap().unwrap().tz, "Asia/Tokyo");

        // 基準の項目が別の日・無い・削除済みなら誤り
        let other = add(
            conn,
            date("2026-11-04"),
            ItemKind::Todo,
            "x",
            Position::Last,
            &tokyo(),
            at(T0),
        )
        .unwrap();
        delete(conn, d_item.id, at(T1)).unwrap();
        for p in [
            Position::After(other.id),
            Position::Before(Ulid::generate()),
            Position::After(d_item.id),
        ] {
            assert_eq!(
                invalid(add(conn, d, ItemKind::Blocker, "x", p, &tokyo(), at(T0))),
                InvalidArgument::InvalidPosition
            );
        }
        assert_eq!(
            invalid(add(
                conn,
                d,
                ItemKind::Todo,
                " ",
                Position::Last,
                &tokyo(),
                at(T0)
            )),
            InvalidArgument::EmptyText
        );
    }

    #[test]
    fn screen_text_and_status_are_written_field_by_field() {
        let (db, _dir) = open();
        let conn = db.connection();
        let item = todo(conn, "a");
        // 画面の保存の前にエージェントが完了にしても、本文の保存は状態を戻さない
        complete_todo(conn, &cli(), item.id, at(T1)).unwrap();
        let out = save_text(conn, item.id, " ab ", at(T2)).unwrap();
        assert!(out.updated);
        assert_eq!(out.item.text, "ab");
        assert_eq!(out.item.status, ItemStatus::Done);
        assert!(!save_text(conn, item.id, "ab", at(T2)).unwrap().updated);
        assert_eq!(
            invalid(save_text(conn, item.id, "", at(T2))),
            InvalidArgument::EmptyText
        );

        // 画面の切り替えは添えた時刻を書く。人の操作は記録しない
        let n = logs(conn, item.id).len();
        let out = set_closed(conn, item.id, None, at(T2)).unwrap();
        assert_eq!(
            (out.item.status, out.item.completed_at),
            (ItemStatus::Open, None)
        );
        let out = set_closed(conn, item.id, Some(at(T1)), at(T2)).unwrap();
        assert_eq!(
            (out.item.status, out.item.completed_at),
            (ItemStatus::Done, Some(at(T1)))
        );
        // 同じ状態でも、時刻が違えば時刻を書く
        let out = set_closed(conn, item.id, Some(at(T0)), at(T2)).unwrap();
        assert!(out.updated);
        assert_eq!(out.item.completed_at, Some(at(T0)));
        assert!(
            !set_closed(conn, item.id, Some(at(T0)), at(T2))
                .unwrap()
                .updated
        );
        assert_eq!(logs(conn, item.id).len(), n);
    }

    #[test]
    fn screen_resolution_notes() {
        let (db, _dir) = open();
        let conn = db.connection();
        let b = blocker(conn, "b");
        set_closed(conn, b.id, Some(at(T1)), at(T1)).unwrap();
        let out = save_resolution_note(conn, b.id, " メモ ", at(T1)).unwrap();
        assert_eq!(out.item.resolution_note.as_deref(), Some("メモ"));
        assert!(
            !save_resolution_note(conn, b.id, "メモ", at(T1))
                .unwrap()
                .updated
        );
        // 未解消に戻しても残る
        let out = set_closed(conn, b.id, None, at(T2)).unwrap();
        assert_eq!(out.item.resolution_note.as_deref(), Some("メモ"));
        assert_eq!(out.item.completed_at, None);
        // 空にすると消す
        let out = save_resolution_note(conn, b.id, " ", at(T2)).unwrap();
        assert_eq!(out.item.resolution_note, None);
        assert_eq!(
            invalid(save_resolution_note(conn, b.id, "a\nb", at(T2))),
            InvalidArgument::MultilineNote
        );
        let t = todo(conn, "t");
        assert!(matches!(
            invalid(save_resolution_note(conn, t.id, "x", at(T2))),
            InvalidArgument::WrongKind {
                item_kind: ItemKind::Todo,
                use_operation: None
            }
        ));
        assert!(logs(conn, b.id).len() == 1);
    }

    #[test]
    fn delete_is_logical_and_happens_once() {
        let (db, _dir) = open();
        let conn = db.connection();
        let item = todo(conn, "a");
        delete(conn, item.id, at(T1)).unwrap();
        assert_eq!(get(conn, item.id).deleted_at, Some(at(T1)));
        assert!(texts(conn, "2026-11-03").is_empty());
        assert!(matches!(
            delete(conn, item.id, at(T2)),
            Err(ItemError::NotFound)
        ));
        assert!(matches!(
            delete(conn, Ulid::generate(), at(T2)),
            Err(ItemError::NotFound)
        ));
    }

    #[test]
    fn move_writes_only_the_moved_item() {
        let (db, _dir) = open();
        let conn = db.connection();
        let a = todo(conn, "a");
        let b = todo(conn, "b");
        let c = todo(conn, "c");
        let out = move_item(conn, c.id, Position::Before(a.id), at(T1)).unwrap();
        assert!(out.updated);
        assert_eq!(texts(conn, "2026-11-03"), ["c", "a", "b"]);
        assert_eq!(get(conn, a.id), a);
        assert_eq!(get(conn, b.id), b);
        // すでにその位置なら書かない
        for p in [Position::First, Position::Before(a.id)] {
            assert!(!move_item(conn, c.id, p, at(T2)).unwrap().updated);
        }
        move_item(conn, a.id, Position::Last, at(T1)).unwrap();
        assert_eq!(texts(conn, "2026-11-03"), ["c", "b", "a"]);
        move_item(conn, a.id, Position::After(c.id), at(T1)).unwrap();
        assert_eq!(texts(conn, "2026-11-03"), ["c", "a", "b"]);
        // 自分自身は基準にできない
        assert_eq!(
            invalid(move_item(conn, a.id, Position::After(a.id), at(T1))),
            InvalidArgument::InvalidPosition
        );
        assert!(logs(conn, a.id).len() == 1);
    }

    #[test]
    fn keys_are_renumbered_when_there_is_no_room() {
        let (db, _dir) = open();
        let conn = db.connection();
        let a = todo(conn, "a");
        let b = todo(conn, "b");
        let c = todo(conn, "c");
        // 同じキーや、形に当てはまらないキー
        conn.execute("UPDATE item SET sort_key = 'a0'", []).unwrap();
        conn.execute("UPDATE item SET sort_key = 'b' WHERE id = ?1", [c.id])
            .unwrap();
        // a・b は同じキー（id の順）。その間に入れる
        let (first, second) = if a.id < b.id { (a, b) } else { (b, a) };
        move_item(conn, c.id, Position::After(first.id), at(T1)).unwrap();
        let items = item_table::list_by_date(conn, date("2026-11-03")).unwrap();
        let ids: Vec<_> = items.iter().map(|i| i.id).collect();
        assert_eq!(ids, [first.id, c.id, second.id]);
        assert!(items.iter().all(|i| sort_key::is_valid(&i.sort_key)));
        assert!(items.windows(2).all(|w| w[0].sort_key < w[1].sort_key));
    }

    #[test]
    fn split_makes_a_new_item_right_after() {
        let (db, _dir) = open();
        let conn = db.connection();
        let a = blocker(conn, "API の調査と修正");
        let z = blocker(conn, "z");
        resolve_blocker(conn, &cli(), a.id, Some("メモ"), at(T1)).unwrap();
        let (first, second) = split(conn, a.id, "API の調査 ", "と修正", at(T2)).unwrap();
        assert_eq!(first.text, "API の調査");
        assert_eq!(first.status, ItemStatus::Resolved);
        assert_eq!(first.resolution_note.as_deref(), Some("メモ"));
        assert_eq!(second.text, "と修正");
        assert_eq!(second.kind, ItemKind::Blocker);
        assert_eq!(second.status, ItemStatus::Open);
        assert_eq!(second.resolution_note, None);
        assert_eq!(second.origin, Origin::Human);
        assert_eq!(second.split_from_id, None);
        assert_eq!(texts(conn, "2026-11-03"), ["API の調査", "と修正", "z"]);
        assert_eq!(get(conn, z.id), z);
        assert!(logs(conn, second.id).is_empty());
        for (f, s) in [("", "x"), ("x", " ")] {
            assert_eq!(
                invalid(split(conn, a.id, f, s, at(T2))),
                InvalidArgument::EmptyText
            );
        }
        carry(conn, &get(conn, z.id), "2026-11-04");
        assert!(matches!(
            conflict(split(conn, z.id, "a", "b", at(T2))),
            Conflict::CarriedOver { .. }
        ));
    }

    #[test]
    fn writes_join_the_callers_transaction() {
        let (db, _dir) = open();
        let d = date("2026-11-03");
        {
            let tx = db.connection().unchecked_transaction().unwrap();
            add_todo(&tx, &cli(), d, "x", &tokyo(), at(T0)).unwrap();
        }
        let conn = db.connection();
        assert!(texts(conn, "2026-11-03").is_empty());
        assert_eq!(day_table::get(conn, d).unwrap(), None);
    }
}
