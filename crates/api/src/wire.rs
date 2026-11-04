//! プロトコル：JSON-RPC 2.0 を、1 行 1 メッセージ（改行区切りの JSON）で送る
//! （docs/design/01-architecture.md 4.6、docs/design/05-mcp-cli-spec.md 4.4）。
//!
//! 1 つの接続で、次の 2 往復だけを行って閉じる（接続を使い回さない。4.4）。
//!
//! 1. クライアントが `hello`（プロトコルの版・トークン・識別子）を送り、サーバーが答える。
//! 2. クライアントが依頼を 1 つ送り、サーバーが応答を 1 つ返す。
//!
//! まとめての依頼（batch）と、応答の要らない通知は受け付けない。

use std::io::{self, BufRead, Read};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// 依頼の `method` の `hello`。
pub const METHOD_HELLO: &str = "hello";

/// JSON として読めない（JSON-RPC 標準）。
pub const PARSE_ERROR: i64 = -32700;
/// 依頼の形が誤っている（JSON-RPC 標準）。
pub const INVALID_REQUEST: i64 = -32600;
/// 知らない `method`（JSON-RPC 標準）。
pub const METHOD_NOT_FOUND: i64 = -32601;
/// `params` が誤っている（JSON-RPC 標準）。
pub const INVALID_PARAMS: i64 = -32602;
/// アプリのエラー（05 4.3）。内容は `error.data.error`（`code`・`message` など）に入る。
/// 5 秒の待ちで断った `busy` や、認証・版の不一致（`api_error`）もこれ。
pub const APP_ERROR: i64 = -32000;

/// `hello` の `params`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloParams {
    /// クライアントが話すプロトコルの版。
    pub protocol: u32,
    /// 認証のトークン。
    pub token: String,
    /// クライアントの識別子（`yunagi mcp --client claude-code` の `claude-code`）。無ければ `unknown`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
}

/// `hello` の応答の `result`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloResult {
    /// サーバーが話すプロトコルの版。
    pub protocol: u32,
    /// アプリの版。
    pub app_version: String,
}

/// 1 行を読んだ結果の誤り。
#[derive(Debug)]
pub enum LineError {
    /// 上限を超えた（改行が見つからないまま上限に達した）。
    TooLong,
    /// 改行の前に接続が切れた。
    Truncated,
    /// 読めなかった（時間切れ・切断を含む）。
    Io(io::Error),
}

impl std::fmt::Display for LineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLong => f.write_str("the line is too long"),
            Self::Truncated => f.write_str("the connection closed in the middle of a line"),
            Self::Io(_) => f.write_str("failed to read a line"),
        }
    }
}

impl std::error::Error for LineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

/// 改行までの 1 行を、最大 `max` バイト（改行を除く）まで読む。何も読まずに閉じられたら `Ok(None)`。
/// 返す行の末尾の改行（と CR）は除く。
pub fn read_line(reader: &mut impl BufRead, max: usize) -> Result<Option<Vec<u8>>, LineError> {
    let mut buf = Vec::new();
    // 上限 + 改行 1 バイトまで読む。ここで止まって改行が無ければ長すぎる
    let limit = u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1);
    let read = reader
        .by_ref()
        .take(limit)
        .read_until(b'\n', &mut buf)
        .map_err(LineError::Io)?;
    if read == 0 {
        return Ok(None);
    }
    if buf.last() != Some(&b'\n') {
        return Err(if buf.len() as u64 >= limit {
            LineError::TooLong
        } else {
            LineError::Truncated
        });
    }
    buf.pop();
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    Ok(Some(buf))
}

/// 成功の応答の 1 行（改行付き）。
pub fn result_line(id: &Value, result: &Value) -> String {
    let mut line = json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string();
    line.push('\n');
    line
}

/// 失敗の応答の 1 行（改行付き）。`data` は付けるときだけ。
pub fn error_line(id: &Value, code: i64, message: &str, data: Option<Value>) -> String {
    let mut error = Map::new();
    error.insert("code".into(), json!(code));
    error.insert("message".into(), json!(message));
    if let Some(data) = data {
        error.insert("data".into(), data);
    }
    let mut line = json!({"jsonrpc": "2.0", "id": id, "error": error}).to_string();
    line.push('\n');
    line
}

/// アプリのエラー（05 4.3 の `{"code": ..., "message": ..., ...}`）を載せた失敗の応答の 1 行。
pub fn app_error_line(id: &Value, error: &Value) -> String {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("error");
    error_line(id, APP_ERROR, message, Some(json!({ "error": error })))
}

/// 読んだ依頼。
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub id: Value,
    pub method: String,
    /// `params`（無ければ空のオブジェクト）。
    pub params: Value,
}

/// 依頼の形が誤っている。応答の `id` と、標準のコードと文を持つ。
#[derive(Debug, Clone, PartialEq)]
pub struct BadRequest {
    pub id: Value,
    pub code: i64,
    pub message: &'static str,
}

impl BadRequest {
    /// この誤りの応答の 1 行。
    pub fn line(&self) -> String {
        error_line(&self.id, self.code, self.message, None)
    }
}

/// 1 行を依頼として読む。JSON でなければ [`PARSE_ERROR`]（`id` は `null`）、依頼の形（`jsonrpc` が
/// `"2.0"`、`method` が文字列、`id` がある、`params` が無いかオブジェクト）が誤っていれば
/// [`INVALID_REQUEST`]。batch（配列）と通知（`id` が無い）は受け付けない。
pub fn parse_request(line: &[u8]) -> Result<Request, BadRequest> {
    let bad = |id: Value, code, message| BadRequest { id, code, message };
    let value: Value =
        serde_json::from_slice(line).map_err(|_| bad(Value::Null, PARSE_ERROR, "Parse error"))?;
    let Value::Object(mut object) = value else {
        return Err(bad(
            Value::Null,
            INVALID_REQUEST,
            "Invalid Request: batches are not supported",
        ));
    };
    let id = object.remove("id");
    let invalid =
        |id: Option<Value>, message| bad(id.unwrap_or(Value::Null), INVALID_REQUEST, message);
    if object.get("jsonrpc") != Some(&json!("2.0")) {
        return Err(invalid(id, "Invalid Request: jsonrpc must be \"2.0\""));
    }
    let Some(id) = id.filter(|id| id.is_string() || id.is_number()) else {
        return Err(invalid(
            None,
            "Invalid Request: id is required (notifications are not supported)",
        ));
    };
    let Some(Value::String(method)) = object.remove("method") else {
        return Err(invalid(
            Some(id),
            "Invalid Request: method must be a string",
        ));
    };
    let params = match object.remove("params") {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(p @ Value::Object(_)) => p,
        Some(_) => {
            return Err(invalid(
                Some(id),
                "Invalid Request: params must be an object",
            ));
        }
    };
    Ok(Request { id, method, params })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn lines(input: &[u8], max: usize) -> Result<Option<Vec<u8>>, LineError> {
        read_line(&mut Cursor::new(input.to_vec()), max)
    }

    #[test]
    fn reads_one_line_without_the_newline() {
        assert_eq!(lines(b"abc\nrest", 10).unwrap(), Some(b"abc".to_vec()));
        assert_eq!(lines(b"abc\r\n", 10).unwrap(), Some(b"abc".to_vec()));
        assert_eq!(lines(b"\n", 10).unwrap(), Some(Vec::new()));
        assert_eq!(lines(b"", 10).unwrap(), None);
    }

    #[test]
    fn a_line_of_exactly_the_limit_is_accepted() {
        assert_eq!(lines(b"abcd\n", 4).unwrap(), Some(b"abcd".to_vec()));
        assert!(matches!(lines(b"abcde\n", 4), Err(LineError::TooLong)));
        assert!(matches!(lines(b"abcdefgh", 4), Err(LineError::TooLong)));
    }

    #[test]
    fn a_line_cut_off_by_a_closed_connection_is_truncated() {
        assert!(matches!(lines(b"abc", 10), Err(LineError::Truncated)));
    }

    #[test]
    fn parses_a_request() {
        let r = parse_request(
            br#"{"jsonrpc":"2.0","id":7,"method":"get_board","params":{"date":"today"}}"#,
        )
        .unwrap();
        assert_eq!(r.id, json!(7));
        assert_eq!(r.method, "get_board");
        assert_eq!(r.params, json!({"date": "today"}));
        let r = parse_request(br#"{"jsonrpc":"2.0","id":"a","method":"open_board"}"#).unwrap();
        assert_eq!(r.params, json!({}));
    }

    #[test]
    fn rejects_malformed_requests_with_the_standard_codes() {
        let code = |line: &[u8]| parse_request(line).unwrap_err().code;
        assert_eq!(code(b"not json"), PARSE_ERROR);
        assert_eq!(code(b"[]"), INVALID_REQUEST);
        assert_eq!(
            code(br#"[{"jsonrpc":"2.0","id":1,"method":"x"}]"#),
            INVALID_REQUEST
        );
        assert_eq!(code(br#"{"id":1,"method":"x"}"#), INVALID_REQUEST);
        assert_eq!(code(br#"{"jsonrpc":"2.0","method":"x"}"#), INVALID_REQUEST);
        assert_eq!(
            code(br#"{"jsonrpc":"2.0","id":1,"method":5}"#),
            INVALID_REQUEST
        );
        assert_eq!(
            code(br#"{"jsonrpc":"2.0","id":1,"method":"x","params":[1]}"#),
            INVALID_REQUEST
        );
        assert_eq!(
            code(br#"{"jsonrpc":"2.0","id":null,"method":"x"}"#),
            INVALID_REQUEST
        );
    }

    #[test]
    fn app_errors_carry_the_error_object_in_data() {
        let line = app_error_line(&json!(1), &json!({"code": "busy", "message": "Try again."}));
        assert!(line.ends_with('\n'));
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["error"]["code"], json!(APP_ERROR));
        assert_eq!(v["error"]["message"], json!("Try again."));
        assert_eq!(v["error"]["data"]["error"]["code"], json!("busy"));
    }
}
