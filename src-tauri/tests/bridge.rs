//! 画面とコアの結合テスト：Tauri コマンドを IPC（画面からの呼び出しと同じ経路）で呼び、結果と
//! イベントを確かめる。WebView は開かない（`tauri::test` の模擬の実行環境）。
//!
//! 既定ディレクトリ（`yunagi_core::storage::data_dir::default_dir`）は環境変数から求まるので、
//! はじめに一時フォルダを指すよう設定する。環境変数の設定はほかのスレッドがあると安全でないため、
//! テストの仕組み（harness）を使わず、`main` から1つずつ順に走らせる（Cargo.toml の `[[test]]`）。

// コアのエラー（`ItemError`）をそのまま受け取って確かめる。大きさはテストでは問題にならない
#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{
    INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets,
};
use tauri::webview::InvokeRequest;
use tauri::{App, Listener, Manager, WebviewWindow, WebviewWindowBuilder};
use yunagi_app::AppCore;
use yunagi_core::domain::agent::{ItemField, ProtectedLines, UiState};
use yunagi_core::domain::item::{self, ItemError};
use yunagi_core::domain::logical_date::LogicalDate;
use yunagi_core::domain::note_body::{LineEdit, LineOp};
use yunagi_core::domain::text_doc::{self, TextDoc, TextDocError};
use yunagi_core::storage::table::agent_client::{self, AgentClient, Scope};
use yunagi_core::storage::table::change_log;
use yunagi_core::storage::{StorageError, Ulid};

/// 待つ時間の上限（イベントとバックアップ）。
const WAIT: Duration = Duration::from_secs(10);

fn main() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_path_buf();
    // SAFETY: まだほかのスレッドを作っていない（テストはこの後、このスレッドで順に走らせる）
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_DATA_HOME", &home);
        std::env::set_var("LOCALAPPDATA", &home);
    }
    let default_dir = yunagi_core::storage::data_dir::default_dir().unwrap();
    assert!(default_dir.path().starts_with(&home));
    let tests: [(&str, Test); 24] = [
        (
            "opens_on_first_use_and_backs_up",
            opens_on_first_use_and_backs_up,
        ),
        ("item_commands", item_commands),
        ("text_doc_commands", text_doc_commands),
        ("note_send_commands", note_send_commands),
        ("carryover_commands", carryover_commands),
        ("search_command", search_command),
        ("copy_handoff_command", copy_handoff_command),
        ("overview_counts", overview_counts),
        ("archive_command", archive_command),
        ("agent_admin_commands", agent_admin_commands),
        (
            "export_markdown_writes_one_file_per_day",
            export_markdown_writes_one_file_per_day,
        ),
        (
            "agent_writes_use_the_same_events_and_the_screen_state",
            agent_writes_use_the_same_events_and_the_screen_state,
        ),
        (
            "the_same_state_from_screen_and_agent_becomes_the_users_change",
            the_same_state_from_screen_and_agent_becomes_the_users_change,
        ),
        (
            "edit_note_refuses_the_lines_the_screen_protects",
            edit_note_refuses_the_lines_the_screen_protects,
        ),
        (
            "save_failures_refuse_agent_writes_until_a_save_succeeds",
            save_failures_refuse_agent_writes_until_a_save_succeeds,
        ),
        (
            "logical_date_changes_notify_and_back_up",
            logical_date_changes_notify_and_back_up,
        ),
        (
            "closing_before_the_open_finishes_keeps_it_closed",
            closing_before_the_open_finishes_keeps_it_closed,
        ),
        (
            "backup_failures_are_reported_once_and_retried",
            backup_failures_are_reported_once_and_retried,
        ),
        (
            "missing_database_is_reported_and_can_be_created",
            missing_database_is_reported_and_can_be_created,
        ),
        (
            "a_damaged_database_can_be_replaced_from_a_backup",
            a_damaged_database_can_be_replaced_from_a_backup,
        ),
        (
            "a_missing_database_can_be_retried_or_pointed_elsewhere",
            a_missing_database_can_be_retried_or_pointed_elsewhere,
        ),
        (
            "a_newer_database_is_reported_without_touching_it",
            a_newer_database_is_reported_without_touching_it,
        ),
        ("settings_commands", settings_commands),
        (
            "data_dir_operations_keep_the_screen_ready",
            data_dir_operations_keep_the_screen_ready,
        ),
    ];
    // `cargo test <名前>` の名前（`-` で始まらない引数）があれば、名前にそれを含むものだけを走らせる
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
        test(default_dir.path());
        println!("ok");
        passed += 1;
    }
    println!("\ntest result: ok. {passed} passed");
}

// ---- 道具 ----

/// 1つのテスト。既定ディレクトリを受け取る。
type Test = fn(&Path);

/// 模擬のアプリ。コアはまだデータベースを開いていない。
struct Harness {
    _app: App<MockRuntime>,
    webview: WebviewWindow<MockRuntime>,
    core: Arc<AppCore>,
    events: Receiver<(&'static str, Value)>,
}

impl Harness {
    fn new() -> Self {
        let app = mock_builder()
            .invoke_handler(yunagi_app::invoke_handler())
            .build(mock_context(noop_assets()))
            .unwrap();
        let core = yunagi_app::manage_core(app.handle());
        let (tx, events) = mpsc::channel();
        for name in [
            yunagi_app::EVENT_STARTUP,
            yunagi_app::EVENT_CHANGED,
            yunagi_app::EVENT_TODAY_CHANGED,
            yunagi_app::EVENT_BACKUP_FAILED,
        ] {
            let tx = tx.clone();
            app.listen_any(name, move |e| {
                let _ = tx.send((name, serde_json::from_str(e.payload()).unwrap()));
            });
        }
        let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        Self {
            _app: app,
            webview,
            core,
            events,
        }
    }

    /// データベースを開いた模擬のアプリ（開いたときの知らせとバックアップは済ませる）。
    fn opened() -> Self {
        let h = Self::new();
        h.core.open();
        assert_eq!(h.next_event(), ("startup", json!({ "state": "ready" })));
        h.core.wait_for_backup();
        h.drain();
        h
    }

    /// 画面からの呼び出しと同じ経路でコマンドを呼ぶ。
    fn invoke(&self, cmd: &str, args: Value) -> Result<Value, Value> {
        let url = if cfg!(windows) {
            "http://tauri.localhost"
        } else {
            "tauri://localhost"
        };
        get_ipc_response(
            &self.webview,
            InvokeRequest {
                cmd: cmd.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: url.parse().unwrap(),
                body: InvokeBody::Json(args),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.into(),
            },
        )
        .map(|body| body.deserialize::<Value>().unwrap())
    }

    fn ok(&self, cmd: &str, args: Value) -> Value {
        self.invoke(cmd, args)
            .unwrap_or_else(|e| panic!("{cmd} failed: {e}"))
    }

    fn err(&self, cmd: &str, args: Value) -> Value {
        match self.invoke(cmd, args) {
            Ok(v) => panic!("{cmd} succeeded: {v}"),
            Err(e) => e,
        }
    }

    fn next_event(&self) -> (&'static str, Value) {
        self.events.recv_timeout(WAIT).expect("no event")
    }

    /// 届いているイベントを捨てる。
    fn drain(&self) {
        while self.events.try_recv().is_ok() {}
    }

    /// まだ届いていないイベントが無いこと。
    fn assert_no_event(&self) {
        if let Ok(e) = self.events.recv_timeout(Duration::from_millis(100)) {
            panic!("unexpected event: {e:?}");
        }
    }

    fn today(&self) -> String {
        self.ok("today", json!({})).as_str().unwrap().to_owned()
    }

    fn add_todo(&self, date: &str, text: &str) -> Value {
        self.ok(
            "add_item",
            json!({ "date": date, "kind": "todo", "text": text, "position": { "at": "last" } }),
        )
    }

    /// エージェントを登録し、`scopes` を許可する。
    fn register_agent(&self, name: &str, scopes: Vec<Scope>) -> Ulid {
        let id = Ulid::generate();
        let client = AgentClient {
            id,
            name: name.into(),
            scopes,
            created_at: yunagi_core::storage::table::now(),
            last_seen_at: None,
        };
        self.core
            .read(|conn| agent_client::insert(conn, &client))
            .unwrap();
        id
    }
}

impl Drop for Harness {
    /// 次のテストが同じ既定ディレクトリのロックを取れるよう、閉じてロックを離す。
    fn drop(&mut self) {
        self.core.wait_for_backup();
        self.core.close();
    }
}

/// ボードの一覧（`todos`・`blockers`）の本文。
fn texts(list: &Value) -> Vec<String> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|bi| bi["item"]["text"].as_str().unwrap().to_owned())
        .collect()
}

fn changed_by(by: &str) -> (&'static str, Value) {
    ("changed", json!({ "by": by }))
}

fn backups(default_dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(default_dir.join("backups"))
        .map(|rd| {
            rd.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

// ---- テスト ----

/// 初めて使うとき：開くまでは `opening` で、コマンドは `not_ready`。開くと `startup` が届き、
/// その日の定期バックアップができる（アーキテクチャ設計書 6.2・6.3）。
fn opens_on_first_use_and_backs_up(default_dir: &Path) {
    let h = Harness::new();
    assert_eq!(
        h.ok("startup_status", json!({})),
        json!({ "state": "opening" })
    );
    assert_eq!(
        h.err("get_board", json!({ "date": "2026-11-03" })),
        json!({ "kind": "not_ready" })
    );
    // 画面からは、データベースが見つからないとき以外は作れない
    assert_eq!(
        h.err("create_database", json!({})),
        json!({ "kind": "invalid_state" })
    );

    h.core.open();
    assert_eq!(h.next_event(), ("startup", json!({ "state": "ready" })));
    assert_eq!(
        h.ok("startup_status", json!({})),
        json!({ "state": "ready" })
    );
    assert!(default_dir.join("yunagi.db").is_file());

    h.core.wait_for_backup();
    let today = h.today();
    let daily: Vec<_> = backups(default_dir)
        .into_iter()
        .filter(|n| n.starts_with(&format!("daily-{today}-")))
        .collect();
    assert_eq!(daily.len(), 1, "{daily:?}");
    h.assert_no_event();

    // 同じ日にもう一度確かめても、作り直さない
    h.core.check_today(yunagi_core::storage::table::now());
    h.core.wait_for_backup();
    assert_eq!(
        backups(default_dir)
            .iter()
            .filter(|n| n.starts_with("daily-"))
            .count(),
        1
    );
}

/// 項目の操作：結果の形、変えたときだけ届く `changed`、エラーの形。
fn item_commands(_: &Path) {
    let h = Harness::opened();
    let date = "2030-01-07";

    let a = h.add_todo(date, "  設計を読む ");
    assert_eq!(a["text"], "設計を読む");
    assert_eq!(a["kind"], "todo");
    assert_eq!(a["status"], "open");
    assert_eq!(a["origin"], "human");
    assert_eq!(a["date"], date);
    assert!(a["createdAt"].is_i64());
    assert_eq!(h.next_event(), changed_by("screen"));
    let a_id = a["id"].as_str().unwrap().to_owned();

    // 同じ本文なら何も変えず、知らせない
    let same = h.ok(
        "save_item",
        json!({ "id": a_id, "fields": { "text": "設計を読む" } }),
    );
    assert_eq!(same["updated"], false);
    h.assert_no_event();

    let edited = h.ok(
        "save_item",
        json!({ "id": a_id, "fields": { "text": "設計書を読む" } }),
    );
    assert_eq!(edited["updated"], true);
    assert_eq!(edited["item"]["text"], "設計書を読む");
    assert_eq!(h.next_event(), changed_by("screen"));

    // 完了時刻は画面が決めた時刻（ミリ秒）のまま
    let closed_at = 1_893_888_000_123_i64;
    let status = |before: &str, closed: Value| {
        json!({ "id": a_id, "fields": { "status": {
            "before": before, "closedAt": closed, "changedAt": closed_at,
        } } })
    };
    let done = h.ok("save_item", status("open", json!(closed_at)));
    assert_eq!(done["item"]["status"], "done");
    assert_eq!(done["item"]["completedAt"], closed_at);
    let reopened = h.ok("save_item", status("done", Value::Null));
    assert_eq!(reopened["item"]["status"], "open");
    assert_eq!(reopened["item"]["completedAt"], Value::Null);
    h.drain();

    // 障害を先頭に足して、解消メモを書く。TODO に解消メモは書けない
    let b = h.ok(
        "add_item",
        json!({ "date": date, "kind": "blocker", "text": "CI が落ちる", "position": { "at": "first" } }),
    );
    let b_id = b["id"].as_str().unwrap().to_owned();
    let noted = h.ok(
        "save_item",
        json!({ "id": b_id, "fields": { "resolutionNote": "キャッシュを消した" } }),
    );
    assert_eq!(noted["item"]["resolutionNote"], "キャッシュを消した");
    assert_eq!(
        h.err(
            "save_item",
            json!({ "id": a_id, "fields": { "resolutionNote": "x" } })
        ),
        json!({ "kind": "invalid_argument", "reason": "wrong_kind" })
    );

    // 分けて、動かす
    let split = h.ok(
        "split_item",
        json!({ "id": a_id, "first": "設計書", "second": "を読む" }),
    );
    assert_eq!(split["original"]["text"], "設計書");
    assert_eq!(split["created"]["text"], "を読む");
    let c_id = split["created"]["id"].as_str().unwrap().to_owned();
    let moved = h.ok(
        "move_item",
        json!({ "id": c_id, "position": { "at": "before", "id": a_id } }),
    );
    assert_eq!(moved["updated"], true);

    let board = h.ok("get_board", json!({ "date": date }));
    assert_eq!(texts(&board["todos"]), ["を読む", "設計書"]);
    assert_eq!(texts(&board["blockers"]), ["CI が落ちる"]);
    assert_eq!(board["todos"][0]["carriedTo"], Value::Null);
    assert_eq!(board["note"], json!({ "headings": [], "length": 0 }));

    // 削除して、取り消す
    h.drain();
    let deleted = h.ok("delete_item", json!({ "id": c_id }));
    assert_eq!(deleted, json!({ "id": c_id, "source": null }));
    assert_eq!(h.next_event(), changed_by("screen"));
    assert_eq!(
        texts(&h.ok("get_board", json!({ "date": date }))["todos"]),
        ["設計書"]
    );
    let restored = h.ok("restore_item", json!({ "deleted": deleted }));
    assert_eq!(restored["deletedAt"], Value::Null);
    assert_eq!(h.next_event(), changed_by("screen"));
    assert_eq!(
        h.err("restore_item", json!({ "deleted": deleted })),
        json!({ "kind": "not_found" })
    );

    // 誤り：空の本文、無い項目、自分の前に動かす
    assert_eq!(
        h.err(
            "add_item",
            json!({ "date": date, "kind": "todo", "text": "  ", "position": { "at": "last" } })
        ),
        json!({ "kind": "invalid_argument", "reason": "empty_text" })
    );
    assert_eq!(
        h.err(
            "save_item",
            json!({ "id": Ulid::generate().to_string(), "fields": { "text": "x" } })
        ),
        json!({ "kind": "not_found" })
    );
    assert_eq!(
        h.err(
            "move_item",
            json!({ "id": a_id, "position": { "at": "after", "id": a_id } })
        ),
        json!({ "kind": "invalid_argument", "reason": "invalid_position" })
    );
    h.assert_no_event();
    // 形の誤り（ID でない文字列）は、Tauri が引数を読めずに断る（文字列のエラー）
    assert!(
        h.err("save_item", json!({ "id": "x", "fields": { "text": "x" } }))
            .is_string()
    );
}

/// ノート・振り返り：版の番号での保存と、行き違いの最新の返し方（アーキテクチャ設計書 4.1）。
fn text_doc_commands(_: &Path) {
    let h = Harness::opened();
    let date = "2030-01-08";
    for doc in ["note", "reflection"] {
        assert_eq!(
            h.ok("get_text_doc", json!({ "doc": doc, "date": date })),
            json!({ "body": "", "rev": 0 })
        );
        let saved = h.ok(
            "save_text_doc",
            json!({ "doc": doc, "date": date, "body": "## 10:00 定例\nメモ", "baseRev": 0 }),
        );
        assert_eq!(
            saved,
            json!({ "result": "saved", "rev": 1, "updated": true })
        );
        assert_eq!(h.next_event(), changed_by("screen"));
        // もとにした版が古い
        let stale = h.ok(
            "save_text_doc",
            json!({ "doc": doc, "date": date, "body": "別の本文", "baseRev": 0 }),
        );
        assert_eq!(
            stale,
            json!({ "result": "stale", "body": "## 10:00 定例\nメモ", "rev": 1 })
        );
        // 同じ本文は書かない
        let same = h.ok(
            "save_text_doc",
            json!({ "doc": doc, "date": date, "body": "## 10:00 定例\nメモ", "baseRev": 1 }),
        );
        assert_eq!(
            same,
            json!({ "result": "saved", "rev": 1, "updated": false })
        );
        h.assert_no_event();
    }
    let board = h.ok("get_board", json!({ "date": date }));
    assert_eq!(board["reflection"], "## 10:00 定例\nメモ");
    assert_eq!(board["note"]["length"], 14);
    assert_eq!(board["note"]["headings"][0]["text"], "10:00 定例");
    assert_eq!(board["note"]["headings"][0]["line"], 1);
}

/// 行送り：本文を保存して項目を作り、印と送り元の行を返す（データモデル設計書 3章 note_send）。
fn note_send_commands(_: &Path) {
    let h = Harness::opened();
    // データベースはテストの間で共有するので、ほかのテストと違う日を使う
    let date = "2030-03-02";
    let body = "## 10:00 定例\n- レート制限を調べる\n- 権限待ち\n```\nログ\n```";
    h.drain();
    let sent = h.ok(
        "send_note_lines",
        json!({ "date": date, "body": body, "baseRev": 0, "from": 1, "to": 5, "kind": "todo" }),
    );
    assert_eq!(sent["result"], "sent");
    assert_eq!(sent["rev"], 1);
    let lines: Vec<u64> = sent["sent"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["line"].as_u64().unwrap())
        .collect();
    assert_eq!(lines, [2, 3]);
    assert_eq!(sent["sent"][0]["item"]["text"], "レート制限を調べる");
    assert_eq!(sent["sent"][0]["item"]["origin"], "note");
    assert_eq!(
        sent["skipped"],
        json!([
            { "line": 1, "reason": "heading" },
            { "line": 4, "reason": "no_text" },
            { "line": 5, "reason": "in_code" },
        ])
    );
    assert_eq!(h.next_event(), changed_by("screen"));
    let first = sent["sent"][0]["item"]["id"].clone();

    // 送った行は送り直さない（本文も同じなので何も変えない）
    let again = h.ok(
        "send_note_lines",
        json!({ "date": date, "body": body, "baseRev": 1, "from": 2, "to": 2, "kind": "blocker" }),
    );
    assert_eq!(
        again,
        json!({ "result": "sent", "rev": 1, "sent": [], "skipped": [{ "line": 2, "reason": "already_sent" }] })
    );
    h.assert_no_event();

    // 印は、エディタの本文（まだ保存していない行を含む）で照合する
    let edited = format!("上に足した\n{body}");
    assert_eq!(
        h.ok("note_marks", json!({ "date": date, "body": edited })),
        json!([
            { "line": 3, "kind": "todo", "itemId": first },
            { "line": 4, "kind": "todo", "itemId": sent["sent"][1]["item"]["id"] },
        ])
    );

    // ボードの項目に、送り元の行を添える
    let board = h.ok("get_board", json!({ "date": date }));
    assert_eq!(
        board["todos"][0]["noteSource"],
        json!({ "date": date, "line": 2, "itemId": first })
    );

    // もとにした版が古い・行の番号の誤り
    assert_eq!(
        h.ok(
            "send_note_lines",
            json!({ "date": date, "body": "別", "baseRev": 0, "from": 1, "to": 1, "kind": "todo" }),
        ),
        json!({ "result": "stale", "body": body, "rev": 1 })
    );
    assert_eq!(
        h.err(
            "send_note_lines",
            json!({ "date": date, "body": body, "baseRev": 1, "from": 0, "to": 1, "kind": "todo" }),
        ),
        json!({ "kind": "invalid_argument", "reason": "note_line" })
    );
    h.assert_no_event();
}

/// 検索パレット（S-04）の検索：全日付を新しい順に探し、種類・日付・本文全体を返す（データモデル設計書
/// 4.1）。件数の上限を超える値は上限までにする。
fn search_command(_: &Path) {
    let h = Harness::opened();
    // ほかのテストと重ならない語と日を使う
    let old = "2030-04-01";
    let new = "2030-04-02";
    let todo = h.add_todo(old, "うなぎ検索の TODO");
    h.ok(
        "save_text_doc",
        json!({ "doc": "note", "date": new, "body": "朝\nうなぎ検索のノート", "baseRev": 0 }),
    );
    h.drain();

    let hits = h.ok("search", json!({ "text": "  うなぎ検索  ", "limit": 10 }));
    let hits = hits.as_array().unwrap();
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert_eq!(hits[0]["kind"], "note");
    assert_eq!(hits[0]["date"], new);
    assert_eq!(hits[0]["refId"], new);
    assert_eq!(hits[0]["body"], "朝\nうなぎ検索のノート");
    assert_eq!(hits[0]["carriedTo"], Value::Null);
    assert_eq!(hits[1]["kind"], "todo");
    assert_eq!(hits[1]["refId"], todo["id"]);
    assert_eq!(hits[1]["date"], old);
    assert_eq!(hits[1]["resolutionNote"], Value::Null);
    // 持ち越し先のたどり方はコア（storage::search）の単体テストで確かめる。ここは渡す形だけ
    assert_eq!(hits[1]["carriedTo"], Value::Null);

    // 件数の上限と、空の検索語
    assert_eq!(
        h.ok("search", json!({ "text": "うなぎ検索", "limit": 1 }))
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        h.ok("search", json!({ "text": "   ", "limit": 10 })),
        json!([])
    );
    assert_eq!(
        h.ok("search", json!({ "text": "うなぎ検索", "limit": u32::MAX }))
            .as_array()
            .unwrap()
            .len(),
        2
    );
    h.assert_no_event();
}

/// 依頼文の書き出し（Mod+Shift+A）：選んだ TODO の依頼文を作ってクリップボードに書く。権限は確かめず、
/// 何も変えない（`changed` を出さない）。書けなければ `clipboard_failed`、項目が無ければ `not_found`、
/// 障害の ID なら `invalid_argument`（`wrong_kind`）。
fn copy_handoff_command(_: &Path) {
    struct Capture(std::sync::Mutex<Vec<String>>);
    impl yunagi_app::clipboard::Writer for Capture {
        fn write(&self, text: &str) -> Result<(), String> {
            self.0.lock().unwrap().push(text.to_owned());
            Ok(())
        }
    }

    let h = Harness::opened();
    let day = "2030-05-01";
    let todo = h.add_todo(day, "依頼する TODO");
    let blocker = h.ok(
        "add_item",
        json!({ "date": day, "kind": "blocker", "text": "待ち", "position": { "at": "last" } }),
    );
    h.drain();
    let copy = |id: &Value| h.invoke("copy_handoff", json!({ "id": id }));

    // 書き込む先が無い（プラグインを入れていない）あいだは、依頼文を作っても書けない
    assert_eq!(copy(&todo["id"]).unwrap_err()["kind"], "clipboard_failed");

    let capture = Arc::new(Capture(Default::default()));
    h._app
        .handle()
        .state::<Arc<yunagi_app::clipboard::HandoffClipboard>>()
        .attach(capture.clone());
    assert_eq!(copy(&todo["id"]).unwrap(), Value::Null);
    let written = capture.0.lock().unwrap().clone();
    assert_eq!(
        written,
        [format!(
            "## Task\n依頼する TODO\n\n(Exported from Yunagi on {})\n",
            h.today()
        )]
    );
    // 障害の ID と無い ID は、何も書かない
    assert_eq!(
        copy(&blocker["id"]).unwrap_err(),
        json!({ "kind": "invalid_argument", "reason": "wrong_kind" })
    );
    assert_eq!(
        copy(&json!(Ulid::generate().to_string())).unwrap_err(),
        json!({ "kind": "not_found" })
    );
    assert_eq!(capture.0.lock().unwrap().len(), 1);
    h.assert_no_event();
}

/// 持ち越しの画面（S-03）のコマンド：状態、一覧、後回しと1回だけの再表示、見直しを含む確定と、
/// 確定の断り。
fn carryover_commands(_: &Path) {
    let h = Harness::opened();
    let today: LogicalDate = h.today().parse().unwrap();
    let yesterday = today.yesterday().unwrap().to_string();
    let today = today.to_string();
    // 前回の日（昨日）に、見直しの回数に達した TODO・ふつうの TODO・障害を置く
    let reviewed = h.add_todo(&yesterday, "3回持ち越した");
    h.core
        .read(|conn| {
            conn.execute(
                "UPDATE item SET carry_count = 3 WHERE id = ?1",
                [reviewed["id"].as_str().unwrap()],
            )
            .map_err(yunagi_core::storage::StorageError::from)
        })
        .unwrap();
    let plain = h.add_todo(&yesterday, "ふつう");
    let blocker = h.ok(
        "add_item",
        json!({ "date": yesterday, "kind": "blocker", "text": "権限待ち", "position": { "at": "last" } }),
    );
    let not_chosen = h.add_todo(&yesterday, "選ばない");
    h.drain();

    assert_eq!(
        h.ok("carryover_status", json!({})),
        json!({ "from": yesterday, "prompt": "show" })
    );
    let list = h.ok("carryover_candidates", json!({}));
    assert_eq!(list["from"], yesterday);
    assert_eq!(list["reviewThreshold"], 3);
    let shown: Vec<(&str, bool)> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["item"]["text"].as_str().unwrap(),
                c["review"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            ("3回持ち越した", true),
            ("ふつう", false),
            ("権限待ち", false),
            ("選ばない", false),
        ]
    );

    // 後回し（Esc）の後は、次に前面に出したとき1回だけ再表示し、以後はパレットから開く。
    // 後回しと、再表示を記録したときは知らせる（ほかの窓が持ち越しの状態を読み直す）
    h.ok("postpone_carryover", json!({}));
    assert_eq!(h.next_event(), changed_by("screen"));
    assert_eq!(
        h.ok("carryover_status", json!({}))["prompt"],
        "reshow_on_focus"
    );
    assert_eq!(h.ok("mark_carryover_reshown", json!({})), json!(true));
    assert_eq!(h.next_event(), changed_by("screen"));
    // 記録済みなら書かず、知らせない
    assert_eq!(h.ok("mark_carryover_reshown", json!({})), json!(false));
    h.assert_no_event();
    assert_eq!(h.ok("carryover_status", json!({}))["prompt"], "on_request");
    h.ok("postpone_carryover", json!({}));
    assert_eq!(h.next_event(), changed_by("screen"));
    assert_eq!(h.ok("carryover_status", json!({}))["prompt"], "on_request");
    h.assert_no_event();

    // 見直しの誤り（障害への見直し）と、画面に出していた前回の日が違うときは、何も書かずに断る
    assert_eq!(
        h.err(
            "confirm_carryover",
            json!({ "decision": {
                "from": yesterday,
                "selection": { "select": "all" },
                "reviews": [{ "id": blocker["id"], "review": { "review": "drop" } }],
            } }),
        ),
        json!({ "kind": "invalid_argument", "reason": "wrong_kind" })
    );
    assert_eq!(
        h.err(
            "confirm_carryover",
            json!({ "decision": { "from": "2000-01-01", "selection": { "select": "none" }, "reviews": [] } }),
        ),
        json!({ "kind": "conflict", "reason": "previous_day_changed", "previousDay": yesterday })
    );
    h.assert_no_event();

    // 選んだものを持ち越す。見直しの「分割する」は、どのボタンでも反映する
    let confirmed = h.ok(
        "confirm_carryover",
        json!({ "decision": {
            "from": yesterday,
            "selection": { "select": "selected", "ids": [plain["id"], blocker["id"]] },
            "reviews": [{ "id": reviewed["id"], "review": { "review": "split", "children": ["前半", " ", "後半"] } }],
        } }),
    );
    assert_eq!(h.next_event(), changed_by("screen"));
    let created: Vec<(&str, &str, i64)> = confirmed["created"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["text"].as_str().unwrap(),
                i["kind"].as_str().unwrap(),
                i["carryCount"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        created,
        [
            ("前半", "todo", 0),
            ("後半", "todo", 0),
            ("ふつう", "todo", 1),
            ("権限待ち", "blocker", 1),
        ]
    );
    assert_eq!(confirmed["created"][0]["splitFromId"], reviewed["id"]);
    assert_eq!(confirmed["created"][2]["carriedFromId"], plain["id"]);
    assert_eq!(confirmed["closed"][0]["id"], reviewed["id"]);
    assert_eq!(confirmed["closed"][0]["status"], "dropped");
    assert_eq!(confirmed["closed"][0]["closeReason"], "split");

    // 今日のボードの先頭に、前回の日の順のまま入る
    let board = h.ok("get_board", json!({ "date": today }));
    assert_eq!(
        texts(&board["todos"])[..3],
        ["前半".to_owned(), "後半".to_owned(), "ふつう".to_owned()]
    );
    assert_eq!(texts(&board["blockers"])[0], "権限待ち");
    // 選ばなかった TODO は前回の日のまま
    let past = h.ok("get_board", json!({ "date": yesterday }));
    let left = past["todos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|bi| bi["item"]["id"] == not_chosen["id"])
        .unwrap();
    assert_eq!(left["item"]["status"], "open");
    assert_eq!(left["carriedTo"], Value::Null);

    // 持ち越しを終えた後は、状態が null になり、二度目の確定は断る
    assert_eq!(h.ok("carryover_status", json!({})), Value::Null);
    assert_eq!(
        h.err(
            "confirm_carryover",
            json!({ "decision": { "from": yesterday, "selection": { "select": "all" }, "reviews": [] } }),
        ),
        json!({ "kind": "conflict", "reason": "carryover_done" })
    );
    h.assert_no_event();
}

/// 画面の骨組みの件数（`get_overview`）：表示している日の件数、今日から始まる Recent、
/// エージェントによる未確認の変更の数。
fn overview_counts(_: &Path) {
    let h = Harness::opened();
    let today = h.ok("today", json!({}));
    // 先の日（今日から決める。決まった日付だと、その日が過ぎると Recent に入ってしまうため）
    let later: LogicalDate = today.as_str().unwrap().parse().unwrap();
    let later = later.add_days(30).unwrap();
    let date = later.to_string();
    let date = date.as_str();
    let before = h.ok("get_overview", json!({ "date": date }));
    assert_eq!(before["recent"][0]["date"], today);
    assert_eq!(before["counts"]["todos"], 0);

    h.add_todo(date, "件数を数える");
    let agent = h.register_agent("overview", vec![Scope::Read, Scope::Todo]);
    h.core
        .agent_write(agent, |conn, a| {
            item::add_todo(
                conn,
                a,
                later,
                "エージェントが足した",
                &jiff::tz::TimeZone::UTC,
                yunagi_core::storage::table::now(),
            )
        })
        .unwrap()
        .unwrap();
    let after = h.ok("get_overview", json!({ "date": date }));
    assert_eq!(
        after["counts"],
        json!({ "todos": 2, "done": 0, "leftTodos": 2, "leftBlockers": 0, "resolved": 0, "carried": 0 })
    );
    assert_eq!(
        after["unseenChanges"].as_u64().unwrap(),
        before["unseenChanges"].as_u64().unwrap() + 1
    );
    // ボードの行には、作ったエージェントの識別子と未確認の印が付く（画面の行は付かない）
    let board = h.ok("get_board", json!({ "date": date }));
    assert_eq!(board["todos"][0]["agentName"], Value::Null);
    assert_eq!(board["todos"][0]["unseen"], false);
    assert_eq!(board["todos"][1]["agentName"], "overview");
    assert_eq!(board["todos"][1]["unseen"], true);
    // 先の日は Recent に入らない
    assert!(
        after["recent"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["date"] != date)
    );
}

/// エージェントの管理（設定のエージェント区分・変更履歴 S-06）：一覧・権限の変更・登録の削除・変更履歴・
/// 確認済み・取り消し。変えたら `changed`（`by: agent`）が届き、取り消しは画面の入力中を見る。
fn agent_admin_commands(_: &Path) {
    let h = Harness::opened();
    let date = "2032-06-01";
    // 既定ディレクトリのデータベースは、ほかのテストと共有する。名前は重ならないようにし、件数は最初の値との差で見る
    let (worker, idle) = ("admin-worker", "admin-idle");
    let agent = h.register_agent(worker, vec![Scope::Read, Scope::Todo]);
    h.register_agent(idle, vec![Scope::Read]);
    let unseen_now = || {
        h.ok("get_overview", json!({ "date": date }))["unseenChanges"]
            .as_u64()
            .unwrap()
    };
    let base = unseen_now();

    // 一覧：固定の cli・unknown が先。履歴が無い登録だけ消せる
    let rows = h.ok("list_agents", json!({}));
    let names: Vec<&str> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(names[..2], ["cli", "unknown"]);
    assert!(names.contains(&worker) && names.contains(&idle));
    let removable = |rows: &Value, name: &str| {
        rows.as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == name)
            .unwrap()["removable"]
            .clone()
    };
    assert_eq!(removable(&rows, "cli"), false);
    assert_eq!(removable(&rows, idle), true);

    // 権限の変更：見せた権限を expect に付ける。変えたら changed（agent）。同じなら知らせない
    h.drain();
    let changed = h.ok(
        "set_agent_scopes",
        json!({ "client": idle, "allow": ["append"], "deny": [], "expect": ["read"] }),
    );
    assert_eq!(
        changed,
        json!({ "updated": true, "scopes": ["read", "append"] })
    );
    assert_eq!(h.next_event(), changed_by("agent"));
    let same = h.ok(
        "set_agent_scopes",
        json!({ "client": idle, "allow": ["append"], "deny": [], "expect": ["read", "append"] }),
    );
    assert_eq!(same["updated"], false);
    h.assert_no_event();
    assert_eq!(
        h.err(
            "set_agent_scopes",
            json!({ "client": idle, "allow": ["todo"], "deny": [], "expect": ["read"] })
        ),
        json!({ "kind": "conflict", "reason": "scopes_changed" })
    );
    assert_eq!(
        h.err(
            "set_agent_scopes",
            json!({ "client": "unknown", "allow": ["todo"], "deny": [], "expect": ["read"] })
        ),
        json!({ "kind": "invalid_argument", "reason": "unknown_write" })
    );
    assert_eq!(
        h.err(
            "set_agent_scopes",
            json!({ "client": "nobody", "allow": ["todo"], "deny": [], "expect": [] })
        ),
        json!({ "kind": "not_found" })
    );
    h.assert_no_event();

    // エージェントが項目を足す → 変更履歴に未確認で出る
    let t = h
        .core
        .agent_write(agent, |conn, a| {
            item::add_todo(
                conn,
                a,
                date.parse().unwrap(),
                "調べる",
                &jiff::tz::TimeZone::UTC,
                yunagi_core::storage::table::now(),
            )
        })
        .unwrap()
        .unwrap();
    h.drain();
    let list = h.ok(
        "list_changes",
        json!({ "client": worker, "unseenOnly": true, "before": null, "limit": 50 }),
    );
    let changes = list["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["operation"], "todo.add");
    assert_eq!(changes[0]["client"], worker);
    assert_eq!(changes[0]["target"]["type"], "item");
    assert_eq!(changes[0]["target"]["item"]["text"], "調べる");
    assert_eq!(changes[0]["summary"]["type"], "added");
    assert_eq!(changes[0]["seenAt"], Value::Null);
    assert_eq!(list["nextBefore"], Value::Null);
    // 一覧は確認済みにしない
    assert_eq!(unseen_now(), base + 1);
    // ちょうど 1 件で終わるときは、続きの目印を返さない（空の続きを出さない）
    let page = h.ok(
        "list_changes",
        json!({ "client": worker, "unseenOnly": false, "before": null, "limit": 1 }),
    );
    assert_eq!(page["changes"].as_array().unwrap().len(), 1);
    assert_eq!(page["nextBefore"], Value::Null);
    // 続きがあるときは、返した最後の記録を目印にする（共有のデータベースには、ほかの記録もある）
    let page = h.ok(
        "list_changes",
        json!({ "client": null, "unseenOnly": false, "before": null, "limit": 1 }),
    );
    assert_eq!(page["changes"][0]["id"], changes[0]["id"]);
    assert_eq!(page["nextBefore"], changes[0]["id"]);
    assert_eq!(
        h.err(
            "list_changes",
            json!({ "client": "nobody", "unseenOnly": false, "before": null, "limit": 1 })
        ),
        json!({ "kind": "not_found" })
    );

    // 履歴のある登録は消せない。履歴の無い登録は消せて、changed（agent）が届く
    assert_eq!(
        h.err("remove_agent", json!({ "client": worker })),
        json!({ "kind": "conflict", "reason": "client_in_use" })
    );
    assert_eq!(
        h.err("remove_agent", json!({ "client": "cli" })),
        json!({ "kind": "invalid_argument", "reason": "fixed_client" })
    );
    h.ok("remove_agent", json!({ "client": idle }));
    assert_eq!(h.next_event(), changed_by("agent"));
    assert_eq!(
        h.err("remove_agent", json!({ "client": idle })),
        json!({ "kind": "not_found" })
    );

    // 取り消し：入力中の欄は書き戻さない。見せた token が合えば取り消し、記録は取り消し済みになる
    let id = changes[0]["id"].as_str().unwrap().to_owned();
    let plan = h.ok("preview_revert", json!({ "changeId": id }));
    assert_eq!(plan["state"], "apply");
    assert_eq!(plan["effect"]["type"], "delete_item");
    assert_eq!(plan["changedSince"], json!([]));
    let token = plan["token"].as_str().unwrap().to_owned();
    h.ok(
        "report_ui_state",
        json!({ "state": {
            "session": "t", "seq": 1,
            "editingItems": [{ "itemId": t.id.to_string(), "field": "text" }],
            "editors": [],
        } }),
    );
    h.drain();
    // 利用者が本文を入力中の項目は、取り消さない（入力を消さないため）
    assert_eq!(
        h.err(
            "revert_change",
            json!({ "changeId": id, "force": false, "expect": token })
        ),
        json!({ "kind": "conflict", "reason": "user_editing" })
    );
    h.assert_no_event();
    h.ok(
        "report_ui_state",
        json!({ "state": { "session": "t", "seq": 2, "editingItems": [], "editors": [] } }),
    );
    // 見せた後に状態が変わっていれば（token が合わなければ）、書かずに止める
    let wrong = h.err(
        "revert_change",
        json!({ "changeId": id, "force": false, "expect": "stale" }),
    );
    assert_eq!(wrong["reason"], "changed");
    assert_eq!(wrong["afterPreview"], true);
    h.assert_no_event();
    let done = h.ok(
        "revert_change",
        json!({ "changeId": id, "force": false, "expect": token }),
    );
    assert_eq!(
        done,
        json!({ "updated": true, "state": "apply", "marked": true })
    );
    assert_eq!(h.next_event(), changed_by("agent"));
    let board = h.ok("get_board", json!({ "date": date }));
    assert_eq!(board["todos"], json!([]));
    // 取り消し済みの記録をもう一度取り消しても、何も書かず、知らせない
    let again = h.ok(
        "revert_change",
        json!({ "changeId": id, "force": false, "expect": token }),
    );
    assert_eq!(again["updated"], false);
    assert_eq!(again["state"], "already_reverted");
    h.assert_no_event();
    // 取り消した記録は確認済みになり、取り消しの記録（人の操作）は取り消せない
    assert_eq!(unseen_now(), base);
    let all = h.ok(
        "list_changes",
        json!({ "client": null, "unseenOnly": false, "before": null, "limit": 50 }),
    );
    // 新しい順の先頭は、いま作った取り消しの記録（人の操作）
    let revert = &all["changes"][0];
    assert_eq!(revert["operation"], "agent.revert");
    assert_eq!(revert["actor"], "human");
    assert_eq!(
        h.err("preview_revert", json!({ "changeId": revert["id"] })),
        json!({ "kind": "invalid_argument", "reason": "not_agent_change" })
    );
    assert_eq!(
        h.err(
            "preview_revert",
            json!({ "changeId": "01ARZ3NDEKTSV4RRFFQ69G5FAV" })
        ),
        json!({ "kind": "not_found" })
    );

    // 確認済み：見せた記録だけに付ける。付けたら件数が変わり changed が届く。付ける物が無ければ届かない
    h.core
        .agent_write(agent, |conn, a| {
            item::add_todo(
                conn,
                a,
                date.parse().unwrap(),
                "もう一つ",
                &jiff::tz::TimeZone::UTC,
                yunagi_core::storage::table::now(),
            )
        })
        .unwrap()
        .unwrap();
    h.drain();
    assert_eq!(unseen_now(), base + 1);
    let unseen = h.ok(
        "list_changes",
        json!({ "client": worker, "unseenOnly": true, "before": null, "limit": 50 }),
    );
    let new_id = unseen["changes"][0]["id"].clone();
    assert_eq!(h.ok("mark_changes_seen", json!({ "ids": [new_id] })), 1);
    assert_eq!(h.next_event(), changed_by("agent"));
    assert_eq!(unseen_now(), base);
    assert_eq!(h.ok("mark_changes_seen", json!({ "ids": [new_id] })), 0);
    h.assert_no_event();

    // ローカル API の状態は、種類だけを返す
    let api = h.ok("api_status", json!({}));
    assert!(["stopped", "listening", "unavailable"].contains(&api["state"].as_str().unwrap()));
}

/// ふりかえり（`get_archive`）：選んだ日の週・記録のある直近の日・プレビュー。何も変えないので
/// `changed` は出ない。
fn archive_command(_: &Path) {
    let h = Harness::opened();
    let today = h.ok("today", json!({}));
    let today_date: LogicalDate = today.as_str().unwrap().parse().unwrap();
    // 今日の前の日（記録を足す日。同じ保存先を使うほかの試験が足した記録も残っているので、
    // 前の日にして、本文は版を読んでから上書きする）と、先の日
    let past = today_date.add_days(-1).unwrap().to_string();
    let later = today_date.add_days(30).unwrap().to_string();
    h.add_todo(&past, "ふりかえる");
    for (doc, body) in [
        ("reflection", "## 良かったこと\n- 進んだ"),
        ("note", "## 10:00 定例\n議事"),
    ] {
        let rev = h.ok("get_text_doc", json!({ "doc": doc, "date": past }))["rev"].clone();
        h.ok(
            "save_text_doc",
            json!({ "doc": doc, "date": past, "body": body, "baseRev": rev }),
        );
    }
    h.drain();

    // 日を決めないと、今日より前で記録のある最も新しい日を選ぶ
    let a = h.ok("get_archive", json!({ "date": null }));
    assert_eq!(a["date"], json!(past));
    assert_eq!(a["today"], today);
    assert_eq!(a["week"].as_array().unwrap().len(), 5);
    // 記録のある直近の日は、今日（ほかの試験が記録を足していれば）を含めて新しい順
    let daily = a["daily"].as_array().unwrap();
    assert!(daily.len() <= 8);
    let entry = daily.iter().find(|d| d["date"] == json!(past)).unwrap();
    assert_eq!(entry["reflection"], "## 良かったこと\n- 進んだ");
    assert!(a["preview"]["counts"]["todos"].as_u64().unwrap() >= 1);
    assert_eq!(
        a["preview"]["note"],
        json!([{ "hour": 10, "minute": 0, "title": "定例", "lines": ["議事"] }])
    );
    // 先の日を選ぼうとしても、今日で止まる
    let a = h.ok("get_archive", json!({ "date": later }));
    assert_eq!(a["date"], today);
    h.assert_no_event();
}

/// Markdown 書き出し：日ごとのファイルを選んだフォルダに書き、パスを返す。データは変えないので
/// `changed` は出ない（アーキテクチャ設計書 6.4）。
fn export_markdown_writes_one_file_per_day(_: &Path) {
    let h = Harness::opened();
    h.add_todo("2030-02-01", "書き出す");
    h.ok(
        "save_text_doc",
        json!({ "doc": "note", "date": "2030-02-03", "body": "メモ", "baseRev": 0 }),
    );
    h.drain();
    let out = tempfile::tempdir().unwrap();
    let dir = out.path().to_str().unwrap();

    // 逆の期間でもよく、書き出すものが無い日（2030-02-02）は書かない
    let paths = h.ok(
        "export_markdown",
        json!({ "from": "2030-02-03", "to": "2030-02-01", "dir": dir }),
    );
    let first = out.path().join("2030-02-01.md");
    let third = out.path().join("2030-02-03.md");
    assert_eq!(
        paths,
        json!([first.to_str().unwrap(), third.to_str().unwrap()])
    );
    assert_eq!(
        std::fs::read_to_string(&first).unwrap(),
        "# 2030-02-01\n\n## TODO\n\n- [ ] 書き出す\n"
    );
    assert_eq!(
        std::fs::read_to_string(&third).unwrap(),
        "# 2030-02-03\n\n## Notes\n\nメモ\n"
    );
    assert!(!out.path().join("2030-02-02.md").exists());
    h.assert_no_event();

    // 何も無い期間は空
    assert_eq!(
        h.ok(
            "export_markdown",
            json!({ "from": "2031-01-01", "to": "2031-01-01", "dir": dir })
        ),
        json!([])
    );
    // 書き出し先に書けないとき（無いフォルダ・相対パス）は export_failed（置き換え終えた数 0）。
    // データベースの読み取りの失敗は storage のまま（画面設計書 7章・アーキテクチャ設計書 6.4）
    for bad in [out.path().join("無い").to_str().unwrap(), "relative"] {
        let err = h.err(
            "export_markdown",
            json!({ "from": "2030-02-01", "to": "2030-02-01", "dir": bad }),
        );
        assert_eq!(err["kind"], "export_failed", "{bad}");
        assert_eq!(err["replaced"], 0, "{bad}");
        assert!(!err["detail"].as_str().unwrap().is_empty(), "{err}");
    }
}

/// エージェントの書き込みも `changed` で知らせる。画面が知らせた入力中の欄は、エージェントの判定に
/// 使われる（アーキテクチャ設計書 4章 6、4.1）。
fn agent_writes_use_the_same_events_and_the_screen_state(_: &Path) {
    let h = Harness::opened();
    let date = "2030-01-09";
    let agent = h.register_agent("claude-code", vec![Scope::Read, Scope::Todo]);
    let todo = h.add_todo(date, "テストを書く");
    let id: Ulid = todo["id"].as_str().unwrap().parse().unwrap();
    let ld: LogicalDate = date.parse().unwrap();
    h.drain();

    // 本文を入力中：本文への edit_todo は user_editing、状態の変更は受け付ける
    h.ok(
        "report_ui_state",
        json!({ "state": {
            "session": "t",
            "seq": 1,
            "editingItems": [{ "itemId": id.to_string(), "field": "text" }],
            "editors": [{ "doc": "note", "date": date, "rev": 3, "lines": [2, 5] }],
        } }),
    );
    assert!(h.core.screen().is_editing(id, ItemField::Text));
    assert_eq!(
        h.core.screen().protected_lines(TextDoc::Note, ld),
        Some(ProtectedLines {
            rev: 3,
            lines: vec![2, 5]
        })
    );
    let tz = jiff::tz::TimeZone::UTC;
    let edit = h
        .core
        .agent_write(agent, |conn, a| {
            item::edit_todo(conn, a, id, "別の本文", yunagi_core::storage::table::now())
        })
        .unwrap();
    assert!(matches!(
        edit,
        Err(ItemError::Conflict(item::Conflict::UserEditing))
    ));
    h.assert_no_event();
    let done = h
        .core
        .agent_write(agent, |conn, a| {
            item::complete_todo(conn, a, id, yunagi_core::storage::table::now())
        })
        .unwrap()
        .unwrap();
    assert!(done.updated);
    assert_eq!(h.next_event(), changed_by("agent"));

    // 入力を終えたと知らせ直すと、受け付ける
    h.ok(
        "report_ui_state",
        json!({ "state": { "session": "t", "seq": 2, "editingItems": [], "editors": [] } }),
    );
    assert!(!h.core.screen().is_editing(id, ItemField::Text));
    assert_eq!(h.core.screen().protected_lines(TextDoc::Note, ld), None);
    h.core
        .agent_write(agent, |conn, a| {
            item::edit_todo(conn, a, id, "別の本文", yunagi_core::storage::table::now())
        })
        .unwrap()
        .unwrap();
    assert_eq!(h.next_event(), changed_by("agent"));
    let board = h.ok("get_board", json!({ "date": date }));
    assert_eq!(board["todos"][0]["item"]["text"], "別の本文");
    assert_eq!(board["todos"][0]["item"]["status"], "done");

    // 何も変えない操作は知らせない
    let again = h
        .core
        .agent_write(agent, |conn, a| {
            item::complete_todo(conn, a, id, yunagi_core::storage::table::now())
        })
        .unwrap()
        .unwrap();
    assert!(!again.updated);
    h.assert_no_event();
    // 権限の無い操作は、何も書かずに断る
    let add = h
        .core
        .agent_write(agent, |conn, a| {
            item::add_blocker(conn, a, ld, "x", &tz, yunagi_core::storage::table::now())
        })
        .unwrap();
    assert!(matches!(add, Err(ItemError::PermissionDenied(_))));
    h.assert_no_event();

    // 守る行の番号は 1 から
    assert_eq!(
        h.err(
            "report_ui_state",
            json!({ "state": { "session": "t", "seq": 3, "editingItems": [], "editors": [{ "doc": "note", "date": date, "rev": 1, "lines": [0] }] } })
        ),
        json!({ "kind": "invalid_argument", "reason": "invalid_line" })
    );
}

/// 利用者が完了にしてから保存が届くまでの間に、エージェントも完了にした：保存の書き込みで
/// エージェントの記録を消し、`changed` で知らせる（アーキテクチャ設計書 4.1）。
fn the_same_state_from_screen_and_agent_becomes_the_users_change(_: &Path) {
    let h = Harness::opened();
    let date = "2030-01-10";
    let agent = h.register_agent("same-state", vec![Scope::Read, Scope::Todo]);
    let todo = h.add_todo(date, "レビューする");
    let id: Ulid = todo["id"].as_str().unwrap().parse().unwrap();
    // 利用者の操作（画面の時計で測った時刻）
    let clicked = yunagi_core::storage::table::now().as_millisecond();
    std::thread::sleep(Duration::from_millis(5));
    h.core
        .agent_write(agent, |conn, a| {
            item::complete_todo(conn, a, id, yunagi_core::storage::table::now())
        })
        .unwrap()
        .unwrap();
    let logs = |h: &Harness| {
        h.core
            .read(|conn| change_log::list_by_target(conn, change_log::Target::Item(id)))
            .unwrap()
    };
    assert_eq!(logs(&h).len(), 1);
    h.drain();
    let saved = h.ok(
        "save_item",
        json!({ "id": id.to_string(), "fields": { "status": {
            "before": "open", "closedAt": clicked, "changedAt": clicked,
        } } }),
    );
    assert_eq!(saved["updated"], true);
    assert_eq!(saved["item"]["status"], "done");
    assert_eq!(saved["item"]["completedAt"], clicked);
    assert_eq!(h.next_event(), changed_by("screen"));
    assert!(logs(&h).is_empty(), "the agent's record was removed");
}

/// 画面が知らせた守る行への `edit_note` は `user_editing`。知らせた版が古い間はどの行も。
fn edit_note_refuses_the_lines_the_screen_protects(_: &Path) {
    let h = Harness::opened();
    let date = "2030-01-11";
    let ld: LogicalDate = date.parse().unwrap();
    let agent = h.register_agent("protected-lines", vec![Scope::Read, Scope::NoteEdit]);
    let saved = h.ok(
        "save_text_doc",
        json!({ "doc": "note", "date": date, "body": "a\nb\nc", "baseRev": 0 }),
    );
    let rev = saved["rev"].as_i64().unwrap();
    h.ok(
        "report_ui_state",
        json!({ "state": {
            "session": "n", "seq": 1, "editingItems": [],
            "editors": [{ "doc": "note", "date": date, "rev": rev, "lines": [2] }],
        } }),
    );
    let tz = jiff::tz::TimeZone::UTC;
    let edit = |line: usize, expected: &str| {
        h.core
            .agent_write(agent, |conn, a| {
                text_doc::edit_note(
                    conn,
                    ld,
                    &LineEdit {
                        line,
                        expected_text: expected.into(),
                        op: LineOp::Replace("X".into()),
                    },
                    a,
                    &tz,
                    yunagi_core::storage::table::now(),
                )
            })
            .unwrap()
    };
    assert!(matches!(edit(2, "b"), Err(TextDocError::UserEditing)));
    h.drain();
    edit(3, "c").unwrap();
    assert_eq!(h.next_event(), changed_by("agent"));
    // 版が進んだので、知らせ直しが届くまではどの行も user_editing
    assert!(matches!(edit(1, "a"), Err(TextDocError::UserEditing)));
    h.ok(
        "report_ui_state",
        json!({ "state": {
            "session": "n", "seq": 2, "editingItems": [],
            "editors": [{ "doc": "note", "date": date, "rev": rev + 1, "lines": [2] }],
        } }),
    );
    edit(1, "a").unwrap();
}

/// 画面の保存の書き込みが失敗したら、次に成功するまでエージェントの書き込みを断る
/// （アーキテクチャ設計書 4.1「保存に失敗している間」）。引数の誤りでは断らない。
fn save_failures_refuse_agent_writes_until_a_save_succeeds(_: &Path) {
    let h = Harness::opened();
    let date = "2030-01-10";
    let agent = h.register_agent("codex", vec![Scope::Read, Scope::Todo]);
    let todo = h.add_todo(date, "保存の失敗");
    let id: Ulid = todo["id"].as_str().unwrap().parse().unwrap();
    let agent_edit = |text: &str| {
        h.core
            .agent_write(agent, |conn, a| {
                item::edit_todo(conn, a, id, text, yunagi_core::storage::table::now())
            })
            .unwrap()
    };

    // 引数の誤りは保存の失敗に数えない
    h.err(
        "save_item",
        json!({ "id": id.to_string(), "fields": { "text": "" } }),
    );
    assert!(!h.core.screen().save_failing());
    let other = h.add_todo(date, "削除する");
    let other_id = other["id"].as_str().unwrap().to_owned();

    // 書き込みを失敗させる（この接続だけの一時的なトリガー）
    h.core
        .read(|conn| {
            conn.execute_batch(
                "CREATE TEMP TRIGGER fail_item_update BEFORE UPDATE OF text ON item \
                 BEGIN SELECT RAISE(FAIL, 'disk is full'); END;",
            )
            .map_err(StorageError::from)
        })
        .unwrap();
    let e = h.err(
        "save_item",
        json!({ "id": id.to_string(), "fields": { "text": "書けない" } }),
    );
    assert_eq!(e["kind"], "storage");
    assert!(
        e["detail"].as_str().unwrap().contains("disk is full"),
        "{e}"
    );
    assert!(h.core.screen().save_failing());
    assert!(matches!(
        agent_edit("エージェント"),
        Err(ItemError::StorageUnavailable(_))
    ));
    // 保存でない操作（削除）は、成功しても保存の失敗中を終えない（削除は item を UPDATE するが、
    // トリガーは本文の更新だけを止める）
    h.ok("delete_item", json!({ "id": other_id }));
    assert!(h.core.screen().save_failing());
    // 読み取りは受け付ける
    h.ok("get_board", json!({ "date": date }));

    h.core
        .read(|conn| {
            conn.execute_batch("DROP TRIGGER fail_item_update;")
                .map_err(StorageError::from)
        })
        .unwrap();
    h.ok(
        "save_item",
        json!({ "id": id.to_string(), "fields": { "text": "書けた" } }),
    );
    assert!(!h.core.screen().save_failing());
    agent_edit("エージェント").unwrap();

    // 保存でない操作の失敗は、保存の失敗中を始めない
    h.add_todo(date, "後ろの項目");
    h.core
        .read(|conn| {
            conn.execute_batch(
                "CREATE TEMP TRIGGER fail_item_move BEFORE UPDATE OF sort_key ON item \
                 BEGIN SELECT RAISE(FAIL, 'disk is full'); END;",
            )
            .map_err(StorageError::from)
        })
        .unwrap();
    let e = h.err(
        "move_item",
        json!({ "id": id.to_string(), "position": { "at": "last" } }),
    );
    assert_eq!(e["kind"], "storage");
    assert!(!h.core.screen().save_failing());
}

/// データベースを開いている途中でアプリが終わったら、開き終わっても閉じたままにし、ロックを離す。
fn closing_before_the_open_finishes_keeps_it_closed(_: &Path) {
    let h = Harness::new();
    h.core.close();
    h.core.open();
    h.assert_no_event();
    assert_eq!(h.err("today", json!({})), json!({ "kind": "not_ready" }));
    let dir = yunagi_core::storage::data_dir::default_dir().unwrap();
    drop(yunagi_core::storage::DbLock::acquire(&dir, Duration::ZERO).unwrap());
}

/// 論理日付が変わったら `today-changed` を知らせ、その日の定期バックアップを作る（6.2）。
fn logical_date_changes_notify_and_back_up(default_dir: &Path) {
    let h = Harness::opened();
    let today: LogicalDate = h.today().parse().unwrap();
    let later = yunagi_core::storage::table::now() + jiff::SignedDuration::from_hours(48);
    let expected = LogicalDate::at(
        later,
        &yunagi_app::app_core::system_tz(),
        Default::default(),
    );
    assert_ne!(expected, today);
    h.core.check_today(later);
    assert_eq!(
        h.next_event(),
        ("today-changed", json!({ "today": expected.to_string() }))
    );
    h.core.wait_for_backup();
    assert!(
        backups(default_dir)
            .iter()
            .any(|n| n.starts_with(&format!("daily-{expected}-"))),
        "{:?}",
        backups(default_dir)
    );
    // 同じ日なら、もう知らせない
    h.core.check_today(later);
    h.core.wait_for_backup();
    h.assert_no_event();
}

/// 定期バックアップを作れなければ、その日に1回だけ `backup-failed` を知らせ、次に確かめたときに
/// 作り直す（6.2）。
fn backup_failures_are_reported_once_and_retried(default_dir: &Path) {
    let h = Harness::opened();
    let later = yunagi_core::storage::table::now() + jiff::SignedDuration::from_hours(24 * 5);
    let day = LogicalDate::at(
        later,
        &yunagi_app::app_core::system_tz(),
        Default::default(),
    );
    // backups/ をフォルダでなくファイルにして、作れなくする
    let backups_dir = default_dir.join("backups");
    let moved = default_dir.join("backups.moved");
    std::fs::rename(&backups_dir, &moved).unwrap();
    std::fs::write(&backups_dir, "").unwrap();

    h.core.check_today(later);
    assert_eq!(h.next_event().0, "today-changed");
    h.core.wait_for_backup();
    let (name, payload) = h.next_event();
    assert_eq!(name, "backup-failed");
    assert!(
        payload["detail"].as_str().unwrap().contains("backups"),
        "{payload}"
    );
    // 同じ日の作り直しの失敗では、もう知らせない
    h.core.check_today(later);
    h.core.wait_for_backup();
    h.assert_no_event();

    // 直れば、次に確かめたときに作る
    std::fs::remove_file(&backups_dir).unwrap();
    std::fs::rename(&moved, &backups_dir).unwrap();
    h.core.check_today(later);
    h.core.wait_for_backup();
    h.assert_no_event();
    assert!(
        backups(default_dir)
            .iter()
            .any(|n| n.starts_with(&format!("daily-{day}-"))),
        "{:?}",
        backups(default_dir)
    );
}

/// 保存先にデータベースが無いとき（初めてでない）：探した場所を返し、選べば新しく作れる（6.3）。
fn missing_database_is_reported_and_can_be_created(default_dir: &Path) {
    let elsewhere: PathBuf = default_dir.parent().unwrap().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::write(
        default_dir.join("config.json"),
        serde_json::to_string(&json!({ "data_dir": elsewhere })).unwrap(),
    )
    .unwrap();

    let h = Harness::new();
    h.core.open();
    let expected = json!({
        "state": "not_found",
        "path": elsewhere.join("yunagi.db").display().to_string(),
        "dataDir": elsewhere.display().to_string(),
        "configuredIn": default_dir.join("config.json").display().to_string(),
        "dataDirExists": true,
    });
    assert_eq!(h.next_event(), ("startup", expected.clone()));
    assert_eq!(h.ok("startup_status", json!({})), expected);
    assert!(!elsewhere.join("yunagi.db").exists());
    assert_eq!(h.err("today", json!({})), json!({ "kind": "not_ready" }));

    assert_eq!(
        h.ok("create_database", json!({})),
        json!({ "state": "ready" })
    );
    assert_eq!(h.next_event(), ("startup", json!({ "state": "ready" })));
    assert!(elsewhere.join("yunagi.db").is_file());
    h.core.wait_for_backup();
    assert!(
        std::fs::read_dir(elsewhere.join("backups"))
            .unwrap()
            .any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("daily-"))
    );
    assert_eq!(
        h.err("create_database", json!({})),
        json!({ "kind": "invalid_state" })
    );

    // 閉じた後のコマンドは not_ready。ロックも離れている
    h.core.close();
    assert_eq!(h.err("today", json!({})), json!({ "kind": "not_ready" }));
    let dir = yunagi_core::storage::data_dir::default_dir().unwrap();
    yunagi_core::storage::DbLock::acquire(&dir, Duration::ZERO).unwrap();
}

/// `config.json` の保存先を `data_dir` にする（データは移さない）。
fn point_config_to(default_dir: &Path, data_dir: &Path) {
    std::fs::write(
        default_dir.join("config.json"),
        serde_json::to_string(&json!({ "data_dir": data_dir })).unwrap(),
    )
    .unwrap();
}

/// 保存先 `data_dir` に、TODO `text` と定期バックアップを持つデータベースを作り、閉じる。
fn create_with_backup(default_dir: &Path, data_dir: &Path, text: &str) {
    std::fs::create_dir_all(data_dir).unwrap();
    point_config_to(default_dir, data_dir);
    let h = Harness::new();
    h.core.open();
    assert_eq!(h.next_event().1["state"], "not_found");
    h.ok("create_database", json!({}));
    h.core.wait_for_backup();
    let today = h.today();
    h.add_todo(&today, text);
}

/// 壊れていて開けないデータベース：開けなかった理由と場所を返し、バックアップの一覧から選んで
/// 置き換えられる。壊れたものは backups/ に残る（アーキテクチャ設計書 6.2）。
fn a_damaged_database_can_be_replaced_from_a_backup(default_dir: &Path) {
    let data_dir = default_dir.parent().unwrap().join("damaged");
    create_with_backup(default_dir, &data_dir, "kept in the backup");
    let db = data_dir.join("yunagi.db");
    std::fs::write(&db, b"this is not a database").unwrap();

    let h = Harness::new();
    h.core.open();
    let (name, failed) = h.next_event();
    assert_eq!(name, "startup");
    assert_eq!(failed["state"], "failed");
    assert_eq!(failed["reason"], "corrupted");
    assert_eq!(failed["database"], db.display().to_string());
    assert_eq!(failed["backup"], Value::Null);
    assert_eq!(h.ok("startup_status", json!({})), failed);

    let list = h.ok("list_backups", json!({}));
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!(list[0]["kind"]["type"], "daily");
    let backup = list[0]["path"].as_str().unwrap().to_owned();

    // 使えないものを選んでも、何も変えずに元の状態に戻る
    let missing = data_dir.join("missing.db").display().to_string();
    assert_eq!(
        h.err("restore_at_startup", json!({ "backup": missing })),
        json!({ "kind": "rejected", "reason": "unreadable" })
    );
    assert_eq!(h.next_event(), ("startup", failed.clone()));
    assert_eq!(std::fs::read(&db).unwrap(), b"this is not a database");

    let restored = h.ok("restore_at_startup", json!({ "backup": backup }));
    assert_eq!(restored["state"], "restored");
    // 確かめるまでは、読み込み直しても戻した結果（退避した場所）が届く
    assert_eq!(h.next_event(), ("startup", restored.clone()));
    assert_eq!(h.ok("startup_status", json!({})), restored);
    let set_aside = PathBuf::from(restored["setAside"].as_str().unwrap());
    assert_eq!(set_aside.parent().unwrap(), data_dir.join("backups"));
    assert_eq!(
        std::fs::read(&set_aside).unwrap(),
        b"this is not a database"
    );
    assert_eq!(
        h.ok("acknowledge_restore", json!({})),
        json!({ "state": "ready" })
    );
    assert_eq!(h.next_event(), ("startup", json!({ "state": "ready" })));
    assert_eq!(
        h.err("acknowledge_restore", json!({})),
        json!({ "kind": "invalid_state" })
    );
    // バックアップの内容が戻っている（バックアップは TODO を足す前に作ったので空）
    let today = h.today();
    let board = h.ok("get_board", json!({ "date": today }));
    assert_eq!(board["todos"], json!([]), "{board}");
    // 開いた後は、起動時の操作はできない
    assert_eq!(
        h.err("restore_at_startup", json!({ "backup": backup })),
        json!({ "kind": "invalid_state" })
    );
    assert_eq!(
        h.err("retry_open", json!({})),
        json!({ "kind": "invalid_state" })
    );
}

/// 見つからないデータベース：やり直す・データベースのある別のフォルダを選び直す（6.3）。
fn a_missing_database_can_be_retried_or_pointed_elsewhere(default_dir: &Path) {
    let root = default_dir.parent().unwrap();
    let existing = root.join("existing");
    create_with_backup(default_dir, &existing, "in existing");
    let gone = root.join("not-connected");
    point_config_to(default_dir, &gone);

    let h = Harness::new();
    h.core.open();
    let (_, not_found) = h.next_event();
    assert_eq!(not_found["state"], "not_found");
    assert_eq!(not_found["dataDirExists"], false);
    // まだ繋いでいないので、やり直しても見つからない
    assert_eq!(h.ok("retry_open", json!({})), not_found);
    assert_eq!(h.next_event(), ("startup", not_found.clone()));
    // バックアップの一覧は、保存先のフォルダが無ければ空
    assert_eq!(h.ok("list_backups", json!({})), json!([]));

    // 使えないフォルダは、何も変えずに断る
    for (dir, reason) in [
        ("relative/path".to_owned(), "invalid_path"),
        (gone.display().to_string(), "not_a_directory"),
        (
            root.join("damaged").join("backups").display().to_string(),
            "no_database",
        ),
    ] {
        assert_eq!(
            h.err("choose_data_dir", json!({ "dir": dir })),
            json!({ "kind": "rejected", "reason": reason }),
            "{dir}"
        );
        assert_eq!(h.next_event(), ("startup", not_found.clone()));
    }

    assert_eq!(
        h.ok(
            "choose_data_dir",
            json!({ "dir": existing.display().to_string() })
        ),
        json!({ "state": "ready" })
    );
    assert_eq!(h.next_event(), ("startup", json!({ "state": "ready" })));
    let today = h.today();
    let board = h.ok("get_board", json!({ "date": today }));
    assert_eq!(board["todos"][0]["item"]["text"], "in existing", "{board}");
    let config: Value =
        serde_json::from_str(&std::fs::read_to_string(default_dir.join("config.json")).unwrap())
            .unwrap();
    assert_eq!(config["data_dir"], existing.display().to_string());
    // 開いた後の一覧は、開いた保存先のもの
    assert_eq!(h.ok("list_backups", json!({})).as_array().unwrap().len(), 1);
}

/// 新しい版で作られたデータベース：開かずに理由と場所を返し、ファイルには触れない。
fn a_newer_database_is_reported_without_touching_it(default_dir: &Path) {
    let data_dir = default_dir.parent().unwrap().join("newer");
    create_with_backup(default_dir, &data_dir, "made by a newer version");
    let db = data_dir.join("yunagi.db");
    rusqlite_set_user_version(&db, 999);
    let before = std::fs::read(&db).unwrap();

    let h = Harness::new();
    h.core.open();
    let (_, failed) = h.next_event();
    assert_eq!(failed["reason"], "unsupported_schema", "{failed}");
    assert_eq!(failed["database"], db.display().to_string());
    assert_eq!(std::fs::read(&db).unwrap(), before);
    // 壊れているわけではないので、バックアップでの置き換えはできない
    assert_eq!(
        h.err("restore_at_startup", json!({ "backup": "/x.db" })),
        json!({ "kind": "invalid_state" })
    );
}

/// 設定の読み書き（画面設計書 7章）：既定値、変える欄だけの保存、誤った値の拒否（何も書かない）、
/// 変えたときだけ届く `changed`、日付の切り替わり時刻を変えたときにすぐ今日を求め直すこと。
fn settings_commands(default_dir: &Path) {
    let base = default_dir.parent().unwrap().join("settings-base");
    create_with_backup(default_dir, &base, "seed");
    let h = Harness::opened();
    let defaults = json!({
        "dayBoundary": "04:00",
        "meterStart": "09:00",
        "meterEnd": "19:00",
        "locale": null,
        "updateCheck": true,
        "theme": "system",
        "fontSize": 15,
        "noteFont": "body",
        "layout": "A",
        "carryoverReviewThreshold": 3,
        "sidebarOpen": true,
        "shortcuts": {},
        "vimMode": false,
        "vimImeRestore": true,
    });
    assert_eq!(h.ok("get_settings", json!({})), defaults);

    // 変える欄だけ。片方だけの始業・終業は、もう片方に今の値を使う
    let saved = h.ok(
        "save_settings",
        json!({ "patch": { "theme": "dark", "meterEnd": "18:30", "locale": "ja", "noteFont": "mono" } }),
    );
    let mut expected = defaults.clone();
    expected["theme"] = json!("dark");
    expected["meterEnd"] = json!("18:30");
    expected["locale"] = json!("ja");
    expected["noteFont"] = json!("mono");
    assert_eq!(saved, expected);
    // 設定はボードのデータではないので、`changed` は出さない（画面は自分で当てる）
    h.assert_no_event();
    assert_eq!(h.ok("get_settings", json!({})), expected);
    // 同じ値を保存しても、知らせない
    h.ok("save_settings", json!({ "patch": { "theme": "dark" } }));
    h.assert_no_event();

    // 誤った値は、同じ保存の正しい欄も含めて何も書かない
    for (patch, reason) in [
        (json!({ "meterStart": "19:00" }), "meter_hours"),
        (
            json!({ "meterStart": "20:00", "theme": "light" }),
            "meter_hours",
        ),
        (json!({ "fontSize": 12, "theme": "light" }), "font_size"),
        (json!({ "fontSize": 20 }), "font_size"),
        (json!({ "carryoverReviewThreshold": 0 }), "review_threshold"),
        (json!({ "dayBoundary": "4:00" }), "time_of_day"),
        (json!({ "meterEnd": "25:00" }), "time_of_day"),
        (json!({ "shortcuts": { "palette": "" } }), "shortcuts"),
    ] {
        assert_eq!(
            h.err("save_settings", json!({ "patch": patch })),
            json!({ "kind": "invalid_argument", "reason": reason }),
            "{patch}"
        );
    }
    assert_eq!(h.ok("get_settings", json!({})), expected);
    h.assert_no_event();

    // ショートカットの割当（欄ごと置き換える）と Vim モード
    let saved = h.ok(
        "save_settings",
        json!({ "patch": {
            "shortcuts": { "palette": "Mod+P" }, "vimMode": true, "vimImeRestore": false
        } }),
    );
    expected["shortcuts"] = json!({ "palette": "Mod+P" });
    expected["vimMode"] = json!(true);
    expected["vimImeRestore"] = json!(false);
    assert_eq!(saved, expected);
    h.ok("save_settings", json!({ "patch": { "shortcuts": {} } }));
    expected["shortcuts"] = json!({});
    assert_eq!(h.ok("get_settings", json!({})), expected);
    h.assert_no_event();

    // 日付の切り替わり時刻を変えると、30 秒を待たずに今日を求め直し、変わっていれば知らせる
    let before = h.today();
    h.ok(
        "save_settings",
        json!({ "patch": { "dayBoundary": "23:59" } }),
    );
    let after = h.today();
    if after != before {
        assert_eq!(h.next_event(), ("today-changed", json!({ "today": after })));
    }
    h.assert_no_event();
    std::fs::remove_file(default_dir.join("config.json")).unwrap();
}

/// 動いている間の保存先の変更・選び直し・復元（画面設計書 7章 S-05 のデータ）：画面を消さない（状態は
/// 「開いた」のまま、`startup` を知らせない）、ほかのコマンドは終わるまで待つだけで失敗しない、
/// 使えないものは理由を付けて断り何も変えない。
fn data_dir_operations_keep_the_screen_ready(default_dir: &Path) {
    let root = default_dir.parent().unwrap();
    let base = root.join("operations-base");
    create_with_backup(default_dir, &base, "seed");
    let h = Harness::opened();
    let ready = json!({ "state": "ready" });
    assert_eq!(
        h.ok("get_data_dir", json!({})),
        json!(base.display().to_string())
    );

    // 復元：定期バックアップ（TODO を足す前）に戻す。今のデータは「復元前」に残る
    let today = h.today();
    let list = h.ok("list_backups", json!({}));
    let daily = list[0]["path"].as_str().unwrap().to_owned();
    assert_eq!(list[0]["kind"]["type"], "daily");
    let board = h.ok("get_board", json!({ "date": today }));
    assert_eq!(texts(&board["todos"]), ["seed"]);
    h.drain();
    h.ok("restore_backup", json!({ "backup": daily }));
    assert_eq!(h.next_event(), changed_by("screen"));
    h.assert_no_event();
    assert_eq!(h.ok("startup_status", json!({})), ready);
    let board = h.ok("get_board", json!({ "date": today }));
    assert!(texts(&board["todos"]).is_empty(), "{board}");
    let kinds: Vec<_> = h
        .ok("list_backups", json!({}))
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["kind"]["type"].as_str().unwrap().to_owned())
        .collect();
    assert!(kinds.contains(&"pre_restore".to_owned()), "{kinds:?}");
    // 使えないバックアップは、何も変えずに断る
    let not_a_backup = base.join("nothing.db");
    std::fs::write(&not_a_backup, "not a database").unwrap();
    let err = h.err(
        "restore_backup",
        json!({ "backup": not_a_backup.display().to_string() }),
    );
    assert_eq!(err["kind"], "rejected", "{err}");
    h.assert_no_event();
    assert_eq!(h.ok("startup_status", json!({})), ready);

    // 保存先の変更：クラウド同期フォルダではない場所は、判定なし。移した後も開いたまま、データは同じ
    h.add_todo(&today, "moves along");
    h.drain();
    let moved = root.join("operations-moved");
    assert_eq!(
        h.ok(
            "check_data_dir",
            json!({ "dir": moved.display().to_string() })
        ),
        Value::Null
    );
    assert_eq!(
        h.ok(
            "change_data_dir",
            json!({ "dir": moved.display().to_string() })
        ),
        json!({ "leftovers": [] })
    );
    h.assert_no_event();
    assert_eq!(h.ok("startup_status", json!({})), ready);
    assert_eq!(
        h.ok("get_data_dir", json!({})),
        json!(moved.display().to_string())
    );
    assert!(moved.join("yunagi.db").is_file());
    assert!(!base.join("yunagi.db").exists());
    let board = h.ok("get_board", json!({ "date": today }));
    assert_eq!(texts(&board["todos"]), ["moves along"]);
    let config: Value =
        serde_json::from_str(&std::fs::read_to_string(default_dir.join("config.json")).unwrap())
            .unwrap();
    assert_eq!(config["data_dir"], moved.display().to_string());
    // 開き直した後も書き込める
    h.add_todo(&today, "after the move");

    // 使えない保存先は、理由を付けて断り、何も変えない
    for (dir, reason) in [
        (moved.display().to_string(), "same_directory"),
        (
            moved.join("backups").join("in").display().to_string(),
            "inside_backups",
        ),
        ("relative/path".to_owned(), "invalid_path"),
    ] {
        assert_eq!(
            h.err("change_data_dir", json!({ "dir": dir })),
            json!({ "kind": "rejected", "reason": reason }),
            "{dir}"
        );
    }
    assert_eq!(
        h.ok("get_data_dir", json!({})),
        json!(moved.display().to_string())
    );
    h.drain();

    // 選び直し：データベースの無いフォルダは断る。ある保存先を選び直すと、開き直してデータを読める
    for (dir, reason) in [
        (base.display().to_string(), "no_database"),
        (
            root.join("operations-none").display().to_string(),
            "not_a_directory",
        ),
    ] {
        assert_eq!(
            h.err("use_data_dir", json!({ "dir": dir })),
            json!({ "kind": "rejected", "reason": reason }),
            "{dir}"
        );
    }
    h.ok(
        "use_data_dir",
        json!({ "dir": moved.display().to_string() }),
    );
    assert_eq!(h.next_event(), changed_by("screen"));
    assert_eq!(h.ok("startup_status", json!({})), ready);
    let board = h.ok("get_board", json!({ "date": today }));
    assert_eq!(texts(&board["todos"]), ["moves along", "after the move"]);
    h.assert_no_event();
    drop(h);
    std::fs::remove_file(default_dir.join("config.json")).unwrap();
}

/// `db` のスキーマの番号を `version` にする（新しい版で作られたことにする）。
fn rusqlite_set_user_version(db: &Path, version: i64) {
    let conn = rusqlite::Connection::open(db).unwrap();
    conn.pragma_update(None, "user_version", version).unwrap();
    // WAL の中身を本体に書き戻してから閉じる（ファイルの中身で比べるため）
    conn.pragma_update(None, "journal_mode", "DELETE").unwrap();
}
