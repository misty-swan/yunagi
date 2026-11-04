//! ローカル API の依頼を `AppCore` の入口へ渡し、結果を応答の JSON にする（MCP・CLI 仕様書 3章・4.2・
//! 4.3、アーキテクチャ設計書 4.6）。
//!
//! 操作の名前と引数の名前は MCP のツール名・引数にそろえる（`get_board`・`add_todo`・`expected_text` など）。
//! 権限・入力中・持ち越しなどの判定はコアの関数が行う（どの経路でも同じ。4章 5）。ここは、引数を
//! 読み、コアの結果とエラーを 05 の JSON にするだけ。
//!
//! 読み取りは [`AppCore::api_read`]、書き込みは [`AppCore::api_write`]（`agent_write` のローカル API
//! 用の版。画面と同じ `changed` を出す）を通る。`date` は受け取ったまま（`today` など）で、
//! アプリが自分の設定とタイムゾーンで日付を求める。

use serde_json::{Map, Value, json};
use yunagi_api::wire::{self, Request};
use yunagi_core::domain::agent::{self, PermissionDenied, ReadError};
use yunagi_core::domain::board::{self, BoardItem};
use yunagi_core::domain::day;
use yunagi_core::domain::item::{
    self, CloseReason, Conflict, InvalidArgument, Item, ItemError, ItemKind, ItemOperation,
};
use yunagi_core::domain::logical_date::{DateSpec, LogicalDate};
use yunagi_core::domain::note_body::{self, LineEdit, LineOp};
use yunagi_core::domain::text_doc::{
    self, InvalidHeading, NoteRead, NoteReadError, TextDocError, Written,
};
use yunagi_core::error::ErrorChain;
use yunagi_core::storage::table::agent_client::Scope;
use yunagi_core::storage::{StorageError, Ulid};

use crate::app_core::{AppCore, Busy, now, system_tz};

/// 依頼を処理し、応答の 1 行（改行付き）を返す。
pub fn handle(core: &AppCore, client: Option<&str>, request: &Request) -> String {
    match run(core, client, request) {
        Ok(result) => wire::result_line(&request.id, &result),
        Err(Failure::NoMethod) => wire::error_line(
            &request.id,
            wire::METHOD_NOT_FOUND,
            "Method not found",
            None,
        ),
        Err(Failure::Params(message)) => wire::error_line(
            &request.id,
            wire::INVALID_PARAMS,
            &format!("Invalid params: {message}"),
            None,
        ),
        Err(Failure::App(error)) => wire::app_error_line(&request.id, &error),
    }
}

/// 失敗。
enum Failure {
    /// 知らない `method`。
    NoMethod,
    /// `params` の形が誤っている（足りない・型が違う・知らない名前）。
    Params(String),
    /// アプリのエラー（05 4.3 の JSON）。
    App(Value),
}

impl From<Busy> for Failure {
    fn from(_: Busy) -> Self {
        Self::App(busy_error())
    }
}

/// 5 秒の待ちで始められなかった（保存先の変更・復元の最中、またはデータベースを開いていない）。
/// 何も反映していない。
pub fn busy_error() -> Value {
    json!({
        "code": "busy",
        "message": "Yunagi is using its data and did not respond within 5 seconds. Nothing was changed. Try the same request again. If it keeps failing, ask the user to restart Yunagi.",
    })
}

fn run(core: &AppCore, client: Option<&str>, request: &Request) -> Result<Value, Failure> {
    let params = request
        .params
        .as_object()
        .ok_or_else(|| Failure::Params("params must be an object".into()))?;
    let call = Call::parse(&request.method, params)?;
    // 呼び出し元のクライアントを求める（初めての識別子は `read` だけで登録し、最後の接続の時刻を更新する）
    let client_id = core
        .api_read(|conn| agent::client_for(conn, client, now()))?
        .map_err(storage_failure)?
        .ok_or_else(|| Failure::Params("client is not a valid identifier".into()))?;
    call.execute(core, client_id)
}

/// 日付の指定。
#[derive(Debug, Clone, Copy)]
struct Date(DateSpec);

/// 読み終えた依頼。引数はデータベースに触れる前にすべて読む。
enum Call {
    GetBoard(Date),
    OpenBoard(Date),
    GetNote(Date, Option<String>),
    AddTodo(Date, String),
    AddBlocker(Date, String),
    EditTodo(Ulid, String),
    EditBlocker(Ulid, String),
    CompleteTodo(Ulid),
    ReopenTodo(Ulid),
    ResolveBlocker(Ulid, Option<String>),
    AppendNote(Date, String, Option<String>),
    AppendReflection(Date, String),
    EditNote(Date, LineEdit),
}

impl Call {
    fn parse(method: &str, params: &Map<String, Value>) -> Result<Self, Failure> {
        let p = Params::new(params);
        let call = match method {
            "get_board" => {
                p.only(&["date"])?;
                Self::GetBoard(p.date()?)
            }
            "open_board" => {
                p.only(&["date"])?;
                Self::OpenBoard(p.date()?)
            }
            "get_note" => {
                p.only(&["date", "heading"])?;
                Self::GetNote(p.date()?, p.optional_string("heading")?)
            }
            "add_todo" => {
                p.only(&["text", "date"])?;
                Self::AddTodo(p.date()?, p.string("text")?)
            }
            "add_blocker" => {
                p.only(&["text", "date"])?;
                Self::AddBlocker(p.date()?, p.string("text")?)
            }
            "edit_todo" => {
                p.only(&["id", "text"])?;
                Self::EditTodo(p.id()?, p.string("text")?)
            }
            "edit_blocker" => {
                p.only(&["id", "text"])?;
                Self::EditBlocker(p.id()?, p.string("text")?)
            }
            "complete_todo" => {
                p.only(&["id"])?;
                Self::CompleteTodo(p.id()?)
            }
            "reopen_todo" => {
                p.only(&["id"])?;
                Self::ReopenTodo(p.id()?)
            }
            "resolve_blocker" => {
                p.only(&["id", "note"])?;
                Self::ResolveBlocker(p.id()?, p.optional_string("note")?)
            }
            "append_note" => {
                p.only(&["text", "date", "heading"])?;
                Self::AppendNote(p.date()?, p.string("text")?, p.optional_string("heading")?)
            }
            "append_reflection" => {
                p.only(&["text", "date"])?;
                Self::AppendReflection(p.date()?, p.string("text")?)
            }
            "edit_note" => {
                p.only(&[
                    "date",
                    "line",
                    "expected_text",
                    "replace",
                    "delete",
                    "move_to",
                ])?;
                Self::EditNote(p.date()?, p.line_edit()?)
            }
            _ => return Err(Failure::NoMethod),
        };
        Ok(call)
    }

    fn execute(self, core: &AppCore, client_id: Ulid) -> Result<Value, Failure> {
        let tz = system_tz();
        let at = now();
        match self {
            Self::GetBoard(date) => core.api_read(|conn| {
                let date = resolve(conn, date)?;
                let b = board::get_board(conn, client_id, date).map_err(read_failure)?;
                Ok(json!({
                    "date": b.date.to_string(),
                    "todos": b.todos.iter().map(|i| item_json(i, ItemKind::Todo)).collect::<Vec<_>>(),
                    "blockers": b.blockers.iter().map(|i| item_json(i, ItemKind::Blocker)).collect::<Vec<_>>(),
                    "reflection": b.reflection,
                    "note": {
                        "headings": b.note.headings.iter().map(|h| h.text.clone()).collect::<Vec<_>>(),
                        "length": b.note.length,
                    },
                }))
            })?,
            Self::OpenBoard(date) => core.api_read(|conn| {
                let date = resolve(conn, date)?;
                let b = board::open_board(conn, client_id, date).map_err(read_failure)?;
                Ok(json!({
                    "date": b.date.to_string(),
                    "todos": b.todos.iter().map(|i| item_json(i, ItemKind::Todo)).collect::<Vec<_>>(),
                    "blockers": b.blockers.iter().map(|i| item_json(i, ItemKind::Blocker)).collect::<Vec<_>>(),
                }))
            })?,
            Self::GetNote(date, heading) => core.api_read(|conn| {
                let date = resolve(conn, date)?;
                let read = text_doc::get_note(conn, client_id, date, heading.as_deref())
                    .map_err(note_read_failure)?;
                Ok(match read {
                    NoteRead::Whole(s) => {
                        json!({"date": date.to_string(), "rev": s.rev, "body": s.body})
                    }
                    NoteRead::Sections { rev, sections } => json!({
                        "date": date.to_string(),
                        "rev": rev,
                        "sections": sections.iter().map(|s| json!({
                            "heading": s.heading,
                            "start_line": s.start_line,
                            "text": s.text,
                        })).collect::<Vec<_>>(),
                    }),
                })
            })?,
            Self::AddTodo(date, text) => write(core, client_id, |conn, agent| {
                let date = resolve(conn, date)?;
                let i = item::add_todo(conn, agent, date, &text, &tz, at).map_err(item_failure)?;
                Ok(added_json(&i))
            })?,
            Self::AddBlocker(date, text) => write(core, client_id, |conn, agent| {
                let date = resolve(conn, date)?;
                let i =
                    item::add_blocker(conn, agent, date, &text, &tz, at).map_err(item_failure)?;
                Ok(added_json(&i))
            })?,
            Self::EditTodo(id, text) => write(core, client_id, |conn, agent| {
                let o = item::edit_todo(conn, agent, id, &text, at).map_err(item_failure)?;
                Ok(json!({"updated": o.updated, "id": o.item.id.to_string(), "text": o.item.text}))
            })?,
            Self::EditBlocker(id, text) => write(core, client_id, |conn, agent| {
                let o = item::edit_blocker(conn, agent, id, &text, at).map_err(item_failure)?;
                Ok(json!({"updated": o.updated, "id": o.item.id.to_string(), "text": o.item.text}))
            })?,
            Self::CompleteTodo(id) => write(core, client_id, |conn, agent| {
                let o = item::complete_todo(conn, agent, id, at).map_err(item_failure)?;
                Ok(status_json(o.updated, &o.item))
            })?,
            Self::ReopenTodo(id) => write(core, client_id, |conn, agent| {
                let o = item::reopen_todo(conn, agent, id, at).map_err(item_failure)?;
                Ok(status_json(o.updated, &o.item))
            })?,
            Self::ResolveBlocker(id, note) => write(core, client_id, |conn, agent| {
                let r = item::resolve_blocker(conn, agent, id, note.as_deref(), at)
                    .map_err(item_failure)?;
                Ok(json!({
                    "updated": r.updated(),
                    "resolved": r.resolved,
                    "note_written": r.note_written,
                    "id": r.item.id.to_string(),
                    "status": r.item.status.as_str(),
                    "resolution_note": r.item.resolution_note,
                }))
            })?,
            Self::AppendNote(date, text, heading) => write(core, client_id, |conn, agent| {
                let date = resolve(conn, date)?;
                let w = text_doc::append_note(conn, date, &text, heading.as_deref(), agent, &tz, at)
                    .map_err(text_doc_failure)?;
                Ok(written_json(&w))
            })?,
            Self::AppendReflection(date, text) => write(core, client_id, |conn, agent| {
                let date = resolve(conn, date)?;
                let w = text_doc::append_reflection(conn, date, &text, agent, &tz, at)
                    .map_err(text_doc_failure)?;
                Ok(written_json(&w))
            })?,
            Self::EditNote(date, edit) => write(core, client_id, |conn, agent| {
                let date = resolve(conn, date)?;
                let w = text_doc::edit_note(conn, date, &edit, agent, &tz, at)
                    .map_err(text_doc_failure)?;
                Ok(written_json(&w))
            })?,
        }
    }
}

/// 書き込みの依頼を `AppCore::api_write` に渡す。成功の JSON は、変えたかを `updated` で持つ。
fn write(
    core: &AppCore,
    client_id: Ulid,
    f: impl FnOnce(&rusqlite::Connection, &agent::Agent<'_>) -> Result<Value, Failure>,
) -> Result<Result<Value, Failure>, Busy> {
    // `api_write` は、変えたかを `Changes` で求める。JSON の `updated` を見る
    let result = core.api_write(client_id, |conn, agent| f(conn, agent).map(Changed))?;
    Ok(result.map(|Changed(v)| v))
}

/// `updated` が `false` でない成功の応答（`changed` を出す対象）。
struct Changed(Value);

impl crate::app_core::Changes for Changed {
    fn changed(&self) -> bool {
        // 追加（`updated` が無い）は常に変えている。それ以外は `updated` に従う
        self.0
            .get("updated")
            .and_then(Value::as_bool)
            .unwrap_or(true)
    }
}

// ---- 引数 ----

struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    fn new(map: &'a Map<String, Value>) -> Self {
        Self(map)
    }

    /// 知らない名前があれば誤り（綴りの間違いを黙って無視しない）。
    fn only(&self, allowed: &[&str]) -> Result<(), Failure> {
        match self.0.keys().find(|k| !allowed.contains(&k.as_str())) {
            Some(key) => Err(Failure::Params(format!("unknown parameter \"{key}\""))),
            None => Ok(()),
        }
    }

    fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key).filter(|v| !v.is_null())
    }

    fn string(&self, key: &str) -> Result<String, Failure> {
        match self.get(key) {
            Some(Value::String(s)) => Ok(s.clone()),
            Some(_) => Err(Failure::Params(format!("{key} must be a string"))),
            None => Err(Failure::Params(format!("{key} is required"))),
        }
    }

    fn optional_string(&self, key: &str) -> Result<Option<String>, Failure> {
        match self.get(key) {
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(Failure::Params(format!("{key} must be a string"))),
            None => Ok(None),
        }
    }

    fn date(&self) -> Result<Date, Failure> {
        match self.optional_string("date")? {
            None => Ok(Date(DateSpec::Today)),
            Some(s) => s.parse().map(Date).map_err(|_| {
                Failure::App(invalid_argument(
                    "date must be YYYY-MM-DD, today or yesterday.",
                ))
            }),
        }
    }

    fn id(&self) -> Result<Ulid, Failure> {
        self.string("id")?.parse().map_err(|_| {
            Failure::App(invalid_argument(
                "id is not a valid item ID. Use an id from get_board.",
            ))
        })
    }

    fn number(&self, key: &str) -> Result<Option<usize>, Failure> {
        match self.get(key) {
            Some(v) => v
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .map(Some)
                .ok_or_else(|| Failure::Params(format!("{key} must be a non-negative integer"))),
            None => Ok(None),
        }
    }

    /// `edit_note` の `line`・`expected_text` と、`replace`・`delete`・`move_to` のうちちょうど 1 つ。
    fn line_edit(&self) -> Result<LineEdit, Failure> {
        let line = self
            .number("line")?
            .ok_or_else(|| Failure::Params("line is required".into()))?;
        let expected_text = self.string("expected_text")?;
        let replace = self.optional_string("replace")?;
        let delete = match self.get("delete") {
            None => false,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err(Failure::Params("delete must be true or false".into())),
        };
        let move_to = self.number("move_to")?;
        let op = match (replace, delete, move_to) {
            (Some(text), false, None) => LineOp::Replace(text),
            (None, true, None) => LineOp::Delete,
            (None, false, Some(to)) => LineOp::MoveTo(to),
            _ => {
                return Err(Failure::Params(
                    "give exactly one of replace, delete (true) and move_to".into(),
                ));
            }
        };
        Ok(LineEdit {
            line,
            expected_text,
            op,
        })
    }
}

/// 日付の指定を、端末の時計・タイムゾーン・切り替わり時刻で論理日付にする。
fn resolve(conn: &rusqlite::Connection, date: Date) -> Result<LogicalDate, Failure> {
    let boundary = day::day_boundary(conn).map_err(storage_failure)?;
    date.0
        .resolve_at(now(), &system_tz(), boundary)
        .ok_or_else(|| Failure::App(invalid_argument("date is out of range.")))
}

// ---- 応答の JSON（05 4.2） ----

fn item_json(b: &BoardItem, kind: ItemKind) -> Value {
    let i = &b.item;
    let mut o = Map::new();
    o.insert("id".into(), json!(i.id.to_string()));
    o.insert("text".into(), json!(i.text));
    o.insert("status".into(), json!(i.status.as_str()));
    o.insert("carry_count".into(), json!(i.carry_count));
    o.insert("origin".into(), json!(i.origin.as_str()));
    o.insert(
        "external_ref".into(),
        json!(i.external_ref_id.map(|id| id.to_string())),
    );
    match kind {
        ItemKind::Todo => {
            o.insert(
                "close_reason".into(),
                json!(i.close_reason.map(CloseReason::as_str)),
            );
        }
        ItemKind::Blocker => {
            o.insert("resolution_note".into(), json!(i.resolution_note));
        }
    }
    o.insert(
        "carried_to_id".into(),
        json!(b.carried_to.map(|c| c.id.to_string())),
    );
    o.insert(
        "carried_to_date".into(),
        json!(b.carried_to.map(|c| c.date.to_string())),
    );
    Value::Object(o)
}

fn added_json(i: &Item) -> Value {
    json!({"id": i.id.to_string(), "date": i.date.to_string(), "text": i.text})
}

fn status_json(updated: bool, i: &Item) -> Value {
    json!({"updated": updated, "id": i.id.to_string(), "status": i.status.as_str()})
}

fn written_json(w: &Written) -> Value {
    json!({"updated": w.updated, "rev": w.rev})
}

// ---- エラー（05 4.3） ----

fn error(code: &str, message: &str) -> Value {
    json!({"code": code, "message": message})
}

fn invalid_argument(message: &str) -> Value {
    error("invalid_argument", message)
}

fn conflict(reason: &str, message: &str) -> Value {
    json!({"code": "conflict", "reason": reason, "message": message})
}

fn permission_denied(p: PermissionDenied) -> Value {
    let what = match p.required_scope {
        Scope::Read => "read Yunagi data",
        Scope::Todo => "modify TODOs",
        Scope::Blocker => "modify blockers",
        Scope::Append => "append to notes and reflections",
        Scope::NoteEdit => "edit existing note lines",
    };
    json!({
        "code": "permission_denied",
        "required_scope": p.required_scope.as_str(),
        "message": format!(
            "This agent is not allowed to {what}. Enable it in Yunagi > Settings > Agents."
        ),
    })
}

fn storage_unavailable_retrying() -> Value {
    error(
        "storage_unavailable",
        "Yunagi is retrying a failed save, so writes are not accepted. Nothing was changed. Try again later.",
    )
}

/// 読み書きの失敗。内部の文は応答に出さず、標準エラーに残す（05 4.3）。
fn storage_failure(e: StorageError) -> Failure {
    eprintln!("yunagi: a local API request failed: {}", ErrorChain(&e));
    Failure::App(error(
        "storage_unavailable",
        "Yunagi could not read or write its data. Nothing was changed. Ask the user to check the app.",
    ))
}

fn read_failure(e: ReadError) -> Failure {
    match e {
        ReadError::PermissionDenied(p) => Failure::App(permission_denied(p)),
        ReadError::Storage(e) => storage_failure(e),
    }
}

fn note_read_failure(e: NoteReadError) -> Failure {
    match e {
        NoteReadError::Read(e) => read_failure(e),
        NoteReadError::InvalidArgument(InvalidHeading::Empty) => {
            Failure::App(invalid_argument("heading is empty."))
        }
        NoteReadError::InvalidArgument(InvalidHeading::Multiline) => {
            Failure::App(invalid_argument("heading must be a single line."))
        }
        NoteReadError::NotFound => Failure::App(error(
            "not_found",
            "The note has no section with this heading. Pass a heading exactly as listed in get_board headings.",
        )),
    }
}

fn text_doc_failure(e: TextDocError) -> Failure {
    Failure::App(match e {
        TextDocError::PermissionDenied(p) => permission_denied(p),
        TextDocError::InvalidArgument(a) => invalid_argument(&note_body_message(&a)),
        TextDocError::StorageUnavailable(_) => storage_unavailable_retrying(),
        TextDocError::Changed(c) => conflict("changed", &c.to_string()),
        TextDocError::UserEditing => conflict(
            "user_editing",
            "The user is editing this line. Try again later.",
        ),
        TextDocError::Storage(e) => return storage_failure(e),
        // 画面の保存だけで起きる（エージェントの書き込みには無い）
        TextDocError::Stale { .. } => {
            error("storage_unavailable", "The note changed. Read it again.")
        }
    })
}

fn note_body_message(a: &note_body::InvalidArgument) -> String {
    a.to_string()
}

fn item_failure(e: ItemError) -> Failure {
    Failure::App(match e {
        ItemError::PermissionDenied(p) => permission_denied(p),
        ItemError::NotFound => error(
            "not_found",
            "No item with this ID exists. It may have been deleted. Read the board again.",
        ),
        ItemError::InvalidArgument(a) => item_invalid_argument(&a),
        ItemError::StorageUnavailable(_) => storage_unavailable_retrying(),
        ItemError::Conflict(Conflict::CarriedOver {
            carried_to_id,
            carried_to_date,
        }) => json!({
            "code": "conflict",
            "reason": "carried_over",
            "carried_to_id": carried_to_id.to_string(),
            "carried_to_date": carried_to_date.to_string(),
            "message": format!(
                "This item was carried over to {carried_to_date}. It was not changed. Send the request to carried_to_id instead."
            ),
        }),
        ItemError::Conflict(Conflict::UserEditing) => conflict(
            "user_editing",
            "The user is editing this item. Try again later.",
        ),
        ItemError::Conflict(Conflict::NoteExists { resolution_note }) => json!({
            "code": "conflict",
            "reason": "note_exists",
            "resolution_note": resolution_note,
            "message": "This blocker already has a different resolution note. It was not changed. To resolve it, send the request without a note.",
        }),
        ItemError::Storage(e) => return storage_failure(e),
        // 画面の削除の取り消しだけで起きる（エージェントの操作には無い）
        ItemError::RestoreRefused(_) => error("invalid_argument", "This request is not supported."),
    })
}

fn item_invalid_argument(a: &InvalidArgument) -> Value {
    match a {
        InvalidArgument::WrongKind {
            item_kind,
            use_operation,
        } => {
            let (is, is_not) = match item_kind {
                ItemKind::Blocker => ("a blocker", "a TODO"),
                ItemKind::Todo => ("a TODO", "a blocker"),
            };
            let advice = match use_operation {
                Some(op) => format!("Use {}.", op.name()),
                None => "Agents cannot reopen blockers; ask the user.".into(),
            };
            json!({
                "code": "invalid_argument",
                "item_kind": item_kind.as_str(),
                "use_operation": use_operation.map(ItemOperation::name),
                "message": format!("This ID is {is}, not {is_not}. It was not changed. {advice}"),
            })
        }
        InvalidArgument::Dropped { close_reason } => {
            let what = match close_reason {
                CloseReason::Dropped => "dropped by the user",
                CloseReason::Split => "split into other TODOs by the user",
                CloseReason::ReturnedExternal => "returned to the external tool by the user",
            };
            json!({
                "code": "invalid_argument",
                "close_reason": close_reason.as_str(),
                "message": format!("This TODO was {what}. It was not changed. Do not retry."),
            })
        }
        InvalidArgument::EmptyText => invalid_argument("The text is empty."),
        InvalidArgument::MultilineText => invalid_argument("The text must be a single line."),
        InvalidArgument::EmptyNote => invalid_argument("The note is empty."),
        InvalidArgument::MultilineNote => invalid_argument("The note must be a single line."),
        InvalidArgument::InvalidPosition | InvalidArgument::NotLinkedExternal => {
            invalid_argument("This request is not supported.")
        }
    }
}
