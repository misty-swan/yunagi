//! Yunagi のデスクトップアプリ。画面（WebView）とコアをつなぐ
//! （docs/design/01-architecture.md 2章）。

// Windows のリリース版で、余分なコンソール窓を出さない。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// 画面から呼ぶ Tauri コマンドの見本。コアの版を返す。
#[tauri::command]
fn core_version() -> &'static str {
    yunagi_core::VERSION
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![core_version])
        .run(tauri::generate_context!())
        .expect("Yunagi の起動に失敗しました");
}
