//! ローカル API の結合テスト（テスト計画書 4.2「ローカル API」）：実際にソケットで待ち受け、
//! `yunagi-api` のクライアントから依頼を送って、コアの結果・権限・`changed`・止め方を確かめる。
//! WebView は開かない（`AppCore` を直接動かす）。
//!
//! 既定ディレクトリは環境変数から求まるので、`bridge` と同じく、テストの仕組み（harness）を使わず
//! `main` から1つずつ順に走らせる（Cargo.toml の `[[test]]`）。UNIX ドメインソケットの試験は
//! UNIX 系だけで走る（Windows の名前付きパイプは実機で確かめる。テスト計画書 8章）。

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use yunagi_api::client::{self, CallError, HelloError};
use yunagi_api::transport::{self, Stream};
use yunagi_api::wire::{self, APP_ERROR};
use yunagi_api::{CLIENT_CONNECT_TIMEOUT, PROTOCOL_VERSION};
use yunagi_app::api::{ApiOptions, ApiState, ApiUnavailableReason};
use yunagi_app::types::UiReport;
use yunagi_app::{AppCore, Notice};
use yunagi_core::domain::agent::{self, ClientLookup};
use yunagi_core::storage::data_dir::{self, DefaultDir};
use yunagi_core::storage::table::agent_client::{self, Scope};
use yunagi_core::storage::{StorageError, Ulid};

/// 待つ時間の上限。
const WAIT: Duration = Duration::from_secs(10);

type Test = fn(&DefaultDir);

pub fn main() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_path_buf();
    // SAFETY: まだほかのスレッドを作っていない（テストはこの後、このスレッドで順に走らせる）
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_DATA_HOME", &home);
        std::env::set_var("LOCALAPPDATA", &home);
    }
    let dir = data_dir::default_dir().unwrap();
    assert!(dir.path().starts_with(&home));
    let tests: [(&str, Test); 9] = [
        (
            "listens_only_while_open_and_cleans_up",
            listens_only_while_open_and_cleans_up,
        ),
        (
            "a_stale_socket_is_replaced_and_a_foreign_file_is_not",
            a_stale_socket_is_replaced_and_a_foreign_file_is_not,
        ),
        (
            "hello_checks_the_version_the_token_and_the_form",
            hello_checks_the_version_the_token_and_the_form,
        ),
        (
            "reads_need_the_read_scope_and_writes_need_theirs",
            reads_need_the_read_scope_and_writes_need_theirs,
        ),
        (
            "writes_go_through_agent_write_and_notify_changed",
            writes_go_through_agent_write_and_notify_changed,
        ),
        (
            "the_screen_state_protects_what_the_user_is_editing",
            the_screen_state_protects_what_the_user_is_editing,
        ),
        (
            "requests_wait_at_most_the_limit_then_answer_busy",
            requests_wait_at_most_the_limit_then_answer_busy,
        ),
        (
            "stopping_finishes_running_requests_and_closes_idle_connections",
            stopping_finishes_running_requests_and_closes_idle_connections,
        ),
        (
            "request_form_errors_use_the_standard_codes",
            request_form_errors_use_the_standard_codes,
        ),
    ];
    let filters: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();
    let mut passed = 0;
    for (name, test) in tests {
        if !filters.is_empty() && !filters.iter().any(|f| name.contains(f.as_str())) {
            continue;
        }
        print!("test {name} ... ");
        test(&dir);
        println!("ok");
        passed += 1;
    }
    println!("\ntest result: ok. {passed} passed");
}

// ---- 道具 ----

/// 動かしているコア（`Drop` で閉じてロックを離す）。
struct Harness {
    core: Arc<AppCore>,
    notices: Receiver<Notice>,
    dir: DefaultDir,
}

impl Harness {
    /// データベースを開いて、待ち受けを始めたコア。
    fn opened(dir: &DefaultDir) -> Self {
        let h = Self::unopened(dir);
        h.core.open();
        h.core.wait_for_backup();
        h.drain();
        h
    }

    fn unopened(dir: &DefaultDir) -> Self {
        let (tx, notices) = mpsc::channel();
        let tx = Mutex::new(tx);
        let core = AppCore::new(Box::new(move |notice| {
            let _ = tx.lock().unwrap().send(notice);
        }));
        Self {
            core,
            notices,
            dir: dir.clone(),
        }
    }

    fn drain(&self) {
        while self.notices.try_recv().is_ok() {}
    }

    /// 次の `changed` の `by`。
    fn next_changed(&self) -> String {
        loop {
            match self.notices.recv_timeout(WAIT).expect("no notice") {
                Notice::Changed(c) => {
                    return serde_json::to_value(c).unwrap()["by"]
                        .as_str()
                        .unwrap()
                        .to_owned();
                }
                _ => continue,
            }
        }
    }

    fn assert_no_changed(&self) {
        while let Ok(n) = self.notices.recv_timeout(Duration::from_millis(100)) {
            assert!(!matches!(n, Notice::Changed(_)), "unexpected {n:?}");
        }
    }

    /// 識別子 `client` で依頼を送る。成功すれば `result`、アプリのエラーなら `Err(error)`。
    fn call(&self, client: Option<&str>, method: &str, params: Value) -> Result<Value, Value> {
        let session = client::connect(&self.dir, client, CLIENT_CONNECT_TIMEOUT)
            .unwrap_or_else(|e| panic!("connect failed: {e:?}"));
        match session.call(method, &params) {
            Ok(v) => Ok(v),
            Err(CallError::Rpc(e)) => Err(e
                .app_error()
                .cloned()
                .unwrap_or_else(|| json!({"rpc_code": e.code, "message": e.message}))),
            Err(e) => panic!("{method} failed: {e:?}"),
        }
    }

    fn ok(&self, client: &str, method: &str, params: Value) -> Value {
        self.call(Some(client), method, params)
            .unwrap_or_else(|e| panic!("{method} returned {e}"))
    }

    fn err(&self, client: &str, method: &str, params: Value) -> Value {
        match self.call(Some(client), method, params) {
            Ok(v) => panic!("{method} succeeded: {v}"),
            Err(e) => e,
        }
    }

    /// 識別子 `client` に、`read` に加えて `scopes` を許可する（初めての識別子は登録される）。
    fn grant(&self, client: &str, scopes: &[Scope]) {
        // テストの間でデータベースを共有するので、前のテストが外した権限を直す
        let mut all = vec![Scope::Read];
        all.extend_from_slice(scopes);
        self.core
            .read(|conn| -> Result<(), StorageError> {
                let id = match agent::client_for(
                    conn,
                    Some(client),
                    yunagi_core::storage::table::now(),
                )? {
                    ClientLookup::Known(id) | ClientLookup::Registered(id) => id,
                    other => panic!("{other:?}"),
                };
                agent_client::update_scopes(conn, id, &all)?;
                Ok(())
            })
            .unwrap();
    }

    /// 生の接続（`hello` はまだ送っていない）と、トークンのファイルのトークン。
    fn raw(&self) -> (Stream, String) {
        let (stream, _) = transport::connect(&self.dir, CLIENT_CONNECT_TIMEOUT).unwrap();
        let info = yunagi_api::token::read(&self.dir).unwrap();
        (stream, info.token)
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.core.wait_for_backup();
        self.core.close();
    }
}

fn socket_path(dir: &DefaultDir) -> std::path::PathBuf {
    yunagi_api::socket_path(dir)
}

fn token_path(dir: &DefaultDir) -> std::path::PathBuf {
    yunagi_api::token_path(dir)
}

fn send_line(stream: &Stream, value: &Value) {
    let mut line = value.to_string();
    line.push('\n');
    (&*stream).write_all(line.as_bytes()).unwrap();
}

fn read_reply(stream: &Stream) -> Option<Value> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(serde_json::from_str(&line).unwrap()),
    }
}

fn hello(token: &str, protocol: u32) -> Value {
    json!({"jsonrpc": "2.0", "id": 0, "method": "hello",
           "params": {"protocol": protocol, "token": token, "client": "claude-code"}})
}

fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn ids(list: &Value) -> Vec<String> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_owned())
        .collect()
}

// ---- テスト ----

/// 待ち受けるのは、データベースを開き終えている間だけ。ソケットとトークンは本人だけの権限で、
/// 閉じるとどちらも消える（01 4.4・4.6）。
fn listens_only_while_open_and_cleans_up(dir: &DefaultDir) {
    let h = Harness::unopened(dir);
    assert_eq!(h.core.api_state(), ApiState::Stopped);
    assert!(!socket_path(dir).exists());
    assert!(!token_path(dir).exists());
    h.core.open();
    assert_eq!(h.core.api_state(), ApiState::Listening { connections: 0 });
    assert_eq!(mode(dir.path()), 0o700);
    assert_eq!(mode(&socket_path(dir)), 0o600);
    assert_eq!(mode(&token_path(dir)), 0o600);
    let token = yunagi_api::token::read(dir).unwrap().token;
    assert!(yunagi_api::token::is_well_formed(&token));
    // 起動のたびにトークンを作り直す
    h.core.wait_for_backup();
    h.core.close();
    assert_eq!(h.core.api_state(), ApiState::Stopped);
    assert!(!socket_path(dir).exists());
    assert!(!token_path(dir).exists());
    let again = Harness::opened(dir);
    assert_ne!(yunagi_api::token::read(dir).unwrap().token, token);
    drop(again);
}

/// 異常終了で残ったソケットは、ロックを持つアプリが消して作り直す。ソケット以外のものは消さず、
/// 待ち受けを始めない（理由は状態に残る）。
fn a_stale_socket_is_replaced_and_a_foreign_file_is_not(dir: &DefaultDir) {
    // 前の実行が残したソケット
    drop(std::os::unix::net::UnixListener::bind(socket_path(dir)).unwrap());
    let h = Harness::opened(dir);
    assert_eq!(h.core.api_state(), ApiState::Listening { connections: 0 });
    h.ok("claude-code", "get_board", json!({}));
    drop(h);
    assert!(!socket_path(dir).exists());

    // ソケットの場所に普通のファイルがある：消さず、待ち受けない。アプリは開く
    std::fs::write(socket_path(dir), "data").unwrap();
    let h = Harness::opened(dir);
    match h.core.api_state() {
        ApiState::Unavailable(u) => assert_eq!(u.reason, ApiUnavailableReason::PathOccupied),
        other => panic!("{other:?}"),
    }
    assert_eq!(std::fs::read_to_string(socket_path(dir)).unwrap(), "data");
    assert!(!token_path(dir).exists());
    assert!(h.core.today().is_ok());
    drop(h);
    std::fs::remove_file(socket_path(dir)).unwrap();
}

/// `hello`：版 → トークン → 識別子の順に確かめる。通らなければ依頼を受けない。
fn hello_checks_the_version_the_token_and_the_form(dir: &DefaultDir) {
    let h = Harness::opened(dir);
    let error = |reply: &Value| reply["error"]["data"]["error"].clone();

    // 版が違う
    let (stream, token) = h.raw();
    send_line(&stream, &hello(&token, PROTOCOL_VERSION + 1));
    let e = error(&read_reply(&stream).unwrap());
    assert_eq!(e["code"], "api_error");
    assert_eq!(e["reason"], "version_mismatch");
    assert_eq!(e["server_protocol"], PROTOCOL_VERSION);

    // トークンが違う（形が誤っているもの・長さが違うもの・1文字違うもの）
    let (_, token) = h.raw();
    let mut one_off = token.clone().into_bytes();
    one_off[0] = if one_off[0] == b'0' { b'1' } else { b'0' };
    for bad in [
        String::from_utf8(one_off).unwrap(),
        token[1..].to_owned(),
        "x".repeat(64),
        String::new(),
    ] {
        let (stream, _) = h.raw();
        send_line(&stream, &hello(&bad, PROTOCOL_VERSION));
        let e = error(&read_reply(&stream).unwrap());
        assert_eq!(e["reason"], "auth_failed", "{bad}");
        // 認証に通らなかった接続では、依頼を受けない（閉じられる）
        assert!(read_reply(&stream).is_none());
    }

    // 古いトークン（ファイルを書き換えたのと同じ）でクライアントが認証に失敗する
    let tokens = yunagi_api::token::read(dir).unwrap();
    let stale = yunagi_api::token::ConnectionInfo {
        token: yunagi_api::token::generate(),
        pipe_name: None,
    };
    std::fs::write(token_path(dir), format!("{}\n", stale.token)).unwrap();
    match client::connect(dir, Some("claude-code"), CLIENT_CONNECT_TIMEOUT) {
        Err(HelloError::AuthFailed) => {}
        other => panic!("{:?}", other.err()),
    }
    std::fs::write(token_path(dir), format!("{}\n", tokens.token)).unwrap();
    assert!(client::connect(dir, Some("claude-code"), CLIENT_CONNECT_TIMEOUT).is_ok());

    // 識別子の形が誤っている
    let (stream, token) = h.raw();
    send_line(
        &stream,
        &json!({"jsonrpc": "2.0", "id": 0, "method": "hello",
                "params": {"protocol": PROTOCOL_VERSION, "token": token, "client": "Bad Name!"}}),
    );
    assert_eq!(
        read_reply(&stream).unwrap()["error"]["code"],
        wire::INVALID_PARAMS
    );

    // hello より先に依頼を送る
    let (stream, _) = h.raw();
    send_line(
        &stream,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "get_board", "params": {}}),
    );
    assert_eq!(
        read_reply(&stream).unwrap()["error"]["code"],
        wire::INVALID_REQUEST
    );

    // 大きすぎる行は読まずに断る。壊れた行は PARSE_ERROR
    let (stream, _) = h.raw();
    let big = "a".repeat(yunagi_api::MAX_HELLO_LINE + 10);
    let _ = (&stream).write_all(big.as_bytes());
    let _ = (&stream).write_all(b"\n");
    assert_eq!(
        read_reply(&stream).unwrap()["error"]["code"],
        wire::INVALID_REQUEST
    );
    let (stream, _) = h.raw();
    (&stream).write_all(b"not json\n").unwrap();
    assert_eq!(
        read_reply(&stream).unwrap()["error"]["code"],
        wire::PARSE_ERROR
    );

    drop(h);

    // hello を送らずに黙っている接続は、時間切れで閉じられる（待ち時間は、待ち受けを始めるときに読む）
    let h = Harness::unopened(dir);
    h.core.set_api_options(ApiOptions {
        hello_timeout: Duration::from_millis(100),
        request_timeout: Duration::from_millis(100),
        ..ApiOptions::default()
    });
    h.core.open();
    let (stream, _) = h.raw();
    assert!(read_reply(&stream).is_none());

    // 1 バイトずつ送って時間切れを延ばし続けても、1 行の合計が期限を超えれば閉じられる
    let (stream, _) = h.raw();
    let started = std::time::Instant::now();
    for _ in 0..30 {
        if (&stream).write_all(b"a").is_err() {
            break;
        }
        thread::sleep(Duration::from_millis(30));
    }
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the trickling connection was kept open"
    );
    assert!(read_reply(&stream).is_none());

    // `hello` の後で依頼を送らずに黙っている接続は、何も反映していないことを伝える `busy` を
    // 返されて閉じられる（後から依頼を送った側が、反映されたか分からない `result_unknown` と取り違えない）
    let (stream, token) = h.raw();
    send_line(&stream, &hello(&token, PROTOCOL_VERSION));
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.contains("\"result\""), "{line}");
    line.clear();
    reader.read_line(&mut line).unwrap();
    let busy: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(busy["error"]["data"]["error"]["code"], "busy");
    assert_eq!(busy["id"], Value::Null);
}

/// 権限はコアが確かめる：登録されたばかりの識別子は `read` だけ（01 4.6、05 2章）。
fn reads_need_the_read_scope_and_writes_need_theirs(dir: &DefaultDir) {
    let h = Harness::opened(dir);
    let date = "2031-02-03";

    // 初めての識別子の最初の依頼は、読み取りでも、設定画面のエージェントの一覧が変わるので `changed`
    // を出す。2 回目からは出さない
    h.ok("fresh-agent", "get_board", json!({"date": date}));
    assert_eq!(h.next_changed(), "agent");
    h.ok("fresh-agent", "get_board", json!({"date": date}));
    h.assert_no_changed();

    // 識別子なし（unknown）と初めての識別子は read だけ
    let board = h.call(None, "get_board", json!({"date": date})).unwrap();
    assert_eq!(board["date"], date);
    assert_eq!(board["todos"], json!([]));
    assert_eq!(board["note"], json!({"headings": [], "length": 0}));
    let denied = h
        .call(None, "add_todo", json!({"date": date, "text": "x"}))
        .unwrap_err();
    assert_eq!(denied["code"], "permission_denied");
    assert_eq!(denied["required_scope"], "todo");
    let denied = h.err(
        "claude-code",
        "add_blocker",
        json!({"date": date, "text": "x"}),
    );
    assert_eq!(denied["required_scope"], "blocker");
    assert_eq!(
        h.err(
            "claude-code",
            "append_note",
            json!({"date": date, "text": "x"})
        )["required_scope"],
        "append"
    );
    assert_eq!(
        h.err(
            "claude-code",
            "edit_note",
            json!({"date": date, "line": 1, "expected_text": "", "delete": true})
        )["required_scope"],
        "note_edit"
    );
    h.assert_no_changed();

    // read を外された識別子は、読み取りも断られる
    h.core
        .read(|conn| -> Result<(), StorageError> {
            let c = agent_client::get_by_name(conn, "claude-code")?.unwrap();
            agent_client::update_scopes(conn, c.id, &[])?;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        h.err("claude-code", "get_board", json!({}))["required_scope"],
        "read"
    );
    assert_eq!(
        h.err("claude-code", "get_note", json!({}))["code"],
        "permission_denied"
    );

    // 登録した識別子と最後の接続
    let seen = h
        .core
        .read(|conn| agent_client::get_by_name(conn, "claude-code"))
        .unwrap()
        .unwrap();
    assert!(seen.last_seen_at.is_some());
}

/// 書き込みは `AppCore::api_write`（`agent_write` の API 用）を通り、画面と同じ `changed`（`by: agent`）を
/// 出す。何も変えなかった書き込みでは出さない。
fn writes_go_through_agent_write_and_notify_changed(dir: &DefaultDir) {
    let h = Harness::opened(dir);
    let date = "2031-03-04";
    h.grant(
        "claude-code",
        &[Scope::Todo, Scope::Blocker, Scope::Append, Scope::NoteEdit],
    );
    h.drain();

    let todo = h.ok(
        "claude-code",
        "add_todo",
        json!({"date": date, "text": "  API を作る  "}),
    );
    assert_eq!(todo["text"], "API を作る");
    assert_eq!(h.next_changed(), "agent");
    let todo_id = todo["id"].as_str().unwrap().to_owned();
    let blocker = h.ok(
        "claude-code",
        "add_blocker",
        json!({"date": date, "text": "権限待ち"}),
    );
    assert_eq!(h.next_changed(), "agent");
    let blocker_id = blocker["id"].as_str().unwrap().to_owned();

    let board = h.ok("claude-code", "get_board", json!({"date": date}));
    assert_eq!(ids(&board["todos"]), std::slice::from_ref(&todo_id));
    assert_eq!(board["todos"][0]["origin"], "agent");
    assert_eq!(board["todos"][0]["status"], "open");
    assert_eq!(board["todos"][0]["close_reason"], Value::Null);
    assert_eq!(board["blockers"][0]["resolution_note"], Value::Null);
    assert!(board["blockers"][0].get("close_reason").is_none());

    // 本文の編集：同じ本文は何も変えない成功（changed を出さない）
    let edited = h.ok(
        "claude-code",
        "edit_todo",
        json!({"id": todo_id, "text": "API を作る"}),
    );
    assert_eq!(edited["updated"], false);
    h.assert_no_changed();
    let edited = h.ok(
        "claude-code",
        "edit_todo",
        json!({"id": todo_id, "text": "ローカル API を作る"}),
    );
    assert_eq!(edited["updated"], true);
    assert_eq!(h.next_changed(), "agent");

    // 種類の違う ID と、完了・取り消し
    let wrong = h.err(
        "claude-code",
        "edit_todo",
        json!({"id": blocker_id, "text": "x"}),
    );
    assert_eq!(wrong["code"], "invalid_argument");
    assert_eq!(wrong["item_kind"], "blocker");
    assert_eq!(wrong["use_operation"], "edit_blocker");
    let done = h.ok("claude-code", "complete_todo", json!({"id": todo_id}));
    assert_eq!(done["updated"], true);
    assert_eq!(done["status"], "done");
    assert_eq!(h.next_changed(), "agent");
    let again = h.ok("claude-code", "complete_todo", json!({"id": todo_id}));
    assert_eq!(again["updated"], false);
    h.assert_no_changed();
    let reopened = h.ok("claude-code", "reopen_todo", json!({"id": todo_id}));
    assert_eq!(reopened["status"], "open");
    assert_eq!(h.next_changed(), "agent");

    // 障害の解消と解消メモ
    let resolved = h.ok(
        "claude-code",
        "resolve_blocker",
        json!({"id": blocker_id, "note": "付与された"}),
    );
    assert_eq!(resolved["updated"], true);
    assert_eq!(resolved["resolution_note"], "付与された");
    assert_eq!(h.next_changed(), "agent");
    let conflict = h.err(
        "claude-code",
        "resolve_blocker",
        json!({"id": blocker_id, "note": "別のメモ"}),
    );
    assert_eq!(conflict["code"], "conflict");
    assert_eq!(conflict["reason"], "note_exists");
    h.assert_no_changed();

    // ノートと振り返り
    let appended = h.ok(
        "claude-code",
        "append_note",
        json!({"date": date, "text": "議事", "heading": "定例"}),
    );
    assert_eq!(appended["updated"], true);
    assert_eq!(h.next_changed(), "agent");
    h.ok(
        "claude-code",
        "append_note",
        json!({"date": date, "text": "二行目"}),
    );
    assert_eq!(h.next_changed(), "agent");
    h.ok(
        "claude-code",
        "append_reflection",
        json!({"date": date, "text": "よかった"}),
    );
    assert_eq!(h.next_changed(), "agent");
    let board = h.ok("claude-code", "get_board", json!({"date": date}));
    assert_eq!(board["reflection"], "よかった");
    assert!(
        board["note"]["headings"][0]
            .as_str()
            .unwrap()
            .ends_with("定例")
    );
    let heading = board["note"]["headings"][0].as_str().unwrap().to_owned();

    let whole = h.ok("claude-code", "get_note", json!({"date": date}));
    let body = whole["body"].as_str().unwrap().to_owned();
    assert!(body.contains("議事") && body.contains("二行目"), "{body}");
    let part = h.ok(
        "claude-code",
        "get_note",
        json!({"date": date, "heading": heading}),
    );
    assert_eq!(part["rev"], whole["rev"]);
    assert_eq!(part["sections"][0]["heading"], heading);
    assert_eq!(part["sections"][0]["start_line"], 1);
    let missing = h.err(
        "claude-code",
        "get_note",
        json!({"date": date, "heading": "無い見出し"}),
    );
    assert_eq!(missing["code"], "not_found");

    // edit_note：行の本文が合わなければ changed、合えば書き換える
    let line_of = |text: &str| {
        body.split('\n')
            .position(|l| l == text)
            .map(|i| i + 1)
            .unwrap()
    };
    let n = line_of("二行目");
    let changed = h.err(
        "claude-code",
        "edit_note",
        json!({"date": date, "line": n, "expected_text": "違う", "replace": "x"}),
    );
    assert_eq!(changed["code"], "conflict");
    assert_eq!(changed["reason"], "changed");
    h.assert_no_changed();
    let edited = h.ok(
        "claude-code",
        "edit_note",
        json!({"date": date, "line": n, "expected_text": "二行目", "replace": "三行目"}),
    );
    assert_eq!(edited["updated"], true);
    assert_eq!(h.next_changed(), "agent");
    // 指定が 1 つでないと、形の誤り
    let bad = h.err(
        "claude-code",
        "edit_note",
        json!({"date": date, "line": n, "expected_text": "三行目"}),
    );
    assert_eq!(bad["rpc_code"], wire::INVALID_PARAMS);

    // 日付の指定
    assert_eq!(
        h.err("claude-code", "get_board", json!({"date": "2031-02-30"}))["code"],
        "invalid_argument"
    );
    let today = h.ok("claude-code", "open_board", json!({"date": "today"}));
    assert_eq!(today["date"], h.core.today().unwrap().to_string());
    assert!(today.get("note").is_none());
}

/// 画面が知らせた入力中の欄・守る行は、API の書き込みの判定に使われる（01 4.1）。保存の失敗中は
/// `storage_unavailable`。
fn the_screen_state_protects_what_the_user_is_editing(dir: &DefaultDir) {
    let h = Harness::opened(dir);
    let date = "2031-04-05";
    h.grant(
        "claude-code",
        &[Scope::Todo, Scope::NoteEdit, Scope::Append],
    );
    let todo = h.ok(
        "claude-code",
        "add_todo",
        json!({"date": date, "text": "書きかけ"}),
    );
    let id = todo["id"].as_str().unwrap().to_owned();
    h.ok(
        "claude-code",
        "append_note",
        json!({"date": date, "text": "守る行"}),
    );
    h.drain();

    let rev = h.ok("claude-code", "get_note", json!({"date": date}))["rev"]
        .as_i64()
        .unwrap();
    let report: UiReport = serde_json::from_value(json!({
        "session": "t", "seq": 1,
        "editingItems": [{"itemId": id, "field": "text"}],
        "editors": [{"doc": "note", "date": date, "rev": rev, "lines": [1]}],
    }))
    .unwrap();
    h.core.screen().report(report).unwrap();

    let busy_editing = h.err(
        "claude-code",
        "edit_todo",
        json!({"id": id, "text": "別の本文"}),
    );
    assert_eq!(busy_editing["code"], "conflict");
    assert_eq!(busy_editing["reason"], "user_editing");
    let line = h.err(
        "claude-code",
        "edit_note",
        json!({"date": date, "line": 1, "expected_text": "守る行", "replace": "x"}),
    );
    assert_eq!(line["reason"], "user_editing");
    // 状態の欄は入力中でも受け付ける
    assert_eq!(
        h.ok("claude-code", "complete_todo", json!({"id": id}))["updated"],
        true
    );
    assert_eq!(h.next_changed(), "agent");

    // 保存の失敗中は、書き込みを断る。読み取りは受け付ける
    h.core.screen().set_save_failing(true);
    assert_eq!(
        h.err(
            "claude-code",
            "add_todo",
            json!({"date": date, "text": "y"})
        )["code"],
        "storage_unavailable"
    );
    assert!(
        h.call(Some("claude-code"), "get_board", json!({"date": date}))
            .is_ok()
    );
    h.core.screen().set_save_failing(false);
}

/// 保存先の変更・復元の最中（データベースを使えない間）の依頼は、最大 `db_wait` 待って、何も反映せずに
/// `busy`。待った後に使えれば、そのまま処理する（01 4.4）。
fn requests_wait_at_most_the_limit_then_answer_busy(dir: &DefaultDir) {
    let h = Harness::opened(dir);
    h.grant("claude-code", &[Scope::Todo]);
    h.core.set_api_options(ApiOptions {
        db_wait: Duration::from_millis(150),
        ..ApiOptions::default()
    });
    let date = "2031-05-06";

    // データベースを 600 ミリ秒使い続ける作業
    let core = Arc::clone(&h.core);
    let holder = thread::spawn(move || {
        core.read(|_| -> Result<(), StorageError> {
            thread::sleep(Duration::from_millis(600));
            Ok(())
        })
        .unwrap();
    });
    thread::sleep(Duration::from_millis(100));
    let busy = h.err(
        "claude-code",
        "add_todo",
        json!({"date": date, "text": "z"}),
    );
    assert_eq!(busy["code"], "busy");
    assert!(
        busy["message"]
            .as_str()
            .unwrap()
            .contains("Nothing was changed")
    );
    h.assert_no_changed();
    holder.join().unwrap();
    // 何も書かれていない
    let board = h.ok("claude-code", "get_board", json!({"date": date}));
    assert_eq!(board["todos"], json!([]));

    // 待ちの上限の中で使えるようになれば処理される
    h.core.set_api_options(ApiOptions {
        db_wait: Duration::from_secs(5),
        ..ApiOptions::default()
    });
    let core = Arc::clone(&h.core);
    let holder = thread::spawn(move || {
        core.read(|_| -> Result<(), StorageError> {
            thread::sleep(Duration::from_millis(300));
            Ok(())
        })
        .unwrap();
    });
    thread::sleep(Duration::from_millis(50));
    let added = h.ok(
        "claude-code",
        "add_todo",
        json!({"date": date, "text": "w"}),
    );
    assert_eq!(added["text"], "w");
    holder.join().unwrap();
}

/// 止めるとき：新しい接続は受けず、依頼をまだ送っていない接続は閉じ、処理中の依頼は最後まで行って
/// 応答を返す。そのあとでソケットとトークンが消え、データベースが閉じる（01 4.4）。
fn stopping_finishes_running_requests_and_closes_idle_connections(dir: &DefaultDir) {
    let h = Harness::opened(dir);
    h.grant("claude-code", &[Scope::Todo]);
    let date = "2031-06-07";

    // hello を済ませて依頼を送らない接続と、hello もしない接続
    let (idle_after_hello, token) = h.raw();
    send_line(&idle_after_hello, &hello(&token, PROTOCOL_VERSION));
    let mut reader = BufReader::new(&idle_after_hello);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.contains("\"result\""), "{line}");
    let (idle, _) = h.raw();

    // 処理中の依頼：データベースを 400 ミリ秒使い続ける作業の後ろで待たせる
    let core = Arc::clone(&h.core);
    let holder = thread::spawn(move || {
        core.read(|_| -> Result<(), StorageError> {
            thread::sleep(Duration::from_millis(400));
            Ok(())
        })
        .unwrap();
    });
    thread::sleep(Duration::from_millis(50));
    let dir2 = dir.clone();
    let running = thread::spawn(move || {
        let session = client::connect(&dir2, Some("claude-code"), CLIENT_CONNECT_TIMEOUT).unwrap();
        session.call("add_todo", &json!({"date": date, "text": "最後まで"}))
    });
    thread::sleep(Duration::from_millis(100));

    // 止める（アプリの終了と同じ）。処理中の依頼が終わってから返る
    let core = Arc::clone(&h.core);
    let closer = thread::spawn(move || core.close());
    // 止め始めたら、ソケットのファイルはすぐ消え、新しいクライアントは「待ち受けが無い」と分かる
    let start = std::time::Instant::now();
    while socket_path(dir).exists() {
        assert!(start.elapsed() < WAIT, "the socket was not removed");
        thread::sleep(Duration::from_millis(10));
    }
    assert!(matches!(
        client::connect(dir, Some("claude-code"), CLIENT_CONNECT_TIMEOUT),
        Err(HelloError::NoListener)
    ));
    // `hello` もしていない接続は黙って閉じられる。`hello` の後で依頼を送っていない接続は、何も
    // 反映していないことを伝える `busy`（`id` は null）を返されて閉じられる（クライアントが後から
    // 依頼を送って、`result_unknown` と取り違えないため）
    assert!(read_reply(&idle).is_none());
    let mut busy = String::new();
    reader.read_line(&mut busy).unwrap();
    let busy: Value = serde_json::from_str(&busy).unwrap();
    assert_eq!(busy["id"], Value::Null);
    assert_eq!(busy["error"]["data"]["error"]["code"], "busy");
    let mut rest = String::new();
    assert_eq!(reader.read_line(&mut rest).unwrap(), 0);

    holder.join().unwrap();
    let result = running
        .join()
        .unwrap()
        .expect("the running request was lost");
    assert_eq!(result["text"], "最後まで");
    closer.join().unwrap();
    assert!(!token_path(dir).exists());
    assert_eq!(h.core.api_state(), ApiState::Stopped);
    // 反映されている（応答を返したあとで閉じた）
    drop(h);
    let h = Harness::opened(dir);
    let board = h.ok("claude-code", "get_board", json!({"date": date}));
    assert_eq!(board["todos"][0]["text"], "最後まで");
}

/// 依頼の形の誤りは、JSON-RPC の標準のコードで返す。
fn request_form_errors_use_the_standard_codes(dir: &DefaultDir) {
    let h = Harness::opened(dir);
    let code = |method: &str, params: Value| -> i64 {
        match h.call(Some("claude-code"), method, params) {
            Ok(v) => panic!("{method} succeeded: {v}"),
            Err(e) => e["rpc_code"].as_i64().unwrap_or(APP_ERROR),
        }
    };
    assert_eq!(code("nope", json!({})), wire::METHOD_NOT_FOUND);
    assert_eq!(
        code("get_board", json!({"datee": "today"})),
        wire::INVALID_PARAMS
    );
    assert_eq!(code("get_board", json!({"date": 5})), wire::INVALID_PARAMS);
    assert_eq!(code("add_todo", json!({})), wire::INVALID_PARAMS);
    assert_eq!(code("complete_todo", json!({"id": "not an id"})), APP_ERROR);
    // 配列のまとめ依頼と、通知（id が無い）
    let (stream, token) = h.raw();
    send_line(&stream, &hello(&token, PROTOCOL_VERSION));
    assert!(read_reply(&stream).unwrap().get("result").is_some());
    send_line(
        &stream,
        &json!([{"jsonrpc": "2.0", "id": 1, "method": "get_board"}]),
    );
    assert_eq!(
        read_reply(&stream).unwrap()["error"]["code"],
        wire::INVALID_REQUEST
    );
    let (stream, token) = h.raw();
    send_line(&stream, &hello(&token, PROTOCOL_VERSION));
    read_reply(&stream).unwrap();
    send_line(
        &stream,
        &json!({"jsonrpc": "2.0", "method": "get_board", "params": {}}),
    );
    assert_eq!(
        read_reply(&stream).unwrap()["error"]["code"],
        wire::INVALID_REQUEST
    );
    let _ = Ulid::generate();
}
