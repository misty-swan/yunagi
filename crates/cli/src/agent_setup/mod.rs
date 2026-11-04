//! `yunagi agent setup`（A-07。docs/design/05-mcp-cli-spec.md 6章）：検出したエージェントの設定に
//! MCP サーバー（`yunagi mcp --client <識別子>`）を登録する。
//!
//! - `claude`：Claude Code。設定ファイル（`~/.claude.json`）は直接書かず、`claude mcp add` を実行して登録する。
//! - `codex`：Codex。`config.toml` の `[mcp_servers.yunagi]` だけを、コメントと並びを保ったまま書き換える。
//! - `files`：バックアップ（元のファイルの隣・上書きしない）と、壊れにくい書き込み。
//!
//! 流れは、検出 → 変更内容を作る（何も書かない）→ 警告と変更内容を表示 → 利用者の確認 → 書き込み。
//! 確認は対話端末（TTY）でしか得られない：エージェントのシェルなど、端末でない実行では、変更内容を
//! 表示して `permission_denied`（終了コード 3）で終わり、何も書かない（05 2章と同じ考え方）。
//! 登録するコマンドには環境変数の値を含めない（05 6章）。

mod claude;
mod codex;
mod files;

use std::ffi::OsString;
use std::io::IsTerminal;
use std::path::PathBuf;

use yunagi_core::storage::data_dir::{self, Platform};
use yunagi_core::storage::dir_mismatch::{self, CandidateStatus};

use crate::connect::AltHome;
use crate::error;
use crate::{Env, Io};

/// 登録する MCP サーバーの名前（両方のエージェントで同じ）。
const SERVER_NAME: &str = "yunagi";
/// 登録するサーバーのコマンド（`PATH` で探される名前。両方のエージェントで同じ）。
const SERVER_COMMAND: &str = "yunagi";
/// `yunagi` が `PATH` に無いときの注意（変更内容の表示）。
const PATH_NOTE: &str = "note: `yunagi` is not on PATH in this terminal; the agent needs it on its PATH to start the server.";

/// 動く環境のうち、`agent setup` が見るもの（試験が差し替える）。
#[derive(Debug, Clone)]
pub struct SetupEnv {
    pub platform: Platform,
    /// 利用者のホーム（`HOME`。Windows は `USERPROFILE`）。
    pub home: Option<PathBuf>,
    /// `CLAUDE_CONFIG_DIR`（Claude Code の設定の置き場所の指定）。
    pub claude_config_dir: Option<PathBuf>,
    /// `CODEX_HOME`（Codex の設定の置き場所の指定）。
    pub codex_home: Option<PathBuf>,
    /// `PATH`（`claude`・`codex`・`yunagi` を探す）。
    pub path: Option<OsString>,
    /// 確認を得られる端末か。
    pub interactive: bool,
    /// `XDG_DATA_HOME` が設定されているか（値は読まない）。
    pub xdg_data_home_set: bool,
    /// バックアップの名前に使う日時（`20261104-120000` の形）。試験が決める。無ければ今の時刻。
    pub stamp: Option<String>,
}

impl SetupEnv {
    /// 動いているプロセスの環境から作る。
    pub fn from_process() -> Self {
        let platform = Platform::current();
        let var = |name: &str| {
            std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Self {
            platform,
            home: var(if platform == Platform::Windows {
                "USERPROFILE"
            } else {
                "HOME"
            }),
            claude_config_dir: var("CLAUDE_CONFIG_DIR"),
            codex_home: var("CODEX_HOME"),
            path: std::env::var_os("PATH"),
            // 質問と変更内容は標準出力に出るので、入力と出力の両方が端末のときだけ確認を得る
            // （`> log.txt` や `| tee` で質問が見えないまま、答えを待たないため）
            interactive: std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
            xdg_data_home_set: var("XDG_DATA_HOME").is_some(),
            stamp: None,
        }
    }

    fn stamp(&self) -> String {
        self.stamp
            .clone()
            .unwrap_or_else(|| jiff::Zoned::now().strftime("%Y%m%d-%H%M%S").to_string())
    }

    fn find(&self, name: &str) -> Option<PathBuf> {
        files::find_in_path(name, self.path.as_deref(), self.platform)
    }

    /// 既定ディレクトリを求める環境変数の名前（05 6章。コアの `default_dir_vars`）。
    fn dir_vars(&self) -> &'static [&'static str] {
        dir_mismatch::default_dir_vars(self.platform)
    }
}

/// 1 つのエージェントの変更内容を作った結果。
enum Prepared<P> {
    /// すでに同じ内容で登録してある。何もしない。
    Unchanged(String),
    /// 自動では登録できない（`claude` コマンドが無い）。利用者が実行するコマンドの案内。
    Manual(Vec<String>),
    /// 変更内容。
    Change(P),
    /// 続けられない（設定ファイルを読めない・TOML として壊れているなど）。何も書かない。
    Failed(String),
}

/// 変更内容を作って、確認を得て、書く。終了コードを返す。
pub fn run(want_claude: bool, want_codex: bool, env: &Env, io: &mut Io<'_>) -> i32 {
    let setup = &env.setup;
    let explicit = want_claude || want_codex;
    let (claude_on, codex_on) = if explicit {
        (want_claude, want_codex)
    } else {
        (claude::detected(setup), codex::detected(setup))
    };
    if !claude_on && !codex_on {
        return crate::fail(
            &error::not_found(
                "No supported agent was found (Claude Code: the `claude` command or its settings; \
                 Codex: the `codex` command or its settings folder). Install one, or name the \
                 agent to set up anyway with --claude-code or --codex.",
            ),
            false,
            io,
        );
    }

    let mut failed = false;
    // 名前を指定されたのに、自動では登録できなかった（`claude` が無い）
    let mut not_registered = false;
    let mut changes: Vec<(&'static str, Box<dyn Change>)> = Vec::new();
    if claude_on {
        match claude::prepare(setup) {
            Prepared::Unchanged(line) => out(io, &format!("Claude Code: {line}")),
            Prepared::Manual(lines) => {
                not_registered = explicit;
                for (i, line) in lines.iter().enumerate() {
                    out(
                        io,
                        &format!("{}{line}", if i == 0 { "Claude Code: " } else { "  " }),
                    );
                }
            }
            Prepared::Change(plan) => changes.push(("Claude Code", Box::new(plan))),
            Prepared::Failed(msg) => {
                failed = true;
                err(
                    io,
                    &format!(
                        "Claude Code: cannot continue. {msg} Nothing was changed for Claude Code."
                    ),
                );
            }
        }
    }
    if codex_on {
        match codex::prepare(setup) {
            Prepared::Unchanged(line) => out(io, &format!("Codex: {line}")),
            Prepared::Manual(lines) => {
                not_registered = explicit;
                for line in lines {
                    out(io, &format!("Codex: {line}"));
                }
            }
            Prepared::Change(plan) => changes.push(("Codex", Box::new(plan))),
            Prepared::Failed(msg) => {
                failed = true;
                err(
                    io,
                    &format!("Codex: cannot continue. {msg} Nothing was changed for Codex."),
                );
            }
        }
    }
    // 終了コード：失敗は 1。名前を指定されたのに登録できなかったときは 4（`not_found`。スクリプトが
    // 登録済みと取り違えないため）。登録済み・取りやめは 0
    let exit_code = |failed: bool| {
        if failed {
            1
        } else if not_registered {
            4
        } else {
            0
        }
    };
    if changes.is_empty() {
        return exit_code(failed);
    }

    // 変更内容の表示（書き込む前）。環境変数の値は出さない
    for line in directory_warnings(env) {
        out(io, &line);
    }
    out(io, "");
    out(io, "Changes to make:");
    for (name, change) in &changes {
        out(io, &format!("- {name}"));
        for line in change.describe() {
            out(io, &format!("    {line}"));
        }
    }
    if !setup.interactive {
        return crate::fail(
            &error::permission_denied(
                "Confirmation needs a terminal. Nothing was changed. Run `yunagi agent setup` \
                 yourself in a terminal.",
            ),
            false,
            io,
        );
    }
    let _ = write!(io.stdout, "\nApply these changes? [y/N] ");
    let _ = io.stdout.flush();
    let mut answer = String::new();
    // 入力が閉じている・読めないときは、「断った」ではなく失敗にする（呼び出した側が区別できるように）
    match io.stdin.read_line(&mut answer) {
        Ok(0) => {
            out(io, "");
            err(
                io,
                "No answer was given (the input is closed). Nothing was changed.",
            );
            return 1;
        }
        Err(e) => {
            out(io, "");
            err(
                io,
                &format!("Could not read the answer ({e}). Nothing was changed."),
            );
            return 1;
        }
        Ok(_) => {}
    }
    let confirmed = matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes");
    if !confirmed {
        out(io, "Cancelled. Nothing was changed.");
        return 0;
    }

    for (name, change) in &changes {
        match change.apply() {
            Ok(lines) => {
                out(io, &format!("{name}: done."));
                for line in lines {
                    out(io, &format!("    {line}"));
                }
            }
            Err(msg) => {
                failed = true;
                err(io, &format!("{name}: failed. {msg}"));
            }
        }
    }
    exit_code(failed)
}

/// 1 つのエージェントへの変更。
trait Change {
    /// 変更内容の表示（1 行ずつ）。
    fn describe(&self) -> Vec<String>;
    /// 書き込む。成功の後に伝える行（バックアップの場所や、取り消し方）を返す。失敗は利用者に伝える文。
    fn apply(&self) -> Result<Vec<String>, String>;
}

fn out(io: &mut Io<'_>, line: &str) {
    let _ = writeln!(io.stdout, "{line}");
}

fn err(io: &mut Io<'_>, line: &str) {
    let _ = writeln!(io.stderr, "{line}");
}

/// 既定ディレクトリのずれの警告（05 6章、アーキテクチャ設計書 4.5）。書き込む前に調べ、ずれていれば
/// （別の候補を使っているプロセスがいる、または自分の既定ディレクトリが無い）変更内容の表示に添える。
/// 登録するコマンドには環境変数の値を含めないので、アプリとエージェントを同じ環境で起動するよう案内する。
fn directory_warnings(env: &Env) -> Vec<String> {
    let vars = env.setup.dir_vars().join(" / ");
    let own = match data_dir::default_dir() {
        Ok(own) => own,
        Err(_) => {
            return vec![format!(
                "Warning: Yunagi's default folder cannot be worked out in this terminal ({vars} \
                 is missing), so it cannot be compared with the one the app uses."
            )];
        }
    };
    let state = dir_mismatch::own_storage(&own);
    let candidate = match &env.alt_home {
        AltHome::Os => dir_mismatch::alternative(&own),
        AltHome::Fixed(home) => {
            dir_mismatch::alternative_from_home(&own, env.setup.platform, home.clone())
        }
    }
    .map(|c| c.probe());
    let in_use = candidate.filter(|c| c.status == CandidateStatus::InUse);
    let reason = match (&in_use, state.default_dir_exists) {
        (Some(c), _) => format!(
            "Yunagi seems to be running with another default folder ({}).",
            c.dir.display()
        ),
        (None, false) => "That folder does not exist, so Yunagi has not run here.".to_owned(),
        (None, true) => return Vec::new(),
    };
    vec![
        format!(
            "Warning: this terminal's default folder for Yunagi is {} (from {vars}). {reason}",
            own.path().display()
        ),
        format!(
            "  The registration does not store {vars}. Start Yunagi and the agent with the same \
             values (for example XDG_DATA_HOME), or the agent reads different data."
        ),
    ]
}

/// 試験用の環境（何も見つからない。確認は得られる）。
#[cfg(test)]
fn test_setup() -> SetupEnv {
    SetupEnv {
        platform: Platform::Linux,
        home: None,
        claude_config_dir: None,
        codex_home: None,
        path: None,
        interactive: true,
        xdg_data_home_set: false,
        stamp: Some("20261104-120000".to_owned()),
    }
}

#[cfg(test)]
mod tests;
