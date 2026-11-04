//! エージェントの変更履歴の一覧と、変更の取り消し（要件 A-08。MCP・CLI 仕様書 7章、
//! データモデル設計書 change_log、アーキテクチャ設計書 4.1）。
//!
//! 利用者が行う管理の操作で、MCP には出さない。呼び出し元のスコープは確かめない（確認は呼び出し側の
//! 責務で、防御の境界ではない。MCP・CLI 仕様書 2章）。
//!
//! - [`list_changes`]：新しい順の一覧。エージェントの記録と、人の取り消しの記録（`agent.revert`）の
//!   両方を返す。確認済みの印（`seen_at`）は付けない（変更履歴の画面で確認したときに付ける）。
//! - [`preview`]：取り消すと何が起きるか（何も書かない）。確認の前に見せる。
//! - [`revert`]：取り消しを行う。対象は、その記録の操作が変えた欄だけ（ほかの欄に人が加えた変更は
//!   残る）。人があとから変えていれば `conflict`（`reason: changed`）で、`force` が越えるのはこの
//!   判定だけ。取り消しは `before` を書き戻す新しい操作で、取り消し自体も `revert_of_id` 付きで
//!   記録する（人の操作なので `actor = human`、`agent_client_id` は空）。

use std::collections::HashMap;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use rusqlite::Connection;
use serde_json::{Map, Value, json};

use super::agent::{StorageUnavailable, UiState};
use super::item::{CloseReason, Item, ItemKind, ItemStatus, item_json};
use super::text_doc::{self, TextDoc, json_string};
use crate::domain::day;
use crate::domain::logical_date::LogicalDate;
use crate::storage::table::change_log::{self, Actor, ChangeLog, RecentFilter, Target};
use crate::storage::table::item::{self as item_table, CarriedTo, ItemChanges};
use crate::storage::table::text_doc as text_doc_table;
use crate::storage::table::{agent_client, format_time};
use crate::storage::{StorageError, Ulid, in_read_transaction, in_transaction};

/// 一覧の件数の既定。
pub const DEFAULT_LIMIT: usize = 50;
/// 一覧の件数の上限（これを超える指定はこの件数にする）。
pub const MAX_LIMIT: usize = 200;
/// 差分に出す行数の上限（取り消す側と戻す側のそれぞれ）。
pub const MAX_DIFF_LINES: usize = 200;

/// 追加の取り消しで「人があとから変えたか」を調べる欄。並び順（`sort_key`）は中身ではない
/// （並べ替えだけで必ず食い違うことを避ける）。
const CONTENT_FIELDS: [&str; 5] = [
    "text",
    "status",
    "close_reason",
    "resolution_note",
    "completed_at",
];

// ---- 一覧 ----

/// 一覧の絞り込み。
#[derive(Debug, Clone, Default)]
pub struct ChangeFilter {
    /// このクライアント（識別子）の記録だけ。
    pub client: Option<String>,
    /// エージェントによる未確認の変更だけ。
    pub unseen_only: bool,
    /// この記録より前（古い）のものだけ（続きを読むための目印）。
    pub before: Option<Ulid>,
    /// 件数。0 は誤り、[`MAX_LIMIT`] を超えれば [`MAX_LIMIT`]。
    pub limit: usize,
}

/// 一覧の1件。
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub log: ChangeLog,
    /// 操作したクライアントの識別子。人の取り消しの記録は `None`。
    pub client_name: Option<String>,
    /// 対象が項目のとき、その項目のいまの様子（削除されていても入る）。
    pub item: Option<ItemRef>,
    pub summary: Summary,
}

/// 対象の項目のいまの様子（一覧に出す行を作るため）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRef {
    pub date: LogicalDate,
    pub kind: ItemKind,
    pub text: String,
    pub deleted: bool,
}

/// 記録が何を変えたか。
#[derive(Debug, Clone, PartialEq)]
pub enum Summary {
    /// 項目を足した。
    ItemAdded { kind: Option<String>, text: String },
    /// 項目の欄を変えた（変えた欄だけ）。
    Fields(Vec<FieldChange>),
    /// ノート・振り返りの本文を変えた（本文は大きいので、文字数だけ）。
    Text {
        before_chars: usize,
        after_chars: usize,
    },
    /// 読み取れない形（壊れた記録）。
    Unknown,
}

/// 項目の1つの欄の変更。
#[derive(Debug, Clone, PartialEq)]
pub struct FieldChange {
    pub name: String,
    pub before: Value,
    pub after: Value,
}

/// 一覧の失敗。
#[derive(Debug)]
pub enum HistoryError {
    /// 指定したクライアントが無い。
    ClientNotFound,
    /// 件数が 0。
    ZeroLimit,
    Storage(StorageError),
}

impl std::fmt::Display for HistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClientNotFound => f.write_str("the client was not found"),
            Self::ZeroLimit => f.write_str("the limit must be 1 or more"),
            Self::Storage(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for HistoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(e) => Some(e),
            _ => None,
        }
    }
}

impl From<StorageError> for HistoryError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

impl From<rusqlite::Error> for HistoryError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Storage(e.into())
    }
}

/// 変更履歴を新しい順に返す。
pub fn list_changes(conn: &Connection, filter: &ChangeFilter) -> Result<Vec<Entry>, HistoryError> {
    if filter.limit == 0 {
        return Err(HistoryError::ZeroLimit);
    }
    let limit = filter.limit.min(MAX_LIMIT);
    in_read_transaction(conn, || {
        let clients: HashMap<Ulid, String> = agent_client::list(conn)?
            .into_iter()
            .map(|c| (c.id, c.name))
            .collect();
        let client = match &filter.client {
            None => None,
            Some(name) => Some(
                clients
                    .iter()
                    .find(|(_, n)| *n == name)
                    .map(|(id, _)| *id)
                    .ok_or(HistoryError::ClientNotFound)?,
            ),
        };
        let logs = change_log::list_recent(
            conn,
            &RecentFilter {
                client,
                unseen_only: filter.unseen_only,
                before: filter.before,
            },
            limit,
        )?;
        let item_ids: Vec<Ulid> = logs
            .iter()
            .filter_map(|l| match l.target {
                Target::Item(id) => Some(id),
                _ => None,
            })
            .collect();
        let items = item_table::get_many(conn, &item_ids)?;
        Ok(logs
            .into_iter()
            .map(|log| Entry {
                client_name: log.agent_client_id.and_then(|id| clients.get(&id).cloned()),
                item: match log.target {
                    Target::Item(id) => items.get(&id).map(|i| ItemRef {
                        date: i.date,
                        kind: i.kind,
                        text: i.text.clone(),
                        deleted: i.deleted_at.is_some(),
                    }),
                    _ => None,
                },
                summary: summarize(&log),
                log,
            })
            .collect())
    })
}

fn object(s: &Option<String>) -> Option<Map<String, Value>> {
    match serde_json::from_str(s.as_deref()?) {
        Ok(Value::Object(m)) => Some(m),
        _ => None,
    }
}

fn body_of(s: &Option<String>) -> Option<String> {
    match serde_json::from_str(s.as_deref()?) {
        Ok(Value::String(b)) => Some(b),
        _ => None,
    }
}

fn summarize(log: &ChangeLog) -> Summary {
    match log.target {
        Target::Item(_) => {
            let Some(after) = object(&log.after) else {
                return Summary::Unknown;
            };
            match object(&log.before) {
                None if log.before.is_none() => Summary::ItemAdded {
                    kind: after.get("kind").and_then(Value::as_str).map(str::to_owned),
                    text: after
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                },
                None => Summary::Unknown,
                Some(before) => Summary::Fields(
                    after
                        .into_iter()
                        .map(|(name, after)| FieldChange {
                            before: before.get(&name).cloned().unwrap_or(Value::Null),
                            name,
                            after,
                        })
                        .collect(),
                ),
            }
        }
        Target::Note(_) | Target::Reflection(_) => {
            match (body_of(&log.before), body_of(&log.after)) {
                (Some(before), Some(after)) => Summary::Text {
                    before_chars: before.chars().count(),
                    after_chars: after.chars().count(),
                },
                _ => Summary::Unknown,
            }
        }
    }
}

// ---- 取り消し ----

/// 取り消しの対象が無い。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Missing {
    /// その ID の記録が無い（利用者の保存と行き違った記録は消えている。アーキテクチャ設計書 4.1）。
    Change,
    /// 記録の対象の項目が削除されている（追加の取り消しを除く）。
    Item,
}

/// 取り消せない記録・状態（`invalid_argument`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevertArgument {
    /// エージェントの変更の記録ではない（取り消しの記録を指した）。
    NotAgentChange,
    /// 読み取れない形の記録。
    Unsupported,
    /// 対象の TODO は取りやめられている。取りやめた TODO の状態と本文は、画面でも変えられない
    /// （データモデル設計書 3章 item）。
    Dropped { close_reason: CloseReason },
}

/// 取り消せない状態（`conflict`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevertConflict {
    /// 人があとから変えている。`fields` は食い違う欄（ノートは `body`）。`after_preview` が真なら、
    /// 確認の画面を見せた後に変わった（`force` でも越えない）。
    Changed {
        fields: Vec<String>,
        after_preview: bool,
    },
    /// 対象の項目は持ち越し済みで、その日の終わりの状態として残す（データモデル設計書 3章 item）。
    CarriedOver {
        carried_to_id: Ulid,
        carried_to_date: LogicalDate,
    },
    /// 利用者が、書き戻す欄を入力中。
    UserEditing,
}

/// 取り消しの失敗。どれも何も書いていない。
#[derive(Debug)]
pub enum RevertError {
    NotFound(Missing),
    InvalidArgument(RevertArgument),
    /// 画面の保存の書き込みに失敗して再試行している間（アーキテクチャ設計書 4.1）。
    StorageUnavailable(StorageUnavailable),
    Conflict(RevertConflict),
    Storage(StorageError),
}

impl std::fmt::Display for RevertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(Missing::Change) => f.write_str("the change record was not found"),
            Self::NotFound(Missing::Item) => f.write_str("the item was deleted"),
            Self::InvalidArgument(RevertArgument::NotAgentChange) => {
                f.write_str("the record is not a change made by an agent")
            }
            Self::InvalidArgument(RevertArgument::Unsupported) => {
                f.write_str("the record cannot be read")
            }
            Self::InvalidArgument(RevertArgument::Dropped { close_reason }) => {
                write!(
                    f,
                    "the TODO was closed ({close_reason}) and cannot be changed"
                )
            }
            Self::StorageUnavailable(e) => e.fmt(f),
            Self::Conflict(RevertConflict::Changed { .. }) => {
                f.write_str("the target was changed after the record")
            }
            Self::Conflict(RevertConflict::CarriedOver {
                carried_to_date, ..
            }) => write!(f, "the item was carried over to {carried_to_date}"),
            Self::Conflict(RevertConflict::UserEditing) => {
                f.write_str("the user is editing the target")
            }
            Self::Storage(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for RevertError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(e) => Some(e),
            _ => None,
        }
    }
}

impl From<StorageError> for RevertError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

impl From<rusqlite::Error> for RevertError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Storage(e.into())
    }
}

impl From<StorageUnavailable> for RevertError {
    fn from(e: StorageUnavailable) -> Self {
        Self::StorageUnavailable(e)
    }
}

/// 取り消しで何が起きるか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanState {
    /// 書き戻す（または削除する）。
    Apply,
    /// すでに取り消し済み。何も書かない。
    AlreadyReverted,
    /// いまの値がすでに戻したい値と同じ（人が先に戻した、など）。何も書かず、記録を取り消し済みにする。
    AlreadyRestored,
}

/// 取り消しの計画（何を書き戻すか）。
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub log: ChangeLog,
    pub client_name: Option<String>,
    pub state: PlanState,
    pub effect: Effect,
    /// 人があとから変えた欄（記録の `after` といまの値が違う欄。ノートは `body`）。空でなければ、
    /// `force` なしでは取り消せない。
    pub drift: Vec<String>,
    /// 確認の画面を見せたいまの状態を表す文字列。[`RevertRequest::expect`] に渡すと、確認の後に
    /// 変わっていれば書かずに止める。
    pub token: String,
}

/// 書き込む内容。
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// 何も書かない（すでに取り消し済み）。
    None,
    /// 追加の取り消し：項目を削除する（画面の削除と同じ。持ち越し先があっても、持ち越し先は残る）。
    DeleteItem {
        item: Item,
        carried_to: Option<CarriedTo>,
    },
    /// 項目の欄を、記録の `before` に戻す。
    Fields { item: Item, fields: Vec<FieldPlan> },
    /// ノート・振り返りの本文を、記録の `before` に戻す。
    Text {
        doc: TextDoc,
        date: LogicalDate,
        diff: TextDiff,
    },
}

/// 戻す1つの欄。
#[derive(Debug, Clone, PartialEq)]
pub struct FieldPlan {
    pub name: String,
    pub current: Value,
    pub restore: Value,
    /// 記録の `after` と違う（人があとから変えた）。
    pub changed_since: bool,
}

/// 本文の差分（いまの本文から、戻す本文への）。共通の先頭と末尾の行を除いた、変わる部分の行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextDiff {
    /// いまの本文にあって、戻すと無くなる行。
    pub removed: Vec<String>,
    /// 戻すと入る行。
    pub added: Vec<String>,
    /// 行数が [`MAX_DIFF_LINES`] を超えて、切り詰めた。
    pub truncated: bool,
}

fn text_diff(current: &str, restore: &str) -> TextDiff {
    let a: Vec<&str> = current.split('\n').collect();
    let b: Vec<&str> = restore.split('\n').collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let cut = |lines: &[&str]| {
        (
            lines
                .iter()
                .take(MAX_DIFF_LINES)
                .map(|l| (*l).to_owned())
                .collect::<Vec<_>>(),
            lines.len() > MAX_DIFF_LINES,
        )
    };
    let (removed, r) = cut(&a[prefix..a.len() - suffix]);
    let (added, w) = cut(&b[prefix..b.len() - suffix]);
    TextDiff {
        removed,
        added,
        truncated: r || w,
    }
}

/// 項目 `item` の欄 `name` の値（記録の `before` / `after` と同じ形）。
fn field_value(item: &Item, name: &str) -> Option<Value> {
    Some(match name {
        "text" => json!(item.text),
        "status" => json!(item.status.as_str()),
        "close_reason" => json!(item.close_reason.map(CloseReason::as_str)),
        "sort_key" => json!(item.sort_key),
        "resolution_note" => json!(item.resolution_note),
        "completed_at" => item
            .completed_at
            .map_or(Value::Null, |t| Value::String(format_time(t))),
        _ => return None,
    })
}

/// 記録の欄の値から、書き戻す内容を作る。形が合わなければ `None`。
fn changes_from(values: &[(String, Value)]) -> Option<ItemChanges> {
    let mut changes = ItemChanges::default();
    for (name, value) in values {
        match name.as_str() {
            "text" => changes.text = Some(value.as_str()?.to_owned()),
            "status" => changes.status = Some(value.as_str()?.parse::<ItemStatus>().ok()?),
            "close_reason" => {
                changes.close_reason = Some(match value {
                    Value::Null => None,
                    v => Some(v.as_str()?.parse::<CloseReason>().ok()?),
                });
            }
            "sort_key" => changes.sort_key = Some(value.as_str()?.to_owned()),
            "resolution_note" => {
                changes.resolution_note = Some(match value {
                    Value::Null => None,
                    v => Some(v.as_str()?.to_owned()),
                });
            }
            "completed_at" => {
                changes.completed_at = Some(match value {
                    Value::Null => None,
                    v => Some(v.as_str()?.parse::<Timestamp>().ok()?),
                });
            }
            _ => return None,
        }
    }
    Some(changes)
}

/// 取り消しの計画を作る（何も書かない）。確認の画面に見せるために使う。
pub fn preview(conn: &Connection, change_id: Ulid) -> Result<Plan, RevertError> {
    in_read_transaction(conn, || plan_in(conn, change_id))
}

/// トランザクションの中で、取り消しの計画を作る。判定の順は 05 7章：記録が無い、取り消しの記録、
/// 取り消し済み、項目が削除済み、持ち越し済み、取りやめた TODO。人があとから変えたか（`drift`）は
/// 計画に入れ、拒むかは [`revert`] が `force` で決める。
fn plan_in(conn: &Connection, change_id: Ulid) -> Result<Plan, RevertError> {
    let log = change_log::get(conn, change_id)?.ok_or(RevertError::NotFound(Missing::Change))?;
    if log.actor != Actor::Agent || log.revert_of_id.is_some() {
        return Err(RevertError::InvalidArgument(RevertArgument::NotAgentChange));
    }
    let client_name = match log.agent_client_id {
        Some(id) => agent_client::get(conn, id)?.map(|c| c.name),
        None => None,
    };
    let done = |log: ChangeLog, client_name, state, effect| Plan {
        log,
        client_name,
        state,
        effect,
        drift: Vec::new(),
        token: String::new(),
    };
    if log.reverted_at.is_some() || change_log::is_reverted_by_any(conn, change_id)? {
        return Ok(done(
            log,
            client_name,
            PlanState::AlreadyReverted,
            Effect::None,
        ));
    }
    let unsupported = || RevertError::InvalidArgument(RevertArgument::Unsupported);
    match log.target {
        Target::Item(item_id) => {
            let item = item_table::get(conn, item_id)?;
            let after = object(&log.after).ok_or_else(unsupported)?;
            if log.before.is_none() {
                // 追加の取り消し：項目の削除
                let item = item.ok_or(RevertError::NotFound(Missing::Item))?;
                if item.deleted_at.is_some() {
                    return Ok(done(
                        log,
                        client_name,
                        PlanState::AlreadyRestored,
                        Effect::DeleteItem {
                            item,
                            carried_to: None,
                        },
                    ));
                }
                let drift: Vec<String> = CONTENT_FIELDS
                    .iter()
                    .filter(|name| field_value(&item, name) != after.get(**name).cloned())
                    .map(|name| (*name).to_owned())
                    .collect();
                let token = token_of(&item, &CONTENT_FIELDS);
                let carried_to = item_table::carried_to(conn, item_id)?;
                return Ok(Plan {
                    log,
                    client_name,
                    state: PlanState::Apply,
                    effect: Effect::DeleteItem { item, carried_to },
                    drift,
                    token,
                });
            }
            let before = object(&log.before).ok_or_else(unsupported)?;
            let item = match item {
                Some(i) if i.deleted_at.is_none() => i,
                _ => return Err(RevertError::NotFound(Missing::Item)),
            };
            if let Some(c) = item_table::carried_to(conn, item_id)? {
                return Err(RevertError::Conflict(RevertConflict::CarriedOver {
                    carried_to_id: c.id,
                    carried_to_date: c.date,
                }));
            }
            if item.status == ItemStatus::Dropped {
                return Err(RevertError::InvalidArgument(RevertArgument::Dropped {
                    close_reason: item.close_reason.unwrap_or(CloseReason::Dropped),
                }));
            }
            if after.is_empty() {
                return Err(unsupported());
            }
            let mut fields = Vec::new();
            for (name, after_value) in &after {
                let current = field_value(&item, name).ok_or_else(unsupported)?;
                let restore = before.get(name).cloned().ok_or_else(unsupported)?;
                fields.push(FieldPlan {
                    // すでに戻したい値になっている欄は、上書きするものが無いので、変えられた扱いにしない
                    changed_since: &current != after_value && current != restore,
                    name: name.clone(),
                    current,
                    restore,
                });
            }
            let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
            let token = token_of(&item, &names);
            if fields.iter().all(|f| f.current == f.restore) {
                return Ok(Plan {
                    log,
                    client_name,
                    state: PlanState::AlreadyRestored,
                    effect: Effect::Fields { item, fields },
                    drift: Vec::new(),
                    token,
                });
            }
            let drift = fields
                .iter()
                .filter(|f| f.changed_since)
                .map(|f| f.name.clone())
                .collect();
            Ok(Plan {
                log,
                client_name,
                state: PlanState::Apply,
                effect: Effect::Fields { item, fields },
                drift,
                token,
            })
        }
        Target::Note(date) | Target::Reflection(date) => {
            let doc = match log.target {
                Target::Note(_) => TextDoc::Note,
                _ => TextDoc::Reflection,
            };
            let before = body_of(&log.before).ok_or_else(unsupported)?;
            let after = body_of(&log.after).ok_or_else(unsupported)?;
            let current = text_doc::get(conn, doc, date)?;
            // 版の番号ではなく本文そのもの（同じ本文の保存で版だけが進んでも、止めないため）
            let token = format!("body:{}", current.body);
            let diff = text_diff(&current.body, &before);
            if current.body == before {
                return Ok(Plan {
                    log,
                    client_name,
                    state: PlanState::AlreadyRestored,
                    effect: Effect::Text { doc, date, diff },
                    drift: Vec::new(),
                    token,
                });
            }
            let drift = if current.body == after {
                Vec::new()
            } else {
                vec!["body".to_owned()]
            };
            Ok(Plan {
                log,
                client_name,
                state: PlanState::Apply,
                effect: Effect::Text { doc, date, diff },
                drift,
                token,
            })
        }
    }
}

/// 項目の欄 `names` のいまの値を表す文字列（確認の後に変わったかを調べる目印）。
fn token_of(item: &Item, names: &[&str]) -> String {
    let values: Vec<Value> = names
        .iter()
        .map(|n| field_value(item, n).unwrap_or(Value::Null))
        .collect();
    Value::Array(values).to_string()
}

/// 取り消しの依頼。
#[derive(Debug, Clone, Copy, Default)]
pub struct RevertRequest<'a> {
    /// 人があとから変えた欄を、記録の前の値で上書きする。確認の画面で見せた値だけを上書きする
    /// （`expect` と合わせて使う）。
    pub force: bool,
    /// 確認の画面を見せたときの [`Plan::token`]。いまの状態と違えば、`force` でも書かずに止める。
    pub expect: Option<&'a str>,
}

/// 取り消しの結果。
#[derive(Debug, Clone, PartialEq)]
pub struct Reverted {
    /// 取り消しの計画（書いた内容。何も書かなかったときもその理由の状態を持つ）。
    pub plan: Plan,
    /// 実際に値を書いたか。すでに取り消し済み・すでに戻っていたときは `false`。
    pub updated: bool,
    /// 書いたときの取り消しの記録（`agent.revert`）の id。
    pub revert_id: Option<Ulid>,
}

/// 記録 `change_id` を取り消す。判定の順は 05 7章。`ui` は画面の状態（アプリの停止中は
/// [`super::agent::NO_UI`]）。
pub fn revert(
    conn: &Connection,
    change_id: Ulid,
    request: &RevertRequest<'_>,
    ui: &dyn UiState,
    tz: &TimeZone,
    now: Timestamp,
) -> Result<Reverted, RevertError> {
    in_transaction(conn, || {
        let plan = plan_in(conn, change_id)?;
        match plan.state {
            PlanState::AlreadyReverted => {
                return Ok(Reverted {
                    plan,
                    updated: false,
                    revert_id: None,
                });
            }
            PlanState::AlreadyRestored => {
                change_log::set_reverted_at(conn, change_id, now)?;
                return Ok(Reverted {
                    plan,
                    updated: false,
                    revert_id: None,
                });
            }
            PlanState::Apply => {}
        }
        if ui.save_failing() {
            return Err(StorageUnavailable.into());
        }
        if is_editing(&plan, ui) {
            return Err(RevertError::Conflict(RevertConflict::UserEditing));
        }
        if request.expect.is_some_and(|e| e != plan.token) {
            return Err(RevertError::Conflict(RevertConflict::Changed {
                fields: plan.drift.clone(),
                after_preview: true,
            }));
        }
        if !plan.drift.is_empty() && !request.force {
            return Err(RevertError::Conflict(RevertConflict::Changed {
                fields: plan.drift.clone(),
                after_preview: false,
            }));
        }
        let (target, before, after) = apply(conn, &plan, tz, now)?;
        let revert_id = Ulid::generate();
        change_log::insert(
            conn,
            &ChangeLog {
                id: revert_id,
                actor: Actor::Human,
                agent_client_id: None,
                operation: REVERT_OPERATION.to_owned(),
                target,
                before: Some(before),
                after: Some(after),
                created_at: now,
                reverted_at: None,
                revert_of_id: Some(change_id),
                seen_at: None,
            },
        )?;
        change_log::set_reverted_at(conn, change_id, now)?;
        Ok(Reverted {
            plan,
            updated: true,
            revert_id: Some(revert_id),
        })
    })
}

/// 取り消しの記録の `operation`。
pub const REVERT_OPERATION: &str = "agent.revert";

/// 利用者が、書き戻す欄（項目の本文・解消メモ、ノート・振り返り）を入力中か。
fn is_editing(plan: &Plan, ui: &dyn UiState) -> bool {
    use super::agent::ItemField;
    match &plan.effect {
        Effect::None => false,
        Effect::DeleteItem { item, .. } => {
            ui.is_editing(item.id, ItemField::Text)
                || ui.is_editing(item.id, ItemField::ResolutionNote)
        }
        Effect::Fields { item, fields } => fields.iter().any(|f| match f.name.as_str() {
            "text" => ui.is_editing(item.id, ItemField::Text),
            "resolution_note" => ui.is_editing(item.id, ItemField::ResolutionNote),
            _ => false,
        }),
        Effect::Text { doc, date, .. } => ui
            .protected_lines(*doc, *date)
            .is_some_and(|p| !p.lines.is_empty()),
    }
}

/// 計画を書き込み、取り消しの記録の対象と `before` / `after` を返す。
fn apply(
    conn: &Connection,
    plan: &Plan,
    tz: &TimeZone,
    now: Timestamp,
) -> Result<(Target, String, String), RevertError> {
    let missing = || RevertError::NotFound(Missing::Item);
    match &plan.effect {
        Effect::None => Err(RevertError::InvalidArgument(RevertArgument::Unsupported)),
        Effect::DeleteItem { item, .. } => {
            if !item_table::soft_delete(conn, item.id, now)? {
                return Err(missing());
            }
            Ok((
                Target::Item(item.id),
                item_json(item).to_string(),
                json!({"deleted_at": format_time(now)}).to_string(),
            ))
        }
        Effect::Fields { item, fields } => {
            let restore: Vec<(String, Value)> = fields
                .iter()
                .map(|f| (f.name.clone(), f.restore.clone()))
                .collect();
            let changes = changes_from(&restore)
                .ok_or(RevertError::InvalidArgument(RevertArgument::Unsupported))?;
            if !item_table::update_fields(conn, item.id, &changes, now)? {
                return Err(missing());
            }
            let map = |pick: fn(&FieldPlan) -> &Value| {
                Value::Object(
                    fields
                        .iter()
                        .map(|f| (f.name.clone(), pick(f).clone()))
                        .collect(),
                )
                .to_string()
            };
            Ok((
                Target::Item(item.id),
                map(|f| &f.current),
                map(|f| &f.restore),
            ))
        }
        Effect::Text { doc, date, .. } => {
            let before = body_of(&plan.log.before)
                .ok_or(RevertError::InvalidArgument(RevertArgument::Unsupported))?;
            let current = text_doc::get(conn, *doc, *date)?;
            day::ensure(conn, *date, tz, now)?;
            text_doc_table::save(conn, *doc, *date, &before, now)?;
            let target = match doc {
                TextDoc::Note => Target::Note(*date),
                TextDoc::Reflection => Target::Reflection(*date),
            };
            Ok((target, json_string(&current.body), json_string(&before)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::test_support::{FakeUi, grant};
    use crate::domain::agent::{Agent, ItemField, NO_UI};
    use crate::domain::item::{self, ItemSave, Position, StatusSave};
    use crate::domain::note_body::{LineEdit, LineOp};
    use crate::storage::Database;
    use crate::storage::table::agent_client::{CLI_ID, Scope};
    use crate::storage::table::test_support::{at, date};

    const T0: &str = "2026-11-03T01:00:00Z";
    const T1: &str = "2026-11-03T02:00:00Z";
    const T2: &str = "2026-11-03T03:00:00Z";
    const T3: &str = "2026-11-03T04:00:00Z";

    fn tokyo() -> TimeZone {
        TimeZone::get("Asia/Tokyo").unwrap()
    }

    fn open() -> (Database, tempfile::TempDir) {
        let (db, dir) = crate::storage::table::test_support::open();
        grant(
            db.connection(),
            CLI_ID,
            &[Scope::Todo, Scope::Blocker, Scope::Append, Scope::NoteEdit],
        );
        (db, dir)
    }

    fn cli() -> Agent<'static> {
        Agent::without_ui(CLI_ID)
    }

    fn add(conn: &Connection, text: &str) -> Item {
        item::add_todo(conn, &cli(), date("2026-11-03"), text, &tokyo(), at(T0)).unwrap()
    }

    /// 項目に付いた記録（古い順）の最後の1件。
    fn last_log(conn: &Connection, id: Ulid) -> ChangeLog {
        change_log::list_by_target(conn, Target::Item(id))
            .unwrap()
            .pop()
            .unwrap()
    }

    fn get(conn: &Connection, id: Ulid) -> Item {
        item_table::get(conn, id).unwrap().unwrap()
    }

    fn do_revert(
        conn: &Connection,
        id: Ulid,
        force: bool,
        now: &str,
    ) -> Result<Reverted, RevertError> {
        revert(
            conn,
            id,
            &RevertRequest {
                force,
                expect: None,
            },
            &NO_UI,
            &tokyo(),
            at(now),
        )
    }

    fn changed_fields(r: Result<Reverted, RevertError>) -> Vec<String> {
        match r {
            Err(RevertError::Conflict(RevertConflict::Changed { fields, .. })) => fields,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_state_change_is_reverted_and_recorded_as_a_human_operation() {
        let (db, _dir) = open();
        let conn = db.connection();
        let t = add(conn, "調査");
        item::complete_todo(conn, &cli(), t.id, at(T1)).unwrap();
        let record = last_log(conn, t.id);
        let r = do_revert(conn, record.id, false, T2).unwrap();
        assert!(r.updated);
        let after = get(conn, t.id);
        assert_eq!((after.status, after.completed_at), (ItemStatus::Open, None));
        // 取り消しの記録：人の操作で、変えた欄だけを現在→戻した値の形で持つ
        let revert_log = last_log(conn, t.id);
        assert_eq!(Some(revert_log.id), r.revert_id);
        assert_eq!(revert_log.actor, Actor::Human);
        assert_eq!(revert_log.agent_client_id, None);
        assert_eq!(revert_log.operation, "agent.revert");
        assert_eq!(revert_log.revert_of_id, Some(record.id));
        assert_eq!(
            serde_json::from_str::<Value>(revert_log.before.as_deref().unwrap()).unwrap(),
            json!({"status": "done", "completed_at": "2026-11-03T02:00:00.000Z"})
        );
        assert_eq!(
            serde_json::from_str::<Value>(revert_log.after.as_deref().unwrap()).unwrap(),
            json!({"status": "open", "completed_at": null})
        );
        assert_eq!(revert_log.seen_at, None);
        // 元の記録は取り消し済みになる
        let record = change_log::get(conn, record.id).unwrap().unwrap();
        assert_eq!(record.reverted_at, Some(at(T2)));
        // 同じ記録を何度取り消しても、結果は同じ（2回目は何も書かない）
        let again = do_revert(conn, record.id, false, T3).unwrap();
        assert!(!again.updated);
        assert_eq!(again.plan.state, PlanState::AlreadyReverted);
        assert_eq!(
            change_log::list_by_target(conn, Target::Item(t.id))
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn only_the_fields_the_operation_changed_are_judged_and_restored() {
        let (db, _dir) = open();
        let conn = db.connection();
        let t = add(conn, "調査");
        item::edit_todo(conn, &cli(), t.id, "調査する", at(T1)).unwrap();
        let edit = last_log(conn, t.id);
        // 人があとから別の欄（状態）を変えても、本文の取り消しは通り、状態は残る
        item::save(
            conn,
            t.id,
            &ItemSave {
                status: Some(StatusSave {
                    before: ItemStatus::Open,
                    closed_at: Some(at(T2)),
                    changed_at: at(T2),
                }),
                ..ItemSave::default()
            },
            at(T2),
        )
        .unwrap();
        do_revert(conn, edit.id, false, T3).unwrap();
        let after = get(conn, t.id);
        assert_eq!(after.text, "調査");
        assert_eq!(after.status, ItemStatus::Done);
    }

    #[test]
    fn a_field_changed_by_a_human_is_a_conflict_that_force_overrides() {
        let (db, _dir) = open();
        let conn = db.connection();
        let t = add(conn, "調査");
        item::edit_todo(conn, &cli(), t.id, "調査する", at(T1)).unwrap();
        let edit = last_log(conn, t.id);
        item::save(
            conn,
            t.id,
            &ItemSave {
                text: Some("人が直した".into()),
                ..ItemSave::default()
            },
            at(T2),
        )
        .unwrap();
        // 差分を見せる計画は、何も書かない
        let plan = preview(conn, edit.id).unwrap();
        assert_eq!(plan.drift, ["text"]);
        assert_eq!(plan.state, PlanState::Apply);
        assert_eq!(get(conn, t.id).text, "人が直した");
        assert_eq!(
            changed_fields(do_revert(conn, edit.id, false, T3)),
            ["text"]
        );
        assert_eq!(get(conn, t.id).text, "人が直した");
        // 確認の後に変わったときは、force でも書かない
        let stale = revert(
            conn,
            edit.id,
            &RevertRequest {
                force: true,
                expect: Some("[\"古い\"]"),
            },
            &NO_UI,
            &tokyo(),
            at(T3),
        );
        assert!(matches!(
            stale,
            Err(RevertError::Conflict(RevertConflict::Changed {
                after_preview: true,
                ..
            }))
        ));
        assert_eq!(get(conn, t.id).text, "人が直した");
        // 見せたいまの値を expect に渡した force は上書きする
        let r = revert(
            conn,
            edit.id,
            &RevertRequest {
                force: true,
                expect: Some(&plan.token),
            },
            &NO_UI,
            &tokyo(),
            at(T3),
        )
        .unwrap();
        assert!(r.updated);
        assert_eq!(get(conn, t.id).text, "調査");
    }

    #[test]
    fn reverting_an_add_deletes_the_item_and_ignores_reordering() {
        let (db, _dir) = open();
        let conn = db.connection();
        let a = add(conn, "A");
        let b = add(conn, "B");
        let add_log = last_log(conn, a.id);
        // 並べ替えは中身の変更ではない
        item::move_item(conn, a.id, Position::After(b.id), at(T1)).unwrap();
        let r = do_revert(conn, add_log.id, false, T2).unwrap();
        assert!(r.updated);
        assert!(get(conn, a.id).deleted_at.is_some());
        assert!(get(conn, b.id).deleted_at.is_none());
        let revert_log = last_log(conn, a.id);
        let after: Value = serde_json::from_str(revert_log.after.as_deref().unwrap()).unwrap();
        assert_eq!(after, json!({"deleted_at": "2026-11-03T03:00:00.000Z"}));
        let before: Value = serde_json::from_str(revert_log.before.as_deref().unwrap()).unwrap();
        assert_eq!(before["text"], "A");
    }

    #[test]
    fn reverting_an_add_after_a_human_edited_it_needs_force() {
        let (db, _dir) = open();
        let conn = db.connection();
        let a = add(conn, "A");
        let add_log = last_log(conn, a.id);
        item::save(
            conn,
            a.id,
            &ItemSave {
                text: Some("A 改".into()),
                ..ItemSave::default()
            },
            at(T1),
        )
        .unwrap();
        assert_eq!(
            changed_fields(do_revert(conn, add_log.id, false, T2)),
            ["text"]
        );
        assert!(get(conn, a.id).deleted_at.is_none());
        assert!(do_revert(conn, add_log.id, true, T2).unwrap().updated);
        assert!(get(conn, a.id).deleted_at.is_some());
    }

    #[test]
    fn an_add_whose_item_was_already_deleted_is_a_success_that_writes_nothing() {
        let (db, _dir) = open();
        let conn = db.connection();
        let a = add(conn, "A");
        let add_log = last_log(conn, a.id);
        item::delete(conn, a.id, at(T1)).unwrap();
        let r = do_revert(conn, add_log.id, false, T2).unwrap();
        assert!(!r.updated);
        assert_eq!(r.plan.state, PlanState::AlreadyRestored);
        // 記録は取り消し済みになり、取り消しの記録は増えない
        assert_eq!(
            change_log::get(conn, add_log.id)
                .unwrap()
                .unwrap()
                .reverted_at,
            Some(at(T2))
        );
        assert_eq!(
            change_log::list_by_target(conn, Target::Item(a.id))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn a_deleted_item_cannot_get_its_fields_back() {
        let (db, _dir) = open();
        let conn = db.connection();
        let a = add(conn, "A");
        item::complete_todo(conn, &cli(), a.id, at(T1)).unwrap();
        let record = last_log(conn, a.id);
        item::delete(conn, a.id, at(T2)).unwrap();
        assert!(matches!(
            do_revert(conn, record.id, true, T3),
            Err(RevertError::NotFound(Missing::Item))
        ));
    }

    #[test]
    fn carried_over_and_dropped_items_are_refused_even_with_force() {
        let (db, _dir) = open();
        let conn = db.connection();
        let a = add(conn, "A");
        item::complete_todo(conn, &cli(), a.id, at(T1)).unwrap();
        let record = last_log(conn, a.id);
        // 持ち越し先を作る
        let next = date("2026-11-04");
        day::ensure(conn, next, &tokyo(), at(T1)).unwrap();
        let copy = Item {
            id: Ulid::generate(),
            date: next,
            carried_from_id: Some(a.id),
            status: ItemStatus::Open,
            completed_at: None,
            ..a.clone()
        };
        item_table::insert(conn, &copy).unwrap();
        for force in [false, true] {
            match do_revert(conn, record.id, force, T2) {
                Err(RevertError::Conflict(RevertConflict::CarriedOver {
                    carried_to_id,
                    carried_to_date,
                })) => {
                    assert_eq!((carried_to_id, carried_to_date), (copy.id, next));
                }
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(get(conn, a.id).status, ItemStatus::Done);

        // 追加の取り消しは削除なので拒まない（持ち越し先は残る）
        let add_log = change_log::list_by_target(conn, Target::Item(a.id))
            .unwrap()
            .remove(0);
        let plan = preview(conn, add_log.id).unwrap();
        assert!(matches!(
            plan.effect,
            Effect::DeleteItem {
                carried_to: Some(c),
                ..
            } if c.id == copy.id
        ));

        // 取りやめた TODO
        let b = add(conn, "B");
        item::edit_todo(conn, &cli(), b.id, "B2", at(T1)).unwrap();
        let record = last_log(conn, b.id);
        let changes = ItemChanges {
            status: Some(ItemStatus::Dropped),
            close_reason: Some(Some(CloseReason::Split)),
            ..ItemChanges::default()
        };
        item_table::update_fields(conn, b.id, &changes, at(T2)).unwrap();
        assert!(matches!(
            do_revert(conn, record.id, true, T3),
            Err(RevertError::InvalidArgument(RevertArgument::Dropped {
                close_reason: CloseReason::Split
            }))
        ));
        assert_eq!(get(conn, b.id).text, "B2");
    }

    #[test]
    fn a_value_already_back_to_the_old_one_is_marked_reverted_without_writing() {
        let (db, _dir) = open();
        let conn = db.connection();
        let t = add(conn, "調査");
        item::complete_todo(conn, &cli(), t.id, at(T1)).unwrap();
        let record = last_log(conn, t.id);
        // 人が先に未完了へ戻した
        item::save(
            conn,
            t.id,
            &ItemSave {
                status: Some(StatusSave {
                    before: ItemStatus::Done,
                    closed_at: None,
                    changed_at: at(T2),
                }),
                ..ItemSave::default()
            },
            at(T2),
        )
        .unwrap();
        let r = do_revert(conn, record.id, false, T3).unwrap();
        assert!(!r.updated);
        assert_eq!(r.plan.state, PlanState::AlreadyRestored);
        assert_eq!(
            change_log::get(conn, record.id)
                .unwrap()
                .unwrap()
                .reverted_at,
            Some(at(T3))
        );
    }

    #[test]
    fn records_that_are_not_agent_changes_or_do_not_exist_are_refused() {
        let (db, _dir) = open();
        let conn = db.connection();
        let t = add(conn, "調査");
        item::complete_todo(conn, &cli(), t.id, at(T1)).unwrap();
        let record = last_log(conn, t.id);
        assert!(matches!(
            do_revert(conn, Ulid::generate(), false, T2),
            Err(RevertError::NotFound(Missing::Change))
        ));
        let r = do_revert(conn, record.id, false, T2).unwrap();
        // 取り消しの記録は取り消せない
        assert!(matches!(
            do_revert(conn, r.revert_id.unwrap(), false, T3),
            Err(RevertError::InvalidArgument(RevertArgument::NotAgentChange))
        ));
    }

    #[test]
    fn a_revert_does_not_count_as_an_agent_state_change_for_the_screen() {
        let (db, _dir) = open();
        let conn = db.connection();
        let t = add(conn, "調査");
        item::complete_todo(conn, &cli(), t.id, at(T0)).unwrap();
        let record = last_log(conn, t.id);
        do_revert(conn, record.id, false, T1).unwrap();
        // 取り消しの後に、画面が完了にした保存が来ても、エージェントとの行き違いと見なさない
        item::save(
            conn,
            t.id,
            &ItemSave {
                status: Some(StatusSave {
                    before: ItemStatus::Open,
                    closed_at: Some(at(T2)),
                    changed_at: at(T1),
                }),
                ..ItemSave::default()
            },
            at(T2),
        )
        .unwrap();
        let after = get(conn, t.id);
        assert_eq!(
            (after.status, after.completed_at),
            (ItemStatus::Done, Some(at(T2)))
        );
        // 記録は消えず、取り消しの記録もそのまま
        assert_eq!(
            change_log::list_by_target(conn, Target::Item(t.id))
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn the_screen_state_blocks_a_revert() {
        let (db, _dir) = open();
        let conn = db.connection();
        let t = add(conn, "調査");
        item::edit_todo(conn, &cli(), t.id, "調査する", at(T1)).unwrap();
        let record = last_log(conn, t.id);
        let ui = FakeUi::editing(t.id, ItemField::Text);
        let r = revert(
            conn,
            record.id,
            &RevertRequest::default(),
            &ui,
            &tokyo(),
            at(T2),
        );
        assert!(matches!(
            r,
            Err(RevertError::Conflict(RevertConflict::UserEditing))
        ));
        let failing = FakeUi {
            save_failing: true,
            ..FakeUi::default()
        };
        let r = revert(
            conn,
            record.id,
            &RevertRequest::default(),
            &failing,
            &tokyo(),
            at(T2),
        );
        assert!(matches!(r, Err(RevertError::StorageUnavailable(_))));
        assert_eq!(get(conn, t.id).text, "調査する");
        // 入力中でなければ通る
        assert!(
            revert(
                conn,
                record.id,
                &RevertRequest::default(),
                &FakeUi::default(),
                &tokyo(),
                at(T2)
            )
            .unwrap()
            .updated
        );
    }

    fn note_body(conn: &Connection, doc: TextDoc, d: &str) -> (String, i64) {
        let s = text_doc::get(conn, doc, date(d)).unwrap();
        (s.body, s.rev)
    }

    #[test]
    fn a_note_is_restored_and_a_later_human_edit_needs_force() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        text_doc::append_note(conn, d, "一行目", None, &cli(), &tokyo(), at(T0)).unwrap();
        text_doc::append_note(conn, d, "二行目", None, &cli(), &tokyo(), at(T1)).unwrap();
        let second = change_log::list_by_target(conn, Target::Note(d))
            .unwrap()
            .pop()
            .unwrap();
        let (before_body, rev) = note_body(conn, TextDoc::Note, "2026-11-03");
        assert!(before_body.contains("二行目"));
        let plan = preview(conn, second.id).unwrap();
        assert_eq!(plan.token, format!("body:{before_body}"));
        match &plan.effect {
            Effect::Text { diff, .. } => {
                assert!(diff.removed.iter().any(|l| l.contains("二行目")));
                assert!(diff.added.iter().all(|l| !l.contains("二行目")));
            }
            other => panic!("{other:?}"),
        }
        // 人が続きを書いた
        text_doc::save(
            conn,
            TextDoc::Note,
            d,
            &format!("{before_body}人の行\n"),
            rev,
            &tokyo(),
            at(T2),
        )
        .unwrap();
        assert_eq!(
            changed_fields(do_revert(conn, second.id, false, T3)),
            ["body"]
        );
        // force は、見せた差分（いまの本文→前の本文）のとおりに本文全体を戻す
        let plan = preview(conn, second.id).unwrap();
        let r = revert(
            conn,
            second.id,
            &RevertRequest {
                force: true,
                expect: Some(&plan.token),
            },
            &NO_UI,
            &tokyo(),
            at(T3),
        )
        .unwrap();
        assert!(r.updated);
        let (body, _) = note_body(conn, TextDoc::Note, "2026-11-03");
        assert!(body.contains("一行目") && !body.contains("二行目") && !body.contains("人の行"));
    }

    #[test]
    fn a_reflection_and_an_edited_note_line_are_restored_when_nobody_changed_them() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        text_doc::append_reflection(conn, d, "良かった", &cli(), &tokyo(), at(T0)).unwrap();
        let log = change_log::list_by_target(conn, Target::Reflection(d))
            .unwrap()
            .pop()
            .unwrap();
        assert!(do_revert(conn, log.id, false, T1).unwrap().updated);
        assert_eq!(note_body(conn, TextDoc::Reflection, "2026-11-03").0, "");

        text_doc::append_note(conn, d, "旧", None, &cli(), &tokyo(), at(T0)).unwrap();
        let edit = LineEdit {
            line: 1,
            expected_text: "旧".into(),
            op: LineOp::Replace("新".into()),
        };
        text_doc::edit_note(conn, d, &edit, &cli(), &tokyo(), at(T1)).unwrap();
        let log = change_log::list_by_target(conn, Target::Note(d))
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(log.operation, "note.edit");
        do_revert(conn, log.id, false, T2).unwrap();
        assert_eq!(note_body(conn, TextDoc::Note, "2026-11-03").0, "旧");
    }

    #[test]
    fn a_protected_note_line_blocks_a_note_revert() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        text_doc::append_note(conn, d, "旧", None, &cli(), &tokyo(), at(T0)).unwrap();
        let log = change_log::list_by_target(conn, Target::Note(d))
            .unwrap()
            .pop()
            .unwrap();
        let ui = FakeUi::default();
        ui.protect(TextDoc::Note, d, 1, &[1]);
        let r = revert(
            conn,
            log.id,
            &RevertRequest::default(),
            &ui,
            &tokyo(),
            at(T1),
        );
        assert!(matches!(
            r,
            Err(RevertError::Conflict(RevertConflict::UserEditing))
        ));
    }

    #[test]
    fn a_field_already_back_to_the_old_value_is_not_overwritten_and_empty_records_are_refused() {
        let (db, _dir) = open();
        let conn = db.connection();
        let t = add(conn, "調査");
        item::complete_todo(conn, &cli(), t.id, at(T1)).unwrap();
        let record = last_log(conn, t.id);
        // 人が状態だけを未完了に戻し、完了時刻は残っている：状態は上書きするものが無い
        let changes = ItemChanges {
            status: Some(ItemStatus::Open),
            ..ItemChanges::default()
        };
        item_table::update_fields(conn, t.id, &changes, at(T2)).unwrap();
        let plan = preview(conn, record.id).unwrap();
        assert_eq!(plan.state, PlanState::Apply);
        assert_eq!(plan.drift, Vec::<String>::new());
        assert!(do_revert(conn, record.id, false, T3).unwrap().updated);
        assert_eq!(get(conn, t.id).completed_at, None);

        // `after` が空の記録は、何も戻さずに取り消し済みにしない
        let empty = ChangeLog {
            id: Ulid::generate(),
            actor: Actor::Agent,
            agent_client_id: Some(CLI_ID),
            operation: "todo.edit".into(),
            target: Target::Item(t.id),
            before: Some("{}".into()),
            after: Some("{}".into()),
            created_at: at(T3),
            reverted_at: None,
            revert_of_id: None,
            seen_at: None,
        };
        change_log::insert(conn, &empty).unwrap();
        assert!(matches!(
            do_revert(conn, empty.id, false, T3),
            Err(RevertError::InvalidArgument(RevertArgument::Unsupported))
        ));
    }

    #[test]
    fn the_text_diff_drops_the_common_head_and_tail() {
        let d = text_diff("a\nb\nc\nd", "a\nx\ny\nd");
        assert_eq!(d.removed, ["b", "c"]);
        assert_eq!(d.added, ["x", "y"]);
        assert!(!d.truncated);
        let d = text_diff("a\n", "a\nb\n");
        assert_eq!((d.removed.len(), d.added), (0, vec!["b".to_owned()]));
        let long = "x\n".repeat(MAX_DIFF_LINES + 5);
        assert!(text_diff("", &long).truncated);
    }

    #[test]
    fn the_history_lists_newest_first_with_filters() {
        let (db, _dir) = open();
        let conn = db.connection();
        let t = add(conn, "調査");
        item::complete_todo(conn, &cli(), t.id, at(T1)).unwrap();
        let record = last_log(conn, t.id);
        do_revert(conn, record.id, false, T2).unwrap();
        let all = list_changes(
            conn,
            &ChangeFilter {
                limit: DEFAULT_LIMIT,
                ..ChangeFilter::default()
            },
        )
        .unwrap();
        let ops: Vec<&str> = all.iter().map(|e| e.log.operation.as_str()).collect();
        assert_eq!(ops, ["agent.revert", "todo.complete", "todo.add"]);
        assert_eq!(all[0].client_name, None);
        assert_eq!(all[1].client_name.as_deref(), Some("cli"));
        assert!(matches!(&all[2].summary, Summary::ItemAdded { text, .. } if text == "調査"));
        assert!(matches!(&all[1].summary, Summary::Fields(f) if f.len() == 2));
        assert_eq!(all[1].item.as_ref().unwrap().text, "調査");
        // 未確認のエージェントの変更だけ
        let unseen = list_changes(
            conn,
            &ChangeFilter {
                unseen_only: true,
                limit: 10,
                ..ChangeFilter::default()
            },
        )
        .unwrap();
        assert_eq!(unseen.len(), 2);
        // 続き（before）と件数
        let page = list_changes(
            conn,
            &ChangeFilter {
                before: Some(all[0].log.id),
                limit: 1,
                ..ChangeFilter::default()
            },
        )
        .unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].log.id, all[1].log.id);
        // クライアントで絞る（人の取り消しの記録は入らない）
        let by_cli = list_changes(
            conn,
            &ChangeFilter {
                client: Some("cli".into()),
                limit: 10,
                ..ChangeFilter::default()
            },
        )
        .unwrap();
        assert_eq!(by_cli.len(), 2);
        assert!(matches!(
            list_changes(
                conn,
                &ChangeFilter {
                    client: Some("nobody".into()),
                    limit: 10,
                    ..ChangeFilter::default()
                }
            ),
            Err(HistoryError::ClientNotFound)
        ));
        assert!(matches!(
            list_changes(conn, &ChangeFilter::default()),
            Err(HistoryError::ZeroLimit)
        ));
    }
}
