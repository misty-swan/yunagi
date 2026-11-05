//! `yunagi uninstall [--purge]`：アプリと CLI を削除する（docs/design/05-mcp-cli-spec.md 3.1、
//! ビルド・配布手順書 4.4、アーキテクチャ設計書 6.5、利用者ガイド 8章）。
//!
//! 順番（ビルド・配布手順書 4.4）：導入の見分け（Homebrew・どちらでもない、は何も消さずに案内して止まる）
//! → 端末で確認できるか → `yunagi.lock` を取る（アプリか MCP サーバーが動いていれば止まる）→ `--purge`
//! なら `config.json` を読んで保存先を求める → 消すものを見せて確認 → `--purge` のデータと WebView の
//! データを消す → アプリ本体と CLI のリンクを消す（Windows は、アンインストーラーを切り離して動かす）→
//! macOS・Linux は最後に `yunagi.lock` と既定ディレクトリを消す。
//!
//! 試験が利用者の実際の環境を消さないための作り（YUN-99）：
//!
//! - 消すものはすべて [`remove::Report`] を通り、通す前に [`remove::Guard`] が確かめる。試験は
//!   `Setup::guard.confine_to` に一時フォルダを入れ、外を消そうとしたら消さずに panic する。
//! - OS を実際に呼ぶ部分（`defaults delete`・Windows の `PATH` の読み書き・切り離しの起動）は
//!   [`Ops`] の後ろに置き、試験は偽物を使う。
//! - 場所は OS の区分と環境変数の組から求める（[`locations`]）。

pub mod locations;
pub mod remove;
pub mod windows;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use yunagi_core::storage::data_dir::{self, APP_IDENTIFIER, Config, DefaultDir, Platform};
use yunagi_core::storage::dir_mismatch::default_dir_vars;
use yunagi_core::storage::{DbLock, LOCK_FILE_NAME, LockError};

use crate::agent_admin::{Answer, confirm};
use crate::error::{self, ErrorObject};
use crate::{Env, Io};

use locations::{Kind, Layout};
use remove::{Guard, Report};
use windows::Detach;

/// OS を実際に呼ぶ部分。試験は偽物に差し替える（実機でしか確かめられない分。テスト計画書 4.9）。
pub trait Ops: std::fmt::Debug + Send + Sync {
    /// macOS：`defaults delete <identifier>`。設定の仕組み（`cfprefsd`）が覚えている値を忘れさせる。
    fn defaults_delete(&self, identifier: &str) -> Result<(), String>;
    /// Windows：ユーザーの `PATH` の生の値（`%VAR%` を展開していないもの）。
    fn read_user_path(&self) -> Result<String, String>;
    /// Windows：ユーザーの `PATH` を書く（`REG_EXPAND_SZ` のまま。環境の変更の通知も送る）。
    fn write_user_path(&self, value: &str) -> Result<(), String>;
    /// Windows：`yunagi.exe` から切り離して後片付けを始める。
    fn spawn_detached(&self, detach: &Detach) -> Result<(), String>;
}

/// 本物の [`Ops`]。
#[derive(Debug, Clone, Copy)]
pub struct SystemOps;

impl Ops for SystemOps {
    fn defaults_delete(&self, identifier: &str) -> Result<(), String> {
        // 値が無いと失敗の終了コードを返すが、それは構わない（消した後にファイルが残っていないかを見る）
        std::process::Command::new("defaults")
            .args(["delete", identifier])
            .output()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn read_user_path(&self) -> Result<String, String> {
        let out = run_powershell(&windows::read_user_path_script())?;
        String::from_utf8(out).map_err(|_| "the PATH value is not valid UTF-8".to_owned())
    }

    fn write_user_path(&self, value: &str) -> Result<(), String> {
        run_powershell(&windows::write_user_path_script(value)).map(|_| ())
    }

    fn spawn_detached(&self, detach: &Detach) -> Result<(), String> {
        spawn_powershell_detached(&windows::detach_script(detach))
    }
}

/// PowerShell で `script` を動かし、標準出力を返す（Windows 以外では動かない）。
fn run_powershell(script: &str) -> Result<Vec<u8>, String> {
    let out = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand"])
        .arg(windows::encoded_command(script))
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(format!("powershell.exe exited with {}", out.status))
    }
}

#[cfg(windows)]
fn spawn_powershell_detached(script: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP：この `yunagi.exe` が終わっても続く
    const FLAGS: u32 = 0x0000_0008 | 0x0000_0200;
    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-EncodedCommand",
        ])
        .arg(windows::encoded_command(script))
        .creation_flags(FLAGS)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn spawn_powershell_detached(_script: &str) -> Result<(), String> {
    Err("starting the Windows uninstaller is only possible on Windows".to_owned())
}

/// 動く環境のうち、`uninstall` が見るもの（試験が差し替える）。
#[derive(Debug, Clone)]
pub struct Setup {
    pub platform: Platform,
    /// 場所を求める環境変数（`HOME`・`XDG_DATA_HOME`・`LOCALAPPDATA`・`USERPROFILE`・`HOMEBREW_PREFIX`）。
    pub vars: BTreeMap<String, OsString>,
    /// 動いている実行ファイルのパス（`std::env::current_exe`）。
    pub exe: Option<PathBuf>,
    /// Homebrew の Cask がアプリを置く場所（`/Applications`）。
    pub applications_dir: PathBuf,
    /// Caskroom を探す Homebrew の prefix（`HOMEBREW_PREFIX`。無ければ `/opt/homebrew` と `/usr/local`）。
    pub brew_prefixes: Vec<PathBuf>,
    /// 消してよいパスの決まり。
    pub guard: Guard,
    /// `yunagi.lock` が取れないときに待つ時間。
    pub lock_wait: Duration,
    /// この `yunagi` のプロセス ID（Windows で終了を待つ）。
    pub pid: u32,
    pub ops: Arc<dyn Ops>,
}

impl Setup {
    /// 動いているプロセスの環境から作る。
    pub fn from_process() -> Self {
        let vars: BTreeMap<String, OsString> = [
            "HOME",
            "XDG_DATA_HOME",
            "LOCALAPPDATA",
            "USERPROFILE",
            "HOMEBREW_PREFIX",
        ]
        .into_iter()
        .filter_map(|name| std::env::var_os(name).map(|v| (name.to_owned(), v)))
        .collect();
        let brew_prefixes = match vars.get("HOMEBREW_PREFIX").filter(|v| !v.is_empty()) {
            Some(prefix) => vec![PathBuf::from(prefix)],
            None => vec![PathBuf::from("/opt/homebrew"), PathBuf::from("/usr/local")],
        };
        let home = vars
            .get(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .map(PathBuf::from);
        Self {
            platform: Platform::current(),
            vars,
            exe: std::env::current_exe().ok(),
            applications_dir: PathBuf::from("/Applications"),
            brew_prefixes,
            guard: Guard {
                confine_to: None,
                protected: home.into_iter().collect(),
            },
            lock_wait: Duration::from_secs(5),
            pid: std::process::id(),
            ops: Arc::new(SystemOps),
        }
    }
}

/// `yunagi uninstall [--purge]`。終了コードを返す。
pub fn run(purge: bool, env: &Env, io: &mut Io<'_>) -> i32 {
    match execute(purge, env, io) {
        Ok(code) => code,
        Err(e) => crate::fail(&e, false, io),
    }
}

fn conflict(reason: &str, message: String) -> ErrorObject {
    json!({"code": "conflict", "reason": reason, "message": message})
}

fn execute(purge: bool, env: &Env, io: &mut Io<'_>) -> Result<i32, ErrorObject> {
    let s = &env.uninstall;
    let layout = match locations::classify(s)? {
        Kind::Standard(layout) => layout,
        Kind::Homebrew => {
            let command = if purge {
                "brew uninstall --cask --zap yunagi"
            } else {
                "brew uninstall --cask yunagi"
            };
            let extra = if purge {
                " If you changed where Yunagi keeps its data in Settings, --zap does not remove the data there; delete that folder by hand."
            } else {
                ""
            };
            return Err(conflict(
                "homebrew",
                format!(
                    "Yunagi was installed with Homebrew, so this command removes nothing (Homebrew keeps its own record). Run: {command}{extra}"
                ),
            ));
        }
        Kind::Unmanaged(exe) => {
            let brew = if s.platform == Platform::MacOs {
                " If you installed it with Homebrew, use: brew uninstall --cask yunagi"
            } else {
                ""
            };
            return Err(conflict(
                "unmanaged_install",
                format!(
                    "This yunagi is running from {}, which is not a place the install script uses, so it is not clear what to remove. Nothing was removed.{brew}",
                    exe.display()
                ),
            ));
        }
    };
    if !env.setup.interactive {
        return Err(error::permission_denied(
            "yunagi uninstall asks before it removes anything, and needs a terminal to ask on. Nothing was removed. Ask the user to run it themselves in a terminal.",
        ));
    }

    // 1. `yunagi.lock`（既定ディレクトリが無ければ、ロックも `config.json` も飛ばす）
    let dir = data_dir::default_dir()
        .map_err(|_| error::default_dir_unset(default_dir_vars(s.platform)))?;
    let lock = if dir.path().is_dir() {
        match DbLock::acquire(&dir, s.lock_wait) {
            Ok(lock) => Some(lock),
            Err(LockError::Busy { .. }) => return Err(running()),
            Err(LockError::Io { path, .. }) => {
                return Err(error::storage_unavailable(format!(
                    "Could not lock {}, so it is not safe to remove anything. Nothing was removed.",
                    path.display()
                )));
            }
        }
    } else {
        None
    };
    let data_dir = if purge {
        Some(
            Config::load(&dir)
                .map_err(|e| config_error(&dir, &e))?
                .data_dir,
        )
    } else {
        None
    };

    // 消すものを集めて見せ、確認を得る
    let plan = Plan::build(s, &layout, &dir, data_dir.as_deref())?;
    if plan.is_empty() {
        let _ = writeln!(io.stdout, "Nothing to remove.");
        return Ok(0);
    }
    plan.print(io, purge);
    let question = if purge {
        "Remove these, including your data? This cannot be undone."
    } else {
        "Remove these?"
    };
    match confirm(io, question) {
        Answer::Yes => {}
        Answer::No => {
            let _ = writeln!(io.stdout, "Cancelled. Nothing was removed.");
            return Ok(0);
        }
        Answer::Unavailable => return Ok(1),
    }

    let mut report = Report::default();
    // 2. データと WebView のデータ
    if purge {
        purge_data(s, &plan, &dir, &mut report)?;
    }
    // 3. アプリ本体と CLI のリンク
    remove_app(s, &layout, &dir, purge, &mut report)?;
    // 4. macOS・Linux は、最後にロックファイルと既定ディレクトリ（開いたまま消せる）
    if purge && s.platform != Platform::Windows && lock.is_some() {
        report.remove(&s.guard, &dir.path().join(LOCK_FILE_NAME));
        remove_default_dir_if_empty(s, dir.path(), &mut report);
    }
    drop(lock);
    Ok(print_report(io, &report, purge, &dir))
}

/// 既定ディレクトリを、空になっていれば消す。残っているものがあれば、消せなかったものとして
/// すでに記録されている（ここでは足さない）。記録が無いのに残っているときだけ、フォルダを記録する。
fn remove_default_dir_if_empty(s: &Setup, dir: &Path, report: &mut Report) {
    let is_empty = fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_none());
    if is_empty || fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_symlink()) {
        report.remove_empty_dir(&s.guard, dir);
    } else if !report.failed.iter().any(|(path, _)| path.starts_with(dir)) {
        report.fail(dir, "it still has files in it");
    }
}

fn running() -> ErrorObject {
    json!({
        "code": "busy",
        "message": "Yunagi (the app, or an AI agent's MCP server) is running. Nothing was removed. Quit them and run this again.",
    })
}

fn config_error(dir: &DefaultDir, e: &data_dir::DataDirError) -> ErrorObject {
    error::storage_unavailable(format!(
        "Could not read {}/config.json ({e}), so it is not known where your data is. Nothing was removed. Fix or delete that file, then run this again.",
        dir.path().display()
    ))
}

/// 消すもののうち、いまある分。
struct Plan {
    /// アプリ本体と CLI のリンクなど。
    app: Vec<PathBuf>,
    /// `--purge` で消す WebView のデータ。
    webview: Vec<PathBuf>,
    /// `--purge` で消す、保存先のデータベースのファイルと `backups/`（保存先を変えているときだけ）。
    data: Vec<PathBuf>,
    /// `--purge` で消す既定ディレクトリ（あるとき）。
    default_dir: Option<PathBuf>,
    /// Windows：ユーザーの `PATH` から外す `bin`。
    windows_path: Option<PathBuf>,
}

fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

impl Plan {
    fn build(
        s: &Setup,
        layout: &Layout,
        dir: &DefaultDir,
        data_dir: Option<&Path>,
    ) -> Result<Self, ErrorObject> {
        let mut app: Vec<PathBuf> = [&layout.link, &layout.desktop_entry]
            .into_iter()
            .flatten()
            .filter(|p| exists(p))
            .cloned()
            .collect();
        if exists(&layout.root) {
            app.push(layout.root.clone());
        }
        let (mut webview, mut data, mut default_dir) = (Vec::new(), Vec::new(), None);
        if let Some(data_dir) = data_dir {
            webview = locations::webview_paths(s)?
                .into_iter()
                .filter(|p| exists(p))
                .collect();
            if !same_place(data_dir, dir.path()) {
                data = locations::data_files(data_dir)
                    .into_iter()
                    .filter(|p| exists(p))
                    .collect();
            }
            default_dir = exists(dir.path()).then(|| dir.path().to_path_buf());
        }
        let windows_path = (s.platform == Platform::Windows).then(|| layout.root.join("bin"));
        Ok(Self {
            app,
            webview,
            data,
            default_dir,
            windows_path,
        })
    }

    fn is_empty(&self) -> bool {
        self.app.is_empty()
            && self.webview.is_empty()
            && self.data.is_empty()
            && self.default_dir.is_none()
            && self.windows_path.is_none()
    }

    fn print(&self, io: &mut Io<'_>, purge: bool) {
        let mut text = String::from("These will be removed:\n");
        let mut section = |title: &str, items: &[PathBuf]| {
            if !items.is_empty() {
                let _ = writeln!(text, "{title}");
                for item in items {
                    let _ = writeln!(text, "  {}", item.display());
                }
            }
        };
        section("The app and this command:", &self.app);
        if let Some(bin) = &self.windows_path {
            section(
                "The folder below is taken out of your user PATH:",
                std::slice::from_ref(bin),
            );
        }
        if purge {
            section(
                "Your data (all of the folder):",
                self.default_dir.as_slice(),
            );
            section(
                "Your data, in the folder you chose in Settings (the folder itself stays):",
                &self.data,
            );
            section("Data the screen made for itself:", &self.webview);
        } else if self.default_dir.is_none() {
            // 何も言わない
        }
        let _ = io.stdout.write_all(text.as_bytes());
        if !purge {
            let _ = writeln!(
                io.stdout,
                "\nYour data is kept. To remove it too, run: yunagi uninstall --purge"
            );
        }
    }
}

/// 同じ場所か（リンクを解いた実体で比べる）。
fn same_place(a: &Path, b: &Path) -> bool {
    let real = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    real(a) == real(b)
}

/// `--purge`：WebView のデータ、保存先のデータ、既定ディレクトリの中の `yunagi.lock` 以外を消す。
fn purge_data(
    s: &Setup,
    plan: &Plan,
    dir: &DefaultDir,
    report: &mut Report,
) -> Result<(), ErrorObject> {
    let plist = if s.platform == Platform::MacOs {
        Some(locations::macos_plist(s)?)
    } else {
        None
    };
    for path in &plan.webview {
        if Some(path) == plist.as_ref() {
            // 設定の仕組みが値を覚えていて書き戻すことがあるので、ファイルを消す前に忘れさせる
            if let Err(e) = s.ops.defaults_delete(APP_IDENTIFIER) {
                report.notes.push(format!(
                    "`defaults delete {APP_IDENTIFIER}` did not run ({e})."
                ));
            }
        }
        report.remove(&s.guard, path);
        if Some(path) == plist.as_ref() && exists(path) {
            report.fail(
                path,
                "it came back after it was removed (macOS wrote it again)",
            );
        }
    }
    for path in &plan.data {
        report.remove(&s.guard, path);
    }
    report.remove_contents_except(&s.guard, dir.path(), LOCK_FILE_NAME);
    Ok(())
}

/// アプリ本体と CLI のリンクを消す。Windows は、`PATH` から `bin` を外し、アンインストーラーを切り離す。
fn remove_app(
    s: &Setup,
    layout: &Layout,
    dir: &DefaultDir,
    purge: bool,
    report: &mut Report,
) -> Result<(), ErrorObject> {
    if s.platform == Platform::Windows {
        remove_windows_app(s, layout, dir, purge, report);
        return Ok(());
    }
    if let Some(link) = &layout.link {
        remove_link(s, link, &layout.root, report);
    }
    if let Some(entry) = &layout.desktop_entry {
        report.remove(&s.guard, entry);
    }
    report.remove(&s.guard, &layout.root);
    Ok(())
}

/// `~/.local/bin/yunagi`：この導入を指すシンボリックリンクのときだけ消す。そうでなければ（手で置いた
/// ファイルなど）触れずに、消し残りとして記録する（install.sh もリンクでないファイルは置き換えない）。
fn remove_link(s: &Setup, link: &Path, root: &Path, report: &mut Report) {
    let Ok(meta) = fs::symlink_metadata(link) else {
        return;
    };
    if meta.file_type().is_symlink() && points_into(link, root, s.platform) {
        report.remove(&s.guard, link);
    } else {
        report.fail(
            link,
            "this is not a link to the Yunagi that is being removed, so it was left as it is",
        );
    }
}

/// シンボリックリンク `link` の先が `root` の中か。
fn points_into(link: &Path, root: &Path, _platform: Platform) -> bool {
    let Ok(target) = fs::read_link(link) else {
        return false;
    };
    let target = if target.is_relative() {
        link.parent().map_or(target.clone(), |p| p.join(&target))
    } else {
        target
    };
    if target.starts_with(root) {
        return true;
    }
    let real_root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    fs::canonicalize(link).is_ok_and(|real| real.starts_with(real_root))
}

fn remove_windows_app(
    s: &Setup,
    layout: &Layout,
    dir: &DefaultDir,
    purge: bool,
    report: &mut Report,
) {
    let bin = layout.root.join("bin");
    match remove_from_user_path(s, &bin) {
        Ok(true) => report.notes.push(format!("{} was taken out of your user PATH.", bin.display())),
        Ok(false) => {}
        Err(e) => report.fail(&bin, format!("could not take it out of your user PATH ({e}); remove it by hand in the environment variable settings")),
    }
    let uninstaller = layout.root.join("uninstall.exe");
    let uninstaller = exists(&uninstaller).then_some(uninstaller);
    if uninstaller.is_none() {
        report.fail(
            &layout.root.join("uninstall.exe"),
            "the Yunagi uninstaller was not found; the folder is removed anyway",
        );
    }
    let default_dir = purge.then(|| dir.path().to_path_buf());
    for path in std::iter::once(&layout.root).chain(default_dir.iter()) {
        if let Err(why) = s.guard.check(path) {
            report.fail(path, why);
            return;
        }
    }
    let detach = Detach {
        wait_pid: s.pid,
        uninstaller,
        install_dir: layout.root.clone(),
        default_dir,
    };
    match s.ops.spawn_detached(&detach) {
        Ok(()) => {
            let mut later = vec![detach.install_dir.display().to_string()];
            if let Some(d) = &detach.default_dir {
                later.push(d.display().to_string());
            }
            report.notes.push(format!(
                "The rest is removed in the background after this command ends: {}. If anything is still there a little later, delete it by hand. Do not start Yunagi until then.",
                later.join(", ")
            ));
        }
        Err(e) => report.fail(
            &layout.root,
            format!("could not start the uninstaller ({e})"),
        ),
    }
}

/// ユーザーの `PATH` から `bin` を外す。外したら `true`。
fn remove_from_user_path(s: &Setup, bin: &Path) -> Result<bool, String> {
    let current = s.ops.read_user_path()?;
    let lookup = |name: &str| {
        s.vars
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .and_then(|(_, v)| v.to_str().map(str::to_owned))
    };
    match windows::remove_entry(&current, &bin.to_string_lossy(), &lookup) {
        Some(new) => s.ops.write_user_path(&new).map(|()| true),
        None => Ok(false),
    }
}

/// 結果を出し、終了コードを返す（消せなかったものがあれば 1）。
fn print_report(io: &mut Io<'_>, report: &Report, purge: bool, dir: &DefaultDir) -> i32 {
    let mut text = String::new();
    if !report.removed.is_empty() {
        text.push_str("\nRemoved:\n");
        for path in &report.removed {
            let _ = writeln!(text, "  {}", path.display());
        }
    }
    if !report.failed.is_empty() {
        text.push_str("\nCould not remove (delete these by hand):\n");
        for (path, why) in &report.failed {
            let _ = writeln!(text, "  {}: {why}", path.display());
        }
    }
    for note in &report.notes {
        let _ = writeln!(text, "\n{note}");
    }
    if report.failed.is_empty() {
        let _ = writeln!(
            text,
            "\nYunagi was uninstalled.{}",
            if purge {
                String::new()
            } else {
                format!(" Your data was kept ({}).", dir.path().display())
            }
        );
    } else {
        let _ = writeln!(text, "\nSome things were not removed.");
    }
    let _ = io.stdout.write_all(text.as_bytes());
    i32::from(!report.failed.is_empty())
}

#[cfg(all(test, unix))]
pub(crate) mod tests_support {
    use super::*;

    /// 何もしない [`Ops`]（Linux の単体テストで、場所の計算だけを確かめるとき）。
    #[derive(Debug)]
    pub struct NoOps;

    impl Ops for NoOps {
        fn defaults_delete(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn read_user_path(&self) -> Result<String, String> {
            Ok(String::new())
        }
        fn write_user_path(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn spawn_detached(&self, _: &Detach) -> Result<(), String> {
            Ok(())
        }
    }

    pub fn setup_for(platform: Platform, vars: &[(&str, &str)]) -> Setup {
        Setup {
            platform,
            vars: vars
                .iter()
                .map(|(k, v)| ((*k).to_owned(), OsString::from(*v)))
                .collect(),
            exe: None,
            applications_dir: PathBuf::from("/Applications"),
            brew_prefixes: vec![PathBuf::from("/opt/homebrew")],
            guard: Guard::default(),
            lock_wait: Duration::ZERO,
            pid: 1,
            ops: Arc::new(NoOps),
        }
    }
}
