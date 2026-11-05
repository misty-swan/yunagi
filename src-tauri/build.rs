fn main() {
    tauri_build::build();

    // Windows の結合テスト（tests/）の実行ファイルにも、Common Controls v6 を要求するマニフェストを埋め込む。
    // tauri-build は本体の実行ファイルにだけマニフェストを埋めるので、`tauri::test` で窓を作るテストは、
    // 起動の前に STATUS_ENTRYPOINT_NOT_FOUND（0xc0000139）で落ちる。
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-tests=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
    }
}
