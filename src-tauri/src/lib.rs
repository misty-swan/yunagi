//! Yunagi のデスクトップアプリ。画面（WebView）とコアを、Tauri コマンドとイベントでつなぐ
//! （docs/design/01-architecture.md 2章・4章 6）。
//!
//! - 画面からコアを呼ぶ：Tauri コマンド（[`commands`]）。受け渡す型は [`types`] で、TypeScript の
//!   定義（`src/lib/bindings.ts`）はそこから作る（`bindings` モジュールのテスト）。
//! - コアから画面に知らせる：Tauri のイベント（[`EVENT_STARTUP`] など）。データが変わると
//!   [`EVENT_CHANGED`] を出し、画面は読み込み直す。エージェントの変更も同じ経路で知らせる
//!   （[`AppCore::agent_write`]）。
//! - 起動・データベースを開く・定期バックアップ・日付の切り替わり：[`AppCore`]。

pub mod api;
pub mod app_core;
#[cfg(test)]
mod bindings;
pub mod clipboard;
pub mod commands;
pub mod global_shortcut;
pub mod ime;
mod screen;
pub mod types;
pub mod updater;

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
/// 窓を前面に出した（[`types::ShowRequested`]）。`yunagi show` と、2 つ目の起動の引き渡し。
pub const EVENT_SHOW_REQUESTED: &str = "show-requested";

/// 自動更新の状態が変わった（[`types::UpdateState`]）。
pub const EVENT_UPDATE_STATE: &str = "update-state";

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
        commands::get_archive,
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
        commands::copy_handoff,
        commands::report_ui_state,
        commands::ime_status,
        commands::ime_leave_insert,
        commands::ime_enter_insert,
        commands::set_global_shortcut,
        commands::take_show_note,
        commands::export_markdown,
        commands::get_settings,
        commands::save_settings,
        commands::get_update_state,
        commands::install_update,
        commands::get_data_dir,
        commands::check_data_dir,
        commands::change_data_dir,
        commands::use_data_dir,
        commands::restore_backup,
        commands::list_agents,
        commands::set_agent_scopes,
        commands::remove_agent,
        commands::api_status,
        commands::list_changes,
        commands::preview_revert,
        commands::revert_change,
        commands::mark_changes_seen,
    ]
}

/// アプリにコアを持たせる（`State<Arc<AppCore>>`）。コアの知らせは、すべての画面へのイベントにする。
/// データベースはまだ開かない（[`AppCore::start`] か [`AppCore::open`] で開く）。
pub fn manage_core<R: Runtime>(app: &AppHandle<R>) -> Arc<AppCore> {
    let handle = app.clone();
    let core = AppCore::new(Box::new(move |notice| emit(&handle, notice)));
    app.manage(Arc::clone(&core));
    // IME の切替（操作仕様書 4.4）。データベースとは関係しないので、コアとは別に持つ
    app.manage(ime::ImeWorker::system(app.clone()));
    // OS 全体のショートカットの登録（操作仕様書 1章）。プラグインは `run` で入れる（窓の無い試験では
    // 入れず、登録は `Unavailable` を返す）
    app.manage(Arc::new(global_shortcut::GlobalShortcutController::new()));
    // 依頼文の書き出し先（クリップボード）。プラグインは `run` で入れる（窓の無い試験では入れず、
    // 書き込みは `ClipboardFailed` を返す）
    app.manage(Arc::new(clipboard::HandoffClipboard::new()));
    // 自動更新（ビルド・配布手順書 5章）。確認の手段とスレッドは `run` で入れる（窓の無い試験では入れず、
    // 状態はいつも `none`）。有効かは、確認のたびにデータベースから読み直す（データベースを開く前は読めず、
    // 確認しない）
    app.manage(updater::UpdateService::new(updater::Hooks {
        update_check: {
            let core = Arc::clone(&core);
            Box::new(move || {
                core.read(yunagi_core::domain::settings::load)
                    .ok()
                    .map(|s| s.update_check)
            })
        },
        notify: {
            let app = app.clone();
            Box::new(move |state| {
                if let Err(e) = app.emit(EVENT_UPDATE_STATE, state) {
                    eprintln!("yunagi: failed to notify the screen: {e}");
                }
            })
        },
        close: {
            let core = Arc::clone(&core);
            Box::new(move || core.close())
        },
        restart: {
            let app = app.clone();
            Box::new(move || app.restart())
        },
    }));
    core
}

fn emit<R: Runtime>(app: &AppHandle<R>, notice: Notice) {
    let result = match notice {
        Notice::Startup(p) => app.emit(EVENT_STARTUP, p),
        Notice::Changed(p) => app.emit(EVENT_CHANGED, p),
        Notice::TodayChanged(p) => app.emit(EVENT_TODAY_CHANGED, p),
        Notice::BackupFailed(p) => app.emit(EVENT_BACKUP_FAILED, p),
        Notice::ShowRequested(p) => show_window(app, p),
    };
    if let Err(e) = result {
        eprintln!("yunagi: failed to notify the screen: {e}");
    }
}

/// 窓を前面に出し（OS 全体のショートカットと同じ部品。[`global_shortcut::bring_to_front`]）、画面に
/// `show-requested` を知らせる。窓が無くても、画面への知らせは出す（窓の操作の失敗はログに出るだけ）。
///
/// 窓の操作はメインスレッドに頼み、待たずに返る。ローカル API の接続のスレッドから呼ばれるので、メイン
/// スレッドの完了を待つと、アプリの終了（[`AppCore::close`] が接続の終わりを待つ）と行き違って止まるため。
fn show_window<R: Runtime>(app: &AppHandle<R>, request: types::ShowRequested) -> tauri::Result<()> {
    let handle = app.clone();
    if let Err(e) = app.run_on_main_thread(move || global_shortcut::bring_to_front(&handle)) {
        eprintln!("yunagi: failed to bring the window to the front: {e}");
    }
    app.emit(EVENT_SHOW_REQUESTED, request)
}

/// コアを持たせる前に届いた、ノートモードで前面に出す頼み（[`handover`]）。コアの `show_note` と同じ
/// 決まりで、画面が `take_show_note` で受け取る。
pub(crate) static EARLY_SHOW_NOTE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// 2 つ目の起動の引き渡し（`tauri-plugin-single-instance`）。引数に `--note` があれば（`yunagi show
/// --note` が起動したとき）、ノートモードで前面に出す。コアは、画面が準備できる前でも頼みを覚える。
fn handover<R: Runtime>(app: &AppHandle<R>, args: &[String]) {
    let note = args.iter().any(|a| a == "--note");
    match app.try_state::<Arc<AppCore>>() {
        Some(core) => core.request_show(note),
        None => {
            // コアを持たせる前（起動の途中）に届いた頼み：覚えておき、画面が受け取る
            // （`commands::take_show_note`）
            EARLY_SHOW_NOTE.store(note, std::sync::atomic::Ordering::Release);
            if let Err(e) = show_window(app, types::ShowRequested { note }) {
                eprintln!("yunagi: failed to notify the screen: {e}");
            }
        }
    }
}

/// アプリを起動する。画面のスレッドを止めないよう、データベースは別のスレッドで開く。
pub fn run() {
    let app = tauri::Builder::default()
        // 2 つ目の起動は、動いているアプリに引き渡して終わる（先に入れる。データベースを開く前に判定する
        // ため）。動いているアプリは窓を前面に出す。データベースを見つけられない・開けない間の起動時の
        // 画面は、ロックも待ち受けも持たないので、`yunagi show` はこの引き渡しで前面に出す
        // （アーキテクチャ設計書 3.5）。D-Bus が使えないなど、見つけられない環境では、そのまま起動する
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            handover(app, &args);
        }))
        // フォルダを選ぶ OS のダイアログ（画面から使う。capabilities/default.json）
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(invoke_handler())
        .setup(|app| {
            manage_core(app.handle()).start();
            // 入れられなくても起動する。OS 全体のショートカットだけが使えなくなる
            global_shortcut::install(app.handle());
            // 入れられなくても起動する。依頼文の書き出しだけが使えなくなる
            clipboard::install(app.handle());
            // 入れられなくても起動する。更新の確認だけが使えなくなる
            updater::install(app.handle());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Yunagi の起動に失敗しました");
    app.run(|app, event| {
        if let RunEvent::Exit = event {
            app.state::<Arc<updater::UpdateService>>().stop();
            app.state::<Arc<AppCore>>().close();
        }
    });
}
