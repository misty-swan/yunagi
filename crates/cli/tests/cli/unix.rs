//! CLI の結合テスト（テスト計画書 4.5「CLI」）：直接モードの読み取り・権限・終了コード、接続の手順
//! （`busy`・`result_unknown`・認証の失敗・既定ディレクトリのずれ）、`doctor` を、同じプロセスの中で
//! `yunagi_cli::run` を呼んで確かめる。アプリ経由の試験は `src-tauri/tests/api.rs`。
//!
//! 既定ディレクトリは環境変数から求まるので、ほかの結合テストと同じく、テストの仕組み（harness）を使わず
//! `main` から 1 つずつ順に走らせる（Cargo.toml の `[[test]]`）。別の候補の既定ディレクトリを求める
//! ホームは [`AltHome::Fixed`] で一時フォルダにする（本物の `~/.local/share/yunagi` を調べない）。
//! ロックと UNIX ドメインソケットを使うので UNIX 系だけで走る。

#![cfg(unix)]

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use jiff::tz::TimeZone;
use serde_json::{Map, Value, json};
use yunagi_api::transport::Listener;
use yunagi_api::{token, wire};
use yunagi_cli::connect::{AltHome, Connector, Timing};
use yunagi_cli::{Env, Io};
use yunagi_core::domain::agent::Agent;
use yunagi_core::domain::{item, text_doc};
use yunagi_core::storage::data_dir::{self, DefaultDir};
use yunagi_core::storage::table::agent_client::{self, CLI_ID, Scope};
use yunagi_core::storage::{Database, DbLock};

/// 試験用の短い待ち（本番は 5 秒・50 ミリ秒・1 秒）。
const TIMING: Timing = Timing {
    wait: Duration::from_millis(600),
    interval: Duration::from_millis(20),
    connect: Duration::from_millis(300),
};

/// 試験の場所。環境変数 `XDG_DATA_HOME` を試験ごとに変える（1 つのスレッドで順に走らせるので安全）。
struct World {
    root: tempfile::TempDir,
}

impl World {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let w = Self { root };
        w.use_data_home("xdg");
        w
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }

    /// 自分の既定ディレクトリの元（`XDG_DATA_HOME`）を `name` の下にする。
    fn use_data_home(&self, name: &str) {
        let home = self.path("home");
        // SAFETY: 試験は 1 つのスレッドで順に走り、環境変数を読む処理は呼び出しの間だけ動く
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::set_var("XDG_DATA_HOME", self.path(name));
        }
    }

    /// 自分の既定ディレクトリ（`XDG_DATA_HOME/yunagi`）。
    fn own(&self) -> DefaultDir {
        data_dir::default_dir().unwrap()
    }

    /// 別の候補の既定ディレクトリ（`AltHome::Fixed` の下の `.local/share/yunagi`）。
    fn candidate_dir(&self) -> PathBuf {
        self.path("althome").join(".local/share/yunagi")
    }

    fn env(&self) -> Env {
        Env {
            alt_home: AltHome::Fixed(Some(self.path("althome"))),
            timing: TIMING,
        }
    }
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Out {
    fn json(&self) -> Value {
        serde_json::from_str(self.stdout.trim()).unwrap_or_else(|e| {
            panic!(
                "stdout is not JSON ({e}): {:?} / {:?}",
                self.stdout, self.stderr
            )
        })
    }
}

fn exec(w: &World, args: &[&str]) -> Out {
    exec_with_stdin(w, args, b"")
}

/// 標準入力 `input` を渡して動かす（`TEXT` の `-`。05 4.1）。
fn exec_with_stdin(w: &World, args: &[&str], input: &[u8]) -> Out {
    let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
    let (mut stdin, mut stdout, mut stderr) = (input, Vec::new(), Vec::new());
    let code = yunagi_cli::run(
        &args,
        &w.env(),
        &mut Io {
            stdin: &mut stdin,
            stdout: &mut stdout,
            stderr: &mut stderr,
        },
    );
    Out {
        code,
        stdout: String::from_utf8(stdout).unwrap(),
        stderr: String::from_utf8(stderr).unwrap(),
    }
}

fn connector(w: &World) -> Connector {
    Connector::from_env("cli", |_| {})
        .unwrap()
        .with_alt_home(AltHome::Fixed(Some(w.path("althome"))))
        .with_timing(TIMING)
}

/// 自分の既定ディレクトリを作り、データベースを作って、項目とノートを入れる。
fn seed(w: &World) -> DefaultDir {
    let dir = w.own();
    data_dir::create_default_dir(&dir).unwrap();
    let lock = DbLock::acquire(&dir, Duration::ZERO).unwrap();
    let db = Database::open(lock).unwrap();
    let conn = db.connection();
    agent_client::update_scopes(
        conn,
        CLI_ID,
        &[Scope::Read, Scope::Todo, Scope::Blocker, Scope::Append],
    )
    .unwrap();
    let cli = Agent::without_ui(CLI_ID);
    let now: jiff::Timestamp = "2026-11-02T01:00:00Z".parse().unwrap();
    let d = "2026-11-02".parse().unwrap();
    item::add_todo(conn, &cli, d, "rate limit survey", &TimeZone::UTC, now).unwrap();
    item::add_blocker(conn, &cli, d, "staging access", &TimeZone::UTC, now).unwrap();
    text_doc::append_note(
        conn,
        d,
        "intro\nthe rate limit is 600",
        Some("sync"),
        &cli,
        &TimeZone::UTC,
        now,
    )
    .unwrap();
    // 既定の権限（read のみ）に戻す
    agent_client::update_scopes(conn, CLI_ID, &[Scope::Read]).unwrap();
    db.close().unwrap();
    dir
}

fn set_cli_scopes(dir: &DefaultDir, scopes: &[Scope]) {
    let lock = DbLock::acquire(dir, Duration::ZERO).unwrap();
    let db = Database::open(lock).unwrap();
    agent_client::update_scopes(db.connection(), CLI_ID, scopes).unwrap();
    db.close().unwrap();
}

/// change_log の行（古い順）を `(operation, actor, agent_client_id)` で返す。
fn change_log(dir: &DefaultDir) -> Vec<(String, String, Option<String>)> {
    let lock = DbLock::acquire(dir, Duration::ZERO).unwrap();
    let db = Database::open(lock).unwrap();
    let rows = {
        let mut statement = db
            .connection()
            .prepare("SELECT operation, actor, agent_client_id FROM change_log ORDER BY rowid")
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    db.close().unwrap();
    rows
}

fn cli_last_seen(dir: &DefaultDir) -> Option<jiff::Timestamp> {
    let lock = DbLock::acquire(dir, Duration::ZERO).unwrap();
    let db = Database::open(lock).unwrap();
    let seen = agent_client::get(db.connection(), CLI_ID)
        .unwrap()
        .unwrap()
        .last_seen_at;
    db.close().unwrap();
    seen
}

/// 別のプロセスがロックを持っている状態にする（ファイルのロックはファイルを開いたものごとに働く）。
fn hold_lock(dir: &Path) -> File {
    fs::create_dir_all(dir).unwrap();
    let f = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("yunagi.lock"))
        .unwrap();
    f.lock().unwrap();
    f
}

type Test = fn(&World);

pub fn main() {
    let tests: [(&str, Test); 24] = [
        ("direct_mode_reads_as_the_cli_client", direct_mode_reads),
        ("reads_need_the_read_scope_first", read_scope),
        ("exit_codes_for_not_found_and_bad_arguments", exit_codes),
        ("the_lock_held_by_someone_else_gives_busy", busy),
        ("nothing_is_created_when_the_data_is_missing", missing_data),
        ("a_different_default_dir_in_use_is_an_error", mismatch),
        ("a_separate_environment_runs_with_a_warning", separate),
        (
            "doctor_reports_the_states_without_a_listener",
            doctor_states,
        ),
        ("a_too_long_socket_path_fails_fast_when_locked", too_long),
        ("a_lost_connection_is_not_resent_for_writes", lost_write),
        ("a_lost_read_is_retried_once", lost_read),
        ("authentication_failures_do_not_fall_back", auth_failures),
        ("a_listener_that_does_not_answer_gives_busy", not_answering),
        ("writes_in_direct_mode_follow_the_same_rules", direct_writes),
        (
            "write_commands_check_the_scope_before_the_item",
            write_scopes,
        ),
        ("write_commands_write_and_record_as_the_cli", write_commands),
        ("write_commands_read_the_text_from_stdin", write_stdin),
        (
            "a_write_does_not_run_in_a_mismatched_default_dir",
            write_mismatch,
        ),
        ("mcp_handshake_and_tool_list", mcp_handshake),
        ("mcp_tools_follow_the_clients_scopes", mcp_scopes),
        ("mcp_keeps_serving_after_connection_errors", mcp_errors),
        ("mcp_warns_once_per_process", mcp_warns_once),
        ("mcp_without_a_default_dir_answers_every_call", mcp_no_dir),
        ("mcp_passes_the_apps_errors_through", mcp_via_app),
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
        std::io::stdout().flush().unwrap();
        test(&World::new());
        println!("ok");
        passed += 1;
    }
    println!("\ntest result: ok. {passed} passed");
}

// ---- 直接モード ----

fn direct_mode_reads(w: &World) {
    let dir = seed(w);
    assert_eq!(cli_last_seen(&dir), None);

    let out = exec(w, &["board", "--date", "2026-11-02"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(out.stdout.contains("# 2026-11-02"));
    assert!(out.stdout.contains("rate limit survey"));
    assert!(out.stdout.contains("staging access"));
    assert!(out.stdout.contains("- sync") || out.stdout.contains("## Note"));
    assert_eq!(out.stderr, "");
    // 直接モードでも最後の接続の時刻を更新する
    assert!(cli_last_seen(&dir).is_some());

    let json = exec(w, &["--json", "board", "get", "--date", "2026-11-02"]).json();
    assert_eq!(json["date"], "2026-11-02");
    assert_eq!(json["todos"][0]["text"], "rate limit survey");
    assert_eq!(json["todos"][0]["origin"], "agent");
    assert!(
        json["note"]["headings"][0]
            .as_str()
            .unwrap()
            .ends_with("sync")
    );

    let open = exec(w, &["board", "open", "--date", "2026-11-02", "--json"]).json();
    assert!(open.get("reflection").is_none());
    assert_eq!(open["blockers"][0]["text"], "staging access");

    let note = exec(w, &["note", "get", "--date", "2026-11-02"]);
    assert_eq!(note.code, 0);
    assert!(note.stdout.contains("the rate limit is 600"));
    let json = exec(w, &["note", "show", "--date", "2026-11-02", "--json"]).json();
    assert!(json["body"].as_str().unwrap().contains("intro"));
    let heading = json!(json["body"].as_str().unwrap().lines().next().unwrap())
        .as_str()
        .unwrap()
        .trim_start_matches("## ")
        .to_owned();
    let section = exec(
        w,
        &[
            "note",
            "get",
            "--date",
            "2026-11-02",
            "--heading",
            &heading,
            "--json",
        ],
    )
    .json();
    assert_eq!(section["sections"][0]["start_line"], 1);

    let found = exec(w, &["search", "rate limit", "--json"]);
    assert_eq!(found.code, 0, "{}", found.stderr);
    let hits = found.json()["hits"].as_array().unwrap().clone();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0]["kind"], "todo");
    assert_eq!(hits[1]["kind"], "note");
    assert_eq!(hits[1]["matches"][0]["line"], 3);
    let text = exec(w, &["search", "rate limit", "--kind", "todo"]);
    assert_eq!(text.code, 0);
    assert!(text.stdout.contains("todo") && !text.stdout.contains("note:"));
    let none = exec(w, &["search", "zzzz"]);
    assert_eq!((none.code, none.stdout.as_str()), (0, "No results.\n"));

    // ロックは残らない
    drop(DbLock::acquire(&dir, Duration::ZERO).unwrap());
}

fn read_scope(w: &World) {
    let dir = seed(w);
    set_cli_scopes(&dir, &[Scope::Append]);
    for args in [
        &["board", "--date", "2026-11-02"][..],
        &["board", "open", "--date", "2026-11-02"],
        &["note", "get", "--date", "2026-11-02"],
        // 権限の誤りが、見出しの誤りや not_found より先に返る
        &[
            "note",
            "get",
            "--date",
            "2026-11-02",
            "--heading",
            "nothing",
        ],
        &["note", "get", "--heading", ""],
        &["search", "rate"],
    ] {
        let out = exec(w, args);
        assert_eq!(out.code, 3, "{args:?}: {}", out.stderr);
        assert!(out.stderr.contains("permission_denied") && out.stderr.contains("read"));
        let json = exec(w, &[args, &["--json"]].concat()).json();
        assert_eq!(json["error"]["code"], "permission_denied");
        assert_eq!(json["error"]["required_scope"], "read");
    }
    set_cli_scopes(&dir, &[Scope::Read]);
    assert_eq!(exec(w, &["board", "--date", "2026-11-02"]).code, 0);
}

fn exit_codes(w: &World) {
    seed(w);
    // 見出しが無い
    let out = exec(
        w,
        &[
            "note",
            "get",
            "--date",
            "2026-11-02",
            "--heading",
            "no such",
        ],
    );
    assert_eq!(out.code, 4, "{}", out.stderr);
    // 日付の形の誤りは、接続する前に invalid_argument
    let out = exec(w, &["board", "--date", "2026-13-40"]);
    assert_eq!(out.code, 2);
    assert!(out.stderr.contains("invalid_argument"));
    // 引数の誤り
    assert_eq!(exec(w, &["search"]).code, 2);
    assert_eq!(exec(w, &["search", "a", "--kind", "nothing"]).code, 2);
    let json = exec(w, &["--json", "board", "--bogus", "x"]).json();
    assert_eq!(json["error"]["code"], "invalid_argument");
}

fn busy(w: &World) {
    let dir = seed(w);
    let held = DbLock::acquire(&dir, Duration::ZERO).unwrap();
    let started = Instant::now();
    let out = exec(w, &["board", "--date", "2026-11-02"]);
    let took = started.elapsed();
    assert_eq!(out.code, 7, "{}", out.stderr);
    assert!(out.stderr.contains("busy"));
    // 期限まで試し直してから返す
    assert!(took >= TIMING.wait, "{took:?}");
    assert!(took < TIMING.wait + Duration::from_secs(2), "{took:?}");
    let json = exec(w, &["--json", "board"]).json();
    assert_eq!(json["error"]["code"], "busy");
    drop(held);
    assert_eq!(exec(w, &["board", "--date", "2026-11-02"]).code, 0);
}

fn missing_data(w: &World) {
    let own = w.own();
    // 既定ディレクトリが無い：作らず、探した場所と環境変数の名前を案内する
    let out = exec(w, &["board"]);
    assert_eq!(out.code, 6, "{}", out.stderr);
    assert!(out.stderr.contains("storage_unavailable"));
    assert!(out.stderr.contains(&own.path().display().to_string()));
    assert!(out.stderr.contains("XDG_DATA_HOME"));
    assert!(!own.path().exists());
    // フォルダはあるがデータベースが無い：新しく作らない
    fs::create_dir_all(own.path()).unwrap();
    let out = exec(w, &["--json", "search", "x"]);
    assert_eq!(out.code, 6);
    assert_eq!(out.json()["error"]["code"], "storage_unavailable");
    assert!(!own.path().join("yunagi.db").exists());
    // `config.json` が壊れていて保存先を求められないときも、開かず、作らない
    fs::write(own.path().join("config.json"), "not json").unwrap();
    let out = exec(w, &["board"]);
    assert_eq!(out.code, 6, "{}", out.stderr);
    assert!(!own.path().join("yunagi.db").exists());
}

fn mismatch(w: &World) {
    let own = w.own();
    let cand = w.candidate_dir();
    // アプリが標準の場所で動いている（ロックを持つ）。自分の既定ディレクトリは無い
    let held = hold_lock(&cand);
    let out = exec(w, &["board"]);
    assert_eq!(out.code, 8, "{}", out.stderr);
    assert!(out.stderr.contains("default_dir_mismatch"));
    let json = exec(w, &["--json", "board"]).json();
    assert_eq!(json["error"]["code"], "api_error");
    assert_eq!(json["error"]["reason"], "default_dir_mismatch");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(message.contains(&own.path().display().to_string()));
    assert!(message.contains(&cand.display().to_string()));
    // 自分の既定ディレクトリは作らず、別の候補にも何も増やさない
    assert!(!own.path().exists());
    let mut names: Vec<_> = fs::read_dir(&cand)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["yunagi.lock"]);

    // 自分の既定ディレクトリはあるが、データベースが無い：同じくずれ。ロックは離し、何も作らない
    fs::create_dir_all(own.path()).unwrap();
    let out = exec(w, &["search", "x"]);
    assert_eq!(out.code, 8);
    assert!(!own.path().join("yunagi.db").exists());
    drop(DbLock::acquire(&own, Duration::ZERO).unwrap());

    // 直接モードの doctor も同じ判定
    let doctor = exec(w, &["doctor", "--json"]).json();
    assert_eq!(doctor["local_api"]["status"], "default_dir_mismatch");
    assert_eq!(doctor["other_default_dir"]["status"], "in_use");

    // 環境をそろえる（自分の既定ディレクトリをアプリと同じにする）と、ずれではなくなる
    drop(held);
    let out = exec(w, &["board"]);
    assert_eq!(out.code, 6, "{}", out.stderr);
    let doctor = exec(w, &["doctor", "--json"]).json();
    assert_eq!(doctor["local_api"]["status"], "no_listener");
    assert_eq!(doctor["other_default_dir"]["status"], "absent");
}

fn separate(w: &World) {
    seed(w);
    let cand = w.candidate_dir();
    let _held = hold_lock(&cand);
    // 候補で動いているプロセスがいても、自分の側に別のデータベースがあれば止めない
    let out = exec(w, &["board", "--date", "2026-11-02"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(out.stdout.contains("rate limit survey"));
    assert_eq!(out.stderr.lines().count(), 1, "{}", out.stderr);
    assert!(out.stderr.contains(&cand.display().to_string()));
    // --json の標準出力は変えない
    let json = exec(w, &["board", "--date", "2026-11-02", "--json"]);
    assert_eq!(json.code, 0);
    assert_eq!(json.json()["date"], "2026-11-02");
    let doctor = exec(w, &["doctor", "--json"]).json();
    assert_eq!(doctor["other_default_dir"]["status"], "in_use_separate");
    assert_eq!(doctor["local_api"]["status"], "no_listener");
}

// ---- doctor ----

fn doctor_states(w: &World) {
    let own = seed(w);
    let out = exec(w, &["doctor"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    for expected in [
        "version:",
        "os:",
        "default directory:",
        "XDG_DATA_HOME",
        "data directory:",
        "local API: no_listener",
    ] {
        assert!(out.stdout.contains(expected), "{expected}: {}", out.stdout);
    }
    let report = exec(w, &["doctor", "--json"]).json();
    assert_eq!(report["version"], yunagi_core::VERSION);
    assert_eq!(report["local_api"]["status"], "no_listener");
    assert_eq!(report["default_dir"]["path"], own.path().to_str().unwrap());
    assert_eq!(
        report["default_dir"]["env_vars"],
        json!(["XDG_DATA_HOME", "HOME"])
    );
    assert_eq!(report["data_dir"]["database_exists"], true);
    assert_eq!(report["other_default_dir"]["status"], "absent");
    // 業務データの内容を含まない
    let text = report.to_string();
    assert!(!text.contains("rate limit") && !text.contains("survey"));
    // ロックを残さない
    drop(DbLock::acquire(&own, Duration::ZERO).unwrap());

    // 別のプロセスがロックを持っていて、待ち受けが無い
    let held = DbLock::acquire(&own, Duration::ZERO).unwrap();
    let started = Instant::now();
    let report = exec(w, &["doctor", "--json"]).json();
    assert_eq!(report["local_api"]["status"], "starting_or_unresponsive");
    // 長く待たない（待つのはロックではなく、一時的につながらないときの 1 秒まで）
    assert!(started.elapsed() < Duration::from_secs(2));
    drop(held);

    // 候補にデータだけある
    fs::create_dir_all(w.candidate_dir()).unwrap();
    fs::write(w.candidate_dir().join("yunagi.db"), b"x").unwrap();
    let report = exec(w, &["doctor", "--json"]).json();
    assert_eq!(report["other_default_dir"]["status"], "has_data");
    assert_eq!(report["local_api"]["status"], "no_listener");
    // 警告もエラーも出ず、従来どおり成功する
    let out = exec(w, &["board", "--date", "2026-11-02"]);
    assert_eq!((out.code, out.stderr.as_str()), (0, ""));

    // 環境変数が無い：既定ディレクトリを求められない
    // SAFETY: 1 つのスレッドで順に走らせている
    unsafe {
        std::env::remove_var("XDG_DATA_HOME");
        std::env::remove_var("HOME");
    }
    let out = exec(w, &["board"]);
    assert_eq!(out.code, 6);
    assert!(out.stderr.contains("XDG_DATA_HOME") && out.stderr.contains("HOME"));
    let report = exec(w, &["doctor", "--json"]);
    assert_eq!(report.code, 0);
    assert!(report.json()["default_dir"]["error"].is_string());
    assert!(report.json()["local_api"].is_null());
}

fn too_long(w: &World) {
    // 既定ディレクトリのパスが長すぎて、ソケットのパスが収まらない
    let long = "a".repeat(120);
    w.use_data_home(&long);
    let own = w.own();
    assert!(yunagi_api::socket_path(&own).as_os_str().len() > 107);
    fs::create_dir_all(own.path()).unwrap();
    let held = DbLock::acquire(&own, Duration::ZERO).unwrap();
    let started = Instant::now();
    let out = exec(w, &["board"]);
    assert_eq!(out.code, 8, "{}", out.stderr);
    assert!(out.stderr.contains("socket_path_too_long"));
    // 待たずにすぐ返す
    assert!(started.elapsed() < TIMING.wait, "{:?}", started.elapsed());
    let report = exec(w, &["doctor", "--json"]).json();
    assert_eq!(report["local_api"]["status"], "socket_path_too_long");
    drop(held);
    // ロックが取れれば、直接モードで処理する（データベースが無いので保存先の案内になる）
    assert_eq!(exec(w, &["board"]).code, 6);
    let report = exec(w, &["doctor", "--json"]).json();
    assert_eq!(report["local_api"]["status"], "no_listener");
}

// ---- 偽のアプリ（接続の手順の試験） ----

/// 1 つの接続での、偽のアプリの振る舞い。
#[derive(Clone)]
enum Conn {
    /// `hello` に答え、依頼を受けて `result` を返す。
    Answer(Value),
    /// `hello` に答え、依頼を受けて、アプリのエラー（`error.data.error` にこのオブジェクト）を返す。
    AppError(Value),
    /// `hello` に答え、依頼を受け取ってから、答えずに閉じる。
    DropAfterRequest,
    /// `hello` に失敗（`reason`）を返す。
    HelloError(&'static str),
    /// 何も答えない（`hello` を読まない）。
    Silent,
}

struct FakeApp {
    requests: Arc<Mutex<Vec<Value>>>,
    handle: thread::JoinHandle<()>,
}

impl FakeApp {
    fn start(dir: &DefaultDir, script: Vec<Conn>) -> Self {
        let listener = Listener::open(dir, &token::generate()).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        let handle = thread::spawn(move || {
            for step in script {
                let stream = listener.accept().unwrap();
                let mut reader = BufReader::new(&stream);
                let mut writer = &stream;
                match step {
                    Conn::Silent => thread::sleep(Duration::from_millis(1500)),
                    Conn::HelloError(reason) => {
                        let _ = read_json(&mut reader);
                        let line = wire::app_error_line(
                            &json!(0),
                            &json!({"code": "api_error", "reason": reason, "message": "no"}),
                        );
                        let _ = writer.write_all(line.as_bytes());
                    }
                    Conn::Answer(_) | Conn::AppError(_) | Conn::DropAfterRequest => {
                        let _hello = read_json(&mut reader);
                        let hello = json!({"protocol": 1, "app_version": "9.9.9"});
                        writer
                            .write_all(wire::result_line(&json!(0), &hello).as_bytes())
                            .unwrap();
                        let request = read_json(&mut reader);
                        seen.lock().unwrap().push(request.clone());
                        match step {
                            Conn::Answer(result) => writer
                                .write_all(wire::result_line(&request["id"], &result).as_bytes())
                                .unwrap(),
                            Conn::AppError(error) => writer
                                .write_all(wire::app_error_line(&request["id"], &error).as_bytes())
                                .unwrap(),
                            _ => {}
                        }
                    }
                }
            }
        });
        Self { requests, handle }
    }

    /// 受け取った依頼（`method` の一覧）。
    fn finish(self) -> Vec<String> {
        self.handle.join().unwrap();
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|r| r["method"].as_str().unwrap().to_owned())
            .collect()
    }
}

fn read_json(reader: &mut impl BufRead) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

fn board_result() -> Value {
    json!({"date": "2026-11-02", "todos": [], "blockers": [], "reflection": "", "note": {"headings": [], "length": 0}})
}

fn params(pairs: &[(&str, &str)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), json!(v)))
        .collect()
}

fn lost_write(w: &World) {
    let own = w.own();
    data_dir::create_default_dir(&own).unwrap();
    let app = FakeApp::start(&own, vec![Conn::DropAfterRequest]);
    // 書き込みは送り直さず、反映されたか分からないエラーを返す
    let e = connector(w)
        .call("add_todo", params(&[("text", "once")]))
        .unwrap_err();
    assert_eq!(e["code"], "api_error");
    assert_eq!(e["reason"], "result_unknown");
    assert_eq!(app.finish(), ["add_todo"]);
    // コマンドからでも同じ：終了コード 8 で、テキストにも reason が出る。依頼は 1 回だけ送られる
    let app = FakeApp::start(&own, vec![Conn::DropAfterRequest]);
    let out = exec(w, &["todo", "add", "once"]);
    assert_eq!(out.code, 8, "{}", out.stderr);
    assert!(out.stderr.contains("api_error (result_unknown)"));
    assert_eq!(app.finish(), ["add_todo"]);
}

fn lost_read(w: &World) {
    let own = w.own();
    data_dir::create_default_dir(&own).unwrap();
    // 読み取りは、手順 1 から 1 回だけやり直す
    let app = FakeApp::start(
        &own,
        vec![Conn::DropAfterRequest, Conn::Answer(board_result())],
    );
    let out = exec(w, &["board", "--json"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.json()["date"], "2026-11-02");
    assert_eq!(app.finish(), ["get_board", "get_board"]);
    // やり直しでも切れたら、同じエラー
    let app = FakeApp::start(&own, vec![Conn::DropAfterRequest, Conn::DropAfterRequest]);
    let out = exec(w, &["board"]);
    assert_eq!(out.code, 8);
    assert!(out.stderr.contains("result_unknown"));
    assert_eq!(app.finish(), ["get_board", "get_board"]);
}

fn auth_failures(w: &World) {
    let own = seed(w);
    // 待ち受けはあるが使えない：直接モードに切り替えない（データベースには触れない）
    for (reason, shown) in [
        ("auth_failed", "auth_failed"),
        ("version_mismatch", "version_mismatch"),
    ] {
        let app = FakeApp::start(&own, vec![Conn::HelloError(reason)]);
        let out = exec(w, &["board", "--date", "2026-11-02"]);
        assert_eq!(out.code, 8, "{}", out.stderr);
        assert!(out.stderr.contains(shown), "{}", out.stderr);
        assert!(app.finish().is_empty());
    }
}

fn not_answering(w: &World) {
    let own = seed(w);
    // 接続は受けるが認証に答えない（固まっている）：まだ何も送っていないので試し直し、期限で busy。
    // 直接モードには切り替えない
    let script = vec![Conn::Silent; 8];
    let app = FakeApp::start(&own, script);
    let started = Instant::now();
    let out = exec(w, &["board", "--date", "2026-11-02"]);
    assert_eq!(out.code, 7, "{}", out.stderr);
    assert!(started.elapsed() >= TIMING.wait);
    // 偽のアプリは決めた数だけ接続を受けるまで終わらないので、待たずに手放す（プロセスの終わりで止まる）
    drop(app);
}

// ---- 直接モードの書き込み ----

fn direct_writes(w: &World) {
    let dir = seed(w);
    let c = connector(w);
    // todo スコープが無ければ permission_denied
    let e = c.call("add_todo", params(&[("text", "x")])).unwrap_err();
    assert_eq!(e["code"], "permission_denied");
    assert_eq!(e["required_scope"], "todo");
    set_cli_scopes(&dir, &[Scope::Read, Scope::Todo]);
    let added = c
        .call(
            "add_todo",
            params(&[("text", "  from cli  "), ("date", "2026-11-02")]),
        )
        .unwrap();
    assert_eq!(added["text"], "from cli");
    let id = added["id"].as_str().unwrap().to_owned();
    // 同じ本文の編集は何も変えない成功
    let same = c
        .call("edit_todo", params(&[("id", &id), ("text", "from cli")]))
        .unwrap();
    assert_eq!(same["updated"], false);
    let done = c.call("complete_todo", params(&[("id", &id)])).unwrap();
    assert_eq!(
        (done["updated"].as_bool(), done["status"].as_str()),
        (Some(true), Some("done"))
    );
    // ID の形の誤りは、接続の前に invalid_argument
    let e = c
        .call("complete_todo", params(&[("id", "nope")]))
        .unwrap_err();
    assert_eq!(e["code"], "invalid_argument");
    // 知らない操作
    assert_eq!(
        c.call("nothing", Map::new()).unwrap_err()["code"],
        "internal"
    );
    drop(DbLock::acquire(&dir, Duration::ZERO).unwrap());
}

// ---- 書き込みのコマンド（YUN-91） ----

/// 存在しないが、形は正しい ID。
const MISSING_ID: &str = "01JZ0000000000000000000000";

fn write_scopes(w: &World) {
    let dir = seed(w);
    let before = change_log(&dir);
    // どの書き込みも、既定（read のみ）では permission_denied（終了コード 3）で、必要なスコープが出る。
    // スコープは項目を見る前に確かめるので、存在しない ID でも not_found にならない
    let cases: [(&[&str], &str); 11] = [
        (&["todo", "add", "x"], "todo"),
        (&["todo", "edit", MISSING_ID, "x"], "todo"),
        (&["todo", "done", MISSING_ID], "todo"),
        (&["todo", "reopen", MISSING_ID], "todo"),
        (&["blocker", "add", "x"], "blocker"),
        (&["blocker", "edit", MISSING_ID, "x"], "blocker"),
        (&["blocker", "resolve", MISSING_ID], "blocker"),
        (&["reflection", "append", "x"], "append"),
        (&["note", "append", "x", "--heading"], "append"),
        (
            &["note", "edit", "--line", "1", "--expect", "x", "--delete"],
            "note_edit",
        ),
        // 本文が空でも、権限の誤りが先
        (&["todo", "add", ""], "todo"),
    ];
    for (args, scope) in cases {
        let out = exec(w, args);
        assert_eq!(out.code, 3, "{args:?}: {}", out.stderr);
        assert!(
            out.stderr.contains(&format!("[required scope: {scope}]")),
            "{args:?}: {}",
            out.stderr
        );
        let json = exec(w, &[args, &["--json"]].concat()).json();
        assert_eq!(json["error"]["code"], "permission_denied");
        assert_eq!(json["error"]["required_scope"], scope);
    }
    // 権限の誤りでは何も記録しない
    assert_eq!(change_log(&dir), before);
    // ID の形の誤りと日付の形の誤りは、権限より先（直してやり直すしかないため。05 3.2）
    assert_eq!(exec(w, &["todo", "done", "nope"]).code, 2);
    assert_eq!(
        exec(w, &["todo", "add", "x", "--date", "2026-13-40"]).code,
        2
    );
}

/// 結果の JSON の欄 `key`（文字列）。
fn field(out: &Out, key: &str) -> String {
    out.json()[key].as_str().unwrap().to_owned()
}

fn write_commands(w: &World) {
    let dir = seed(w);
    set_cli_scopes(
        &dir,
        &[
            Scope::Read,
            Scope::Todo,
            Scope::Blocker,
            Scope::Append,
            Scope::NoteEdit,
        ],
    );
    let baseline = change_log(&dir).len();
    let day = ["--date", "2026-11-02"];
    let run = |args: &[&str]| exec(w, &[args, &day].concat());

    // TODO
    let added = run(&["todo", "add", "  write the spec  "]);
    assert_eq!(added.code, 0, "{}", added.stderr);
    let id = {
        let out = run(&["todo", "add", "second", "--json"]);
        assert_eq!(out.json()["date"], "2026-11-02");
        field(&out, "id")
    };
    assert!(added.stdout.starts_with("Added TODO "));
    assert!(added.stdout.ends_with(" (2026-11-02).\n"));
    let out = exec(w, &["todo", "edit", &id, "second"]);
    assert_eq!(
        (out.code, out.stdout.as_str()),
        (0, "Same text. Nothing changed.\n")
    );
    let out = exec(w, &["todo", "edit", &id, "second (edited)"]);
    assert_eq!((out.code, out.stdout.as_str()), (0, "Edited.\n"));
    let out = exec(w, &["todo", "done", &id]);
    assert_eq!((out.code, out.stdout.as_str()), (0, "Completed.\n"));
    let out = exec(w, &["todo", "complete", &id]);
    assert_eq!(out.stdout, "Already done. Nothing changed.\n");
    let json = exec(w, &["todo", "complete", &id, "--json"]).json();
    assert_eq!(
        (json["updated"].as_bool(), json["status"].as_str()),
        (Some(false), Some("done"))
    );
    assert_eq!(exec(w, &["todo", "reopen", &id]).stdout, "Reopened.\n");
    assert_eq!(
        exec(w, &["todo", "reopen", &id]).stdout,
        "Already open. Nothing changed.\n"
    );
    // 存在しない ID は not_found（終了コード 4）
    assert_eq!(exec(w, &["todo", "done", MISSING_ID]).code, 4);

    // 障害：種類の違う ID は invalid_argument と、使う操作
    let blocker = field(&run(&["blocker", "add", "waiting", "--json"]), "id");
    let out = exec(w, &["todo", "done", &blocker]);
    assert_eq!(out.code, 2, "{}", out.stderr);
    assert!(
        out.stderr.contains("[use yunagi blocker resolve]"),
        "{}",
        out.stderr
    );
    let out = exec(w, &["blocker", "resolve", &id]);
    assert!(
        out.stderr.contains("[use yunagi todo complete]"),
        "{}",
        out.stderr
    );
    assert_eq!(
        exec(w, &["blocker", "edit", &blocker, "waiting (edited)"]).stdout,
        "Edited.\n"
    );
    assert_eq!(
        exec(w, &["blocker", "resolve", &blocker, "--note", "fixed"]).stdout,
        "Resolved. Note added.\n"
    );
    // やり直しは何も変えない成功。別の解消メモは conflict（note_exists。終了コード 5）で、既存の本文が見える
    assert_eq!(
        exec(w, &["blocker", "resolve", &blocker, "--note", "fixed"]).stdout,
        "Already resolved. Nothing changed.\n"
    );
    let out = exec(w, &["blocker", "resolve", &blocker, "--note", "other"]);
    assert_eq!(out.code, 5, "{}", out.stderr);
    assert!(out.stderr.contains("conflict (note_exists)"));
    assert!(out.stderr.contains("[existing resolution note: fixed]"));

    // 振り返りとノート
    assert_eq!(
        run(&["reflection", "append", "went well"]).stdout,
        "Appended.\n"
    );
    assert_eq!(
        run(&["reflect", "append", "second line"]).stdout,
        "Appended.\n"
    );
    let board = run(&["board", "--json"]).json();
    assert_eq!(board["reflection"], "went well\nsecond line");
    assert_eq!(
        run(&["note", "append", "memo line", "--heading", "retro"]).stdout,
        "Appended.\n"
    );
    let note = run(&["note", "get", "--json"]).json();
    let lines: Vec<&str> = note["body"].as_str().unwrap().split('\n').collect();
    let memo = lines.iter().position(|l| *l == "memo line").unwrap() + 1;
    assert!(lines[memo - 2].ends_with(" retro"), "{lines:?}");
    // 行を書き換える。--expect が違えば conflict（changed）
    let line = memo.to_string();
    let out = run(&[
        "note",
        "edit",
        "--line",
        &line,
        "--expect",
        "not this",
        "--replace",
        "x",
    ]);
    assert_eq!(out.code, 5, "{}", out.stderr);
    assert!(out.stderr.contains("conflict (changed)"));
    let out = run(&[
        "note",
        "edit",
        "--line",
        &line,
        "--expect",
        "memo line",
        "--replace",
        "memo line",
    ]);
    assert_eq!(out.stdout, "Nothing changed.\n");
    let out = run(&[
        "note",
        "edit",
        "--line",
        &line,
        "--expect",
        "memo line",
        "--replace",
        "memo (fixed)",
    ]);
    assert_eq!(out.stdout, "Edited.\n");
    let out = run(&[
        "note",
        "edit",
        "--line",
        &line,
        "--expect=memo (fixed)",
        "--delete",
    ]);
    assert_eq!(out.stdout, "Edited.\n");
    assert!(!run(&["note", "get"]).stdout.contains("memo"));
    // 本文の誤りはコアが invalid_argument にする（行番号 0）
    let out = run(&["note", "edit", "--line", "0", "--expect", "x", "--delete"]);
    assert_eq!(out.code, 2, "{}", out.stderr);

    // 記録：変えた操作だけが、CLI クライアントの agent として残る（何も変えなかった成功は残さない）
    let log = change_log(&dir);
    let ops: Vec<&str> = log[baseline..].iter().map(|r| r.0.as_str()).collect();
    assert_eq!(
        ops,
        [
            "todo.add",
            "todo.add",
            "todo.edit",
            "todo.complete",
            "todo.reopen",
            "blocker.add",
            "blocker.edit",
            "blocker.resolve",
            "reflection.append",
            "reflection.append",
            "note.append",
            "note.edit",
            "note.edit",
        ]
    );
    let cli = Some(CLI_ID.to_string());
    for row in &log[baseline..] {
        assert_eq!((row.1.as_str(), &row.2), ("agent", &cli), "{row:?}");
    }
    drop(DbLock::acquire(&dir, Duration::ZERO).unwrap());
}

fn write_stdin(w: &World) {
    let dir = seed(w);
    set_cli_scopes(
        &dir,
        &[Scope::Read, Scope::Todo, Scope::Blocker, Scope::Append],
    );
    // `-` は標準入力から読む。末尾の改行は、コアが前後の空白として除く（やり直しが同じ本文になる）
    let out = exec_with_stdin(
        w,
        &["todo", "add", "-", "--date", "2026-11-02", "--json"],
        "from stdin\n".as_bytes(),
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.json()["text"], "from stdin");
    let id = field(&out, "id");
    let out = exec_with_stdin(w, &["todo", "edit", &id, "-"], "from stdin\n".as_bytes());
    assert_eq!(out.stdout, "Same text. Nothing changed.\n");
    let out = exec_with_stdin(
        w,
        &["reflection", "append", "-", "--date", "2026-11-02"],
        "git commit subject\n".as_bytes(),
    );
    assert_eq!(out.stdout, "Appended.\n");
    let blocker = field(&exec(w, &["blocker", "add", "b", "--json"]), "id");
    let out = exec_with_stdin(
        w,
        &["blocker", "resolve", &blocker, "--note", "-"],
        b"fixed\n",
    );
    assert_eq!(out.stdout, "Resolved. Note added.\n");
    // 標準入力が UTF-8 でない・大きすぎる：接続の前に invalid_argument（ロックを別のプロセスが持っていても
    // busy にならず、すぐ返る）
    let held = hold_lock(dir.path());
    let started = Instant::now();
    let out = exec_with_stdin(w, &["todo", "add", "-"], &[0xff, 0xfe]);
    assert_eq!(out.code, 2, "{}", out.stderr);
    let big = vec![b'a'; yunagi_cli::args::MAX_STDIN_TEXT + 1];
    let out = exec_with_stdin(w, &["todo", "add", "-"], &big);
    assert_eq!(out.code, 2, "{}", out.stderr);
    assert!(started.elapsed() < TIMING.wait);
    drop(held);
}

fn write_mismatch(w: &World) {
    let own = w.own();
    let cand = w.candidate_dir();
    // アプリが標準の場所で動いている。自分の既定ディレクトリは無い：書き込みは何も書かずに断る
    let held = hold_lock(&cand);
    for args in [
        &["todo", "add", "x"][..],
        &["blocker", "resolve", MISSING_ID],
        &["note", "append", "x"],
    ] {
        let out = exec(w, args);
        assert_eq!(out.code, 8, "{args:?}: {}", out.stderr);
        assert!(
            out.stderr.contains("default_dir_mismatch"),
            "{}",
            out.stderr
        );
    }
    assert!(!own.path().exists());
    drop(held);
}

// ---- MCP サーバー（YUN-92） ----

/// `yunagi mcp` の 1 回の実行。
struct McpRun {
    code: i32,
    /// 標準出力の行（どれも JSON のメッセージでなければならない）。
    responses: Vec<Value>,
    stderr: String,
}

/// `yunagi mcp <args>` に `lines`（1 行 1 メッセージ）を渡して、入力が終わるまで動かす。
fn mcp_run(w: &World, args: &[&str], lines: &[Value]) -> McpRun {
    let mut input = lines
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    input.push('\n');
    let mut full = vec!["mcp"];
    full.extend_from_slice(args);
    let out = exec_with_stdin(w, &full, input.as_bytes());
    let responses = out
        .stdout
        .lines()
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("stdout has a line that is not JSON ({e}): {line:?}"))
        })
        .collect();
    McpRun {
        code: out.code,
        responses,
        stderr: out.stderr,
    }
}

fn rpc(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

fn tool_call(id: u64, name: &str, arguments: Value) -> Value {
    rpc(
        id,
        "tools/call",
        json!({"name": name, "arguments": arguments}),
    )
}

/// ツールの結果を `(isError, 中身)` にする。中身は text の JSON。
fn tool_outcome(response: &Value) -> (bool, Value) {
    let result = &response["result"];
    let text = result["content"][0]["text"].as_str().unwrap_or_else(|| {
        panic!("no text content: {response}");
    });
    (
        result["isError"].as_bool().unwrap(),
        serde_json::from_str(text).unwrap(),
    )
}

fn client_row(dir: &DefaultDir, name: &str) -> agent_client::AgentClient {
    let lock = DbLock::acquire(dir, Duration::ZERO).unwrap();
    let db = Database::open(lock).unwrap();
    let client = agent_client::get_by_name(db.connection(), name)
        .unwrap()
        .unwrap_or_else(|| panic!("{name} is not registered"));
    db.close().unwrap();
    client
}

fn set_client_scopes(dir: &DefaultDir, name: &str, scopes: &[Scope]) {
    let id = client_row(dir, name).id;
    let lock = DbLock::acquire(dir, Duration::ZERO).unwrap();
    let db = Database::open(lock).unwrap();
    agent_client::update_scopes(db.connection(), id, scopes).unwrap();
    db.close().unwrap();
}

fn mcp_handshake(w: &World) {
    seed(w);
    let init = rpc(
        1,
        "initialize",
        json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}),
    );
    let meta = json!({"io.modelcontextprotocol/protocolVersion": "2026-07-28"});
    let run = mcp_run(
        w,
        &["--client", "claude-code"],
        &[
            init,
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            rpc(2, "tools/list", json!({})),
            rpc(3, "server/discover", json!({"_meta": meta})),
            rpc(4, "ping", json!({})),
        ],
    );
    // 終了コードは 0、標準エラーには何も出ない。通知には答えない
    assert_eq!((run.code, run.stderr.as_str()), (0, ""));
    assert_eq!(run.responses.len(), 4);
    assert_eq!(run.responses[0]["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(run.responses[0]["result"]["serverInfo"]["name"], "yunagi");
    let names: Vec<&str> = run.responses[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 14);
    for name in [
        "get_board",
        "open_board",
        "get_note",
        "search",
        "add_todo",
        "edit_todo",
        "complete_todo",
        "reopen_todo",
        "add_blocker",
        "edit_blocker",
        "resolve_blocker",
        "append_reflection",
        "append_note",
        "edit_note",
    ] {
        assert!(names.contains(&name), "{name}");
    }
    // 導入（A-07）と権限・履歴（A-08）は MCP に出さない
    assert!(!names.iter().any(|n| n.starts_with("agent")));
    assert_eq!(
        run.responses[2]["result"]["supportedVersions"],
        json!(["2026-07-28"])
    );
    assert_eq!(run.responses[3]["result"], json!({}));
    // 標準入力が閉じられたら終わる：何も渡さなくても 0
    let empty = exec_with_stdin(w, &["mcp"], b"");
    assert_eq!((empty.code, empty.stdout.as_str()), (0, ""));
    // 識別子の形の誤りは、起動のときに終了コード 2 で断る（標準出力には何も出さない）
    let bad = exec_with_stdin(w, &["mcp", "--client", "Bad Name"], b"");
    assert_eq!((bad.code, bad.stdout.as_str()), (2, ""));
    assert!(bad.stderr.contains("invalid_argument"), "{}", bad.stderr);
}

fn mcp_scopes(w: &World) {
    let dir = seed(w);
    let reads = [
        tool_call(1, "get_board", json!({"date": "2026-11-02"})),
        tool_call(
            2,
            "add_todo",
            json!({"text": "from mcp", "date": "2026-11-02"}),
        ),
        tool_call(3, "search", json!({"query": "rate limit"})),
        tool_call(4, "get_note", json!({"date": "2026-11-02"})),
        tool_call(5, "open_board", json!({"date": "2026-11-02"})),
    ];
    // 識別子を付けた初めての接続は、read だけで登録される。書き込みは permission_denied
    let run = mcp_run(w, &["--client", "claude-code"], &reads);
    assert_eq!((run.code, run.stderr.as_str()), (0, ""));
    let (is_error, board) = tool_outcome(&run.responses[0]);
    assert!(!is_error);
    assert_eq!(board["todos"][0]["text"], "rate limit survey");
    assert_eq!(board["todos"][0]["carried_to_id"], Value::Null);
    let (is_error, denied) = tool_outcome(&run.responses[1]);
    assert!(is_error);
    assert_eq!(denied["error"]["code"], "permission_denied");
    assert_eq!(denied["error"]["required_scope"], "todo");
    // search の結果の項目に carried_to_id・carried_to_date が入る
    let (_, found) = tool_outcome(&run.responses[2]);
    assert_eq!(found["hits"][0]["kind"], "todo");
    assert_eq!(found["hits"][0]["carried_to_id"], Value::Null);
    assert_eq!(found["hits"][0]["carried_to_date"], Value::Null);
    assert!(!tool_outcome(&run.responses[3]).0);
    assert!(!tool_outcome(&run.responses[4]).0);
    let client = client_row(&dir, "claude-code");
    assert_eq!(client.scopes, [Scope::Read]);
    assert!(client.last_seen_at.is_some());
    // 識別子の無い接続は unknown クライアント：読み取りだけ
    let run = mcp_run(w, &[], &reads);
    assert!(!tool_outcome(&run.responses[0]).0);
    let (is_error, denied) = tool_outcome(&run.responses[1]);
    assert!(is_error);
    assert_eq!(denied["error"]["code"], "permission_denied");
    let before = change_log(&dir).len();
    // 利用者が許可すると書き込める。MCP の書き込みは、そのクライアントとして change_log に残る
    set_client_scopes(&dir, "claude-code", &[Scope::Read, Scope::Todo]);
    let run = mcp_run(
        w,
        &["--client", "claude-code"],
        &[tool_call(
            1,
            "add_todo",
            json!({"text": "from mcp", "date": "2026-11-02"}),
        )],
    );
    let (is_error, added) = tool_outcome(&run.responses[0]);
    assert!(!is_error, "{added}");
    let id = added["id"].as_str().unwrap().to_owned();
    let run = mcp_run(
        w,
        &["--client", "claude-code"],
        &[
            tool_call(1, "complete_todo", json!({"id": id})),
            tool_call(2, "complete_todo", json!({"id": id})),
            tool_call(3, "edit_todo", json!({"id": "nope", "text": "x"})),
            tool_call(4, "edit_todo", json!({"id": id, "text": "x", "extra": 1})),
            tool_call(5, "reopen_todo", json!({"id": MISSING_ID})),
        ],
    );
    assert_eq!(tool_outcome(&run.responses[0]).1["updated"], true);
    // 何も変えない成功（やり直してよい）
    let (is_error, again) = tool_outcome(&run.responses[1]);
    assert!(!is_error);
    assert_eq!(again["updated"], false);
    // 引数の誤りは、ツールのエラー（invalid_argument）
    let (is_error, bad_id) = tool_outcome(&run.responses[2]);
    assert!(is_error);
    assert_eq!(bad_id["error"]["code"], "invalid_argument");
    let (is_error, unknown_arg) = tool_outcome(&run.responses[3]);
    assert!(is_error);
    assert_eq!(unknown_arg["error"]["code"], "invalid_argument");
    assert_eq!(
        tool_outcome(&run.responses[4]).1["error"]["code"],
        "not_found"
    );
    let rows = change_log(&dir);
    assert_eq!(rows.len(), before + 2, "{rows:?}");
    let id_text = client.id.to_string();
    assert_eq!(rows[before].0, "todo.add");
    assert_eq!(rows[before + 1].0, "todo.complete");
    for row in &rows[before..] {
        assert_eq!(row.2.as_deref(), Some(id_text.as_str()), "{row:?}");
    }
}

fn mcp_errors(w: &World) {
    let dir = seed(w);
    let calls = [
        tool_call(1, "get_board", json!({"date": "2026-11-02"})),
        rpc(2, "ping", json!({})),
        tool_call(3, "add_todo", json!({"text": "x"})),
    ];
    // ロックを別のプロセスが持っている：busy は書き込みも読み取りもツールのエラーで、サーバーは続く
    let held = DbLock::acquire(&dir, Duration::ZERO).unwrap();
    let started = Instant::now();
    let run = mcp_run(w, &["--client", "claude-code"], &calls);
    assert!(started.elapsed() >= TIMING.wait * 2);
    assert_eq!(run.code, 0);
    assert_eq!(run.responses.len(), 3);
    for i in [0, 2] {
        let (is_error, e) = tool_outcome(&run.responses[i]);
        assert!(is_error);
        assert_eq!(e["error"]["code"], "busy");
    }
    assert_eq!(run.responses[1]["result"], json!({}));
    drop(held);
    // 解放すれば、次の接続で同じ依頼が通る
    let run = mcp_run(w, &["--client", "claude-code"], &calls[..1]);
    assert!(!tool_outcome(&run.responses[0]).0);

    // 既定ディレクトリのずれ：default_dir_mismatch もツールのエラーで、サーバーは続く
    w.use_data_home("xdg-other");
    let cand = w.candidate_dir();
    let _app = hold_lock(&cand);
    let run = mcp_run(w, &["--client", "claude-code"], &calls);
    assert_eq!(run.code, 0);
    assert_eq!(run.responses.len(), 3);
    for i in [0, 2] {
        let (is_error, e) = tool_outcome(&run.responses[i]);
        assert!(is_error);
        assert_eq!(e["error"]["code"], "api_error");
        assert_eq!(e["error"]["reason"], "default_dir_mismatch");
    }
    assert_eq!(run.responses[1]["result"], json!({}));
}

fn mcp_warns_once(w: &World) {
    seed(w);
    let cand = w.candidate_dir();
    let _app = hold_lock(&cand);
    // 環境を分けている形：止めずに、警告は標準エラーにプロセスごとに 1 回（呼び出しごとではない）
    let run = mcp_run(
        w,
        &["--client", "claude-code"],
        &[
            tool_call(1, "get_board", json!({"date": "2026-11-02"})),
            tool_call(2, "get_board", json!({"date": "2026-11-02"})),
        ],
    );
    assert_eq!(run.code, 0);
    assert!(run.responses.iter().all(|r| !tool_outcome(r).0));
    assert_eq!(run.stderr.lines().count(), 1, "{}", run.stderr);
    assert!(run.stderr.contains(&cand.display().to_string()));
}

fn mcp_no_dir(w: &World) {
    // 既定ディレクトリを求める環境変数が無い：終了せず、ツールの呼び出しのたびに storage_unavailable
    // SAFETY: 試験は 1 つのスレッドで順に走る
    unsafe {
        std::env::remove_var("XDG_DATA_HOME");
        std::env::remove_var("HOME");
    }
    let run = mcp_run(
        w,
        &["--client", "claude-code"],
        &[
            tool_call(1, "get_board", json!({})),
            rpc(2, "ping", json!({})),
            tool_call(3, "get_board", json!({})),
        ],
    );
    w.use_data_home("xdg");
    assert_eq!(run.code, 0);
    assert_eq!(run.responses.len(), 3);
    for i in [0, 2] {
        let (is_error, e) = tool_outcome(&run.responses[i]);
        assert!(is_error);
        assert_eq!(e["error"]["code"], "storage_unavailable");
        assert!(
            e["error"]["message"]
                .as_str()
                .unwrap()
                .contains("XDG_DATA_HOME")
        );
    }
    assert_eq!(run.stderr.lines().count(), 1, "{}", run.stderr);
}

fn mcp_via_app(w: &World) {
    let own = w.own();
    data_dir::create_default_dir(&own).unwrap();
    let denied = json!({"code": "permission_denied", "required_scope": "todo", "message": "This agent is not allowed to modify TODOs."});
    let app = FakeApp::start(
        &own,
        vec![
            Conn::AppError(denied.clone()),
            Conn::DropAfterRequest,
            Conn::Answer(board_result()),
        ],
    );
    let run = mcp_run(
        w,
        &["--client", "claude-code"],
        &[
            tool_call(1, "add_todo", json!({"text": "x"})),
            tool_call(2, "add_todo", json!({"text": "x"})),
            rpc(3, "ping", json!({})),
            tool_call(4, "get_board", json!({})),
        ],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    // アプリのエラー（error.data.error）はそのままツールのエラーに載る
    assert_eq!(
        tool_outcome(&run.responses[0]),
        (true, json!({"error": denied}))
    );
    // 依頼を送った後に切れた書き込みは送り直さず、反映されたか分からないエラー。サーバーは続く
    let (is_error, e) = tool_outcome(&run.responses[1]);
    assert!(is_error);
    assert_eq!(e["error"]["reason"], "result_unknown");
    assert_eq!(run.responses[2]["result"], json!({}));
    assert_eq!(tool_outcome(&run.responses[3]).1["date"], "2026-11-02");
    assert_eq!(app.finish(), ["add_todo", "add_todo", "get_board"]);
}
