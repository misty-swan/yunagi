//! CLI の結合テスト（テスト計画書 4.5「CLI」）。本体は `cli/unix.rs`（ロックと UNIX ドメインソケットを
//! 使うので UNIX 系だけで走る。Windows ではコンパイルだけ通して何も走らせない。Windows の名前付きパイプは
//! 実機で確かめる。テスト計画書 4.5）。`cli/uninstall.rs` は `yunagi uninstall`（一時フォルダの中だけで
//! 消す。YUN-99）。

#[cfg(unix)]
#[path = "cli/unix.rs"]
mod unix;

#[cfg(unix)]
#[path = "cli/uninstall.rs"]
mod uninstall;

fn main() {
    #[cfg(unix)]
    {
        unix::main();
        uninstall::main();
    }
    #[cfg(not(unix))]
    println!("skipped: the CLI tests use file locks and UNIX domain sockets");
}
