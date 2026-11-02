//! コンソール用バイナリ `yunagi`（docs/design/01-architecture.md 2章、
//! docs/design/05-mcp-cli-spec.md）。サブコマンドは後続の課題で作る。

fn main() {
    println!("{}", version_line());
}

/// 版の表示。版はコア（yunagi-core）のものを使う。
fn version_line() -> String {
    format!("yunagi {}", yunagi_core::VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `yunagi` が使うコアの版が、ワークスペースの版と一致すること。
    /// コアの版を別に上げてしまい、GUI と `yunagi` で食い違うのを防ぐ。
    #[test]
    fn version_line_uses_workspace_version() {
        assert_eq!(
            version_line(),
            format!("yunagi {}", env!("CARGO_PKG_VERSION"))
        );
    }
}
