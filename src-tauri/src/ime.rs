//! 日本語入力（IME）の切替（操作仕様書 4.4、アーキテクチャ設計書 7章）。
//!
//! Vim モードで挿入モードを抜けたとき（ノーマルモードに戻ったとき）に、IME を英数入力に切り替え、
//! 挿入モードに入ったとき、抜けた時点の入力状態に戻す。切替は OS ごとに別の仕組みで行う
//! （[`Backend`]）。どれも「ベストエフォート」で、切り替えられなくても画面の操作は止めない
//! （失敗はログに出すだけ。ノーマルモード中の扱いは画面側が保証する。操作仕様書 4.4 の 2）。
//!
//! - Linux：fcitx5 は `fcitx5-remote`、IBus は `ibus engine` のコマンドを呼ぶ。どちらも D-Bus で
//!   常駐プロセスに頼むコマンドで、D-Bus の部品（zbus など）を新たに依存に入れずに済む。
//! - macOS：Carbon の入力ソース API（Text Input Sources。`TISSelectInputSource` など）。メインスレッドで呼ぶ。
//! - Windows：IMM のオープン状態（`WM_IME_CONTROL` の `IMC_SETOPENSTATUS`）。
//!
//! 切替の呼び出しは、1本の専用スレッド（[`ImeWorker`]）に順に渡す。Esc の直後に `i` を押しても、
//! 英数への切替より先に復元が走らないようにするため。呼ぶ側（画面）は待たない。

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

use crate::types::{ImeMethod, ImeStatus};

/// 切替の失敗。ログに出すだけ。
type Failure = String;

/// 1つの OS の入力切替の仕組み。入力状態は、仕組みごとに決めた文字列（呼び出し側は中身を見ない）で
/// 受け渡す：fcitx5 は `active` / `inactive`、IBus はエンジン名、macOS は入力ソース ID、Windows は
/// `open` / `closed`。
pub trait Backend: Send {
    fn method(&self) -> ImeMethod;
    /// いまの入力状態。
    fn current(&mut self) -> Result<String, Failure>;
    /// 英数入力へ切り替える（`current` は、直前に得たいまの入力状態）。
    fn to_english(&mut self, current: &str) -> Result<(), Failure>;
    /// `saved`（[`Backend::current`] で得たもの）の状態へ戻す。
    fn restore(&mut self, saved: &str) -> Result<(), Failure>;
}

/// 挿入モードの出入りに合わせた切替の状態。直前に挿入モードを抜けた時点の入力状態を、1つだけ持つ
/// （整理モードのリストとエディタで共通。操作仕様書 4.4 の 3）。
pub struct Session {
    detect: Box<dyn FnMut() -> Option<Box<dyn Backend>> + Send>,
    backend: Option<Box<dyn Backend>>,
    /// 仕組みが見つからなかったとき、この時刻までは探し直さない。
    none_until: Option<Instant>,
    saved: Option<String>,
}

/// 使える仕組みが見つからなかったあと、探し直すまでの時間。
const DETECT_BACKOFF: Duration = Duration::from_secs(5);

impl Session {
    /// `detect` は、使える仕組みを探す（見つからなければ `None`。切替のたびに、無ければ探し直す。
    /// 利用者が後から IME を起動した場合に使えるようにするため。見つからなかった直後の
    /// 5 秒の間は探し直さない）。
    pub fn new(detect: impl FnMut() -> Option<Box<dyn Backend>> + Send + 'static) -> Self {
        Self {
            detect: Box::new(detect),
            backend: None,
            none_until: None,
            saved: None,
        }
    }

    fn backend(&mut self) -> Option<&mut Box<dyn Backend>> {
        if self.backend.is_none() {
            // 見つからなかった直後は、探し直さない（IME の無い環境で、Esc のたびにコマンドを
            // 起動し続けないため）
            if self.none_until.is_some_and(|t| Instant::now() < t) {
                return None;
            }
            self.backend = (self.detect)();
            self.none_until = self
                .backend
                .is_none()
                .then(|| Instant::now() + DETECT_BACKOFF);
        }
        self.backend.as_mut()
    }

    /// 挿入モードを抜けた：いまの入力状態を覚えて、英数入力へ切り替える。
    pub fn leave_insert(&mut self) {
        // 前の状態は、新しく覚え直す（切替に失敗しても、古い状態に戻さない）
        self.saved = None;
        let Some(b) = self.backend() else { return };
        let result = match b.current() {
            Ok(cur) => {
                let r = b.to_english(&cur);
                // 切替に失敗しても、抜けた時点の状態は覚える（戻すときに、実際の状態へそろう）
                self.saved = Some(cur);
                r
            }
            Err(e) => Err(e),
        };
        if let Err(e) = result {
            eprintln!("yunagi: failed to switch the input method to English: {e}");
            // 仕組みが使えなくなったのかもしれないので、しばらく置いてからもう一度探す（失敗が続く環境で、
            // Esc のたびにコマンドを起動し続けない）
            self.backend = None;
            self.none_until = Some(Instant::now() + DETECT_BACKOFF);
        }
    }

    /// 挿入モードに入った：覚えていた入力状態を捨て、`restore` なら抜けた時点の入力状態に戻す
    /// （覚えているものがなければ何もしない）。設定が無効のときも入ったことは知らせる（抜けた後に
    /// 入らないまま、もう一度抜けた状態を、古い入力状態で上書きしないため）。
    pub fn enter_insert(&mut self, restore: bool) {
        let Some(saved) = self.saved.take() else {
            return;
        };
        if !restore {
            return;
        }
        let Some(b) = self.backend() else { return };
        // 戻せなくても（入力ソースが無効にされたなど）、仕組みそのものは使えるので捨てない
        if let Err(e) = b.restore(&saved) {
            eprintln!("yunagi: failed to restore the input method: {e}");
        }
    }
}

enum Msg {
    LeaveInsert,
    EnterInsert { restore: bool },
}

/// 切替を順に行う専用のスレッド。アプリに1つ持たせる（`State<ImeWorker>`）。
pub struct ImeWorker {
    tx: Mutex<Sender<Msg>>,
}

impl ImeWorker {
    /// 専用のスレッドを起動する。`session` はそのスレッドが持つ。
    pub fn new(mut session: Session) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("yunagi-ime".into())
            .spawn(move || {
                for msg in rx {
                    match msg {
                        Msg::LeaveInsert => session.leave_insert(),
                        Msg::EnterInsert { restore } => session.enter_insert(restore),
                    }
                }
            })
            .expect("IME の切替のスレッドを起動できません");
        Self { tx: Mutex::new(tx) }
    }

    /// この OS の仕組みで動かす。
    pub fn system<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Self {
        Self::new(Session::new(move || system_backend(&app)))
    }

    pub fn leave_insert(&self) {
        self.send(Msg::LeaveInsert);
    }

    pub fn enter_insert(&self, restore: bool) {
        self.send(Msg::EnterInsert { restore });
    }

    fn send(&self, msg: Msg) {
        if let Ok(tx) = self.tx.lock() {
            // スレッドが終わっていたら（アプリの終了中）何もしない
            let _ = tx.send(msg);
        }
    }
}

/// この環境で入力の切替ができるか（設定画面の制約の表示に使う。操作仕様書 4.4 の 4）。
pub fn status() -> ImeStatus {
    let method = system_method();
    ImeStatus {
        available: method.is_some(),
        method,
    }
}

/// この OS で使う仕組み（Linux は、fcitx5 か IBus が動いているときだけ）。
fn system_method() -> Option<ImeMethod> {
    #[cfg(target_os = "linux")]
    return linux::detect(&SystemRunner).map(|b| b.method());
    #[cfg(target_os = "macos")]
    return Some(ImeMethod::MacosInputSource);
    #[cfg(target_os = "windows")]
    return Some(ImeMethod::WindowsImm);
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    None
}

#[cfg(target_os = "linux")]
fn system_backend<R: tauri::Runtime>(_: &tauri::AppHandle<R>) -> Option<Box<dyn Backend>> {
    linux::detect(&SystemRunner)
}

#[cfg(target_os = "macos")]
fn system_backend<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Option<Box<dyn Backend>> {
    Some(Box::new(mac::InputSources::new(app.clone())))
}

#[cfg(target_os = "windows")]
fn system_backend<R: tauri::Runtime>(_: &tauri::AppHandle<R>) -> Option<Box<dyn Backend>> {
    Some(Box::new(win::Imm))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn system_backend<R: tauri::Runtime>(_: &tauri::AppHandle<R>) -> Option<Box<dyn Backend>> {
    None
}

/// コマンドを呼ぶ口（テストでは差し替える）。
pub trait Runner: Send {
    /// 呼んだ結果（標準出力）。コマンドが無い・失敗したときは `None`。
    fn run(&self, program: &str, args: &[&str]) -> Option<String>;
}

/// 実際にコマンドを呼ぶ。
#[derive(Clone)]
pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&self, program: &str, args: &[&str]) -> Option<String> {
        // 常駐プロセスが固まっていても、切替のスレッドが止まり続けないよう、待つ時間に上限を置く
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut stdout = child.stdout.take()?;
        // 出力は別のスレッドで読む（読まないと、出力が多いときにコマンドが書き込みで止まる）
        let reader = std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = stdout.read_to_string(&mut buf);
            buf
        });
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        let status = loop {
            match child.try_wait().ok()? {
                Some(status) => break status,
                None if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                None => std::thread::sleep(Duration::from_millis(5)),
            }
        };
        let out = reader.join().ok()?;
        status.success().then(|| out.trim().to_owned())
    }
}

/// コマンドの終わりを待つ時間の上限。
const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);

pub mod linux {
    //! Linux：fcitx5（`fcitx5-remote`）と IBus（`ibus engine`）。どちらも常駐プロセスが無ければ
    //! 使えない環境として扱い、設定画面に制約を表示する。

    use super::{Backend, Failure, ImeMethod, Runner};

    /// IBus で英数入力にするエンジン（キーボード配列のエンジン）。
    const IBUS_ENGLISH: &str = "xkb:us::eng";

    /// 使える仕組みを探す：fcitx5 が動いていればそれを、そうでなければ IBus を使う
    /// （fcitx5 は IBus と同じ名前の D-Bus 口も出すので、先に fcitx5 を見る）。
    pub fn detect<R: Runner + Clone + 'static>(runner: &R) -> Option<Box<dyn Backend>> {
        let fcitx = Fcitx5(runner.clone());
        if fcitx.state().is_some() {
            return Some(Box::new(fcitx));
        }
        let ibus = Ibus(runner.clone());
        if ibus.engine().is_some() {
            return Some(Box::new(ibus));
        }
        None
    }

    /// fcitx5。`fcitx5-remote` は、入力の状態を `1`（英数・キーボード配列）・`2`（IME が有効）で出す
    /// （`0` は fcitx5 が動いていない）。
    pub struct Fcitx5<R>(pub R);

    impl<R: Runner> Fcitx5<R> {
        fn state(&self) -> Option<bool> {
            match self.0.run("fcitx5-remote", &[])?.as_str() {
                "1" => Some(false),
                "2" => Some(true),
                _ => None,
            }
        }

        fn call(&self, arg: &str) -> Result<(), Failure> {
            self.0
                .run("fcitx5-remote", &[arg])
                .map(|_| ())
                .ok_or_else(|| format!("fcitx5-remote {arg} failed"))
        }
    }

    impl<R: Runner> Backend for Fcitx5<R> {
        fn method(&self) -> ImeMethod {
            ImeMethod::Fcitx5
        }

        fn current(&mut self) -> Result<String, Failure> {
            match self.state() {
                Some(true) => Ok("active".into()),
                Some(false) => Ok("inactive".into()),
                None => Err("fcitx5 is not running".into()),
            }
        }

        fn to_english(&mut self, current: &str) -> Result<(), Failure> {
            if current == "active" {
                self.call("-c")
            } else {
                Ok(())
            }
        }

        fn restore(&mut self, saved: &str) -> Result<(), Failure> {
            self.call(if saved == "active" { "-o" } else { "-c" })
        }
    }

    /// IBus。エンジン名（`mozc-jp`・`xkb:us::eng` など）で状態を持つ。
    pub struct Ibus<R>(pub R);

    impl<R: Runner> Ibus<R> {
        fn engine(&self) -> Option<String> {
            let name = self.0.run("ibus", &["engine"])?;
            (!name.is_empty()).then_some(name)
        }

        fn set(&self, name: &str) -> Result<(), Failure> {
            self.0
                .run("ibus", &["engine", name])
                .map(|_| ())
                .ok_or_else(|| format!("ibus engine {name} failed"))
        }
    }

    impl<R: Runner> Backend for Ibus<R> {
        fn method(&self) -> ImeMethod {
            ImeMethod::Ibus
        }

        fn current(&mut self) -> Result<String, Failure> {
            self.engine().ok_or_else(|| "ibus is not running".into())
        }

        fn to_english(&mut self, current: &str) -> Result<(), Failure> {
            if current.starts_with("xkb:") {
                Ok(())
            } else {
                self.set(IBUS_ENGLISH)
            }
        }

        fn restore(&mut self, saved: &str) -> Result<(), Failure> {
            self.set(saved)
        }
    }
}

#[cfg(target_os = "macos")]
mod mac {
    //! macOS：入力ソース API（Carbon の Text Input Sources）。この API はメインスレッドで呼ぶ
    //! （別のスレッドから呼ぶと macOS が異常終了させることがある）ので、アプリのメインスレッドに
    //! 頼んで、終わるのを待つ。
    //!
    //! 実機の macOS で確かめていない（テスト計画書 4.2）。

    use std::ffi::c_void;
    use std::sync::mpsc;

    use core_foundation::array::{CFArray, CFArrayRef};
    use core_foundation::base::{CFRelease, CFType, CFTypeRef, TCFType};
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::string::{CFString, CFStringRef};

    use super::{Backend, Failure, ImeMethod};

    type InputSource = *const c_void;

    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        fn TISCopyCurrentKeyboardInputSource() -> InputSource;
        fn TISCopyCurrentASCIICapableKeyboardInputSource() -> InputSource;
        fn TISGetInputSourceProperty(source: InputSource, key: CFStringRef) -> CFTypeRef;
        fn TISSelectInputSource(source: InputSource) -> i32;
        fn TISCreateInputSourceList(properties: CFDictionaryRef, include_all: u8) -> CFArrayRef;
        static kTISPropertyInputSourceID: CFStringRef;
    }

    /// 入力ソースの ID（`com.apple.keylayout.ABC` など）。`source` は借りるだけ。
    unsafe fn id_of(source: InputSource) -> Option<String> {
        if source.is_null() {
            return None;
        }
        // SAFETY: `source` は有効な入力ソース。取得した文字列は `source` が持つ（get の規則）
        let p = unsafe { TISGetInputSourceProperty(source, kTISPropertyInputSourceID) };
        if p.is_null() {
            return None;
        }
        Some(unsafe { CFString::wrap_under_get_rule(p as CFStringRef) }.to_string())
    }

    pub struct InputSources<R: tauri::Runtime> {
        app: tauri::AppHandle<R>,
    }

    impl<R: tauri::Runtime> InputSources<R> {
        pub fn new(app: tauri::AppHandle<R>) -> Self {
            Self { app }
        }

        /// `f` をメインスレッドで動かして、結果を待つ。
        fn on_main<T: Send + 'static>(
            &self,
            f: impl FnOnce() -> T + Send + 'static,
        ) -> Result<T, Failure> {
            let (tx, rx) = mpsc::channel();
            // 待ちきれなかった処理は、メインスレッドが空いてから遅れて走らないよう取り消す（遅れて走ると、
            // 後に頼んだ切替と順序が入れ替わる）
            let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let flag = std::sync::Arc::clone(&cancelled);
            self.app
                .run_on_main_thread(move || {
                    if flag.load(std::sync::atomic::Ordering::SeqCst) {
                        return;
                    }
                    let _ = tx.send(f());
                })
                .map_err(|e| e.to_string())?;
            // メインスレッドが固まっていても、切替のスレッドが止まり続けないよう、待つ時間に上限を置く
            rx.recv_timeout(std::time::Duration::from_secs(2))
                .map_err(|e| {
                    cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
                    e.to_string()
                })
        }
    }

    impl<R: tauri::Runtime> Backend for InputSources<R> {
        fn method(&self) -> ImeMethod {
            ImeMethod::MacosInputSource
        }

        fn current(&mut self) -> Result<String, Failure> {
            self.on_main(|| unsafe {
                let src = TISCopyCurrentKeyboardInputSource();
                let id = id_of(src);
                if !src.is_null() {
                    CFRelease(src);
                }
                id.ok_or_else(|| "no current input source".to_owned())
            })?
        }

        fn to_english(&mut self, current: &str) -> Result<(), Failure> {
            let current = current.to_owned();
            self.on_main(move || unsafe {
                let src = TISCopyCurrentASCIICapableKeyboardInputSource();
                if src.is_null() {
                    return Err("no ASCII-capable input source".to_owned());
                }
                let result = if id_of(src).as_deref() == Some(current.as_str()) {
                    Ok(())
                } else {
                    match TISSelectInputSource(src) {
                        0 => Ok(()),
                        status => Err(format!("TISSelectInputSource failed: {status}")),
                    }
                };
                CFRelease(src);
                result
            })?
        }

        fn restore(&mut self, saved: &str) -> Result<(), Failure> {
            let saved = saved.to_owned();
            self.on_main(move || unsafe {
                let key = CFString::wrap_under_get_rule(kTISPropertyInputSourceID);
                let filter = CFDictionary::from_CFType_pairs(&[(
                    key.as_CFType(),
                    CFString::new(&saved).as_CFType(),
                )]);
                let list = TISCreateInputSourceList(filter.as_concrete_TypeRef(), 0);
                if list.is_null() {
                    return Err(format!("input source not found: {saved}"));
                }
                let list = CFArray::<CFType>::wrap_under_create_rule(list);
                let Some(src) = list.get(0) else {
                    return Err(format!("input source not found: {saved}"));
                };
                match TISSelectInputSource(src.as_CFTypeRef()) {
                    0 => Ok(()),
                    status => Err(format!("TISSelectInputSource failed: {status}")),
                }
            })?
        }
    }
}

#[cfg(target_os = "windows")]
mod win {
    //! Windows：IMM のオープン状態（`IMC_SETOPENSTATUS`）。閉じると英数入力（直接入力）になる。
    //! 前面にあるのがこのアプリの窓のときだけ触る（別のアプリの入力状態を変えないため）。入力の
    //! フォーカスは WebView2 の子の窓にあるので、フォーカスのある窓の IME の窓へ送る。
    //!
    //! 実機の Windows で確かめていない（テスト計画書 4.2）。

    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::Input::Ime::ImmGetDefaultIMEWnd;
    use windows::Win32::UI::WindowsAndMessaging::{
        GUITHREADINFO, GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId,
        SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_IME_CONTROL,
    };

    use super::{Backend, Failure, ImeMethod};

    const IMC_GETOPENSTATUS: usize = 0x0005;
    const IMC_SETOPENSTATUS: usize = 0x0006;

    pub struct Imm;

    /// このアプリが前面にあるときの、IME の窓。
    fn ime_window() -> Result<HWND, Failure> {
        // SAFETY: 窓のハンドルを問い合わせるだけ（Win32 の呼び出し）
        unsafe {
            let fg = GetForegroundWindow();
            if fg.0.is_null() {
                return Err("no foreground window".into());
            }
            let mut pid = 0u32;
            let thread = GetWindowThreadProcessId(fg, Some(&mut pid));
            if pid != std::process::id() {
                return Err("Yunagi is not in the foreground".into());
            }
            let mut info = GUITHREADINFO {
                cbSize: size_of::<GUITHREADINFO>() as u32,
                ..Default::default()
            };
            let target = match GetGUIThreadInfo(thread, &mut info) {
                Ok(()) if !info.hwndFocus.0.is_null() => info.hwndFocus,
                _ => fg,
            };
            let ime = ImmGetDefaultIMEWnd(target);
            if ime.0.is_null() {
                return Err("no IME window".into());
            }
            Ok(ime)
        }
    }

    fn control(command: usize, value: isize) -> Result<isize, Failure> {
        let ime = ime_window()?;
        // SAFETY: IME の窓へ、オープン状態の問い合わせ・設定の通知を送るだけ
        // 応答しない窓で止まり続けないよう、待つ時間に上限を置く（固まっている窓は待たない）
        let mut result = 0usize;
        let sent = unsafe {
            SendMessageTimeoutW(
                ime,
                WM_IME_CONTROL,
                WPARAM(command),
                LPARAM(value),
                SMTO_ABORTIFHUNG,
                1000,
                Some(&mut result),
            )
        };
        if sent.0 == 0 {
            return Err("the IME window did not respond".into());
        }
        Ok(result as isize)
    }

    impl Backend for Imm {
        fn method(&self) -> ImeMethod {
            ImeMethod::WindowsImm
        }

        fn current(&mut self) -> Result<String, Failure> {
            Ok(if control(IMC_GETOPENSTATUS, 0)? != 0 {
                "open"
            } else {
                "closed"
            }
            .into())
        }

        fn to_english(&mut self, current: &str) -> Result<(), Failure> {
            if current == "open" {
                control(IMC_SETOPENSTATUS, 0).map(|_| ())
            } else {
                Ok(())
            }
        }

        fn restore(&mut self, saved: &str) -> Result<(), Failure> {
            control(IMC_SETOPENSTATUS, isize::from(saved == "open")).map(|_| ())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use super::linux::{Fcitx5, Ibus};
    use super::*;

    /// 呼ばれた順を記録し、状態を持つ偽のバックエンド。
    #[derive(Clone, Default)]
    struct Fake {
        log: Arc<Mutex<Vec<String>>>,
        state: Arc<Mutex<String>>,
        fail: Arc<Mutex<bool>>,
    }

    impl Backend for Fake {
        fn method(&self) -> ImeMethod {
            ImeMethod::Fcitx5
        }

        fn current(&mut self) -> Result<String, Failure> {
            if *self.fail.lock().unwrap() {
                return Err("down".into());
            }
            Ok(self.state.lock().unwrap().clone())
        }

        fn to_english(&mut self, current: &str) -> Result<(), Failure> {
            self.log
                .lock()
                .unwrap()
                .push(format!("english from {current}"));
            *self.state.lock().unwrap() = "english".into();
            Ok(())
        }

        fn restore(&mut self, saved: &str) -> Result<(), Failure> {
            self.log.lock().unwrap().push(format!("restore {saved}"));
            *self.state.lock().unwrap() = saved.into();
            Ok(())
        }
    }

    fn session(fake: &Fake) -> Session {
        let fake = fake.clone();
        Session::new(move || Some(Box::new(fake.clone())))
    }

    #[test]
    fn leaving_insert_switches_to_english_and_entering_restores_the_state_at_exit() {
        let fake = Fake::default();
        *fake.state.lock().unwrap() = "kana".into();
        let mut s = session(&fake);
        s.leave_insert();
        assert_eq!(*fake.state.lock().unwrap(), "english");
        // ノーマルモード中に利用者が入力状態を変えても、戻すのは抜けた時点のもの
        *fake.state.lock().unwrap() = "other".into();
        s.enter_insert(true);
        assert_eq!(*fake.state.lock().unwrap(), "kana");
        assert_eq!(
            *fake.log.lock().unwrap(),
            ["english from kana", "restore kana"]
        );
    }

    #[test]
    fn restoring_happens_once_per_exit_and_not_before_the_first_exit() {
        let fake = Fake::default();
        *fake.state.lock().unwrap() = "kana".into();
        let mut s = session(&fake);
        s.enter_insert(true);
        assert!(fake.log.lock().unwrap().is_empty());
        s.leave_insert();
        s.enter_insert(true);
        s.enter_insert(true);
        assert_eq!(
            *fake.log.lock().unwrap(),
            ["english from kana", "restore kana"]
        );
    }

    #[test]
    fn the_latest_exit_wins() {
        let fake = Fake::default();
        let mut s = session(&fake);
        *fake.state.lock().unwrap() = "first".into();
        s.leave_insert();
        *fake.state.lock().unwrap() = "second".into();
        s.leave_insert();
        s.enter_insert(true);
        assert_eq!(*fake.state.lock().unwrap(), "second");
    }

    #[test]
    fn a_failure_is_not_reported_and_the_backend_is_not_used_again_right_away() {
        let fake = Fake::default();
        let counter = Arc::new(Mutex::new(0));
        let c = Arc::clone(&counter);
        let f = fake.clone();
        let mut s = Session::new(move || {
            *c.lock().unwrap() += 1;
            Some(Box::new(f.clone()))
        });
        *fake.fail.lock().unwrap() = true;
        s.leave_insert();
        s.enter_insert(true);
        assert!(fake.log.lock().unwrap().is_empty());
        // 失敗の直後は探し直さず、何もしない
        *fake.fail.lock().unwrap() = false;
        *fake.state.lock().unwrap() = "kana".into();
        s.leave_insert();
        assert_eq!(*counter.lock().unwrap(), 1);
        assert_eq!(*fake.state.lock().unwrap(), "kana");
        // 間隔を過ぎれば、探し直して使える
        s.none_until = Some(Instant::now());
        s.leave_insert();
        assert_eq!(*counter.lock().unwrap(), 2);
        assert_eq!(*fake.state.lock().unwrap(), "english");
    }

    #[test]
    fn nothing_happens_without_a_backend() {
        let mut s = Session::new(|| None);
        s.leave_insert();
        s.enter_insert(true);
    }

    #[test]
    fn a_missing_backend_is_not_looked_up_again_right_away() {
        let counter = Arc::new(Mutex::new(0));
        let c = Arc::clone(&counter);
        let mut s = Session::new(move || {
            *c.lock().unwrap() += 1;
            None
        });
        for _ in 0..5 {
            s.leave_insert();
            s.enter_insert(true);
        }
        assert_eq!(*counter.lock().unwrap(), 1);
    }

    #[test]
    fn entering_without_restore_forgets_the_exit_state() {
        let fake = Fake::default();
        let mut s = session(&fake);
        *fake.state.lock().unwrap() = "kana".into();
        s.leave_insert();
        // 設定が無効：戻さないが、入ったことで覚えていた状態は捨てる
        s.enter_insert(false);
        s.enter_insert(true);
        assert_eq!(*fake.log.lock().unwrap(), ["english from kana"]);
        assert_eq!(*fake.state.lock().unwrap(), "english");
    }

    #[test]
    fn a_failed_restore_keeps_the_backend() {
        let fake = Fake::default();
        let counter = Arc::new(Mutex::new(0));
        let c = Arc::clone(&counter);
        let f = fake.clone();
        let mut s = Session::new(move || {
            *c.lock().unwrap() += 1;
            Some(Box::new(f.clone()))
        });
        *fake.state.lock().unwrap() = "kana".into();
        s.leave_insert();
        s.enter_insert(true);
        s.leave_insert();
        s.enter_insert(true);
        assert_eq!(*counter.lock().unwrap(), 1);
    }

    /// 呼ばれたコマンドを記録し、あらかじめ決めた出力を返す偽の `Runner`。
    #[derive(Clone, Default)]
    struct Script {
        out: Arc<Mutex<HashMap<String, Option<String>>>>,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl Script {
        fn on(&self, line: &str, out: Option<&str>) {
            self.out
                .lock()
                .unwrap()
                .insert(line.into(), out.map(Into::into));
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Runner for Script {
        fn run(&self, program: &str, args: &[&str]) -> Option<String> {
            let line = std::iter::once(program)
                .chain(args.iter().copied())
                .collect::<Vec<_>>()
                .join(" ");
            self.calls.lock().unwrap().push(line.clone());
            self.out.lock().unwrap().get(&line).cloned().flatten()
        }
    }

    #[test]
    fn fcitx5_is_switched_with_fcitx5_remote() {
        let r = Script::default();
        r.on("fcitx5-remote", Some("2"));
        let mut b = Fcitx5(r.clone());
        assert_eq!(b.current().unwrap(), "active");
        r.on("fcitx5-remote -c", Some(""));
        b.to_english("active").unwrap();
        r.on("fcitx5-remote -o", Some(""));
        b.restore("active").unwrap();
        assert_eq!(
            r.calls(),
            ["fcitx5-remote", "fcitx5-remote -c", "fcitx5-remote -o"]
        );
    }

    #[test]
    fn fcitx5_already_in_english_is_left_alone_and_restored_as_english() {
        let r = Script::default();
        r.on("fcitx5-remote", Some("1"));
        let mut b = Fcitx5(r.clone());
        assert_eq!(b.current().unwrap(), "inactive");
        b.to_english("inactive").unwrap();
        assert_eq!(r.calls(), ["fcitx5-remote"]);
        r.on("fcitx5-remote -c", Some(""));
        b.restore("inactive").unwrap();
        assert_eq!(r.calls().last().unwrap(), "fcitx5-remote -c");
    }

    #[test]
    fn fcitx5_not_running_is_an_error() {
        let r = Script::default();
        r.on("fcitx5-remote", Some("0"));
        assert!(Fcitx5(r).current().is_err());
    }

    #[test]
    fn ibus_is_switched_by_engine_name() {
        let r = Script::default();
        r.on("ibus engine", Some("mozc-jp"));
        let mut b = Ibus(r.clone());
        assert_eq!(b.current().unwrap(), "mozc-jp");
        r.on("ibus engine xkb:us::eng", Some(""));
        b.to_english("mozc-jp").unwrap();
        r.on("ibus engine mozc-jp", Some(""));
        b.restore("mozc-jp").unwrap();
        assert_eq!(
            r.calls(),
            [
                "ibus engine",
                "ibus engine xkb:us::eng",
                "ibus engine mozc-jp"
            ]
        );
    }

    #[test]
    fn ibus_in_a_keyboard_engine_is_left_alone() {
        let r = Script::default();
        let mut b = Ibus(r.clone());
        b.to_english("xkb:jp::jpn").unwrap();
        assert!(r.calls().is_empty());
    }

    #[test]
    fn detection_prefers_fcitx5_then_ibus_then_none() {
        let r = Script::default();
        assert!(super::linux::detect(&r).is_none());
        r.on("ibus engine", Some("mozc-jp"));
        assert_eq!(super::linux::detect(&r).unwrap().method(), ImeMethod::Ibus);
        r.on("fcitx5-remote", Some("1"));
        assert_eq!(
            super::linux::detect(&r).unwrap().method(),
            ImeMethod::Fcitx5
        );
        // fcitx5-remote が `0`（動いていない）を返すなら使えない
        r.on("fcitx5-remote", Some("0"));
        assert_eq!(super::linux::detect(&r).unwrap().method(), ImeMethod::Ibus);
        r.on("ibus engine", Some(""));
        assert!(super::linux::detect(&r).is_none());
    }

    #[test]
    fn the_worker_runs_calls_in_order() {
        let fake = Fake::default();
        *fake.state.lock().unwrap() = "kana".into();
        let worker = ImeWorker::new(session(&fake));
        for _ in 0..3 {
            worker.leave_insert();
            worker.enter_insert(true);
        }
        // スレッドが全部を処理し終えるまで待つ（送る口を閉じると、スレッドは残りを処理して終わる）
        let log = Arc::clone(&fake.log);
        drop(worker);
        for _ in 0..200 {
            if log.lock().unwrap().len() >= 6 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let log = log.lock().unwrap().clone();
        assert_eq!(log.len(), 6);
        assert!(
            log.chunks(2)
                .all(|c| c == ["english from kana", "restore kana"])
        );
    }
}
