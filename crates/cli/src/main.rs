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

    /// GUI（src-tauri）の設定ファイル。読み込んだファイルが変わると、テストもビルドし直される。
    const TAURI_CONF: &str = include_str!("../../../src-tauri/tauri.conf.json");
    const APP_CARGO_TOML: &str = include_str!("../../../src-tauri/Cargo.toml");

    /// `yunagi` が表示するコアの版（yunagi_core::VERSION）が、`yunagi` 自身の版と一致すること。
    /// コア（crates/core/Cargo.toml）だけ版を直接書くと失敗する。
    #[test]
    fn version_line_matches_cli_version() {
        assert_eq!(
            version_line(),
            format!("yunagi {}", env!("CARGO_PKG_VERSION"))
        );
    }

    /// GUI の版も同じワークスペースの版になっていること。
    /// Tauri 2 は tauri.conf.json に "version" が無いと、アプリのクレート（src-tauri/Cargo.toml）の版を使う。
    /// どちらかに版を直接書くと GUI と `yunagi` で版が食い違いうるので、それを防ぐ。
    #[test]
    fn gui_uses_workspace_version() {
        assert!(
            !TAURI_CONF.contains("\"version\""),
            "src-tauri/tauri.conf.json に version を書かない（Cargo の版を使う）"
        );
        let package = APP_CARGO_TOML
            .split("\n[")
            .next()
            .expect("src-tauri/Cargo.toml の [package] が読めない");
        assert!(
            package
                .lines()
                .any(|line| line.trim() == "version.workspace = true"),
            "src-tauri/Cargo.toml の [package] は version.workspace = true にする"
        );
    }
}
