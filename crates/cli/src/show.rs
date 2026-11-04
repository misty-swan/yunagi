//! `yunagi show [--note]`：アプリの窓を前面に出す（docs/design/05-mcp-cli-spec.md 3.1、
//! docs/design/01-architecture.md 4.4）。Wayland で OS 全体のショートカットが効かないときに、
//! デスクトップ環境のキーボード設定から呼ぶ。
//!
//! - **ローカル API だけを使う**。窓はアプリにしか無いので、直接モードは無い（[`crate::connect::Connector`]
//!   の接続の手順は使わない）。ローカル API の操作 `show_app`（[`yunagi_api::show`]）を頼む。
//! - 待ち受けが無ければ、アプリを起動する（1 回だけ）。起動した後は、待ち受けが開くまで、接続の手順 1
//!   と同じ試し方で [`Options::start_wait`]（30 秒）まで試し直す。データベースを開くには、マイグレーション
//!   などで数秒かかることがある。
//! - 起動したアプリは、この実行が終わっても動き続けるよう切り離す（標準入出力は閉じ、端末の
//!   プロセスグループから外す）。
//! - アプリの探し方は [`locate`]。見つからなければ何も起動せず `api_error`（`app_not_found`）。
//! - 待ち受けはあるが使えない（認証・版・権限）は、接続の手順と同じ `api_error`。
//!
//! 2 つ目のアプリを起動してしまわないことは、アプリ側の単一インスタンスの仕組みに任せる（データベースを
//! 開く前の画面の間は、アプリはロックも待ち受けも持たず、この実行からは「動いていない」ように見えるが、
//! 起動すると、動いているアプリに引き渡して終わり、動いているアプリが窓を前面に出す。01 3.5）。

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use yunagi_api::client::{self, CallError, HelloError};
use yunagi_api::show;
use yunagi_core::error::ErrorChain;
use yunagi_core::storage::data_dir::DefaultDir;

use crate::connect::{Timing, rpc_failure};
use crate::error::{self, ErrorObject};

/// 起動してから、待ち受けが開くまで待つ上限（アーキテクチャ設計書 4.4）。
pub const START_WAIT: Duration = Duration::from_secs(30);

/// ソケットのパスが長すぎてアプリに API で頼めないとき（起動の引き渡しだけが頼り）に、起動した
/// プロセスが引き渡して終わるのを待つ時間。この間に動き続けていれば、新しく起動したアプリとみなす。
const HANDOVER_GRACE: Duration = Duration::from_secs(2);

/// アプリの場所を示す環境変数（実行ファイル。macOS は `.app`）。置き場所が標準でない導入や、開発で使う。
pub const APP_ENV: &str = "YUNAGI_APP";

/// アプリの起動のしかた。
#[derive(Debug, Clone)]
pub enum Launch {
    /// 本物：[`APP_ENV`]、なければ、この実行ファイルの置き場所から探す（[`locate`]）。
    Os,
    /// 試験：このプログラムを（引数なしで）起動する。偽のアプリ。
    Program(PathBuf),
}

/// `yunagi show` の設定（試験が差し替えられる）。
#[derive(Debug, Clone)]
pub struct Options {
    pub launch: Launch,
    /// 起動してから待ち受けが開くまで待つ上限（[`START_WAIT`]）。
    pub start_wait: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            launch: Launch::Os,
            start_wait: START_WAIT,
        }
    }
}

/// 起動するもの。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// 実行ファイル（Linux の AppImage、Windows の exe など）。
    Program(PathBuf),
    /// macOS のアプリ（`.app`）。`open` で起動する。
    MacApp(PathBuf),
}

/// アプリを探す。`exe` はこの実行ファイルの実体（リンクを解いたもの）、`app_env` は [`APP_ENV`] の値、
/// `os` は OS（`std::env::consts::OS`）。置き場所は 07 4.1 の配置に合わせる。
///
/// - [`APP_ENV`] があればそれだけを見る（無い・空のときだけ、下を探す）。
/// - Linux：`../Yunagi.AppImage`（`~/.local/opt/yunagi/bin/yunagi` の隣）、同じ場所の `yunagi-app`
///   （開発のビルドや、配置の決まっていない tar.gz・.deb）、`../yunagi-app`。
/// - macOS：この実行ファイルを含む `.app`。無ければ同じ場所の `yunagi-app`（開発のビルド）。
/// - Windows：`..\Yunagi.exe`（`%LOCALAPPDATA%\Programs\Yunagi\bin\yunagi.exe` の隣）、同じ場所の
///   `yunagi-app.exe`。
pub fn locate(exe: &Path, app_env: Option<&OsStr>, os: &str) -> Option<Target> {
    if let Some(path) = app_env.filter(|v| !v.is_empty()) {
        // 実行ファイル（macOS は `.app` のフォルダも）を指していなければ、見つからないものとする
        // （実行できないものを指して、起動の失敗に見せないため）
        let path = PathBuf::from(path);
        let target = as_target(path, os);
        let valid = match &target {
            Target::MacApp(app) => app.is_dir(),
            Target::Program(path) => path.is_file(),
        };
        return valid.then_some(target);
    }
    let dir = exe.parent()?;
    let parent = dir.parent();
    let found = |candidates: Vec<PathBuf>| candidates.into_iter().find(|p| p.is_file());
    match os {
        "macos" => {
            if let Some(app) = exe
                .ancestors()
                .find(|a| a.extension().is_some_and(|e| e == "app") && a.is_dir())
            {
                return Some(Target::MacApp(app.to_path_buf()));
            }
            found(vec![dir.join("yunagi-app")]).map(Target::Program)
        }
        "windows" => found(
            parent
                .map(|p| p.join("Yunagi.exe"))
                .into_iter()
                .chain([dir.join("yunagi-app.exe")])
                .collect(),
        )
        .map(Target::Program),
        _ => found(
            parent
                .map(|p| p.join("Yunagi.AppImage"))
                .into_iter()
                .chain([dir.join("yunagi-app")])
                .chain(parent.map(|p| p.join("yunagi-app")))
                .collect(),
        )
        .map(Target::Program),
    }
}

fn as_target(path: PathBuf, os: &str) -> Target {
    if os == "macos" && path.extension().is_some_and(|e| e == "app") {
        Target::MacApp(path)
    } else {
        Target::Program(path)
    }
}

/// 起動したアプリ。
struct Launched {
    child: Child,
    /// 実行ファイルを直接起動したか（macOS の `open` ではなく）。直接起動したプロセスが、待ち受けが
    /// 開く前に成功で終わったのは、動いているアプリに引き渡した（単一インスタンス）ということ。`open` は
    /// アプリの起動を待たずに成功で終わるので、そうは言えない。
    direct: bool,
}

/// アプリを起動する。この実行が終わっても動き続けるよう、標準入出力を閉じて切り離す。`note` が真なら、
/// 引数 `--note` を付ける（引き渡しを受けた、動いているアプリが、ノートモードで出す。`lib.rs` の `handover`）。
fn spawn(target: &Target, note: bool) -> io::Result<Child> {
    let mut command = match target {
        Target::Program(path) => {
            let mut c = Command::new(path);
            if note {
                c.arg("--note");
            }
            c
        }
        Target::MacApp(app) => {
            let mut c = Command::new("open");
            c.arg(app);
            if note {
                c.args(["--args", "--note"]);
            }
            c
        }
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // 端末のプロセスグループから外す（端末の Ctrl-C や、端末を閉じたときの SIGHUP で止めない）
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
        command.creation_flags(0x0000_0008 | 0x0000_0200);
    }
    // 実行ファイルを書いた直後だと、ETXTBSY（使用中）で失敗することがある。少し待って試し直す
    let mut tries = 0;
    loop {
        match command.spawn() {
            Err(e) if e.kind() == io::ErrorKind::ExecutableFileBusy && tries < 10 => {
                tries += 1;
                thread::sleep(Duration::from_millis(50));
            }
            other => return other,
        }
    }
}

#[cfg(test)]
mod tests;

fn app_not_found() -> ErrorObject {
    error::api_error(
        "app_not_found",
        format!(
            "Yunagi is not running, and the Yunagi app was not found next to this command, so it could not be started. Nothing was changed. Start Yunagi yourself, or set the environment variable {APP_ENV} to the path of the Yunagi app."
        ),
    )
}

fn launch_failed() -> ErrorObject {
    error::api_error(
        "launch_failed",
        "Yunagi is not running, and starting it failed. Nothing was changed. Start Yunagi yourself.",
    )
}

fn start_timeout(wait: Duration) -> ErrorObject {
    error::api_error(
        "start_timeout",
        format!(
            "Yunagi was started, but its local API did not open within {} seconds, so it could not be asked to come to the front. It may still be starting, or be waiting for the user (for example, to choose the data folder). Look for its window. Nothing was changed.",
            wait.as_secs()
        ),
    )
}

/// `yunagi show` を行う。成功すれば `{"requested": true, "launched": 起動したか}`。
///
/// `warn` は、内部の失敗の診断の行（応答の `message` には含めない）を出す先。
pub fn run(
    dir: &DefaultDir,
    note: bool,
    options: &Options,
    timing: Timing,
    warn: &dyn Fn(&str),
) -> Result<Value, ErrorObject> {
    let params = Value::Object(show::params(note));
    // 起動する前は、一時的につながらないものを接続の手順と同じ期限（5 秒）まで試し直す。起動したら、
    // 待ち受けが開くまでの期限（30 秒）に延ばす
    let mut deadline = Instant::now() + timing.wait;
    let mut launched: Option<Launched> = None;
    // ソケットのパスが長すぎる：API では頼めない。起動の引き渡し（アプリ側の単一インスタンス）が頼り
    let mut handover_only = false;
    loop {
        let mut no_listener = false;
        let left = deadline.saturating_duration_since(Instant::now());
        match client::connect(dir, Some("cli"), left.min(timing.connect)) {
            Ok(session) => match session.call(show::METHOD, &params) {
                Ok(_) => return Ok(json!({"requested": true, "launched": launched.is_some()})),
                Err(CallError::Rpc(e)) => return Err(rpc_failure(&e)),
                // 前面に出す頼みは何度送っても同じなので、反映されたか分からなくても、頼み直してよい
                Err(CallError::ConnectionLost | CallError::NotSent(_)) => {}
                Err(CallError::Protocol(message)) => {
                    warn(&format!(
                        "yunagi: unexpected response from Yunagi: {message}"
                    ));
                    return Err(unexpected());
                }
            },
            Err(HelloError::NoListener) => no_listener = true,
            Err(HelloError::SocketPathTooLong(_)) => {
                handover_only = true;
                no_listener = true;
            }
            // 待ち受けはあるが一時的につながらない・認証に答えない（開き直しの最中など）：試し直す
            Err(HelloError::Transient(_) | HelloError::NotAnswering) => {}
            Err(HelloError::Denied(_)) => return Err(error::denied()),
            Err(HelloError::AuthFailed) => return Err(error::auth_failed()),
            Err(HelloError::VersionMismatch { .. }) => return Err(error::version_mismatch()),
            Err(HelloError::Io(e)) => {
                warn(&format!(
                    "yunagi: could not talk to the local API: {}",
                    ErrorChain(&e)
                ));
                return Err(unexpected());
            }
        }
        // 待ち受けが無い（アプリが動いていない、まだデータベースを開いていない）ときは、起動する。
        // 待ち受けがあって一時的につながらないだけのときは、アプリがいるので起動しない
        if launched.is_none() && no_listener {
            launched = Some(start(options, note, warn)?);
            deadline = Instant::now() + options.start_wait;
        }
        if let Some(app) = &mut launched {
            match app.child.try_wait() {
                Ok(Some(status)) if !status.success() => {
                    warn(&format!("yunagi: the app exited with {status}"));
                    return Err(launch_failed());
                }
                // 待ち受けが開く前に成功で終わった：動いているアプリ（起動時の画面など）に引き渡した。
                // アプリが窓を前面に出しているので、待ち受けが開くのを待たない
                // （起動したのではないので `launched` は偽）
                Ok(Some(_)) if app.direct => {
                    return Ok(json!({"requested": true, "launched": false}));
                }
                _ => {}
            }
            // 引き渡しが頼りのとき、動き続けていれば、新しく起動したアプリ
            if handover_only && deadline > Instant::now() + HANDOVER_GRACE {
                deadline = Instant::now() + HANDOVER_GRACE;
            }
        }
        let now = Instant::now();
        if now >= deadline {
            if handover_only {
                // 起動したアプリは動き続けている。窓はアプリが出す
                return Ok(json!({"requested": true, "launched": true}));
            }
            return Err(if launched.is_some() {
                start_timeout(options.start_wait)
            } else {
                error::busy()
            });
        }
        thread::sleep(timing.interval.min(deadline - now));
    }
}

fn unexpected() -> ErrorObject {
    error::api_error(
        "unexpected",
        "Yunagi's response could not be understood. Ask the user to update Yunagi and the yunagi command to the same version.",
    )
}

fn start(options: &Options, note: bool, warn: &dyn Fn(&str)) -> Result<Launched, ErrorObject> {
    let target = match &options.launch {
        Launch::Program(path) => Target::Program(path.clone()),
        Launch::Os => {
            let exe = std::env::current_exe().map_err(|e| {
                warn(&format!(
                    "yunagi: could not find this command's location: {}",
                    ErrorChain(&e)
                ));
                app_not_found()
            })?;
            locate(
                &exe,
                std::env::var_os(APP_ENV).as_deref(),
                std::env::consts::OS,
            )
            .ok_or_else(app_not_found)?
        }
    };
    match spawn(&target, note) {
        Ok(child) => Ok(Launched {
            child,
            direct: matches!(target, Target::Program(_)),
        }),
        Err(e) => {
            warn(&format!(
                "yunagi: could not start {target:?}: {}",
                ErrorChain(&e)
            ));
            Err(launch_failed())
        }
    }
}
