//! `agent setup` の流れの試験。実際の `claude`・`codex` は呼ばない：`claude` は、受け取った引数を記録する
//! 偽の実行ファイル（一時フォルダの `PATH` に置く）、Codex は一時フォルダの `CODEX_HOME` で試す。
//! 利用者の本物の設定には触れない（環境変数は `SetupEnv` で渡し、プロセスの環境は書き換えない）。

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::*;
use crate::connect::Timing;

struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        for sub in ["bin", "claude-config", "codex"] {
            fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        Self { dir }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// 偽の `claude`：引数を `calls.log` に 1 行ずつ記録する。`fail-add` があれば `mcp add` だけ失敗する。
    fn fake_claude(&self) {
        let script = "#!/bin/sh\ndir=$(dirname \"$0\")\necho \"$@\" >> \"$dir/calls.log\"\nif [ \"$1 $2\" = \"mcp add\" ] && [ -f \"$dir/fail-add\" ]; then echo \"MCP server yunagi already exists in user config\" >&2; exit 1; fi\nexit 0\n";
        let exe = self.path("bin/claude");
        fs::write(&exe, script).unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.path("bin/calls.log"))
            .map(|t| t.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }

    fn claude_json(&self, mcp_servers: &serde_json::Value) {
        fs::write(
            self.path("claude-config/.claude.json"),
            serde_json::json!({"numStartups": 1, "mcpServers": mcp_servers}).to_string(),
        )
        .unwrap();
    }

    fn setup(&self, interactive: bool) -> SetupEnv {
        SetupEnv {
            claude_config_dir: Some(self.path("claude-config")),
            codex_home: Some(self.path("codex")),
            path: Some(self.path("bin").into_os_string()),
            interactive,
            ..test_setup()
        }
    }

    fn files(&self, sub: &str) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.path(sub))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

struct Ran {
    code: i32,
    stdout: String,
    stderr: String,
}

fn go(setup: SetupEnv, claude: bool, codex: bool, input: &str) -> Ran {
    let env = Env {
        alt_home: AltHome::Fixed(None),
        timing: Timing::default(),
        setup,
    };
    let (mut stdin, mut stdout, mut stderr) = (input.as_bytes(), Vec::new(), Vec::new());
    let code = run(
        claude,
        codex,
        &env,
        &mut Io {
            stdin: &mut stdin,
            stdout: &mut stdout,
            stderr: &mut stderr,
        },
    );
    Ran {
        code,
        stdout: String::from_utf8(stdout).unwrap(),
        stderr: String::from_utf8(stderr).unwrap(),
    }
}

const ADD: &str =
    "mcp add --scope user --transport stdio yunagi -- yunagi mcp --client claude-code";

#[test]
fn without_a_terminal_the_changes_are_shown_and_nothing_is_written() {
    let f = Fixture::new();
    f.fake_claude();
    f.claude_json(&serde_json::json!({}));
    let r = go(f.setup(false), true, true, "y\n");
    assert_eq!(r.code, 3, "{}{}", r.stdout, r.stderr);
    assert!(r.stderr.contains("permission_denied"), "{}", r.stderr);
    assert!(r.stdout.contains("Changes to make:") && r.stdout.contains("claude mcp add"));
    assert!(r.stdout.contains("[mcp_servers.yunagi]"), "{}", r.stdout);
    // 何も書かず、`claude` も呼ばない（`y` を渡されても確認にならない）
    assert!(f.calls().is_empty());
    assert_eq!(f.files("codex"), Vec::<String>::new());
    assert_eq!(f.files("claude-config"), [".claude.json"]);
}

#[test]
fn declining_changes_nothing() {
    let f = Fixture::new();
    f.fake_claude();
    for answer in ["n\n", "\n", "yes please\n", ""] {
        let r = go(f.setup(true), true, true, answer);
        assert_eq!(r.code, 0, "{answer:?}");
        assert!(
            r.stdout.contains("Cancelled. Nothing was changed."),
            "{}",
            r.stdout
        );
        assert!(f.calls().is_empty());
        assert_eq!(f.files("codex"), Vec::<String>::new());
    }
}

#[test]
fn confirming_registers_both_with_a_reference_copy_and_no_env_values() {
    let f = Fixture::new();
    f.fake_claude();
    f.claude_json(&serde_json::json!({"other": {"command": "o"}}));
    let before = fs::read(f.path("claude-config/.claude.json")).unwrap();
    fs::write(f.path("codex/config.toml"), "# mine\nmodel = \"x\"\n").unwrap();
    let r = go(f.setup(true), false, false, "y\n");
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    assert_eq!(f.calls(), [ADD]);
    // Claude Code の設定ファイルは直接書かず、元の隣に参考のコピーを残す
    assert_eq!(
        fs::read(f.path("claude-config/.claude.json")).unwrap(),
        before
    );
    assert_eq!(
        fs::read(f.path("claude-config/.claude.json.yunagi-backup-20261104-120000")).unwrap(),
        before
    );
    let codex = fs::read_to_string(f.path("codex/config.toml")).unwrap();
    assert!(codex.starts_with("# mine\nmodel = \"x\"\n"), "{codex}");
    assert!(
        codex.contains("args = [\"mcp\", \"--client\", \"codex\"]"),
        "{codex}"
    );
    assert_eq!(
        fs::read_to_string(f.path("codex/config.toml.yunagi-backup-20261104-120000")).unwrap(),
        "# mine\nmodel = \"x\"\n"
    );
    assert!(r.stdout.contains("Claude Code: done.") && r.stdout.contains("Codex: done."));
    // 登録済みの内容が同じなら、もう一度実行しても何も変えず、確認も出さない
    f.claude_json(
        &serde_json::json!({"yunagi": {"type": "stdio", "command": "yunagi",
        "args": ["mcp", "--client", "claude-code"], "env": {}}}),
    );
    let again = go(f.setup(true), false, false, "");
    assert_eq!(again.code, 0, "{}{}", again.stdout, again.stderr);
    assert!(
        again.stdout.contains("already registered"),
        "{}",
        again.stdout
    );
    assert!(!again.stdout.contains("Apply these changes?"));
    assert_eq!(f.calls().len(), 1);
}

#[test]
fn a_different_registration_is_replaced_after_removing_it() {
    let f = Fixture::new();
    f.fake_claude();
    f.claude_json(
        &serde_json::json!({"yunagi": {"type": "stdio", "command": "yunagi",
        "args": ["mcp"], "env": {}}}),
    );
    let r = go(f.setup(true), true, false, "y\n");
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    assert!(
        r.stdout.contains("registered now: yunagi mcp"),
        "{}",
        r.stdout
    );
    assert_eq!(f.calls(), ["mcp remove --scope user yunagi", ADD]);
}

#[test]
fn a_failed_replacement_restores_the_old_registration() {
    let f = Fixture::new();
    f.fake_claude();
    fs::write(f.path("bin/fail-add"), "").unwrap();
    f.claude_json(
        &serde_json::json!({"yunagi": {"type": "stdio", "command": "yunagi",
        "args": ["mcp"], "env": {}}}),
    );
    let r = go(f.setup(true), true, false, "y\n");
    assert_eq!(r.code, 1);
    assert!(
        r.stderr.contains("The old one was restored"),
        "{}",
        r.stderr
    );
    let calls = f.calls();
    assert_eq!(calls.len(), 3, "{calls:?}");
    assert!(
        calls[2].starts_with("mcp add-json --scope user yunagi {"),
        "{calls:?}"
    );
}

#[test]
fn old_environment_values_are_never_put_on_a_command_line() {
    let f = Fixture::new();
    f.fake_claude();
    fs::write(f.path("bin/fail-add"), "").unwrap();
    f.claude_json(
        &serde_json::json!({"yunagi": {"type": "stdio", "command": "yunagi",
        "args": ["mcp"], "env": {"TOKEN": "secret-value"}}}),
    );
    let r = go(f.setup(true), true, false, "y\n");
    assert_eq!(r.code, 1);
    // 表示にも、コマンドの引数にも、値は出ない
    for text in [&r.stdout, &r.stderr, &f.calls().join("\n")] {
        assert!(!text.contains("secret-value"), "{text}");
    }
    assert_eq!(f.calls().len(), 2, "{:?}", f.calls());
    assert!(r.stdout.contains("TOKEN"), "{}", r.stdout);
}

#[test]
fn a_failed_add_for_an_unreadable_file_is_reported_as_it_is() {
    let f = Fixture::new();
    f.fake_claude();
    fs::write(f.path("bin/fail-add"), "").unwrap();
    fs::write(f.path("claude-config/.claude.json"), "{broken").unwrap();
    let r = go(f.setup(true), true, false, "y\n");
    assert_eq!(r.code, 1);
    assert!(
        r.stdout
            .contains("could not read the existing registration"),
        "{}",
        r.stdout
    );
    assert!(
        r.stderr.contains("already exists in user config"),
        "{}",
        r.stderr
    );
    // 取り除かない（黙って上書きしない）
    assert_eq!(f.calls(), [ADD]);
}

#[test]
fn without_the_claude_command_the_command_to_run_is_shown() {
    let f = Fixture::new();
    let r = go(f.setup(true), true, false, "");
    assert_eq!(r.code, 0);
    assert!(
        r.stdout.contains("`claude` command was not found"),
        "{}",
        r.stdout
    );
    assert!(
        r.stdout
            .contains("claude mcp add --scope user --transport stdio yunagi")
    );
    assert!(!r.stdout.contains("Apply these changes?"));
}

#[test]
fn only_the_named_agent_is_touched() {
    let f = Fixture::new();
    f.fake_claude();
    let r = go(f.setup(true), false, true, "y\n");
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    assert!(f.calls().is_empty());
    assert!(f.path("codex/config.toml").is_file());
    let r = go(f.setup(true), true, false, "y\n");
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    assert_eq!(f.calls(), [ADD]);
}

#[test]
fn nothing_is_registered_when_no_agent_is_found() {
    let dir = tempfile::tempdir().unwrap();
    let setup = SetupEnv {
        home: Some(dir.path().to_owned()),
        path: Some(dir.path().join("empty").into_os_string()),
        ..test_setup()
    };
    let r = go(setup, false, false, "y\n");
    assert_eq!(r.code, 4, "{}{}", r.stdout, r.stderr);
    assert!(r.stderr.contains("not_found"), "{}", r.stderr);
    assert!(fs::read_dir(dir.path()).unwrap().next().is_none());
}

#[test]
fn agents_are_detected_by_their_settings_too() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    fs::create_dir_all(home.join(".codex")).unwrap();
    let setup = SetupEnv {
        home: Some(home.to_owned()),
        path: None,
        ..test_setup()
    };
    assert!(codex::detected(&setup));
    assert!(!claude::detected(&setup));
    fs::write(Path::new(home).join(".claude.json"), "{}").unwrap();
    assert!(claude::detected(&setup));
    // `CODEX_HOME` の指定が先
    let moved = SetupEnv {
        codex_home: Some(home.join("elsewhere")),
        ..setup
    };
    assert!(!codex::detected(&moved));
}

#[test]
fn broken_toml_does_not_stop_the_other_agent() {
    let f = Fixture::new();
    f.fake_claude();
    fs::write(f.path("codex/config.toml"), "broken = [\n").unwrap();
    let r = go(f.setup(true), false, false, "y\n");
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("Codex: cannot continue"), "{}", r.stderr);
    assert_eq!(f.calls(), [ADD]);
    assert_eq!(
        fs::read_to_string(f.path("codex/config.toml")).unwrap(),
        "broken = [\n"
    );
    assert_eq!(f.files("codex"), ["config.toml"]);
}
