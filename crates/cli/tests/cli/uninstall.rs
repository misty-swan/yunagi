//! `yunagi uninstall` の結合テスト（テスト計画書 4.5「CLI」。ビルド・配布手順書 4.4、アーキテクチャ設計書
//! 6.5。YUN-99）。
//!
//! **利用者の実際の環境には触れない**：場所はすべて一時フォルダの中に作り、`HOME`・`XDG_DATA_HOME` を
//! 一時フォルダに差し替える。動かす前に毎回、差し替えが効いていることを表明する（[`Sandbox::run`]）。
//! 消す処理は `Setup::guard.confine_to` で一時フォルダの外を消そうとしたら消さずに panic する。
//! 実機でしか確かめられない部分（`defaults delete`・Windows の `PATH` の読み書きと切り離しの起動）は
//! 偽物（[`FakeOps`]）に呼ばれた内容で確かめる（実機の項目はテスト計画書 4.9）。
//!
//! 既定ディレクトリは環境変数から求まるので、ほかの結合テストと同じく 1 つのスレッドで順に走らせる。

#![cfg(unix)]

use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use yunagi_cli::uninstall::windows::Detach;
use yunagi_cli::uninstall::{Ops, Setup};
use yunagi_cli::{Env, Io};
use yunagi_core::storage::DbLock;
use yunagi_core::storage::data_dir::{self, APP_IDENTIFIER, Platform};

/// 実機でしか確かめられない部分の偽物。呼ばれた内容を覚える。
#[derive(Debug, Default)]
struct FakeOps {
    state: Mutex<FakeState>,
}

#[derive(Debug, Default)]
struct FakeState {
    defaults_deleted: Vec<String>,
    /// Windows：いまのユーザーの `PATH`。
    user_path: String,
    written_path: Option<String>,
    detached: Option<Detach>,
    /// 切り離しの起動を失敗させる。
    spawn_fails: bool,
}

impl FakeOps {
    fn state(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Ops for FakeOps {
    fn defaults_delete(&self, identifier: &str) -> Result<(), String> {
        self.state().defaults_deleted.push(identifier.to_owned());
        Ok(())
    }
    fn read_user_path(&self) -> Result<String, String> {
        Ok(self.state().user_path.clone())
    }
    fn write_user_path(&self, value: &str) -> Result<(), String> {
        self.state().written_path = Some(value.to_owned());
        Ok(())
    }
    fn spawn_detached(&self, detach: &Detach) -> Result<(), String> {
        let mut state = self.state();
        if state.spawn_fails {
            return Err("fake failure".to_owned());
        }
        state.detached = Some(detach.clone());
        Ok(())
    }
}

/// 試験の場所。`HOME`・`XDG_DATA_HOME` を一時フォルダの下に差し替える。
struct Sandbox {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    ops: Arc<FakeOps>,
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Sandbox {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        let sandbox = Self {
            _tmp: tmp,
            root,
            ops: Arc::new(FakeOps::default()),
        };
        // SAFETY: 試験は 1 つのスレッドで順に走り、環境変数を読む処理は呼び出しの間だけ動く
        unsafe {
            std::env::set_var("HOME", sandbox.home());
            std::env::set_var("XDG_DATA_HOME", sandbox.xdg());
            std::env::remove_var("HOMEBREW_PREFIX");
        }
        fs::create_dir_all(sandbox.home()).unwrap();
        fs::create_dir_all(sandbox.xdg()).unwrap();
        sandbox
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }
    fn xdg(&self) -> PathBuf {
        self.root.join("xdg")
    }
    /// 既定ディレクトリ（`XDG_DATA_HOME/yunagi`）。
    fn default_dir(&self) -> PathBuf {
        self.xdg().join("yunagi")
    }
    fn webview_dir(&self) -> PathBuf {
        self.xdg().join(APP_IDENTIFIER)
    }
    /// Linux の導入先（`~/.local/opt/yunagi`）。
    fn linux_root(&self) -> PathBuf {
        self.home().join(".local/opt/yunagi")
    }
    fn link(&self) -> PathBuf {
        self.home().join(".local/bin/yunagi")
    }
    fn desktop_entry(&self) -> PathBuf {
        self.xdg()
            .join(format!("applications/{APP_IDENTIFIER}.desktop"))
    }

    fn write(&self, path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// install.sh が置くもの（Linux）と、利用者のほかのファイル（消えてはいけないもの）を作る。
    fn install_linux(&self) {
        self.write(&self.linux_root().join("Yunagi.AppImage"), "app");
        self.write(&self.linux_root().join("bin/yunagi"), "cli");
        fs::create_dir_all(self.link().parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(self.linux_root().join("bin/yunagi"), self.link()).unwrap();
        self.write(&self.desktop_entry(), "[Desktop Entry]");
        self.write(&self.home().join("Documents/keep.txt"), "mine");
        self.write(&self.home().join(".local/bin/other-tool"), "tool");
        self.write(&self.xdg().join("other-app/keep"), "theirs");
    }

    /// アプリが使った跡（既定ディレクトリと WebView のデータ）を作る。
    fn use_app(&self) {
        let dir = self.default_dir();
        self.write(&dir.join("config.json"), "{}");
        self.write(&dir.join("yunagi.db"), "db");
        self.write(&dir.join("yunagi.db-wal"), "wal");
        self.write(&dir.join("backups/b1.db"), "backup");
        self.write(&dir.join("token"), "secret");
        // ロックファイルは本物の取り方で作る
        drop(DbLock::acquire(&data_dir::default_dir().unwrap(), Duration::ZERO).unwrap());
        self.write(&self.webview_dir().join("WebKitCache/x"), "cache");
    }

    /// 動かす前の表明：差し替えが効いていて、消す先が一時フォルダの中だけであること。
    fn assert_isolated(&self, setup: &Setup) {
        let under = |p: &Path| {
            fs::canonicalize(p)
                .unwrap_or_else(|_| p.to_path_buf())
                .starts_with(&self.root)
        };
        assert!(
            under(Path::new(&std::env::var("HOME").unwrap())),
            "HOME is not replaced"
        );
        assert!(
            under(Path::new(&std::env::var("XDG_DATA_HOME").unwrap())),
            "XDG_DATA_HOME is not replaced"
        );
        assert!(
            under(data_dir::default_dir().unwrap().path()),
            "the default dir is outside the sandbox"
        );
        assert!(
            setup.exe.as_deref().is_none_or(under),
            "the executable is outside the sandbox"
        );
        assert!(under(&setup.applications_dir) && setup.brew_prefixes.iter().all(|p| under(p)));
        for var in ["HOME", "XDG_DATA_HOME", "LOCALAPPDATA"] {
            if let Some(v) = setup.vars.get(var) {
                assert!(under(Path::new(v)), "{var} is outside the sandbox");
            }
        }
        assert_eq!(setup.guard.confine_to.as_deref(), Some(self.root.as_path()));
        assert!(setup.guard.protected.iter().any(|p| p == &self.home()));
    }

    /// `Setup`：本物の環境から作り、実行ファイル・Homebrew の場所・偽物・消してよい場所を差し替える。
    fn setup(&self, platform: Platform, exe: &Path) -> Setup {
        let mut setup = Setup::from_process();
        setup.platform = platform;
        setup.vars.clear();
        setup.vars.insert("HOME".into(), self.home().into());
        setup.vars.insert("XDG_DATA_HOME".into(), self.xdg().into());
        setup
            .vars
            .insert("LOCALAPPDATA".into(), self.root.join("local").into());
        setup.exe = Some(exe.to_path_buf());
        setup.applications_dir = self.root.join("Applications");
        setup.brew_prefixes = vec![self.root.join("brew")];
        setup.guard.confine_to = Some(self.root.clone());
        setup.guard.protected = vec![self.home()];
        setup.lock_wait = Duration::ZERO;
        setup.pid = 4242;
        setup.ops = self.ops.clone();
        setup
    }

    fn linux_setup(&self) -> Setup {
        self.setup(Platform::Linux, &self.linux_root().join("bin/yunagi"))
    }

    fn run(&self, setup: Setup, args: &[&str], answer: &str, interactive: bool) -> Out {
        self.assert_isolated(&setup);
        let mut env = Env::default();
        env.setup.interactive = interactive;
        env.uninstall = setup;
        let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        let (mut stdin, mut stdout, mut stderr) = (answer.as_bytes(), Vec::new(), Vec::new());
        let code = yunagi_cli::run(
            &args,
            &env,
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

    /// 利用者のほかのファイルが残っていること。
    fn assert_others_kept(&self) {
        for kept in [
            self.home().join("Documents/keep.txt"),
            self.home().join(".local/bin/other-tool"),
            self.xdg().join("other-app/keep"),
        ] {
            assert!(kept.exists(), "{} was removed", kept.display());
        }
    }
}

fn gone(path: &Path) -> bool {
    fs::symlink_metadata(path).is_err()
}

fn without_purge_keeps_the_data() {
    let w = Sandbox::new();
    w.install_linux();
    w.use_app();
    let out = w.run(w.linux_setup(), &["uninstall"], "y\n", true);
    assert_eq!(out.code, 0, "{} / {}", out.stdout, out.stderr);
    assert!(gone(&w.linux_root()) && gone(&w.link()) && gone(&w.desktop_entry()));
    // データと WebView のデータは残る
    assert!(w.default_dir().join("yunagi.db").exists());
    assert!(w.default_dir().join("yunagi.lock").exists());
    assert!(w.webview_dir().join("WebKitCache/x").exists());
    w.assert_others_kept();
    assert!(out.stdout.contains("Your data was kept"), "{}", out.stdout);
}

fn purge_removes_everything_of_yunagi() {
    let w = Sandbox::new();
    w.install_linux();
    w.use_app();
    let out = w.run(w.linux_setup(), &["uninstall", "--purge"], "y\n", true);
    assert_eq!(out.code, 0, "{} / {}", out.stdout, out.stderr);
    for path in [
        w.linux_root(),
        w.link(),
        w.desktop_entry(),
        w.default_dir(),
        w.webview_dir(),
    ] {
        assert!(gone(&path), "{} is still there", path.display());
    }
    w.assert_others_kept();
    assert!(
        w.xdg().exists() && w.home().exists(),
        "only Yunagi's folders go"
    );
}

fn purge_with_a_changed_data_dir_removes_only_the_database_files() {
    let w = Sandbox::new();
    w.install_linux();
    w.use_app();
    let notes = w.root.join("notes");
    for name in [
        "yunagi.db",
        "yunagi.db-wal",
        "yunagi.db-shm",
        "yunagi.db-journal",
    ] {
        w.write(&notes.join(name), "x");
    }
    w.write(&notes.join("backups/b.db"), "x");
    w.write(&notes.join("mine.txt"), "mine");
    w.write(
        &w.default_dir().join("config.json"),
        &serde_json::json!({ "data_dir": notes }).to_string(),
    );
    let out = w.run(w.linux_setup(), &["uninstall", "--purge"], "y\n", true);
    assert_eq!(out.code, 0, "{} / {}", out.stdout, out.stderr);
    assert!(gone(&w.default_dir()));
    for name in [
        "yunagi.db",
        "yunagi.db-wal",
        "yunagi.db-shm",
        "yunagi.db-journal",
        "backups",
    ] {
        assert!(gone(&notes.join(name)), "{name}");
    }
    // 保存先のフォルダ自体と、利用者のほかのファイルは消さない
    assert!(notes.join("mine.txt").exists());
}

fn a_data_dir_outside_the_sandbox_is_stopped_by_the_guard() {
    let w = Sandbox::new();
    w.install_linux();
    w.use_app();
    // 別の一時フォルダ（試験の場所の外）を保存先にする：ガードが消す前に止める
    let outside = tempfile::tempdir().unwrap();
    w.write(&outside.path().join("yunagi.db"), "precious");
    w.write(
        &w.default_dir().join("config.json"),
        &serde_json::json!({ "data_dir": outside.path() }).to_string(),
    );
    let result = catch_unwind(AssertUnwindSafe(|| {
        w.run(w.linux_setup(), &["uninstall", "--purge"], "y\n", true)
    }));
    assert!(
        result.is_err(),
        "the guard must stop the removal outside the sandbox"
    );
    assert!(outside.path().join("yunagi.db").exists());
}

fn it_stops_when_the_app_or_an_agent_holds_the_lock() {
    let w = Sandbox::new();
    w.install_linux();
    w.use_app();
    let held = DbLock::acquire(&data_dir::default_dir().unwrap(), Duration::ZERO).unwrap();
    let out = w.run(w.linux_setup(), &["uninstall", "--purge"], "y\n", true);
    assert_eq!(out.code, 7, "{} / {}", out.stdout, out.stderr);
    assert!(out.stderr.contains("busy") && out.stderr.contains("Nothing was removed"));
    assert!(w.linux_root().exists() && w.default_dir().join("yunagi.db").exists());
    assert!(w.webview_dir().exists());
    drop(held);
}

fn a_broken_config_stops_purge_but_not_a_plain_uninstall() {
    let w = Sandbox::new();
    w.install_linux();
    w.use_app();
    w.write(&w.default_dir().join("config.json"), "{ not json");
    let out = w.run(w.linux_setup(), &["uninstall", "--purge"], "y\n", true);
    assert_eq!(out.code, 6, "{} / {}", out.stdout, out.stderr);
    assert!(out.stderr.contains("Nothing was removed"));
    assert!(w.linux_root().exists() && w.default_dir().join("yunagi.db").exists());
    // `--purge` が無ければ `config.json` は読まない
    let out = w.run(w.linux_setup(), &["uninstall"], "y\n", true);
    assert_eq!(out.code, 0, "{} / {}", out.stdout, out.stderr);
    assert!(gone(&w.linux_root()));
}

fn it_needs_a_terminal_and_a_yes() {
    let w = Sandbox::new();
    w.install_linux();
    w.use_app();
    let out = w.run(w.linux_setup(), &["uninstall", "--purge"], "y\n", false);
    assert_eq!(out.code, 3, "{} / {}", out.stdout, out.stderr);
    assert!(w.linux_root().exists());
    for answer in ["n\n", "\n", ""] {
        let out = w.run(w.linux_setup(), &["uninstall", "--purge"], answer, true);
        assert!(
            w.linux_root().exists() && w.default_dir().exists(),
            "answer {answer:?}"
        );
        assert_eq!(out.code, i32::from(answer.is_empty()), "answer {answer:?}");
    }
}

fn an_install_that_is_not_ours_is_not_touched() {
    let w = Sandbox::new();
    w.install_linux();
    w.use_app();
    // どちらでもない（開発用のビルドなど）
    let elsewhere = w.root.join("target/debug/yunagi");
    w.write(&elsewhere, "dev");
    let out = w.run(
        w.setup(Platform::Linux, &elsewhere),
        &["uninstall", "--purge"],
        "y\n",
        true,
    );
    assert_eq!(out.code, 5, "{} / {}", out.stdout, out.stderr);
    assert!(out.stderr.contains(&elsewhere.display().to_string()));
    assert!(w.linux_root().exists() && w.default_dir().exists() && elsewhere.exists());

    // Homebrew の Cask（macOS）：/Applications/Yunagi.app の中で、Caskroom がある
    let cask = w.root.join("Applications/Yunagi.app/Contents/MacOS/yunagi");
    w.write(&cask, "cask");
    fs::create_dir_all(w.root.join("brew/Caskroom/yunagi")).unwrap();
    let out = w.run(
        w.setup(Platform::MacOs, &cask),
        &["uninstall", "--purge"],
        "y\n",
        true,
    );
    assert_eq!(out.code, 5, "{} / {}", out.stdout, out.stderr);
    assert!(
        out.stderr.contains("brew uninstall --cask --zap yunagi"),
        "{}",
        out.stderr
    );
    let out = w.run(w.setup(Platform::MacOs, &cask), &["uninstall"], "y\n", true);
    assert!(out.stderr.contains("brew uninstall --cask yunagi") && !out.stderr.contains("--zap"));
    assert!(w.default_dir().exists() && cask.exists());
}

fn a_never_started_install_has_no_default_dir_to_lock() {
    let w = Sandbox::new();
    w.install_linux();
    w.write(&w.webview_dir().join("x"), "cache");
    assert!(!w.default_dir().exists());
    let out = w.run(w.linux_setup(), &["uninstall", "--purge"], "y\n", true);
    assert_eq!(out.code, 0, "{} / {}", out.stdout, out.stderr);
    assert!(gone(&w.linux_root()) && gone(&w.webview_dir()));
    // ロックのために既定ディレクトリを作らない
    assert!(!w.default_dir().exists());
    w.assert_others_kept();
}

fn a_link_that_is_not_ours_is_left_and_listed() {
    let w = Sandbox::new();
    w.install_linux();
    w.use_app();
    fs::remove_file(w.link()).unwrap();
    w.write(&w.link(), "a file the user put here");
    let out = w.run(w.linux_setup(), &["uninstall", "--purge"], "y\n", true);
    assert_eq!(out.code, 1, "{} / {}", out.stdout, out.stderr);
    assert!(w.link().exists());
    assert!(
        out.stdout.contains("Could not remove")
            && out.stdout.contains(&w.link().display().to_string())
    );
    // ほかは消し続ける。既定ディレクトリも消える
    assert!(gone(&w.linux_root()) && gone(&w.default_dir()) && gone(&w.webview_dir()));
}

fn symlinks_are_removed_without_touching_their_targets() {
    let w = Sandbox::new();
    w.install_linux();
    w.use_app();
    // WebView の場所が、別の場所へのリンクになっている
    let target = w.root.join("elsewhere");
    w.write(&target.join("keep"), "mine");
    fs::remove_dir_all(w.webview_dir()).unwrap();
    std::os::unix::fs::symlink(&target, w.webview_dir()).unwrap();
    // 既定ディレクトリの中にも、外へのリンクがある
    std::os::unix::fs::symlink(&target, w.default_dir().join("linked")).unwrap();
    let out = w.run(w.linux_setup(), &["uninstall", "--purge"], "y\n", true);
    assert_eq!(out.code, 0, "{} / {}", out.stdout, out.stderr);
    assert!(gone(&w.webview_dir()) && gone(&w.default_dir()));
    assert!(target.join("keep").exists());
}

fn a_failure_is_listed_and_the_rest_goes_on() {
    use std::os::unix::fs::PermissionsExt;
    let w = Sandbox::new();
    w.install_linux();
    w.use_app();
    // 中のファイルを消せないフォルダ（書き込み権限が無い）
    let locked = w.default_dir().join("locked");
    w.write(&locked.join("f"), "x");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).unwrap();
    let can_still_write = fs::write(locked.join("probe"), "x").is_ok();
    let out = w.run(w.linux_setup(), &["uninstall", "--purge"], "y\n", true);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
    if can_still_write {
        eprintln!("(skipped: running as a user that ignores permissions)");
        return;
    }
    assert_eq!(out.code, 1, "{} / {}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains(&locked.display().to_string()),
        "{}",
        out.stdout
    );
    // ほかは消えている。残ったものを含む既定ディレクトリは、残る
    assert!(gone(&w.linux_root()) && gone(&w.webview_dir()));
    assert!(gone(&w.default_dir().join("yunagi.db")));
    assert!(locked.exists());
}

fn macos_removes_the_five_places_and_forgets_the_preferences() {
    let w = Sandbox::new();
    let app_exe = w
        .home()
        .join("Applications/Yunagi.app/Contents/MacOS/yunagi");
    w.write(&app_exe, "cli");
    w.write(
        &w.home().join("Library/Application Support/Other/keep"),
        "theirs",
    );
    fs::create_dir_all(w.link().parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&app_exe, w.link()).unwrap();
    let lib = w.home().join("Library");
    let places = [
        lib.join("WebKit").join(APP_IDENTIFIER).join("x"),
        lib.join("Caches").join(APP_IDENTIFIER).join("x"),
        lib.join("Preferences")
            .join(format!("{APP_IDENTIFIER}.plist")),
        lib.join("Saved Application State")
            .join(format!("{APP_IDENTIFIER}.savedState"))
            .join("x"),
    ];
    for place in &places {
        w.write(place, "x");
    }
    w.use_app();
    let out = w.run(
        w.setup(Platform::MacOs, &app_exe),
        &["uninstall", "--purge"],
        "y\n",
        true,
    );
    assert_eq!(out.code, 0, "{} / {}", out.stdout, out.stderr);
    assert_eq!(w.ops.state().defaults_deleted, [APP_IDENTIFIER]);
    for place in &places {
        let top = lib.join(
            place
                .strip_prefix(&lib)
                .unwrap()
                .components()
                .take(2)
                .collect::<PathBuf>(),
        );
        assert!(gone(&top), "{}", top.display());
    }
    assert!(gone(&w.home().join("Applications/Yunagi.app")) && gone(&w.link()));
    // Application Support の下のほかのアプリには触れない
    assert!(
        w.home()
            .join("Library/Application Support/Other/keep")
            .exists()
    );
    // 親のフォルダは消さない（`~/Library/WebKit/` など）
    assert!(lib.join("WebKit").is_dir() && lib.join("Caches").is_dir());
}

fn windows_leaves_the_install_dir_and_default_dir_to_the_detached_cleanup() {
    let w = Sandbox::new();
    let local = w.root.join("local");
    let root = local.join("Programs/Yunagi");
    let exe = root.join("bin/yunagi.exe");
    w.write(&exe, "cli");
    w.write(&root.join("uninstall.exe"), "nsis");
    w.use_app();
    // Windows の WebView の場所は `%LOCALAPPDATA%\<id>`（ここでは `local`）
    w.write(&local.join(APP_IDENTIFIER).join("EBWebView/x"), "cache");
    w.write(&local.join("Other/keep"), "theirs");
    let bin = root.join("bin").display().to_string();
    w.ops.state().user_path = format!("%JAVA_HOME%\\bin;{bin};C:\\tools");
    let out = w.run(
        w.setup(Platform::Windows, &exe),
        &["uninstall", "--purge"],
        "y\n",
        true,
    );
    assert_eq!(out.code, 0, "{} / {}", out.stdout, out.stderr);
    let state = w.ops.state();
    // PATH から bin だけが外れる（ほかは書かれたまま）
    assert_eq!(
        state.written_path.as_deref(),
        Some("%JAVA_HOME%\\bin;C:\\tools")
    );
    // 動いている yunagi.exe は消さず、終了を待つ後片付けに任せる
    let detach = state.detached.clone().expect("the cleanup is started");
    assert_eq!(detach.wait_pid, 4242);
    assert_eq!(detach.install_dir, root);
    assert_eq!(detach.uninstaller, Some(root.join("uninstall.exe")));
    assert_eq!(detach.default_dir, Some(w.default_dir()));
    assert!(exe.exists() && root.join("uninstall.exe").exists());
    // この場では、WebView のデータと、既定ディレクトリの中の yunagi.lock 以外を消す
    assert!(gone(&local.join(APP_IDENTIFIER)));
    assert!(w.default_dir().join("yunagi.lock").exists());
    assert!(gone(&w.default_dir().join("yunagi.db")) && gone(&w.default_dir().join("config.json")));
    assert!(local.join("Other/keep").exists());
    assert!(out.stdout.contains("background"), "{}", out.stdout);
}

fn windows_without_purge_does_not_remove_the_default_dir() {
    let w = Sandbox::new();
    let root = w.root.join("local/Programs/Yunagi");
    let exe = root.join("bin/yunagi.exe");
    w.write(&exe, "cli");
    w.write(&root.join("uninstall.exe"), "nsis");
    w.use_app();
    let out = w.run(
        w.setup(Platform::Windows, &exe),
        &["uninstall"],
        "y\n",
        true,
    );
    assert_eq!(out.code, 0, "{} / {}", out.stdout, out.stderr);
    let detach = w.ops.state().detached.clone().unwrap();
    assert_eq!(detach.default_dir, None);
    assert!(w.default_dir().join("yunagi.db").exists());
}

fn windows_reports_a_cleanup_that_could_not_start() {
    let w = Sandbox::new();
    let root = w.root.join("local/Programs/Yunagi");
    let exe = root.join("bin/yunagi.exe");
    w.write(&exe, "cli");
    w.ops.state().spawn_fails = true;
    let out = w.run(
        w.setup(Platform::Windows, &exe),
        &["uninstall"],
        "y\n",
        true,
    );
    assert_eq!(out.code, 1, "{} / {}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("could not start the uninstaller"),
        "{}",
        out.stdout
    );
}

fn a_symlinked_default_dir_loses_only_the_link_and_the_data_is_listed() {
    let w = Sandbox::new();
    w.install_linux();
    w.use_app();
    // 既定ディレクトリを別の場所へ移してリンクにする
    let moved = w.root.join("moved-data");
    fs::rename(w.default_dir(), &moved).unwrap();
    std::os::unix::fs::symlink(&moved, w.default_dir()).unwrap();
    let out = w.run(w.linux_setup(), &["uninstall", "--purge"], "y\n", true);
    assert_eq!(out.code, 1, "{} / {}", out.stdout, out.stderr);
    assert!(gone(&w.default_dir()));
    assert!(moved.join("yunagi.db").exists(), "the target is untouched");
    assert!(
        out.stdout.contains(&moved.display().to_string()),
        "{}",
        out.stdout
    );
    assert!(gone(&w.linux_root()) && gone(&w.webview_dir()));
}

fn a_dangling_relative_link_into_the_install_is_removed() {
    let w = Sandbox::new();
    w.install_linux();
    fs::remove_file(w.link()).unwrap();
    // 先がもう無い（部分的に消えた後）相対リンク
    std::os::unix::fs::symlink("../opt/yunagi/bin/yunagi-gone", w.link()).unwrap();
    let out = w.run(w.linux_setup(), &["uninstall"], "y\n", true);
    assert_eq!(out.code, 0, "{} / {}", out.stdout, out.stderr);
    assert!(gone(&w.link()));
}

fn macos_forgets_the_preferences_even_without_the_plist_file() {
    let w = Sandbox::new();
    let app_exe = w
        .home()
        .join("Applications/Yunagi.app/Contents/MacOS/yunagi");
    w.write(&app_exe, "cli");
    w.write(
        &w.home()
            .join("Library/Caches")
            .join(APP_IDENTIFIER)
            .join("x"),
        "x",
    );
    let out = w.run(
        w.setup(Platform::MacOs, &app_exe),
        &["uninstall", "--purge"],
        "y\n",
        true,
    );
    assert_eq!(out.code, 0, "{} / {}", out.stdout, out.stderr);
    assert_eq!(w.ops.state().defaults_deleted, [APP_IDENTIFIER]);
}

fn windows_without_an_uninstaller_notes_it_and_still_cleans_up() {
    let w = Sandbox::new();
    let root = w.root.join("local/Programs/Yunagi");
    let exe = root.join("bin/yunagi.exe");
    w.write(&exe, "cli");
    let out = w.run(
        w.setup(Platform::Windows, &exe),
        &["uninstall"],
        "y\n",
        true,
    );
    assert_eq!(out.code, 0, "{} / {}", out.stdout, out.stderr);
    assert!(out.stdout.contains("was not found"), "{}", out.stdout);
    assert_eq!(w.ops.state().detached.clone().unwrap().uninstaller, None);
}

fn windows_does_not_change_the_path_when_the_cleanup_cannot_start() {
    let w = Sandbox::new();
    let root = w.root.join("local/Programs/Yunagi");
    let exe = root.join("bin/yunagi.exe");
    w.write(&exe, "cli");
    w.use_app();
    w.ops.state().user_path = root.join("bin").display().to_string();
    w.ops.state().spawn_fails = true;
    let out = w.run(
        w.setup(Platform::Windows, &exe),
        &["uninstall", "--purge"],
        "y\n",
        true,
    );
    // アプリが残ったままデータだけ消えることもない
    assert!(w.default_dir().join("yunagi.db").exists() && w.webview_dir().exists());
    assert_eq!(out.code, 1);
    assert_eq!(w.ops.state().written_path, None);
}

pub fn main() {
    let tests: [(&str, fn()); 20] = [
        (
            "uninstall_symlinked_default_dir",
            a_symlinked_default_dir_loses_only_the_link_and_the_data_is_listed,
        ),
        (
            "uninstall_dangling_relative_link",
            a_dangling_relative_link_into_the_install_is_removed,
        ),
        (
            "uninstall_macos_defaults_without_plist",
            macos_forgets_the_preferences_even_without_the_plist_file,
        ),
        (
            "uninstall_windows_without_an_uninstaller",
            windows_without_an_uninstaller_notes_it_and_still_cleans_up,
        ),
        (
            "uninstall_windows_keeps_the_path_when_the_cleanup_cannot_start",
            windows_does_not_change_the_path_when_the_cleanup_cannot_start,
        ),
        (
            "uninstall_keeps_the_data_without_purge",
            without_purge_keeps_the_data,
        ),
        (
            "uninstall_purge_removes_everything_of_yunagi",
            purge_removes_everything_of_yunagi,
        ),
        (
            "uninstall_purge_with_a_changed_data_dir_removes_only_the_database_files",
            purge_with_a_changed_data_dir_removes_only_the_database_files,
        ),
        (
            "uninstall_guard_stops_a_data_dir_outside_the_sandbox",
            a_data_dir_outside_the_sandbox_is_stopped_by_the_guard,
        ),
        (
            "uninstall_stops_when_the_lock_is_held",
            it_stops_when_the_app_or_an_agent_holds_the_lock,
        ),
        (
            "uninstall_broken_config_stops_purge_only",
            a_broken_config_stops_purge_but_not_a_plain_uninstall,
        ),
        (
            "uninstall_needs_a_terminal_and_a_yes",
            it_needs_a_terminal_and_a_yes,
        ),
        (
            "uninstall_does_not_touch_installs_that_are_not_ours",
            an_install_that_is_not_ours_is_not_touched,
        ),
        (
            "uninstall_without_a_default_dir_does_not_create_it",
            a_never_started_install_has_no_default_dir_to_lock,
        ),
        (
            "uninstall_lists_a_link_that_is_not_ours",
            a_link_that_is_not_ours_is_left_and_listed,
        ),
        (
            "uninstall_removes_symlinks_without_following_them",
            symlinks_are_removed_without_touching_their_targets,
        ),
        (
            "uninstall_lists_failures_and_goes_on",
            a_failure_is_listed_and_the_rest_goes_on,
        ),
        (
            "uninstall_macos_places_and_preferences",
            macos_removes_the_five_places_and_forgets_the_preferences,
        ),
        (
            "uninstall_windows_hands_the_rest_to_the_detached_cleanup",
            windows_leaves_the_install_dir_and_default_dir_to_the_detached_cleanup,
        ),
        (
            "uninstall_windows_without_purge_and_with_a_failed_start",
            || {
                windows_without_purge_does_not_remove_the_default_dir();
                windows_reports_a_cleanup_that_could_not_start();
            },
        ),
    ];
    let mut passed = 0;
    for (name, test) in tests {
        print!("test {name} ... ");
        test();
        println!("ok");
        passed += 1;
    }
    println!("\ntest result: ok. {passed} passed (uninstall)");
}
