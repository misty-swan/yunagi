//! ローカル API の依頼を `AppCore` の入口へ渡し、結果を応答の JSON にする（MCP・CLI 仕様書 3章・4.2・
//! 4.3、アーキテクチャ設計書 4.6）。
//!
//! 引数を読み、コアを呼んで 05 の JSON にする部分は `yunagi_api::ops` にある（直接モードの CLI・MCP と
//! 同じものを使い、応答が食い違わないようにするため）。ここは、それを
//! 読み取りは [`AppCore::api_read`]、書き込みは [`AppCore::api_write`]（`agent_write` のローカル API
//! 用の版。画面と同じ `changed` を出す）で包み、待てなかったときの `busy` を足すだけ。

use std::cell::Cell;

use serde_json::Value;
use yunagi_api::ops::{self, AdminCall, Call, Failure, ReadCall, Reply, WriteCall};
use yunagi_api::wire::{self, Request};

use crate::app_core::{AppCore, Busy, now, system_tz};

pub use yunagi_api::ops::busy_error;

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

fn run(core: &AppCore, client: Option<&str>, request: &Request) -> Result<Value, Failure> {
    let call = Call::parse(&request.method, &request.params)?;
    // データベースを使えるまで待つ期限は、依頼ごとに 1 回だけ求める（クライアントを求める処理も、
    // 操作の本体も、同じ期限と同じ錠の中で行う）
    let registered = Cell::new(false);
    let deadline = core.api_deadline();
    let result = match &call {
        Call::Read(read) => read_call(core, read, deadline, client, &registered),
        Call::Write(write) => write_call(core, write, deadline, client, &registered),
        Call::Admin(admin) => admin_call(core, admin, deadline, client, &registered),
    };
    // 初めての識別子を登録したら、設定画面のエージェントの一覧が変わるので、画面に知らせる
    if registered.get() {
        core.notify_agent_registered();
    }
    result
}

fn read_call(
    core: &AppCore,
    call: &ReadCall,
    deadline: std::time::Instant,
    client: Option<&str>,
    registered: &Cell<bool>,
) -> Result<Value, Failure> {
    core.api_read(deadline, |conn| {
        let client_id = ops::resolve_client(conn, client, registered)?;
        call.execute(conn, client_id)
    })
    .map_err(|Busy| Failure::App(busy_error()))?
}

/// 書き込みの依頼を `AppCore::api_write` に渡す。
///
/// 書き込む時刻とタイムゾーンは、錠を取ってから（待った後に）1 回だけ求める（日付の求め方にも同じ
/// 時刻を使う）。待つ間に時刻が進んでも、記録の時刻と日付がずれないようにする。
fn write_call(
    core: &AppCore,
    call: &WriteCall,
    deadline: std::time::Instant,
    client: Option<&str>,
    registered: &Cell<bool>,
) -> Result<Value, Failure> {
    let result = core
        .api_write(
            deadline,
            |conn| ops::resolve_client(conn, client, registered),
            |conn, agent| call.execute(conn, agent, &system_tz(), now()).map(Changed),
        )
        .map_err(|Busy| Failure::App(busy_error()))??;
    Ok(result.0.value)
}

/// 管理の操作（権限・変更履歴・取り消し。05 4.4）を `AppCore::api_write` に渡す。一覧と取り消しの計画は
/// 何も変えない（`changed` を出さない）が、取り消しは画面の入力中・保存の失敗中の状態を見るので、
/// 書き込みと同じ経路で行う。
fn admin_call(
    core: &AppCore,
    call: &AdminCall,
    deadline: std::time::Instant,
    client: Option<&str>,
    registered: &Cell<bool>,
) -> Result<Value, Failure> {
    let result = core
        .api_write(
            deadline,
            |conn| ops::resolve_client(conn, client, registered),
            |conn, agent| call.execute(conn, agent, &system_tz(), now()).map(Changed),
        )
        .map_err(|Busy| Failure::App(busy_error()))??;
    Ok(result.0.value)
}

/// 書き込みの成功の応答と、データを実際に変えたか（コアの結果から決める。`changed` を出す対象かの
/// 判断に使う）。
struct Changed(Reply);

impl crate::app_core::Changes for Changed {
    fn changed(&self) -> bool {
        self.0.changed
    }
}
