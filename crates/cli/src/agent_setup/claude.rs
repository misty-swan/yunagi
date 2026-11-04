//! Claude Code への登録（docs/design/05-mcp-cli-spec.md 6章）。
//!
//! 設定ファイル（`~/.claude.json`。`CLAUDE_CONFIG_DIR` があれば `$CLAUDE_CONFIG_DIR/.claude.json`）は、
//! Claude Code のほかの状態も入るので書き換えない。`claude mcp add --scope user --transport stdio yunagi --
//! yunagi mcp --client claude-code` を実行して登録する（`env` は付けない）。
//!
//! 登録済みかは、設定ファイルを**読むだけ**で調べる（最上位の `mcpServers.yunagi` が user スコープ）。
//! `claude mcp get` は、全スコープを見るうえ、サーバーを起動して接続を試すので使わない。同じ名前の
//! `claude mcp add` は、上書きせず終了コード 1 で断られる（2026-11-04、Claude Code 2.1.318 で確かめた）ので、
//! 内容が違うときは `claude mcp remove --scope user yunagi` で外してから付け直す。付け直しに失敗したら、
//! 元の登録を `claude mcp add-json` で戻す。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

use super::{Change, Prepared, SERVER_NAME, SetupEnv, files};

/// サーバーとして起動するコマンドと引数（登録する中身）。
const SERVER_COMMAND: &str = "yunagi";
const SERVER_ARGS: [&str; 3] = ["mcp", "--client", "claude-code"];

/// 利用者に案内する、登録のコマンド。
const ADD_COMMAND: &str =
    "claude mcp add --scope user --transport stdio yunagi -- yunagi mcp --client claude-code";

/// Claude Code を使っていると見てよいか：`claude` コマンドがあるか、設定ファイルがある。
pub fn detected(setup: &SetupEnv) -> bool {
    setup.find("claude").is_some() || config_path(setup).is_some_and(|p| p.is_file())
}

/// 設定ファイルの場所。`CLAUDE_CONFIG_DIR` を先に見る（2026-11-04 に確かめた）。
fn config_path(setup: &SetupEnv) -> Option<PathBuf> {
    setup
        .claude_config_dir
        .as_ref()
        .or(setup.home.as_ref())
        .map(|dir| dir.join(".claude.json"))
}

/// 設定ファイルの `mcpServers.yunagi` の状態。
#[derive(Debug, PartialEq)]
enum Registration {
    /// 無い。
    Absent,
    /// 同じ内容で登録してある。
    Same,
    /// 違う内容で登録してある（今の登録）。
    Different(Value),
    /// 読めない・形が違う（調べられない）。
    Unknown,
}

fn registration(config: Option<&Path>) -> Registration {
    let Some(config) = config else {
        return Registration::Absent;
    };
    let text = match fs::read_to_string(config) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Registration::Absent,
        Err(_) => return Registration::Unknown,
    };
    let Ok(root) = serde_json::from_str::<Value>(&text) else {
        return Registration::Unknown;
    };
    let servers = match root.get("mcpServers") {
        None => return Registration::Absent,
        Some(Value::Object(servers)) => servers,
        Some(_) => return Registration::Unknown,
    };
    match servers.get(SERVER_NAME) {
        None => Registration::Absent,
        Some(entry) if is_desired(entry) => Registration::Same,
        Some(entry) => Registration::Different(entry.clone()),
    }
}

/// 登録したい内容（`type` は無いか `stdio`、`env` は無いか空）と同じか。
fn is_desired(entry: &Value) -> bool {
    let kind_ok = entry
        .get("type")
        .is_none_or(|t| t.as_str() == Some("stdio"));
    let command_ok = entry.get("command").and_then(Value::as_str) == Some(SERVER_COMMAND);
    let args_ok = entry
        .get("args")
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().map(Value::as_str).collect::<Vec<_>>() == SERVER_ARGS.map(Some));
    let env_ok = match entry.get("env") {
        None => true,
        Some(Value::Object(env)) => env.is_empty(),
        Some(_) => false,
    };
    kind_ok && command_ok && args_ok && env_ok
}

/// 登録の中身を 1 行にしたもの（環境変数は名前だけ。値は出さない）。
fn summarize(entry: &Value) -> String {
    let command = entry.get("command").and_then(Value::as_str);
    let Some(command) = command else {
        return "(not a command-based server)".to_owned();
    };
    let args = entry
        .get("args")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    let mut line = format!("{command} {args}").trim_end().to_owned();
    if let Some(env) = entry.get("env").and_then(Value::as_object)
        && !env.is_empty()
    {
        let names: Vec<&str> = env.keys().map(String::as_str).collect();
        line.push_str(&format!(" (env: {}; values hidden)", names.join(", ")));
    }
    line
}

/// 変更内容を作る（何も書かない）。
pub fn prepare(setup: &SetupEnv) -> Prepared<Plan> {
    let config = config_path(setup);
    let Some(exe) = setup.find("claude") else {
        return Prepared::Manual(vec![
            "the `claude` command was not found, so nothing was registered.".to_owned(),
            "Run this where `claude` works:".to_owned(),
            format!("  {ADD_COMMAND}"),
        ]);
    };
    let action = match registration(config.as_deref()) {
        Registration::Same => {
            return Prepared::Unchanged(
                "already registered (user scope, same command). Nothing to change.".to_owned(),
            );
        }
        Registration::Different(old) => Action::Update(old),
        Registration::Absent | Registration::Unknown => Action::Add,
    };
    Prepared::Change(Plan {
        exe,
        unreadable: config
            .as_deref()
            .is_some_and(|c| c.is_file() && registration(Some(c)) == Registration::Unknown),
        config,
        action,
        stamp: setup.stamp(),
        yunagi_on_path: setup.find(SERVER_COMMAND).is_some(),
        allowlist_note: setup.xdg_data_home_set
            && setup.platform == yunagi_core::storage::data_dir::Platform::Linux,
    })
}

#[derive(Debug)]
enum Action {
    Add,
    Update(Value),
}

#[derive(Debug)]
pub struct Plan {
    exe: PathBuf,
    config: Option<PathBuf>,
    /// 設定ファイルがあるのに読めなかった（登録済みかが分からない）。
    unreadable: bool,
    action: Action,
    stamp: String,
    yunagi_on_path: bool,
    allowlist_note: bool,
}

impl Change for Plan {
    fn describe(&self) -> Vec<String> {
        let mut lines = Vec::new();
        match &self.action {
            Action::Add => lines.push(format!("add (user scope): {ADD_COMMAND}")),
            Action::Update(old) => {
                lines.push(format!("registered now: {}", summarize(old)));
                lines.push(format!(
                    "replace (user scope): `claude mcp remove --scope user yunagi`, then {ADD_COMMAND}"
                ));
            }
        }
        if self.unreadable {
            lines.push(
                "could not read the existing registration, so the add is tried as it is; if \
                 `yunagi` is already registered, Claude Code says so and nothing is changed."
                    .to_owned(),
            );
        }
        lines.push("no environment variable values are stored in the registration.".to_owned());
        if let Some(config) = &self.config
            && config.is_file()
        {
            lines.push(format!(
                "reference copy of {} before the change: {}.yunagi-backup-{} (Claude Code rewrites \
                 this file while it runs, so do not restore it as a whole; to undo, run \
                 `claude mcp remove --scope user yunagi`)",
                config.display(),
                config.display(),
                self.stamp
            ));
        }
        if !self.yunagi_on_path {
            lines.push(
                "note: `yunagi` is not on PATH in this terminal; the agent needs it on its PATH to start the server."
                    .to_owned(),
            );
        }
        if self.allowlist_note {
            lines.push(
                "note: if you start Claude Code with CLAUDE_CODE_MCP_ALLOWLIST_ENV=1 or \
                 CLAUDE_CODE_SUBPROCESS_ENV_SCRUB=1, XDG_DATA_HOME is not passed to `yunagi mcp`, \
                 and it can read a different data folder from the app."
                    .to_owned(),
            );
        }
        lines
    }

    fn apply(&self) -> Result<Vec<String>, String> {
        let mut done = Vec::new();
        if let Some(config) = &self.config
            && let Ok(bytes) = fs::read(config)
        {
            let backup = files::create_backup(config, &bytes, &self.stamp)
                .map_err(|e| format!("Could not make the backup, so nothing was changed: {e}"))?;
            done.push(format!("reference copy: {}", backup.display()));
        }
        let add = [
            "mcp",
            "add",
            "--scope",
            "user",
            "--transport",
            "stdio",
            SERVER_NAME,
            "--",
        ];
        let add: Vec<&str> = add
            .iter()
            .copied()
            .chain([SERVER_COMMAND])
            .chain(SERVER_ARGS)
            .collect();
        if let Action::Update(old) = &self.action {
            run(
                &self.exe,
                &["mcp", "remove", "--scope", "user", SERVER_NAME],
            )
            .map_err(|e| {
                format!("Could not remove the old registration, so nothing was changed. {e}")
            })?;
            if let Err(e) = run(&self.exe, &add) {
                return Err(self.restore(old, &e));
            }
            done.push("replaced the old registration".to_owned());
        } else {
            run(&self.exe, &add).map_err(|e| format!("`claude mcp add` failed. {e}"))?;
            done.push("registered with `claude mcp add`".to_owned());
        }
        done.push("undo: claude mcp remove --scope user yunagi".to_owned());
        Ok(done)
    }
}

impl Plan {
    /// 付け直しに失敗したとき、元の登録を戻す。伝える文を返す。
    fn restore(&self, old: &Value, cause: &str) -> String {
        let has_env_values = old
            .get("env")
            .and_then(Value::as_object)
            .is_some_and(|env| !env.is_empty());
        if has_env_values {
            // 環境変数の値を、コマンドの引数（プロセスの一覧から見える）に載せない
            return format!(
                "The new registration failed ({cause}) after the old one was removed. The old \
                 one has environment variables, so it was not restored automatically: add it \
                 again yourself (the reference copy of the settings file is next to it)."
            );
        }
        match run(
            &self.exe,
            &[
                "mcp",
                "add-json",
                "--scope",
                "user",
                SERVER_NAME,
                &old.to_string(),
            ],
        ) {
            Ok(()) => format!("The new registration failed ({cause}). The old one was restored."),
            Err(e) => format!(
                "The new registration failed ({cause}), and restoring the old one failed too \
                 ({e}). Run: {ADD_COMMAND}"
            ),
        }
    }
}

/// `claude` を、標準入力なしで実行する。失敗は、`claude` が出した文をそのまま返す。
fn run(exe: &Path, args: &[&str]) -> Result<(), String> {
    let output = Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Could not run {}: {e}", exe.display()))?;
    if output.status.success() {
        return Ok(());
    }
    let mut text = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !stdout.is_empty() {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(&stdout);
    }
    if text.is_empty() {
        text = format!("exit status {}", output.status);
    }
    Err(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_setup::test_setup;
    use serde_json::json;

    fn write_config(dir: &Path, value: &Value) -> PathBuf {
        let path = dir.join(".claude.json");
        fs::write(&path, value.to_string()).unwrap();
        path
    }

    #[test]
    fn registration_states_from_the_settings_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude.json");
        assert_eq!(registration(Some(&path)), Registration::Absent);
        assert_eq!(registration(None), Registration::Absent);

        write_config(dir.path(), &json!({"numStartups": 3}));
        assert_eq!(registration(Some(&path)), Registration::Absent);

        // `claude mcp add` が書く形（`env` は空の表）と、`type` が無い古い形は同じ内容
        let same = json!({"type": "stdio", "command": "yunagi",
            "args": ["mcp", "--client", "claude-code"], "env": {}});
        write_config(dir.path(), &json!({"mcpServers": {"yunagi": same}}));
        assert_eq!(registration(Some(&path)), Registration::Same);
        let old_form = json!({"command": "yunagi", "args": ["mcp", "--client", "claude-code"]});
        write_config(dir.path(), &json!({"mcpServers": {"yunagi": old_form}}));
        assert_eq!(registration(Some(&path)), Registration::Same);

        // 識別子・コマンド・環境変数・種類のどれかが違えば、違う登録
        for different in [
            json!({"type": "stdio", "command": "yunagi", "args": ["mcp"]}),
            json!({"type": "stdio", "command": "yunagi", "args": ["mcp", "--client", "codex"]}),
            json!({"type": "stdio", "command": "/opt/yunagi", "args": ["mcp", "--client", "claude-code"]}),
            json!({"type": "stdio", "command": "yunagi", "args": ["mcp", "--client", "claude-code"],
                "env": {"XDG_DATA_HOME": "/x"}}),
            json!({"type": "http", "url": "http://example.invalid"}),
        ] {
            write_config(
                dir.path(),
                &json!({"mcpServers": {"yunagi": different.clone()}}),
            );
            assert_eq!(
                registration(Some(&path)),
                Registration::Different(different)
            );
        }

        // 読めない・形が違うときは調べられない
        fs::write(&path, "{not json").unwrap();
        assert_eq!(registration(Some(&path)), Registration::Unknown);
        write_config(dir.path(), &json!({"mcpServers": []}));
        assert_eq!(registration(Some(&path)), Registration::Unknown);
    }

    #[test]
    fn config_dir_wins_over_home() {
        let mut setup = test_setup();
        setup.home = Some(PathBuf::from("/h"));
        assert_eq!(config_path(&setup), Some(PathBuf::from("/h/.claude.json")));
        setup.claude_config_dir = Some(PathBuf::from("/c"));
        assert_eq!(config_path(&setup), Some(PathBuf::from("/c/.claude.json")));
    }

    #[test]
    fn summaries_hide_environment_values() {
        let entry = json!({"command": "x", "args": ["a", "b"], "env": {"TOKEN": "secret"}});
        let line = summarize(&entry);
        assert!(line.contains("x a b") && line.contains("TOKEN"), "{line}");
        assert!(!line.contains("secret"), "{line}");
    }
}
