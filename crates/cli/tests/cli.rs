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
    let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
    let (mut stdin, mut stdout, mut stderr) = (&b""[..], Vec::new(), Vec::new());
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

fn main() {
    let tests: [(&str, Test); 14] = [
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
                    Conn::Answer(_) | Conn::DropAfterRequest => {
                        let _hello = read_json(&mut reader);
                        let hello = json!({"protocol": 1, "app_version": "9.9.9"});
                        writer
                            .write_all(wire::result_line(&json!(0), &hello).as_bytes())
                            .unwrap();
                        let request = read_json(&mut reader);
                        seen.lock().unwrap().push(request.clone());
                        if let Conn::Answer(result) = step {
                            writer
                                .write_all(wire::result_line(&request["id"], &result).as_bytes())
                                .unwrap();
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
