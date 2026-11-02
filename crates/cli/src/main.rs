//! コンソール用バイナリ `yunagi`（docs/design/01-architecture.md 2章、
//! docs/design/05-mcp-cli-spec.md）。サブコマンドは後続の課題で作る。

fn main() {
    println!("yunagi {}", yunagi_core::VERSION);
}
