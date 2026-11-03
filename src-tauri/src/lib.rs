//! Yunagi のデスクトップアプリ。画面（WebView）とコアを、Tauri コマンドとイベントでつなぐ
//! （docs/design/01-architecture.md 2章・4章 6）。
//!
//! - 画面からコアを呼ぶ：Tauri コマンド（[`commands`]）。受け渡す型は [`types`] で、TypeScript の
//!   定義（`src/lib/bindings.ts`）はそこから作る（`bindings` モジュールのテスト）。
//! - コアから画面に知らせる：Tauri のイベント（[`EVENT_STARTUP`] など）。データが変わると
//!   [`EVENT_CHANGED`] を出し、画面は読み込み直す。エージェントの変更も同じ経路で知らせる
//!   （[`AppCore::agent_write`]）。
//! - 起動・データベースを開く・定期バックアップ・日付の切り替わり：[`AppCore`]。

pub mod app_core;
#[cfg(test)]
mod bindings;
pub mod commands;
mod screen;
pub mod types;

use std::sync::Arc;

use tauri::ipc::Invoke;
use tauri::{AppHandle, Emitter, Manager, RunEvent, Runtime};

pub use app_core::{AppCore, Notice};
pub use screen::ScreenState;

/// データベースを開く処理の状態が変わった（[`types::Startup`]）。
pub const EVENT_STARTUP: &str = "startup";
/// データが変わった（[`types::Changed`]）。
pub const EVENT_CHANGED: &str = "changed";
/// 今日の論理日付が変わった（[`types::TodayChanged`]）。
pub const EVENT_TODAY_CHANGED: &str = "today-changed";
/// 定期バックアップを作れなかった（[`types::BackupFailed`]）。
pub const EVENT_BACKUP_FAILED: &str = "backup-failed";

/// 画面から呼べるコマンドの一覧。
pub fn invoke_handler<R: Runtime>() -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        commands::startup_status,
        commands::create_database,
        commands::retry_open,
        commands::choose_data_dir,
        commands::restore_at_startup,
        commands::acknowledge_restore,
        commands::list_backups,
        commands::today,
        commands::get_board,
        commands::get_overview,
        commands::add_item,
        commands::save_item,
        commands::delete_item,
        commands::restore_item,
        commands::move_item,
        commands::split_item,
        commands::get_text_doc,
        commands::save_text_doc,
        commands::send_note_lines,
        commands::note_marks,
        commands::carryover_status,
        commands::carryover_candidates,
        commands::postpone_carryover,
        commands::mark_carryover_reshown,
        commands::confirm_carryover,
        commands::search,
        commands::report_ui_state,
        commands::export_markdown,
        commands::get_settings,
        commands::save_settings,
        commands::check_data_dir,
        commands::change_data_dir,
        commands::use_data_dir,
        commands::restore_backup,
    ]
}

/// アプリにコアを持たせる（`State<Arc<AppCore>>`）。コアの知らせは、すべての画面へのイベントにする。
/// データベースはまだ開かない（[`AppCore::start`] か [`AppCore::open`] で開く）。
pub fn manage_core<R: Runtime>(app: &AppHandle<R>) -> Arc<AppCore> {
    let handle = app.clone();
    let core = AppCore::new(Box::new(move |notice| emit(&handle, notice)));
    app.manage(Arc::clone(&core));
    core
}

fn emit<R: Runtime>(app: &AppHandle<R>, notice: Notice) {
    let result = match notice {
        Notice::Startup(p) => app.emit(EVENT_STARTUP, p),
        Notice::Changed(p) => app.emit(EVENT_CHANGED, p),
        Notice::TodayChanged(p) => app.emit(EVENT_TODAY_CHANGED, p),
        Notice::BackupFailed(p) => app.emit(EVENT_BACKUP_FAILED, p),
    };
    if let Err(e) = result {
        eprintln!("yunagi: failed to notify the screen: {e}");
    }
}

/// アプリを起動する。画面のスレッドを止めないよう、データベースは別のスレッドで開く。
pub fn run() {
    let app = tauri::Builder::default()
        .invoke_handler(invoke_handler())
        .setup(|app| {
            manage_core(app.handle()).start();
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Yunagi の起動に失敗しました");
    app.run(|app, event| {
        if let RunEvent::Exit = event {
            app.state::<Arc<AppCore>>().close();
        }
    });
}
