//! ローカル API の操作：引数を読み、コアの入口を呼び、結果とエラーを 05 の JSON にする
//! （MCP・CLI 仕様書 3章・4.2・4.3・4.4、アーキテクチャ設計書 4.6）。
//!
//! アプリ（ローカル API の待ち受け。`src-tauri/src/api/dispatch.rs`）と、直接モードの CLI・MCP サーバー
//! （`crates/cli`）が同じものを使う。どちらの経路でも、応答の JSON とエラーが食い違わないようにするため
//! （アーキテクチャ設計書 4章 5）。ここは、データベースの接続（[`Connection`]）と呼び出し元を受け取って
//! 結果を返すだけで、ロックの取り方・待ち方・画面への通知は呼び出し側が持つ。
//!
//! 操作の名前と引数の名前は MCP のツール名・引数にそろえる（`get_board`・`add_todo`・`expected_text` など）。
//! 権限・入力中・持ち越しなどの判定はコアの関数が行う。`date` は受け取ったまま（`today` など）で、
//! 呼び出し側の設定とタイムゾーンで論理日付を求める。

use std::cell::Cell;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use rusqlite::Connection;
use serde_json::{Map, Value, json};
use yunagi_core::domain::agent::{self, Agent, ClientLookup, PermissionDenied, ReadError};
use yunagi_core::domain::agent_admin::{self, ScopeArgument, ScopeError};
use yunagi_core::domain::board::{self, BoardItem};
use yunagi_core::domain::day;
use yunagi_core::domain::history::{
    self, ChangeFilter, HistoryError, Missing, PlanState, RevertArgument, RevertConflict,
    RevertError, RevertRequest,
};
use yunagi_core::domain::item::{
    self, CloseReason, Conflict, InvalidArgument, Item, ItemError, ItemKind, ItemOperation,
};
use yunagi_core::domain::logical_date::{DateSpec, LogicalDate};
use yunagi_core::domain::note_body::{self, LineEdit, LineOp};
use yunagi_core::domain::search;
use yunagi_core::domain::text_doc::{
    self, InvalidHeading, NoteRead, NoteReadError, TextDoc, TextDocError, Written,
};
use yunagi_core::error::ErrorChain;
use yunagi_core::storage::search::{SearchKind, SearchQuery};
use yunagi_core::storage::table::agent_client::Scope;
use yunagi_core::storage::table::change_log::Target;
use yunagi_core::storage::table::format_time;
use yunagi_core::storage::{StorageError, Ulid};

/// 失敗。
#[derive(Debug)]
pub enum Failure {
    /// 知らない `method`。
    NoMethod,
    /// `params` の形が誤っている（足りない・型が違う・知らない名前）。
    Params(String),
    /// アプリのエラー（05 4.3 の JSON）。
    App(Value),
}

/// 待っても始められなかった（保存先の変更・復元の最中、またはデータベースを開いていない）。
/// 何も反映していない。待つ時間は設定できる（アプリの `ApiOptions::db_wait`。既定 5 秒）ので、文には書かない。
pub fn busy_error() -> Value {
    json!({
        "code": "busy",
        "message": "Yunagi is using its data and did not respond in time. Nothing was changed. Try the same request again. If it keeps failing, ask the user to restart Yunagi.",
    })
}

/// 呼び出し元のクライアントを求める（初めての識別子は `read` だけで登録し、`registered` を立てる）。
/// 最後に接続した時刻の更新は、できたらやる（ディスクがいっぱいなどで書けなくても、読み取りは
/// 断らない）。
pub fn resolve_client(
    conn: &Connection,
    client: Option<&str>,
    registered: &Cell<bool>,
) -> Result<Ulid, Failure> {
    let at = now();
    let id = match agent::client_for(conn, client, at).map_err(storage_failure)? {
        ClientLookup::Known(id) => id,
        ClientLookup::Registered(id) => {
            registered.set(true);
            id
        }
        ClientLookup::InvalidName => {
            return Err(Failure::Params("client is not a valid identifier".into()));
        }
        ClientLookup::TooMany => {
            return Err(Failure::App(invalid_argument(
                "Too many clients are registered. Ask the user to remove unused ones in Yunagi > Settings > Agents.",
            )));
        }
    };
    if let Err(e) = agent::touch_client(conn, id, at) {
        eprintln!(
            "yunagi: could not record the last connection of a client: {}",
            ErrorChain(&e)
        );
    }
    Ok(id)
}

/// 今の時刻（ミリ秒まで。コアが記録する精度）。
pub fn now() -> Timestamp {
    yunagi_core::storage::table::now()
}

/// 端末のタイムゾーン。求められなければ UTC。
///
/// 呼ぶたびに求めるが、jiff が5分間は覚えているので、毎回ファイルを読むことはない。覚え続けると、
/// 起動中に端末のタイムゾーンを変えたとき（移動など）に追いつけないので、覚えない。
pub fn system_tz() -> TimeZone {
    TimeZone::try_system().unwrap_or(TimeZone::UTC)
}

/// 日付の指定。
#[derive(Debug, Clone, Copy)]
pub struct Date(DateSpec);

/// 読み終えた依頼。引数はデータベースに触れる前にすべて読む。
#[derive(Debug)]
pub enum Call {
    Read(ReadCall),
    Write(WriteCall),
    /// 利用者が行う管理の操作（権限・変更履歴・取り消し。A-08）。MCP には出さない（05 3章）。
    Admin(AdminCall),
}

impl Call {
    /// データを何も変えない依頼か。変えなければ、依頼を送った後に接続が切れても 1 回だけやり直せる
    /// （05 4.3 の `result_unknown`）。
    pub fn is_read_only(&self) -> bool {
        match self {
            Self::Read(_) => true,
            Self::Write(_) => false,
            Self::Admin(a) => a.is_read_only(),
        }
    }
}

/// 管理の操作（A-08。05 4.4）。呼び出し元のスコープは確かめない（閲覧は不要。変更と取り消しの確認は、
/// 呼び出し側の CLI の端末か、アプリの設定画面が行う。確認は防御の境界ではない。05 2章）。
#[derive(Debug)]
pub enum AdminCall {
    /// クライアントの一覧。
    ListAgents,
    /// クライアントの権限を変える。
    SetScopes {
        client: String,
        allow: Vec<Scope>,
        deny: Vec<Scope>,
    },
    /// 変更履歴の一覧。
    ListChanges(ChangesArgs),
    /// 取り消しで何が起きるか（何も書かない）。
    PreviewRevert(Ulid),
    /// 取り消す。
    Revert {
        id: Ulid,
        force: bool,
        expect: Option<String>,
    },
}

/// `list_changes` の引数。
#[derive(Debug)]
pub struct ChangesArgs {
    pub client: Option<String>,
    pub unseen_only: bool,
    pub before: Option<Ulid>,
    pub limit: usize,
}

/// 読み取りの依頼。
#[derive(Debug)]
pub enum ReadCall {
    GetBoard(Date),
    OpenBoard(Date),
    GetNote(Date, Option<String>),
    Search(SearchArgs),
}

/// `search` の引数。
#[derive(Debug)]
pub struct SearchArgs {
    pub query: String,
    pub from: Option<Date>,
    pub to: Option<Date>,
    pub kinds: Vec<SearchKind>,
    pub limit: usize,
}

/// 書き込みの依頼。
#[derive(Debug)]
pub enum WriteCall {
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

/// 書き込みの成功の応答の JSON と、データを実際に変えたか（コアの結果から決める。アプリが `changed` を
/// 出す対象かの判断に使う）。
#[derive(Debug)]
pub struct Reply {
    pub value: Value,
    pub changed: bool,
}

impl Reply {
    fn new(value: Value, changed: bool) -> Self {
        Self { value, changed }
    }
}

impl Call {
    /// 操作の名前と引数を読む。
    pub fn parse(method: &str, params: &Map<String, Value>) -> Result<Self, Failure> {
        let p = Params::new(params);
        let call = match method {
            "get_board" => {
                p.only(&["date"])?;
                Self::Read(ReadCall::GetBoard(p.date()?))
            }
            "open_board" => {
                p.only(&["date"])?;
                Self::Read(ReadCall::OpenBoard(p.date()?))
            }
            "get_note" => {
                p.only(&["date", "heading"])?;
                Self::Read(ReadCall::GetNote(p.date()?, p.optional_string("heading")?))
            }
            "search" => {
                p.only(&["query", "from", "to", "kinds", "limit"])?;
                Self::Read(ReadCall::Search(p.search()?))
            }
            "add_todo" => {
                p.only(&["text", "date"])?;
                Self::Write(WriteCall::AddTodo(p.date()?, p.string("text")?))
            }
            "add_blocker" => {
                p.only(&["text", "date"])?;
                Self::Write(WriteCall::AddBlocker(p.date()?, p.string("text")?))
            }
            "edit_todo" => {
                p.only(&["id", "text"])?;
                Self::Write(WriteCall::EditTodo(p.id()?, p.string("text")?))
            }
            "edit_blocker" => {
                p.only(&["id", "text"])?;
                Self::Write(WriteCall::EditBlocker(p.id()?, p.string("text")?))
            }
            "complete_todo" => {
                p.only(&["id"])?;
                Self::Write(WriteCall::CompleteTodo(p.id()?))
            }
            "reopen_todo" => {
                p.only(&["id"])?;
                Self::Write(WriteCall::ReopenTodo(p.id()?))
            }
            "resolve_blocker" => {
                p.only(&["id", "note"])?;
                Self::Write(WriteCall::ResolveBlocker(
                    p.id()?,
                    p.optional_string("note")?,
                ))
            }
            "append_note" => {
                p.only(&["text", "date", "heading"])?;
                Self::Write(WriteCall::AppendNote(
                    p.date()?,
                    p.string("text")?,
                    p.optional_string("heading")?,
                ))
            }
            "append_reflection" => {
                p.only(&["text", "date"])?;
                Self::Write(WriteCall::AppendReflection(p.date()?, p.string("text")?))
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
                Self::Write(WriteCall::EditNote(p.date()?, p.line_edit()?))
            }
            "list_agents" => {
                p.only(&[])?;
                Self::Admin(AdminCall::ListAgents)
            }
            "set_agent_scopes" => {
                p.only(&["client", "allow", "deny"])?;
                Self::Admin(AdminCall::SetScopes {
                    client: p.string("client")?,
                    allow: p.scopes("allow")?,
                    deny: p.scopes("deny")?,
                })
            }
            "list_changes" => {
                p.only(&["client", "unseen", "before", "limit"])?;
                Self::Admin(AdminCall::ListChanges(p.changes()?))
            }
            "preview_revert" => {
                p.only(&["change_id"])?;
                Self::Admin(AdminCall::PreviewRevert(p.change_id("change_id")?))
            }
            "revert_change" => {
                p.only(&["change_id", "force", "expect"])?;
                Self::Admin(AdminCall::Revert {
                    id: p.change_id("change_id")?,
                    force: p.boolean("force")?,
                    expect: p.optional_string("expect")?,
                })
            }
            _ => return Err(Failure::NoMethod),
        };
        Ok(call)
    }
}

impl ReadCall {
    /// 読み取りを行う。`client_id` は [`resolve_client`] で求めたもの。
    pub fn execute(&self, conn: &Connection, client_id: Ulid) -> Result<Value, Failure> {
        let at = now();
        match self {
            Self::GetBoard(date) => {
                let date = resolve(conn, *date, at)?;
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
            }
            Self::OpenBoard(date) => {
                let date = resolve(conn, *date, at)?;
                let b = board::open_board(conn, client_id, date).map_err(read_failure)?;
                Ok(json!({
                    "date": b.date.to_string(),
                    "todos": b.todos.iter().map(|i| item_json(i, ItemKind::Todo)).collect::<Vec<_>>(),
                    "blockers": b.blockers.iter().map(|i| item_json(i, ItemKind::Blocker)).collect::<Vec<_>>(),
                }))
            }
            Self::GetNote(date, heading) => {
                let date = resolve(conn, *date, at)?;
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
            }
            Self::Search(args) => {
                let from = args.from.map(|d| resolve(conn, d, at)).transpose()?;
                let to = args.to.map(|d| resolve(conn, d, at)).transpose()?;
                let query = SearchQuery {
                    text: &args.query,
                    from,
                    to,
                    kinds: &args.kinds,
                    limit: args.limit,
                };
                let found = search::search(conn, client_id, &query).map_err(read_failure)?;
                Ok(json!({
                    "query": args.query.trim(),
                    "hits": found.iter().map(found_json).collect::<Vec<_>>(),
                }))
            }
        }
    }
}

impl WriteCall {
    /// 書き込みを行う。`agent` は [`resolve_client`] で求めた呼び出し元（アプリでは画面の状態つき、直接
    /// モードでは画面なし）。`tz`・`at` は、錠を取った後に 1 回だけ求めた値。
    pub fn execute(
        &self,
        conn: &Connection,
        agent: &Agent<'_>,
        tz: &TimeZone,
        at: Timestamp,
    ) -> Result<Reply, Failure> {
        match self {
            Self::AddTodo(date, text) => {
                let date = resolve(conn, *date, at)?;
                let i = item::add_todo(conn, agent, date, text, tz, at).map_err(item_failure)?;
                Ok(Reply::new(added_json(&i), true))
            }
            Self::AddBlocker(date, text) => {
                let date = resolve(conn, *date, at)?;
                let i = item::add_blocker(conn, agent, date, text, tz, at).map_err(item_failure)?;
                Ok(Reply::new(added_json(&i), true))
            }
            Self::EditTodo(id, text) => {
                let o = item::edit_todo(conn, agent, *id, text, at).map_err(item_failure)?;
                let value =
                    json!({"updated": o.updated, "id": o.item.id.to_string(), "text": o.item.text});
                Ok(Reply::new(value, o.updated))
            }
            Self::EditBlocker(id, text) => {
                let o = item::edit_blocker(conn, agent, *id, text, at).map_err(item_failure)?;
                let value =
                    json!({"updated": o.updated, "id": o.item.id.to_string(), "text": o.item.text});
                Ok(Reply::new(value, o.updated))
            }
            Self::CompleteTodo(id) => {
                let o = item::complete_todo(conn, agent, *id, at).map_err(item_failure)?;
                Ok(Reply::new(status_json(o.updated, &o.item), o.updated))
            }
            Self::ReopenTodo(id) => {
                let o = item::reopen_todo(conn, agent, *id, at).map_err(item_failure)?;
                Ok(Reply::new(status_json(o.updated, &o.item), o.updated))
            }
            Self::ResolveBlocker(id, note) => {
                let r = item::resolve_blocker(conn, agent, *id, note.as_deref(), at)
                    .map_err(item_failure)?;
                let value = json!({
                    "updated": r.updated(),
                    "resolved": r.resolved,
                    "note_written": r.note_written,
                    "id": r.item.id.to_string(),
                    "status": r.item.status.as_str(),
                    "resolution_note": r.item.resolution_note,
                });
                Ok(Reply::new(value, r.updated()))
            }
            Self::AppendNote(date, text, heading) => {
                let date = resolve(conn, *date, at)?;
                let w = text_doc::append_note(conn, date, text, heading.as_deref(), agent, tz, at)
                    .map_err(text_doc_failure)?;
                Ok(Reply::new(written_json(&w), w.updated))
            }
            Self::AppendReflection(date, text) => {
                let date = resolve(conn, *date, at)?;
                let w = text_doc::append_reflection(conn, date, text, agent, tz, at)
                    .map_err(text_doc_failure)?;
                Ok(Reply::new(written_json(&w), w.updated))
            }
            Self::EditNote(date, edit) => {
                let date = resolve(conn, *date, at)?;
                let w = text_doc::edit_note(conn, date, edit, agent, tz, at)
                    .map_err(text_doc_failure)?;
                Ok(Reply::new(written_json(&w), w.updated))
            }
        }
    }
}

impl AdminCall {
    /// データを何も変えない操作か。
    pub fn is_read_only(&self) -> bool {
        matches!(
            self,
            Self::ListAgents | Self::ListChanges(_) | Self::PreviewRevert(_)
        )
    }

    /// 管理の操作を行う。`agent` は [`resolve_client`] で求めた呼び出し元（画面の状態を持つ。取り消しが
    /// 入力中・保存の失敗中を確かめるため）。
    pub fn execute(
        &self,
        conn: &Connection,
        agent: &Agent<'_>,
        tz: &TimeZone,
        at: Timestamp,
    ) -> Result<Reply, Failure> {
        match self {
            Self::ListAgents => {
                let clients = agent_admin::list_clients(conn).map_err(storage_failure)?;
                let agents: Vec<Value> = clients
                    .iter()
                    .map(|c| {
                        json!({
                            "id": c.id.to_string(),
                            "name": c.name,
                            "scopes": c.scopes.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                            "created_at": format_time(c.created_at),
                            "last_seen_at": c.last_seen_at.map(format_time),
                        })
                    })
                    .collect();
                Ok(Reply::new(json!({"agents": agents}), false))
            }
            Self::SetScopes {
                client,
                allow,
                deny,
            } => {
                let c =
                    agent_admin::change_scopes(conn, client, allow, deny).map_err(scope_failure)?;
                let names =
                    |scopes: &[Scope]| scopes.iter().map(|s| s.as_str()).collect::<Vec<_>>();
                let value = json!({
                    "updated": c.updated,
                    "client": c.client.name,
                    "before": names(&c.before),
                    "scopes": names(&c.client.scopes),
                });
                Ok(Reply::new(value, c.updated))
            }
            Self::ListChanges(args) => {
                let filter = ChangeFilter {
                    client: args.client.clone(),
                    unseen_only: args.unseen_only,
                    before: args.before,
                    limit: args.limit,
                };
                let entries = history::list_changes(conn, &filter).map_err(history_failure)?;
                let shown = args.limit.min(history::MAX_LIMIT);
                let next_before = (entries.len() == shown)
                    .then(|| entries.last().map(|e| e.log.id.to_string()))
                    .flatten();
                let value = json!({
                    "changes": entries.iter().map(entry_json).collect::<Vec<_>>(),
                    "next_before": next_before,
                });
                Ok(Reply::new(value, false))
            }
            Self::PreviewRevert(id) => {
                let plan = history::preview(conn, *id).map_err(revert_failure)?;
                Ok(Reply::new(plan_json(&plan), false))
            }
            Self::Revert { id, force, expect } => {
                let request = RevertRequest {
                    force: *force,
                    expect: expect.as_deref(),
                };
                let r = history::revert(conn, *id, &request, agent.ui, tz, at)
                    .map_err(revert_failure)?;
                let state = match (r.updated, r.plan.state) {
                    (true, _) => "reverted",
                    (false, PlanState::AlreadyReverted) => "already_reverted",
                    (false, _) => "already_restored",
                };
                let mut value = json!({
                    "updated": r.updated,
                    "id": r.plan.log.id.to_string(),
                    "state": state,
                    "revert_id": r.revert_id.map(|i| i.to_string()),
                    "effect": effect_json(&r.plan.effect),
                });
                value["operation"] = json!(r.plan.log.operation);
                // 値を書かなくても、記録を取り消し済みにしたときは、履歴の画面を読み直せるよう知らせる
                let changed = r.updated || r.plan.state == PlanState::AlreadyRestored;
                Ok(Reply::new(value, changed))
            }
        }
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
        Ok(self.optional_date("date")?.unwrap_or(Date(DateSpec::Today)))
    }

    fn optional_date(&self, key: &str) -> Result<Option<Date>, Failure> {
        match self.optional_string(key)? {
            None => Ok(None),
            Some(s) => s.parse().map(|d| Some(Date(d))).map_err(|_| {
                Failure::App(invalid_argument(&format!(
                    "{key} must be YYYY-MM-DD, today or yesterday."
                )))
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

    /// 変更履歴の記録の ID。
    fn change_id(&self, key: &str) -> Result<Ulid, Failure> {
        self.string(key)?.parse().map_err(|_| {
            Failure::App(invalid_argument(
                "change_id is not a valid change ID. Use an ID from the change history.",
            ))
        })
    }

    /// 真偽の引数。無ければ偽。
    fn boolean(&self, key: &str) -> Result<bool, Failure> {
        match self.get(key) {
            None => Ok(false),
            Some(Value::Bool(b)) => Ok(*b),
            Some(_) => Err(Failure::Params(format!("{key} must be true or false"))),
        }
    }

    /// スコープの名前の配列（無ければ空）。
    fn scopes(&self, key: &str) -> Result<Vec<Scope>, Failure> {
        match self.get(key) {
            None => Ok(Vec::new()),
            Some(Value::Array(items)) => {
                let mut scopes = Vec::new();
                for item in items {
                    let scope = item
                        .as_str()
                        .and_then(|s| s.parse::<Scope>().ok())
                        .ok_or_else(|| {
                            Failure::App(invalid_argument(&format!(
                                "{key} must be a list of read, todo, blocker, append and note_edit."
                            )))
                        })?;
                    if !scopes.contains(&scope) {
                        scopes.push(scope);
                    }
                }
                Ok(scopes)
            }
            Some(_) => Err(Failure::Params(format!("{key} must be a list of strings"))),
        }
    }

    /// `list_changes` の引数。`limit` は 1 以上（無ければ既定の件数）。
    fn changes(&self) -> Result<ChangesArgs, Failure> {
        let limit = match self.number("limit")? {
            None => history::DEFAULT_LIMIT,
            Some(0) => {
                return Err(Failure::App(invalid_argument("limit must be 1 or more.")));
            }
            Some(n) => n,
        };
        Ok(ChangesArgs {
            client: self.optional_string("client")?,
            unseen_only: self.boolean("unseen")?,
            before: match self.optional_string("before")? {
                None => None,
                Some(s) => Some(s.parse().map_err(|_| {
                    Failure::App(invalid_argument(
                        "before is not a valid change ID. Use an ID from the change history.",
                    ))
                })?),
            },
            limit,
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

    /// `search` の引数。`kinds` は種類の名前の配列（`todo`・`blocker`・`note`・`reflection`。無いか空な
    /// らすべて）、`limit` は 1 以上の整数（無ければ既定の件数）。
    fn search(&self) -> Result<SearchArgs, Failure> {
        let kinds = match self.get("kinds") {
            None => Vec::new(),
            Some(Value::Array(items)) => {
                let mut kinds = Vec::new();
                for item in items {
                    let kind = item
                        .as_str()
                        .and_then(|s| s.parse::<SearchKind>().ok())
                        .ok_or_else(|| {
                            Failure::App(invalid_argument(
                                "kinds must be a list of todo, blocker, note and reflection.",
                            ))
                        })?;
                    if !kinds.contains(&kind) {
                        kinds.push(kind);
                    }
                }
                kinds
            }
            Some(_) => return Err(Failure::Params("kinds must be a list of strings".into())),
        };
        let limit = match self.number("limit")? {
            None => search::DEFAULT_LIMIT,
            Some(0) => {
                return Err(Failure::App(invalid_argument("limit must be 1 or more.")));
            }
            Some(n) => n,
        };
        Ok(SearchArgs {
            query: self.string("query")?,
            from: self.optional_date("from")?,
            to: self.optional_date("to")?,
            kinds,
            limit,
        })
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
fn resolve(conn: &Connection, date: Date, at: Timestamp) -> Result<LogicalDate, Failure> {
    let boundary = day::day_boundary(conn).map_err(storage_failure)?;
    date.0
        .resolve_at(at, &system_tz(), boundary)
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

/// 検索結果の 1 件（05 4.4 の `search`）。項目は `id`・`text`（障害は解消済みなら `resolution_note`）・持ち越し先、
/// ノート・振り返りは一致した行（`matches`）。
fn found_json(found: &search::Found) -> Value {
    let hit = &found.hit;
    let mut o = Map::new();
    o.insert("kind".into(), json!(hit.kind.as_str()));
    o.insert("date".into(), json!(hit.date.to_string()));
    match hit.kind {
        SearchKind::Todo | SearchKind::Blocker => {
            o.insert("id".into(), json!(hit.ref_id));
            o.insert("text".into(), json!(hit.body));
            if hit.kind == SearchKind::Blocker {
                o.insert("resolution_note".into(), json!(hit.resolution_note));
            }
            o.insert(
                "carried_to_id".into(),
                json!(hit.carried_to.map(|c| c.id.to_string())),
            );
            o.insert(
                "carried_to_date".into(),
                json!(hit.carried_to.map(|c| c.date.to_string())),
            );
        }
        SearchKind::Note | SearchKind::Reflection => {
            o.insert(
                "matches".into(),
                json!(
                    found
                        .lines
                        .iter()
                        .map(|l| json!({"line": l.line, "text": l.text}))
                        .collect::<Vec<_>>()
                ),
            );
        }
    }
    Value::Object(o)
}

/// 変更履歴の1件（05 4.4 の `list_changes`）。
fn entry_json(e: &history::Entry) -> Value {
    let log = &e.log;
    let target = match (log.target, &e.item) {
        (Target::Item(id), item) => {
            let mut o = json!({"type": "item", "id": id.to_string()});
            if let Some(i) = item {
                o["date"] = json!(i.date.to_string());
                o["kind"] = json!(i.kind.as_str());
                o["text"] = json!(i.text);
                o["deleted"] = json!(i.deleted);
            }
            o
        }
        (Target::Note(date), _) => json!({"type": "note", "date": date.to_string()}),
        (Target::Reflection(date), _) => json!({"type": "reflection", "date": date.to_string()}),
    };
    let summary = match &e.summary {
        history::Summary::ItemAdded { kind, text } => {
            json!({"type": "added", "kind": kind, "text": text})
        }
        history::Summary::Fields(fields) => json!({
            "type": "fields",
            "fields": fields.iter().map(|f| json!({
                "name": f.name, "before": f.before, "after": f.after,
            })).collect::<Vec<_>>(),
        }),
        history::Summary::Text {
            before_chars,
            after_chars,
        } => json!({"type": "text", "before_chars": before_chars, "after_chars": after_chars}),
        history::Summary::Unknown => json!({"type": "unknown"}),
    };
    json!({
        "id": log.id.to_string(),
        "created_at": format_time(log.created_at),
        "actor": log.actor.as_str(),
        "client": e.client_name,
        "operation": log.operation,
        "target": target,
        "summary": summary,
        "seen_at": log.seen_at.map(format_time),
        "reverted_at": log.reverted_at.map(format_time),
        "revert_of": log.revert_of_id.map(|i| i.to_string()),
    })
}

/// 取り消しの計画（05 4.4 の `preview_revert`）。
fn plan_json(p: &history::Plan) -> Value {
    let state = match p.state {
        PlanState::Apply => "apply",
        PlanState::AlreadyReverted => "already_reverted",
        PlanState::AlreadyRestored => "already_restored",
    };
    json!({
        "id": p.log.id.to_string(),
        "operation": p.log.operation,
        "client": p.client_name,
        "created_at": format_time(p.log.created_at),
        "state": state,
        "token": p.token,
        "changed_since": p.drift,
        "effect": effect_json(&p.effect),
    })
}

fn effect_json(e: &history::Effect) -> Value {
    match e {
        history::Effect::None => json!({"type": "none"}),
        history::Effect::DeleteItem { item, carried_to } => json!({
            "type": "delete_item",
            "id": item.id.to_string(),
            "date": item.date.to_string(),
            "kind": item.kind.as_str(),
            "text": item.text,
            "carried_to_id": carried_to.map(|c| c.id.to_string()),
            "carried_to_date": carried_to.map(|c| c.date.to_string()),
        }),
        history::Effect::Fields { item, fields } => json!({
            "type": "fields",
            "id": item.id.to_string(),
            "date": item.date.to_string(),
            "kind": item.kind.as_str(),
            "text": item.text,
            "fields": fields.iter().map(|f| json!({
                "name": f.name,
                "current": f.current,
                "restore": f.restore,
                "changed_since": f.changed_since,
            })).collect::<Vec<_>>(),
        }),
        history::Effect::Text {
            doc, date, diff, ..
        } => json!({
            "type": "text",
            "doc": match doc {
                TextDoc::Note => "note",
                TextDoc::Reflection => "reflection",
            },
            "date": date.to_string(),
            "removed": diff.removed,
            "added": diff.added,
            "truncated": diff.truncated,
        }),
    }
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
    eprintln!("yunagi: a request failed: {}", ErrorChain(&e));
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

/// 権限の変更の失敗。
fn scope_failure(e: ScopeError) -> Failure {
    Failure::App(match e {
        ScopeError::NotFound => error(
            "not_found",
            "No client with this name is registered. A client is registered when it first connects.",
        ),
        ScopeError::InvalidArgument(ScopeArgument::Nothing) => {
            invalid_argument("Give at least one scope to allow or deny.")
        }
        ScopeError::InvalidArgument(ScopeArgument::Both(scope)) => {
            invalid_argument(&format!("The scope {scope} is both allowed and denied."))
        }
        ScopeError::InvalidArgument(ScopeArgument::UnknownWrite(scope)) => {
            invalid_argument(&format!(
                "The unknown client (connections without an identifier) can only have the read scope, not {scope}. Register the agent with `yunagi agent setup` so that it has its own name."
            ))
        }
        ScopeError::Storage(e) => return storage_failure(e),
    })
}

/// 変更履歴の一覧の失敗。
fn history_failure(e: HistoryError) -> Failure {
    match e {
        HistoryError::ClientNotFound => Failure::App(error(
            "not_found",
            "No client with this name is registered.",
        )),
        HistoryError::ZeroLimit => Failure::App(invalid_argument("limit must be 1 or more.")),
        HistoryError::Storage(e) => storage_failure(e),
    }
}

/// 取り消しの失敗。どれも何も書いていない。
fn revert_failure(e: RevertError) -> Failure {
    Failure::App(match e {
        RevertError::NotFound(Missing::Change) => error(
            "not_found",
            "No change record with this ID exists. It may have been replaced by the user's own change. Read the change history again.",
        ),
        RevertError::NotFound(Missing::Item) => error(
            "not_found",
            "The item of this change was deleted, so there is nothing to revert.",
        ),
        RevertError::InvalidArgument(RevertArgument::NotAgentChange) => invalid_argument(
            "This record is not a change made by an agent (it is a revert). Revert the original change instead.",
        ),
        RevertError::InvalidArgument(RevertArgument::Unsupported) => {
            invalid_argument("This record cannot be reverted because its content cannot be read.")
        }
        RevertError::InvalidArgument(RevertArgument::Dropped { close_reason }) => {
            let what = match close_reason {
                CloseReason::Dropped => "dropped",
                CloseReason::Split => "split into other TODOs",
                CloseReason::ReturnedExternal => "returned to the external tool",
            };
            json!({
                "code": "invalid_argument",
                "close_reason": close_reason.as_str(),
                "message": format!(
                    "This TODO was {what} by the user. A closed TODO keeps its text and state, so the change was not reverted."
                ),
            })
        }
        RevertError::StorageUnavailable(_) => storage_unavailable_retrying(),
        RevertError::Conflict(RevertConflict::Changed {
            fields,
            after_preview: true,
        }) => json!({
            "code": "conflict",
            "reason": "changed",
            "changed_fields": fields,
            "message": "The target changed after the preview. Nothing was changed. Review it again.",
        }),
        RevertError::Conflict(RevertConflict::Changed {
            fields,
            after_preview: false,
        }) => json!({
            "code": "conflict",
            "reason": "changed",
            "changed_fields": fields,
            "message": format!(
                "The target was changed after this record (changed: {}). Nothing was changed. Review the difference, then run again with force to overwrite it.",
                fields.join(", ")
            ),
        }),
        RevertError::Conflict(RevertConflict::CarriedOver {
            carried_to_id,
            carried_to_date,
        }) => json!({
            "code": "conflict",
            "reason": "carried_over",
            "carried_to_id": carried_to_id.to_string(),
            "carried_to_date": carried_to_date.to_string(),
            "message": format!(
                "This item was carried over to {carried_to_date}, and a carried-over item keeps the state it ended the day with. Nothing was changed. Fix the carried-over item by hand if needed."
            ),
        }),
        RevertError::Conflict(RevertConflict::UserEditing) => {
            conflict("user_editing", "The user is editing this. Try again later.")
        }
        RevertError::Storage(e) => return storage_failure(e),
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
