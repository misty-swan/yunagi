//! コンソール用バイナリ `yunagi` の本体（docs/design/01-architecture.md 2章、
//! docs/design/05-mcp-cli-spec.md）。
//!
//! - [`connect`]：ローカル API と直接モードの切り替え（アーキテクチャ設計書 4.4・4.5）。CLI と
//!   `yunagi mcp`（同じバイナリ）が共有する。
//! - [`args`]：引数の読み取りと、`TEXT` の `-`（標準入力）。
//! - [`output`]：テキスト出力と、エラーの表示。
//! - [`doctor`]：`yunagi doctor`。
//! - [`mcp`]：`yunagi mcp`（MCP サーバー。stdio）。ツールの定義は `mcp_tools`。
//! - [`agent_setup`]：`yunagi agent setup`（Claude Code・Codex への登録）。
//! - [`agent_admin`]：`yunagi agent permissions`・`agent revert`（権限の変更と取り消し。端末での確認つき）。
//!   `agent history` は読み取りなので、ふつうの操作として [`output`] が整える。
//! - [`error`]：エラー（05 4.3）と終了コード（05 4.1）。
//!
//! 実行ファイルの `main` は [`run`] を呼ぶだけにして、試験が同じプロセスの中で、入出力と環境を差し替えて
//! 動かせるようにしている。

pub mod agent_admin;
pub mod agent_setup;
pub mod args;
pub mod connect;
pub mod doctor;
pub mod error;
pub mod mcp;
mod mcp_tools;
pub mod output;

use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};

use args::{Command, Operation};
use connect::{AltHome, Connector, Timing};
use error::ErrorObject;

/// 入出力。
pub struct Io<'a> {
    pub stdin: &'a mut dyn BufRead,
    pub stdout: &'a mut dyn Write,
    pub stderr: &'a mut dyn Write,
}

/// 動く環境のうち、試験が差し替えるもの。
#[derive(Debug, Clone)]
pub struct Env {
    /// 別の候補の既定ディレクトリを求めるホーム。
    pub alt_home: AltHome,
    /// 接続の手順の時間。
    pub timing: Timing,
    /// `agent setup` が見る環境（エージェントの設定の場所・`PATH`・端末か）。
    pub setup: agent_setup::SetupEnv,
}

impl Default for Env {
    fn default() -> Self {
        Self {
            alt_home: AltHome::Os,
            timing: Timing::default(),
            setup: agent_setup::SetupEnv::from_process(),
        }
    }
}

/// コマンドを行い、終了コード（05 4.1）を返す。`args` はコマンド名を除いた引数。
pub fn run(args: &[String], env: &Env, io: &mut Io<'_>) -> i32 {
    let parsed = match args::parse(args) {
        Ok(parsed) => parsed,
        // 誤りの出し方は、読み取りの規則で決めた `--json` の指定に従う
        Err(e) => return fail(&e.error, e.json, io),
    };
    let json = parsed.json;
    match parsed.command {
        Command::Version => {
            let _ = writeln!(io.stdout, "yunagi {}", yunagi_core::VERSION);
            0
        }
        Command::Help => {
            let _ = io.stdout.write_all(args::USAGE.as_bytes());
            0
        }
        Command::Operation(op) => operation(op, json, env, io),
        Command::Doctor => run_doctor(json, env, io),
        Command::Mcp { client } => mcp::run(client.as_deref(), env, io),
        Command::AgentSetup { claude_code, codex } => agent_setup::run(claude_code, codex, env, io),
        Command::AgentPermissions {
            client,
            allow,
            deny,
        } => agent_admin::permissions(client.as_deref(), &allow, &deny, json, env, io),
        Command::AgentRevert { change_id, force } => {
            agent_admin::revert(&change_id, force, env, io)
        }
    }
}

/// 引数 `args` で `--json` が指定されているか（引数の読み取りと同じ規則。誤りがあっても読み終えて決める）。
pub fn json_requested(args: &[String]) -> bool {
    match args::parse(args) {
        Ok(parsed) => parsed.json,
        Err(e) => e.json,
    }
}

/// 引数が UTF-8 でないときの失敗（`invalid_argument`、終了コード 2）。`json` は、引数から `--json` を
/// 読めなかったときは `false`。
pub fn invalid_utf8_argument(json: bool, io: &mut Io<'_>) -> i32 {
    fail(
        &error::invalid_argument("an argument is not valid UTF-8"),
        json,
        io,
    )
}

/// 警告（環境を分けている形。4.5 の 3）をためておく入れ物。標準エラーには、操作の後にまとめて出す。
type Warnings = Arc<Mutex<Vec<String>>>;

/// 識別子 `client`（CLI は `cli`、`yunagi mcp` は `--client` の値か `unknown`）の接続の道具。
fn connector(env: &Env, warnings: &Warnings, client: &str) -> Result<Connector, ErrorObject> {
    let sink = Arc::clone(warnings);
    let connector = Connector::from_env(client, move |line| {
        sink.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(line.to_owned());
    })?;
    Ok(connector
        .with_alt_home(env.alt_home.clone())
        .with_timing(env.timing))
}

fn flush_warnings(warnings: &Warnings, io: &mut Io<'_>) {
    for line in warnings
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .drain(..)
    {
        let _ = writeln!(io.stderr, "{line}");
    }
}

fn operation(mut op: Operation, json: bool, env: &Env, io: &mut Io<'_>) -> i32 {
    // `-` の本文は、依頼を組み立て終えてから接続する（接続の前に標準入力を読む。05 4.1）
    if let Err(e) = op.read_stdin(io.stdin) {
        return fail(&e, json, io);
    }
    let Operation { method, params, .. } = op;
    let result = call_api(env, io, method, params);
    match result {
        Ok(value) => {
            if json {
                let _ = writeln!(io.stdout, "{value}");
            } else {
                let _ = io
                    .stdout
                    .write_all(output::render(method, &value).as_bytes());
            }
            0
        }
        Err(e) => fail(&e, json, io),
    }
}

/// 操作 `method` を `cli` クライアントとして行う（接続の手順は 05 4.3・アーキテクチャ設計書 4.4）。環境を
/// 分けている形の警告は、操作の後に標準エラーへ出す。
pub(crate) fn call_api(
    env: &Env,
    io: &mut Io<'_>,
    method: &'static str,
    params: serde_json::Map<String, Value>,
) -> Result<Value, ErrorObject> {
    ApiClient::new(env).call(io, method, params)
}

/// 1 回のコマンドの中で操作を何度か行うための接続の道具（`cli` クライアント）。環境を分けている形の
/// 警告（4.5 の 3）が、プロセスで 1 回だけ出るよう、道具と警告の入れ物を使い回す。
pub(crate) struct ApiClient {
    warnings: Warnings,
    connector: Result<Connector, ErrorObject>,
}

impl ApiClient {
    pub(crate) fn new(env: &Env) -> Self {
        let warnings = Warnings::default();
        let connector = connector(env, &warnings, "cli");
        Self {
            warnings,
            connector,
        }
    }

    pub(crate) fn call(
        &self,
        io: &mut Io<'_>,
        method: &'static str,
        params: serde_json::Map<String, Value>,
    ) -> Result<Value, ErrorObject> {
        let result = match &self.connector {
            Ok(c) => c.call(method, params),
            Err(e) => Err(e.clone()),
        };
        flush_warnings(&self.warnings, io);
        result
    }
}

fn run_doctor(json: bool, env: &Env, io: &mut Io<'_>) -> i32 {
    let warnings = Warnings::default();
    let report = match connector(env, &warnings, "cli") {
        Ok(c) => doctor::diagnose(&c),
        Err(e) => doctor::diagnose_without_dir(error::message(&e)),
    };
    flush_warnings(&warnings, io);
    if json {
        let _ = writeln!(io.stdout, "{report}");
    } else {
        let _ = io.stdout.write_all(doctor::render_text(&report).as_bytes());
    }
    0
}

/// エラーを出して、終了コードを返す。`--json` なら標準出力に `{"error": ...}`、そうでなければ標準エラーに文。
pub(crate) fn fail(error: &ErrorObject, json: bool, io: &mut Io<'_>) -> i32 {
    if json {
        let _ = writeln!(io.stdout, "{}", wrap(error));
    } else {
        let _ = io.stderr.write_all(output::render_error(error).as_bytes());
    }
    error::exit_code(error)
}

fn wrap(error: &ErrorObject) -> Value {
    json!({"error": error})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_args(args: &[&str]) -> (i32, String, String) {
        let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        let (mut stdin, mut out, mut err) = (&b""[..], Vec::new(), Vec::new());
        let code = run(
            &args,
            &Env::default(),
            &mut Io {
                stdin: &mut stdin,
                stdout: &mut out,
                stderr: &mut err,
            },
        );
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    /// GUI（src-tauri）の設定ファイル。読み込んだファイルが変わると、テストもビルドし直される。
    const TAURI_CONF: &str = include_str!("../../../src-tauri/tauri.conf.json");
    const APP_CARGO_TOML: &str = include_str!("../../../src-tauri/Cargo.toml");

    /// `yunagi` が表示するコアの版（yunagi_core::VERSION）が、`yunagi` 自身の版と一致すること。
    /// コア（crates/core/Cargo.toml）だけ版を直接書くと失敗する。
    #[test]
    fn version_line_matches_cli_version() {
        let (code, out, _) = run_args(&["--version"]);
        assert_eq!(code, 0);
        assert_eq!(out, format!("yunagi {}\n", env!("CARGO_PKG_VERSION")));
    }

    /// GUI の版も同じワークスペースの版になっていること。
    /// Tauri 2 は tauri.conf.json に "version" が無いと、アプリのクレート（src-tauri/Cargo.toml）の版を使う。
    /// どちらかに版を直接書くと GUI と `yunagi` で版が食い違いうるので、それを防ぐ。
    #[test]
    fn gui_uses_workspace_version() {
        assert!(
            !TAURI_CONF.contains("\"version\""),
            "src-tauri/tauri.conf.json に version を書かない（Cargo の版を使う）"
        );
        let package = APP_CARGO_TOML
            .split("\n[")
            .next()
            .expect("src-tauri/Cargo.toml の [package] が読めない");
        assert!(
            package
                .lines()
                .any(|line| line.trim() == "version.workspace = true"),
            "src-tauri/Cargo.toml の [package] は version.workspace = true にする"
        );
    }

    #[test]
    fn argument_mistakes_exit_with_2_in_both_formats() {
        let (code, out, err) = run_args(&["nothing"]);
        assert_eq!((code, out.as_str()), (2, ""));
        assert!(err.starts_with("yunagi: invalid_argument: unknown command: nothing"));
        let (code, out, err) = run_args(&["--json", "nothing"]);
        assert_eq!((code, err.as_str()), (2, ""));
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["error"]["code"], "invalid_argument");
    }

    #[test]
    fn help_goes_to_standard_output() {
        let (code, out, err) = run_args(&["--help"]);
        assert_eq!((code, err.as_str()), (0, ""));
        assert!(out.starts_with("Usage: yunagi"));
    }
}
