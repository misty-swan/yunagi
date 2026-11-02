// 比較用の使い捨て。Svelte 版・Solid 版で同じ内容（パッケージ名だけが違う）。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::Write;
use std::sync::OnceLock;
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();

fn elapsed_ms() -> f64 {
    START.get().map(|s| s.elapsed().as_secs_f64() * 1000.0).unwrap_or(0.0)
}

fn say(line: String) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

/// フロントエンドが画面を組み上げてレイアウトを終えたら呼ぶ。返り値は起動モード（YUN2_MODE）。
#[tauri::command]
fn mounted(front_ms: f64, visibility: String) -> String {
    say(format!("MOUNTED rust_ms={:.1} front_ms={:.1} visibility={visibility}", elapsed_ms(), front_ms));
    std::env::var("YUN2_MODE").unwrap_or_default()
}

/// フロントエンドが最初の描画まで進んだら呼ぶ（見えないウィンドウでは呼ばれない）。
#[tauri::command]
fn ready(front_ms: f64) {
    say(format!("READY rust_ms={:.1} front_ms={:.1}", elapsed_ms(), front_ms));
}

/// ベンチマークの結果（JSON 文字列）を受け取って出力し、終了する。
#[tauri::command]
fn report(app: tauri::AppHandle, json: String) {
    say(format!("BENCH {json}"));
    app.exit(0);
}

fn main() {
    START.get_or_init(Instant::now);
    tauri::Builder::default()
        .setup(|_app| {
            say(format!("SETUP rust_ms={:.1}", elapsed_ms()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![mounted, ready, report])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
