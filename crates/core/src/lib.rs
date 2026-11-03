//! Yunagi のコア。データの読み書きはすべてこの層を通る
//! （docs/design/01-architecture.md 1章・2章）。
//!
//! GUI（src-tauri）とコンソール用バイナリ `yunagi`（crates/cli）の両方が、
//! このクレートを使う。

pub mod domain;
pub mod error;
pub mod storage;

/// コアの版。GUI と `yunagi` が同じコアを使っていることの確認にも使う。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
