//! クリップボードへの書き込み：依頼文の書き出し（Mod+Shift+A。操作仕様書 2章、MCP・CLI 仕様書 5章）。
//!
//! - 書き込みは Rust 側だけで行う（Tauri のプラグイン `tauri-plugin-clipboard-manager` の Rust の API）。
//!   画面には、依頼文を作って書き込むまでを 1 つにしたコマンド（`copy_handoff`）だけを見せ、プラグインの
//!   JavaScript の API と権限（`capabilities/default.json` の `clipboard-manager:*`）は渡さない。画面から
//!   `navigator.clipboard` で書くと、依頼文を作る間にキー操作の後という扱いが切れて、WebKit 系
//!   （macOS の WKWebView、Linux の WebKitGTK）で断られうるため。画面がクリップボードを読んだり、
//!   任意の文を書いたりできないようにするためでもある。
//! - プラグインを起動できなくても、アプリは起動する（[`install`]。書き込みは `ClipboardFailed`）。

use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::types::CommandError;

/// クリップボードへの書き込み。プラグインを使う本物（[`PluginWriter`]）と、テストの偽物がある。
pub trait Writer: Send + Sync {
    /// 文 `text` を、クリップボードの文字列にする。失敗したら理由（ログ用の文）を返す。
    fn write(&self, text: &str) -> Result<(), String>;
}

/// 依頼文を書き込む先を持つ（アプリに 1 つ）。
#[derive(Default)]
pub struct HandoffClipboard {
    /// 書き込む先。使えないとき（プラグインを起動できなかった）は `None`。
    writer: Mutex<Option<Arc<dyn Writer>>>,
}

impl HandoffClipboard {
    /// 書き込む先はまだ無い状態（[`HandoffClipboard::attach`] で使えるようにする）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 書き込む先を使えるようにする。
    pub fn attach(&self, writer: Arc<dyn Writer>) {
        *crate::app_core::lock(&self.writer) = Some(writer);
    }

    /// `text` をクリップボードに書く。書けなければ [`CommandError::ClipboardFailed`]。
    pub fn write(&self, text: &str) -> Result<(), CommandError> {
        // 書き込みの間は錠を持たない（OS の応答を待つことがあるため）
        let writer = crate::app_core::lock(&self.writer).clone();
        let Some(writer) = writer else {
            return Err(CommandError::ClipboardFailed {
                detail: "the clipboard is unavailable".into(),
            });
        };
        writer
            .write(text)
            .map_err(|detail| CommandError::ClipboardFailed { detail })
    }
}

/// プラグインで書く本物。
struct PluginWriter<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> Writer for PluginWriter<R> {
    fn write(&self, text: &str) -> Result<(), String> {
        self.app
            .clipboard()
            .write_text(text.to_owned())
            .map_err(|e| e.to_string())
    }
}

/// プラグインを入れて、書き込む先を使えるようにする。入れられなくても、アプリは起動を続ける
/// （以後の書き込みは `ClipboardFailed`）。
pub fn install<R: Runtime>(app: &AppHandle<R>) {
    match app.plugin(tauri_plugin_clipboard_manager::init()) {
        Ok(()) => {
            app.state::<Arc<HandoffClipboard>>()
                .attach(Arc::new(PluginWriter { app: app.clone() }));
        }
        Err(e) => eprintln!("yunagi: the clipboard is unavailable: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(Result<(), String>, Mutex<Vec<String>>);

    impl Writer for Fake {
        fn write(&self, text: &str) -> Result<(), String> {
            self.1.lock().unwrap().push(text.to_owned());
            self.0.clone()
        }
    }

    #[test]
    fn writing_without_a_destination_or_with_a_failure_gives_clipboard_failed() {
        let clipboard = HandoffClipboard::new();
        assert!(matches!(
            clipboard.write("x"),
            Err(CommandError::ClipboardFailed { .. })
        ));
        let fake = Arc::new(Fake(Err("denied".into()), Mutex::default()));
        clipboard.attach(fake.clone());
        assert!(matches!(
            clipboard.write("x"),
            Err(CommandError::ClipboardFailed { detail }) if detail == "denied"
        ));
        let ok = Arc::new(Fake(Ok(()), Mutex::default()));
        clipboard.attach(ok.clone());
        clipboard.write("y").unwrap();
        assert_eq!(*ok.1.lock().unwrap(), ["y"]);
    }
}
