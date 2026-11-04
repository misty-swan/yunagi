//! CLI・MCP が返すエラー（docs/design/05-mcp-cli-spec.md 4.3）と、終了コード（4.1）。
//!
//! エラーは 05 4.3 の JSON のオブジェクト（`code`・`reason`・`message` など）のまま持つ。アプリ（ローカル
//! API）と直接モードがコアの結果から作るエラー（`yunagi_api::ops`）と同じ形で、MCP サーバーがそのまま返せる。
//! ここで作るのは、操作がコアに届く前（接続・ロック・既定ディレクトリ）の失敗と、引数の誤り。

use std::path::Path;

use serde_json::{Value, json};
use yunagi_core::storage::data_dir::DefaultDir;

/// 05 4.3 のエラーのオブジェクト（`{"code": ..., "message": ..., ...}`）。
pub type ErrorObject = Value;

/// エラーのコード（`code`）。無ければ空。
pub fn code(error: &ErrorObject) -> &str {
    error.get("code").and_then(Value::as_str).unwrap_or("")
}

/// エラーの理由（`reason`）。
pub fn reason(error: &ErrorObject) -> Option<&str> {
    error.get("reason").and_then(Value::as_str)
}

/// エラーの文（`message`）。
pub fn message(error: &ErrorObject) -> &str {
    error.get("message").and_then(Value::as_str).unwrap_or("")
}

/// 終了コード（05 4.1 の表）。表にないコードは 1（一般エラー）。
pub fn exit_code(error: &ErrorObject) -> i32 {
    match code(error) {
        "invalid_argument" => 2,
        "permission_denied" => 3,
        "not_found" => 4,
        "conflict" => 5,
        "storage_unavailable" => 6,
        "busy" => 7,
        "api_error" => 8,
        _ => 1,
    }
}

/// 引数の誤り（終了コード 2）。
pub fn invalid_argument(message: impl Into<String>) -> ErrorObject {
    json!({"code": "invalid_argument", "message": message.into()})
}

/// 対象が見つからない（終了コード 4。`agent setup` が、登録できるエージェントを見つけられなかったとき）。
pub fn not_found(message: impl Into<String>) -> ErrorObject {
    json!({"code": "not_found", "message": message.into()})
}

/// 確認の端末が無いなどで、利用者の確認を得る操作を確定できない（終了コード 3）。スコープの不足ではない
/// ので `required_scope` は付けない（`agent setup`。05 2章・6章）。
pub fn permission_denied(message: impl Into<String>) -> ErrorObject {
    json!({"code": "permission_denied", "message": message.into()})
}

/// どの分類にも当たらない失敗（終了コード 1）。
pub fn internal(message: impl Into<String>) -> ErrorObject {
    json!({"code": "internal", "message": message.into()})
}

/// ローカル API を使えない（終了コード 8）。`reason` は 05 4.3 の `api_error` の理由。
pub fn api_error(reason: &str, message: impl Into<String>) -> ErrorObject {
    json!({"code": "api_error", "reason": reason, "message": message.into()})
}

/// 保存先を利用できない（終了コード 6）。
pub fn storage_unavailable(message: impl Into<String>) -> ErrorObject {
    json!({"code": "storage_unavailable", "message": message.into()})
}

/// データを使えない：ローカル API にもつながらず、ロックも 5 秒の間に取れなかった（終了コード 7）。
pub fn busy() -> ErrorObject {
    yunagi_api::ops::busy_error()
}

/// 依頼を送った後、応答の前に接続が切れた。反映されたかが分からない。
pub fn result_unknown() -> ErrorObject {
    api_error(
        "result_unknown",
        "The connection to Yunagi was lost after the request was sent. It may or may not have been applied. Read the data again before retrying.",
    )
}

/// トークンが合わない。
pub fn auth_failed() -> ErrorObject {
    api_error(
        "auth_failed",
        "Yunagi did not accept the connection token. Nothing was changed. Ask the user to restart Yunagi.",
    )
}

/// プロトコルの版が合わない。
pub fn version_mismatch() -> ErrorObject {
    api_error(
        "version_mismatch",
        "This command and Yunagi use different protocol versions. Nothing was changed. Ask the user to update Yunagi and the yunagi command to the same version.",
    )
}

/// ソケット・パイプを開く権限が無い。
pub fn denied() -> ErrorObject {
    api_error(
        "denied",
        "This command is not allowed to connect to Yunagi. Nothing was changed. Another user's Yunagi may be using the same location.",
    )
}

/// ソケットのパスが長すぎて、アプリが待ち受けられない（UNIX 系）。
pub fn socket_path_too_long() -> ErrorObject {
    api_error(
        "socket_path_too_long",
        "Yunagi's data folder path is too long for its local API socket, so this command cannot reach the running app. Nothing was changed. Do not retry. Ask the user to quit Yunagi (the same command then works without it), or to use a shorter data folder through an environment variable such as XDG_DATA_HOME.",
    )
}

/// アプリと環境変数が違い、既定ディレクトリが別の場所になっている。`mine` はこのコマンドの、`other` は
/// 使っているプロセスのいる既定ディレクトリ。
pub fn default_dir_mismatch(mine: &Path, other: &Path) -> ErrorObject {
    api_error(
        "default_dir_mismatch",
        format!(
            "Yunagi is running with a different data folder ({}) than this command uses ({}). Nothing was changed. Ask the user to run the agent with the same environment variables as Yunagi (for example XDG_DATA_HOME). If Yunagi is not running, the same request may be retried.",
            other.display(),
            mine.display()
        ),
    )
}

/// 既定ディレクトリが無い（アプリが起動時に作る。CLI・MCP は作らない）。探した場所と、求めた環境変数の
/// 名前を書く（アーキテクチャ設計書 4.5 の 4）。
pub fn default_dir_missing(dir: &DefaultDir, vars: &[&str]) -> ErrorObject {
    storage_unavailable(format!(
        "Yunagi's data folder was not found at {} (from the environment variable {}). Nothing was changed. Ask the user to open Yunagi once, and check that Yunagi and this command are started with the same environment variables (for example XDG_DATA_HOME).",
        dir.path().display(),
        vars.join(" / ")
    ))
}

/// 既定ディレクトリを求める環境変数が無い。
pub fn default_dir_unset(vars: &[&str]) -> ErrorObject {
    storage_unavailable(format!(
        "Yunagi's data folder could not be determined because the environment variable {} is not set. Nothing was changed. Ask the user to set it, the same as for Yunagi.",
        vars.join(" / ")
    ))
}

/// 保存先にデータベースが無い。探した場所を書く（アーキテクチャ設計書 6.3）。CLI・MCP からは新しく作らない。
pub fn database_not_found(path: &Path) -> ErrorObject {
    storage_unavailable(format!(
        "No Yunagi database was found at {}. Nothing was changed. Ask the user to open Yunagi (it creates the database, or lets them choose where it is). This command does not create it.",
        path.display()
    ))
}

/// 読み書きの失敗。内部の文は含めない（標準エラーに残す。05 4.3）。
pub fn storage_failed() -> ErrorObject {
    storage_unavailable(
        "Yunagi could not read or write its data. Nothing was changed. Ask the user to check the app.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_follow_the_table() {
        let of = |code: &str| exit_code(&json!({"code": code}));
        assert_eq!(of("invalid_argument"), 2);
        assert_eq!(of("permission_denied"), 3);
        assert_eq!(of("not_found"), 4);
        assert_eq!(of("conflict"), 5);
        assert_eq!(of("storage_unavailable"), 6);
        assert_eq!(of("busy"), 7);
        assert_eq!(of("api_error"), 8);
        assert_eq!(of("internal"), 1);
        assert_eq!(exit_code(&json!({})), 1);
    }

    #[test]
    fn connection_errors_carry_their_reason() {
        assert_eq!(reason(&result_unknown()), Some("result_unknown"));
        assert_eq!(reason(&auth_failed()), Some("auth_failed"));
        assert_eq!(reason(&version_mismatch()), Some("version_mismatch"));
        assert_eq!(reason(&denied()), Some("denied"));
        assert_eq!(
            reason(&socket_path_too_long()),
            Some("socket_path_too_long")
        );
        let e = default_dir_mismatch(Path::new("/a/yunagi"), Path::new("/b/yunagi"));
        assert_eq!(reason(&e), Some("default_dir_mismatch"));
        assert!(message(&e).contains("/a/yunagi") && message(&e).contains("/b/yunagi"));
        assert_eq!(code(&busy()), "busy");
    }
}
