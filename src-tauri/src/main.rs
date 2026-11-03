//! Yunagi のデスクトップアプリの入口。中身は `yunagi_app`（src/lib.rs）。

// Windows のリリース版で、余分なコンソール窓を出さない。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    yunagi_app::run();
}
