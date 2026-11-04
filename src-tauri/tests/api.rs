//! ローカル API の結合テスト（テスト計画書 4.5「ローカル API の待ち受け」）：実際にソケットで待ち受け、
//! `yunagi-api` のクライアントから依頼を送って、コアの結果・権限・`changed`・止め方を確かめる。
//! WebView は開かない（`AppCore` を直接動かす）。
//!
//! 既定ディレクトリは環境変数から求まるので、`bridge` と同じく、テストの仕組み（harness）を使わず
//! `main` から1つずつ順に走らせる（Cargo.toml の `[[test]]`）。試験の本体は UNIX ドメインソケットを
//! 使うので UNIX 系だけで走る（`api/unix.rs`）。Windows の名前付きパイプは実機で確かめる
//! （テスト計画書 4.5）ため、Windows ではコンパイルだけ通して何も走らせない。

#[cfg(unix)]
#[path = "api/unix.rs"]
mod unix;

fn main() {
    #[cfg(unix)]
    unix::main();
    #[cfg(not(unix))]
    println!("skipped: the local API tests use UNIX domain sockets");
}
