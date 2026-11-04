//! OS 全体のショートカット：Yunagi を前面に呼び出す（操作仕様書 1章・2章、アーキテクチャ設計書 7章）。
//!
//! - 登録は Rust 側だけで行う（Tauri のプラグイン `tauri-plugin-global-shortcut` の Rust の API）。
//!   画面には、割当を渡す 1 つのコマンド（`set_global_shortcut`）だけを見せ、プラグインの JavaScript の
//!   API と権限（`capabilities/default.json` の `global-shortcut:*`・窓の操作）は渡さない。
//!   画面が任意のキーを OS に登録したり、窓を操作したりできないようにするため。
//! - 割当は画面が持つ（設定の `shortcuts`。keymap.ts）。画面は、いまの割当が変わるたびにこのコマンドを
//!   呼び、登録し直してもらう。割当が使えない状態のときは `None` を渡して外す。
//! - 登録できなかったときは、保存を断らず、理由を返す（[`GlobalShortcutState`]）。ほかのアプリが閉じれば
//!   使えるようになるため。
//! - プラグインを起動できなくても、アプリは起動する（[`install`]。状態は `Unavailable`）。
//!
//! 環境ごとの違い（実機の確認はテスト計画書 4.2）：
//! - Windows：`RegisterHotKey`。ほかのアプリが使っていれば `ERROR_HOTKEY_ALREADY_REGISTERED`（`Taken`）。
//! - macOS：`RegisterEventHotKey`。失敗は `Failed`（ほかのアプリが使っているときの返し方は未確認）。
//! - Linux の X11：`XGrabKey`。ほかのアプリが使っていれば `BadAccess`（`Taken`）。
//! - Linux の Wayland：XWayland の窓にフォーカスがあるときだけ効く。X の画面に接続できないと、プラグインは
//!   登録の失敗を返さずに成功とする（部品が黙って何もしない）ため、成否を知る手段が無い。そのため Wayland
//!   では、登録できても常に案内（デスクトップ環境のキー設定に `yunagi show` を割り当てる）を出す。

use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_global_shortcut::{Code, GlobalShortcut, Modifiers, Shortcut, ShortcutState};

use crate::types::{GlobalShortcutState, GlobalShortcutStatus};

/// 登録の失敗の種類。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterError {
    /// ほかのアプリ（または同じキーの登録）が使っている。
    Taken,
    /// そのほかの失敗。
    Other(String),
}

/// OS への登録。プラグインを使う本物（`PluginBackend`）と、テストの偽物がある。
pub trait Backend: Send {
    fn register(&mut self, shortcut: Shortcut) -> Result<(), RegisterError>;
    fn unregister(&mut self, shortcut: Shortcut);
}

/// 割当の文字列（`Mod+Shift+D`）を、OS に登録するキーにする。`mac` は `Mod` を ⌘（`SUPER`）にするか
/// （そうでなければ Ctrl）。
///
/// OS 全体のショートカットに使えるのは、修飾キーが 1 つ以上付いた英字・数字のキーだけ（keymap.ts
/// の `problemOf`。記号や名前のキーは、配列や OS で位置や扱いが違う）。形が合わなければ `None`。
pub fn parse(text: &str, mac: bool) -> Option<Shortcut> {
    let mut parts: Vec<&str> = text.split('+').collect();
    let key = parts.pop()?;
    let mut mods = Modifiers::empty();
    for p in parts {
        let m = match p {
            "Mod" if mac => Modifiers::SUPER,
            "Mod" => Modifiers::CONTROL,
            "Alt" => Modifiers::ALT,
            "Shift" => Modifiers::SHIFT,
            _ => return None,
        };
        if mods.contains(m) {
            return None;
        }
        mods |= m;
    }
    // Shift だけでは文字の入力と区別できない（keymap.ts の `no_modifier`）
    if !mods.intersects(Modifiers::SUPER | Modifiers::CONTROL | Modifiers::ALT) {
        return None;
    }
    let code = match key.as_bytes() {
        [c @ b'A'..=b'Z'] => format!("Key{}", *c as char),
        [c @ b'0'..=b'9'] => format!("Digit{}", *c as char),
        _ => return None,
    };
    let code: Code = code.parse().ok()?;
    Some(Shortcut::new(Some(mods), code))
}

/// Wayland のセッションか（Linux の環境変数。`XDG_SESSION_TYPE` が `wayland`、または `WAYLAND_DISPLAY` がある）。
pub fn is_wayland(session_type: Option<&str>, wayland_display: Option<&str>) -> bool {
    session_type.is_some_and(|s| s.eq_ignore_ascii_case("wayland"))
        || wayland_display.is_some_and(|d| !d.is_empty())
}

/// いまの環境が Wayland のセッションか（Linux だけ）。
fn wayland_now() -> bool {
    cfg!(target_os = "linux")
        && is_wayland(
            std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
            std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
        )
}

/// プラグインが返した失敗の文を、見分ける（プラグインは失敗を文で返すため。global-hotkey の
/// `AlreadyRegistered` の文は `HotKey already registered`）。
fn classify(message: &str) -> RegisterError {
    if message.starts_with("HotKey already registered") {
        RegisterError::Taken
    } else {
        RegisterError::Other(message.to_string())
    }
}

/// 窓を前面に出す（最小化を解き、隠れていれば表示し、フォーカスを移す）。窓が無い・操作できないときは
/// ログに出すだけ。
fn bring_to_front<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window("main") else {
        eprintln!("yunagi: the main window was not found");
        return;
    };
    for (what, result) in [
        ("unminimize", window.unminimize()),
        ("show", window.show()),
        ("set focus", window.set_focus()),
    ] {
        if let Err(e) = result {
            eprintln!("yunagi: failed to {what} the window: {e}");
        }
    }
}

/// プラグインを使った登録。
struct PluginBackend<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> PluginBackend<R> {
    fn plugin(&self) -> Option<tauri::State<'_, GlobalShortcut<R>>> {
        self.app.try_state::<GlobalShortcut<R>>()
    }
}

impl<R: Runtime> Backend for PluginBackend<R> {
    fn register(&mut self, shortcut: Shortcut) -> Result<(), RegisterError> {
        let Some(plugin) = self.plugin() else {
            return Err(RegisterError::Other("plugin is not installed".into()));
        };
        // 押したときだけ前面に出す（プラグインは、離したときにも処理を呼ぶ）
        plugin
            .on_shortcut(shortcut, |app, _, event| {
                if event.state == ShortcutState::Pressed {
                    bring_to_front(app);
                }
            })
            .map_err(|e| classify(&e.to_string()))
    }

    fn unregister(&mut self, shortcut: Shortcut) {
        if let Some(plugin) = self.plugin()
            && let Err(e) = plugin.unregister(shortcut)
        {
            eprintln!("yunagi: failed to unregister the global shortcut: {e}");
        }
    }
}

#[derive(Default)]
struct Inner {
    /// 部品が使えるときの登録の窓口。使えないとき（起動できなかった）は `None`。
    backend: Option<Box<dyn Backend>>,
    /// いま OS に登録している割当。
    current: Option<Shortcut>,
}

/// OS 全体のショートカットの登録を持つ（アプリに 1 つ）。登録は 1 つずつ順に行う。
#[derive(Default)]
pub struct GlobalShortcutController {
    inner: Mutex<Inner>,
    /// 判定を差し替えるため（テスト）。
    wayland: Option<bool>,
}

impl GlobalShortcutController {
    /// 部品はまだ使えない状態（[`GlobalShortcutController::attach`] で使えるようにする）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 部品を使えるようにする。
    pub fn attach(&self, backend: Box<dyn Backend>) {
        self.lock().backend = Some(backend);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        crate::app_core::lock(&self.inner)
    }

    fn status(&self, state: GlobalShortcutState) -> GlobalShortcutStatus {
        GlobalShortcutStatus {
            state,
            wayland: self.wayland.unwrap_or_else(wayland_now),
        }
    }

    /// OS に登録する割当を `binding` に変える（`None` は外す）。前の登録は先に外す。`mac` は
    /// `Mod` が ⌘ か。結果を返す。
    pub fn set(&self, binding: Option<&str>, mac: bool) -> GlobalShortcutStatus {
        let mut inner = self.lock();
        let Inner { backend, current } = &mut *inner;
        let Some(backend) = backend else {
            return self.status(GlobalShortcutState::Unavailable);
        };
        if let Some(old) = current.take() {
            backend.unregister(old);
        }
        let Some(text) = binding else {
            return self.status(GlobalShortcutState::Off);
        };
        let Some(shortcut) = parse(text, mac) else {
            eprintln!("yunagi: the global shortcut `{text}` can't be registered");
            return self.status(GlobalShortcutState::Failed);
        };
        match backend.register(shortcut) {
            Ok(()) => {
                *current = Some(shortcut);
                self.status(GlobalShortcutState::Registered)
            }
            Err(RegisterError::Taken) => self.status(GlobalShortcutState::Taken),
            Err(RegisterError::Other(e)) => {
                eprintln!("yunagi: failed to register the global shortcut `{text}`: {e}");
                self.status(GlobalShortcutState::Failed)
            }
        }
    }
}

/// プラグインを入れて、登録の窓口を使えるようにする。入れられなくても、アプリは起動を続ける
/// （以後の登録は `Unavailable` を返す）。
pub fn install<R: Runtime>(app: &AppHandle<R>) {
    match app.plugin(tauri_plugin_global_shortcut::Builder::new().build()) {
        Ok(()) => {
            app.state::<Arc<GlobalShortcutController>>()
                .attach(Box::new(PluginBackend { app: app.clone() }));
        }
        Err(e) => eprintln!("yunagi: the global shortcut is unavailable: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_maps_mod_by_os() {
        let pc = parse("Mod+Shift+D", false).unwrap();
        assert_eq!(
            pc,
            Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyD)
        );
        let mac = parse("Mod+Shift+D", true).unwrap();
        assert_eq!(
            mac,
            Shortcut::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::KeyD)
        );
        assert_eq!(
            parse("Alt+7", false).unwrap(),
            Shortcut::new(Some(Modifiers::ALT), Code::Digit7)
        );
    }

    #[test]
    fn parse_rejects_what_the_screen_does_not_assign() {
        for bad in [
            "",
            "D",
            "Shift+D",
            "Mod+",
            "Mod+Mod+D",
            "Mod+[",
            "Mod+Enter",
            "Mod+Shift+d",
            "Mod+DD",
            "Ctrl+D",
            "Mod+ArrowUp",
        ] {
            assert!(parse(bad, false).is_none(), "{bad}");
        }
    }

    #[test]
    fn wayland_is_detected_from_the_session() {
        assert!(is_wayland(Some("wayland"), None));
        assert!(is_wayland(Some("Wayland"), None));
        assert!(is_wayland(None, Some("wayland-1")));
        assert!(is_wayland(Some("x11"), Some("wayland-0")));
        assert!(!is_wayland(Some("x11"), None));
        assert!(!is_wayland(None, Some("")));
        assert!(!is_wayland(None, None));
    }

    #[test]
    fn classify_finds_the_taken_key() {
        assert_eq!(
            classify("HotKey already registered: HotKey { mods: CONTROL, key: KeyD, id: 1 }"),
            RegisterError::Taken
        );
        assert_eq!(
            classify("Unable to register hotkey: x"),
            RegisterError::Other("Unable to register hotkey: x".into())
        );
    }

    /// 登録の呼び出しを記録し、`taken` のキーは断る偽物。
    #[derive(Default)]
    struct Fake {
        log: Arc<Mutex<Vec<String>>>,
        taken: Option<Shortcut>,
        fail: bool,
    }

    impl Backend for Fake {
        fn register(&mut self, s: Shortcut) -> Result<(), RegisterError> {
            self.log.lock().unwrap().push(format!("register {}", s.key));
            if self.fail {
                return Err(RegisterError::Other("x".into()));
            }
            if self.taken == Some(s) {
                return Err(RegisterError::Taken);
            }
            Ok(())
        }
        fn unregister(&mut self, s: Shortcut) {
            self.log
                .lock()
                .unwrap()
                .push(format!("unregister {}", s.key));
        }
    }

    fn controller(fake: Fake, wayland: bool) -> GlobalShortcutController {
        let c = GlobalShortcutController {
            wayland: Some(wayland),
            ..Default::default()
        };
        c.attach(Box::new(fake));
        c
    }

    #[test]
    fn without_the_plugin_it_is_unavailable() {
        let c = GlobalShortcutController {
            wayland: Some(false),
            ..Default::default()
        };
        assert_eq!(
            c.set(Some("Mod+Shift+D"), false).state,
            GlobalShortcutState::Unavailable
        );
        assert_eq!(c.set(None, false).state, GlobalShortcutState::Unavailable);
    }

    #[test]
    fn registers_replaces_and_removes() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let c = controller(
            Fake {
                log: log.clone(),
                ..Default::default()
            },
            false,
        );
        assert_eq!(
            c.set(Some("Mod+Shift+D"), false).state,
            GlobalShortcutState::Registered
        );
        // 割当を変えると、前の登録を先に外してから登録する
        assert_eq!(
            c.set(Some("Mod+Shift+E"), false).state,
            GlobalShortcutState::Registered
        );
        assert_eq!(c.set(None, false).state, GlobalShortcutState::Off);
        // 外した後に外しても、何もしない
        assert_eq!(c.set(None, false).state, GlobalShortcutState::Off);
        assert_eq!(
            *log.lock().unwrap(),
            [
                "register KeyD",
                "unregister KeyD",
                "register KeyE",
                "unregister KeyE"
            ]
        );
    }

    #[test]
    fn setting_the_same_key_again_registers_it_again() {
        // 同じキーのまま呼ばれても（データベースを開き直したときなど）、外してから登録し直す
        let log = Arc::new(Mutex::new(Vec::new()));
        let c = controller(
            Fake {
                log: log.clone(),
                ..Default::default()
            },
            false,
        );
        c.set(Some("Mod+Shift+D"), false);
        c.set(Some("Mod+Shift+D"), false);
        assert_eq!(
            *log.lock().unwrap(),
            ["register KeyD", "unregister KeyD", "register KeyD"]
        );
    }

    #[test]
    fn a_taken_key_is_reported_and_leaves_nothing_registered() {
        let taken = parse("Mod+Shift+D", false);
        let log = Arc::new(Mutex::new(Vec::new()));
        let c = controller(
            Fake {
                log: log.clone(),
                taken,
                ..Default::default()
            },
            false,
        );
        assert_eq!(
            c.set(Some("Mod+Shift+D"), false).state,
            GlobalShortcutState::Taken
        );
        // 登録できなかったキーは、後で外さない（登録していないため）
        assert_eq!(c.set(None, false).state, GlobalShortcutState::Off);
        assert_eq!(*log.lock().unwrap(), ["register KeyD"]);
    }

    #[test]
    fn other_failures_and_unparsable_keys_fail() {
        let c = controller(
            Fake {
                fail: true,
                ..Default::default()
            },
            false,
        );
        assert_eq!(
            c.set(Some("Mod+Shift+D"), false).state,
            GlobalShortcutState::Failed
        );
        assert_eq!(
            c.set(Some("Mod+["), false).state,
            GlobalShortcutState::Failed
        );
    }

    #[test]
    fn the_status_carries_the_wayland_flag() {
        let c = controller(Fake::default(), true);
        let s = c.set(Some("Mod+Shift+D"), false);
        assert_eq!(
            s,
            GlobalShortcutStatus {
                state: GlobalShortcutState::Registered,
                wayland: true
            }
        );
    }
}
