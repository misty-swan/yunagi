//! MCP サーバー `yunagi mcp`（stdio。docs/design/05-mcp-cli-spec.md 4.5、
//! docs/design/01-architecture.md 4.4）。
//!
//! ツールの呼び出しを [`Connector::call`](crate::connect::Connector::call) に渡して、返り値かエラー
//! （05 4.2・4.3）をそのままツールの結果に載せるだけの薄い層。権限・入力中・持ち越しの判定と変更履歴は
//! コアが行い、接続の選び方（ローカル API か直接モードか、`busy`・`api_error`）は `Connector` が行う。
//! この層はそれらを判定し直さない。
//!
//! プロトコルは自前の JSON-RPC 2.0（1 行 1 メッセージ）で、SDK は使わない（理由は 01 4.7）。必要なのは
//! `initialize`・`server/discover`・`ping`・`tools/list`・`tools/call` と通知を受けることだけ。最新の仕様
//! （2026-07-28）は `initialize` が無くリクエストごとに `_meta` でバージョンを送り、旧い仕様
//! （2025-11-25 以前）は `initialize` から始まるので、両方に答える。接続の状態は持たず、握手の前の
//! `tools/call` も受ける。
//!
//! 標準出力にはメッセージ以外を書かない（診断は標準エラー）。依頼は 1 件ずつ順に処理する（最大 5 秒待つ
//! 間は `ping` にも答えない）。

use std::io::{self, BufRead, Read, Write};

use serde_json::{Map, Value, json};
use yunagi_api::MAX_REQUEST_LINE;

use crate::error::{self, ErrorObject};
use crate::{Env, Io, Warnings, connector, mcp_tools};

/// 最新の仕様の版（リクエストごとに `_meta` で送られる）。
const MODERN_VERSION: &str = "2026-07-28";
/// `_meta` の、プロトコルの版のキー。
const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
/// `initialize` で答えられる旧い仕様の版（新しい順）。
const LEGACY_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// 読む 1 行の大きさの上限。ツールの引数の上限（[`MAX_ARGUMENTS`]）に、JSON の包みと、エスケープの
/// 書き方の違いの余裕を足した大きさ。
const MAX_LINE: usize = 2 * MAX_REQUEST_LINE;
/// ツールの引数（JSON にしたときの大きさ）の上限。ローカル API の依頼の 1 行の上限から包みの余裕を引いた
/// 大きさで、CLI の標準入力の本文と同じ決まり（05 4.1）。直接モードで通る依頼が、アプリ経由では断られる
/// （経路で結果が変わる）ことがないよう、アプリに届く前にここで同じ大きさで断る。
const MAX_ARGUMENTS: usize = MAX_REQUEST_LINE - 16 * 1024;

/// エージェントへの案内（`initialize`・`server/discover` の `instructions`）。
const INSTRUCTIONS: &str = "Yunagi is the user's daily work note. Read the board first (get_board or open_board) to get item IDs, then use the other tools. The user decides in Yunagi what you may do; a permission_denied error names the scope you need, so tell the user instead of retrying. After an error: busy means nothing was changed, so send the same request again; conflict with reason user_editing, retry later; conflict with reason changed, read again before retrying; api_error with reason result_unknown, the write may or may not have been applied, so read before retrying; any other api_error, do not retry and tell the user.";

/// JSON-RPC のエラーコード。
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
/// 最新の仕様の、対応しない版（`data.supported`・`data.requested`）。
const UNSUPPORTED_VERSION: i64 = -32022;

/// ツールの呼び出しを行うもの（`Connector::call` か、試験の偽物）。
type CallFn<'a> = dyn FnMut(&str, Map<String, Value>) -> Result<Value, ErrorObject> + 'a;

/// `yunagi mcp [--client ID]`。標準入力が閉じられたら 0 で終わる。識別子の形は引数を読むときに確かめ
/// ている。既定ディレクトリを求められないとき（環境変数が無い）は終了せず、ツールの呼び出しのたびに
/// その `storage_unavailable` を返す（クライアントに、起動に失敗した理由が伝わる）。
pub fn run(client: Option<&str>, env: &Env, io: &mut Io<'_>) -> i32 {
    let warnings = Warnings::default();
    // 接続の道具は起動時に 1 つだけ作る：環境を分けている形の警告を、プロセスごとに 1 回にするため
    let connector = connector(env, &warnings, client.unwrap_or("unknown"));
    if let Err(e) = &connector {
        let _ = writeln!(io.stderr, "yunagi: {}", error::message(e));
    }
    let mut call = |method: &str, params: Map<String, Value>| match &connector {
        Ok(c) => c.call(method, params),
        Err(e) => Err(e.clone()),
    };
    let mut after_call = |stderr: &mut dyn Write| {
        for line in warnings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain(..)
        {
            let _ = writeln!(stderr, "{line}");
        }
    };
    // 接続の道具を作るときに出た警告も、最初の呼び出しを待たずに出す
    after_call(&mut *io.stderr);
    serve(io.stdin, io.stdout, io.stderr, &mut call, &mut after_call)
}

/// 標準入力 `input` の依頼を 1 行ずつ読み、応答を `output` に 1 行ずつ書く。`after_call` は、ツールを
/// 1 回呼ぶたびに、たまった警告を標準エラー `stderr` に出すために呼ぶ。入力が終わったら 0。標準出力に
/// 書けなくなったら（クライアントが閉じた）1。
pub fn serve(
    mut input: &mut dyn BufRead,
    output: &mut dyn Write,
    stderr: &mut dyn Write,
    call: &mut CallFn<'_>,
    after_call: &mut dyn FnMut(&mut dyn Write),
) -> i32 {
    loop {
        let response = match read_message(&mut input, MAX_LINE) {
            Ok(Some(line)) if line.iter().all(u8::is_ascii_whitespace) => continue,
            Ok(Some(line)) => handle(&line, call, &mut || after_call(&mut *stderr)),
            // 入力が閉じられた（クライアントが終わらせた）
            Ok(None) => return 0,
            Err(LineError::TooLong) => {
                // 残りの行を読み捨てて、次の依頼から続ける
                if let Err(e) = skip_line(&mut *input) {
                    let _ = writeln!(stderr, "yunagi: could not read from standard input: {e}");
                    return 1;
                }
                Some(rpc_error(
                    &Value::Null,
                    INVALID_REQUEST,
                    "The message is too large.",
                    None,
                ))
            }
            Err(LineError::Io(e)) => {
                let _ = writeln!(stderr, "yunagi: could not read from standard input: {e}");
                return 1;
            }
        };
        if let Some(response) = response
            && write_line(output, &response).is_err()
        {
            return 1;
        }
    }
}

/// 1 行を読めなかった理由。
enum LineError {
    /// 上限を超えた。
    TooLong,
    Io(io::Error),
}

/// 改行までの 1 行を、最大 `max` バイト（改行を除く）まで読む。何も読まずに閉じられたら `Ok(None)`。
/// 改行の無いまま閉じられた最後の行も、1 行として返す（クライアントが最後の改行を付けずに閉じても、
/// 依頼を黙って捨てないため。ローカル API の `wire::read_line` は、閉じ方の誤りとして捨てる）。
fn read_message(input: &mut dyn BufRead, max: usize) -> Result<Option<Vec<u8>>, LineError> {
    let mut buf = Vec::new();
    // 上限 + 改行 1 バイトまで読む。ここで止まって改行が無ければ長すぎる
    let limit = u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1);
    let read = input
        .take(limit)
        .read_until(b'\n', &mut buf)
        .map_err(LineError::Io)?;
    if read == 0 {
        return Ok(None);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
    } else if buf.len() as u64 >= limit {
        return Err(LineError::TooLong);
    }
    Ok(Some(buf))
}

/// 改行まで読み捨てる。
fn skip_line(input: &mut dyn BufRead) -> io::Result<()> {
    loop {
        let buf = input.fill_buf()?;
        if buf.is_empty() {
            return Ok(());
        }
        match buf.iter().position(|b| *b == b'\n') {
            Some(i) => {
                input.consume(i + 1);
                return Ok(());
            }
            None => {
                let n = buf.len();
                input.consume(n);
            }
        }
    }
}

fn write_line(output: &mut dyn Write, message: &Value) -> io::Result<()> {
    let mut line = message.to_string();
    line.push('\n');
    output.write_all(line.as_bytes())?;
    output.flush()
}

fn rpc_error(id: &Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({"code": code, "message": message});
    if let Some(data) = data {
        error["data"] = data;
    }
    json!({"jsonrpc": "2.0", "id": id, "error": error})
}

fn rpc_result(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// 1 行を処理して、応答（通知には無い）を返す。`after_call` はツールを呼んだ後に呼ぶ。
fn handle(line: &[u8], call: &mut CallFn<'_>, after_call: &mut dyn FnMut()) -> Option<Value> {
    let message: Value = match serde_json::from_slice(line) {
        Ok(v) => v,
        Err(_) => {
            return Some(rpc_error(
                &Value::Null,
                PARSE_ERROR,
                "The line is not valid JSON.",
                None,
            ));
        }
    };
    let Value::Object(mut message) = message else {
        let text = if message.is_array() {
            "Batch requests are not supported. Send one message per line."
        } else {
            "The message must be a JSON object."
        };
        return Some(rpc_error(&Value::Null, INVALID_REQUEST, text, None));
    };
    // 応答（このサーバーは依頼を送らないので、来ても無視する）
    if message.contains_key("result") || message.contains_key("error") {
        return None;
    }
    // 読める `id` は、依頼の形の誤りの応答にも付ける（クライアントがどの依頼の誤りか分かるように）
    let known_id = message
        .get("id")
        .filter(|id| id.is_string() || id.is_number())
        .cloned()
        .unwrap_or(Value::Null);
    let Some(method) = message
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return Some(rpc_error(
            &known_id,
            INVALID_REQUEST,
            "The message has no method.",
            None,
        ));
    };
    // `id` の無いものは通知：応答しない
    let id = message.get("id")?.clone();
    if !(id.is_string() || id.is_number()) {
        return Some(rpc_error(
            &Value::Null,
            INVALID_REQUEST,
            "id must be a string or a number.",
            None,
        ));
    }
    let mut params = match message.remove("params") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(p)) => p,
        Some(_) => {
            return Some(rpc_error(
                &id,
                INVALID_PARAMS,
                "params must be an object.",
                None,
            ));
        }
    };
    // 最新の仕様の依頼は、`_meta` に版がある
    let modern = match params.get("_meta").and_then(|m| m.get(META_VERSION)) {
        None => false,
        Some(Value::String(version)) if version == MODERN_VERSION => true,
        // 旧い仕様の版を名乗る依頼は、旧い仕様の依頼として答える（断らない）
        Some(Value::String(version)) if LEGACY_VERSIONS.contains(&version.as_str()) => false,
        Some(Value::String(version)) => {
            return Some(rpc_error(
                &id,
                UNSUPPORTED_VERSION,
                "Unsupported protocol version",
                Some(json!({"supported": [MODERN_VERSION], "requested": version})),
            ));
        }
        Some(_) => {
            return Some(rpc_error(
                &id,
                INVALID_PARAMS,
                "The protocol version in _meta must be a string.",
                None,
            ));
        }
    };
    let outcome = match method.as_str() {
        "initialize" => initialize(&params),
        "ping" => Ok(json!({})),
        "server/discover" => Ok(discover()),
        "tools/list" => {
            Ok(json!({"tools": mcp_tools::list(), "ttlMs": 300_000, "cacheScope": "public"}))
        }
        "tools/call" => {
            let result = tools_call(&mut params, call);
            after_call();
            result
        }
        _ => Err((METHOD_NOT_FOUND, format!("Method not found: {method}"))),
    };
    Some(match outcome {
        Ok(mut result) => {
            // 最新の仕様の結果には `resultType` を付ける。キャッシュの案内は、それを求められる結果
            // （`server/discover`・`tools/list`）だけに付けてあり、旧い仕様の依頼には付けない
            if modern {
                result["resultType"] = json!("complete");
            } else if let Some(object) = result.as_object_mut() {
                object.remove("ttlMs");
                object.remove("cacheScope");
            }
            rpc_result(&id, result)
        }
        Err((code, text)) => rpc_error(&id, code, &text, None),
    })
}

fn server_info() -> Value {
    json!({"name": "yunagi", "title": "Yunagi", "version": yunagi_core::VERSION})
}

/// 旧い仕様の握手。依頼された版に対応していればその版、そうでなければ対応する最新の版で答える。
fn initialize(params: &Map<String, Value>) -> Result<Value, (i64, String)> {
    let Some(requested) = params.get("protocolVersion").and_then(Value::as_str) else {
        return Err((INVALID_PARAMS, "protocolVersion is required.".to_owned()));
    };
    let version = LEGACY_VERSIONS
        .iter()
        .find(|v| **v == requested)
        .unwrap_or(&LEGACY_VERSIONS[0]);
    Ok(json!({
        "protocolVersion": version,
        "capabilities": {"tools": {}},
        "serverInfo": server_info(),
        "instructions": INSTRUCTIONS,
    }))
}

/// 最新の仕様の `server/discover`。
fn discover() -> Value {
    json!({
        "supportedVersions": [MODERN_VERSION],
        "capabilities": {"tools": {}},
        "_meta": {"io.modelcontextprotocol/serverInfo": {"name": "yunagi", "version": yunagi_core::VERSION}},
        "instructions": INSTRUCTIONS,
        "ttlMs": 300_000,
        "cacheScope": "public",
    })
}

/// `tools/call`。知らないツールと `name` の欠けはプロトコルのエラー、それ以外の失敗（引数の誤りを含む）は
/// ツールの結果の `isError`（仕様の Error Handling）。
fn tools_call(
    params: &mut Map<String, Value>,
    call: &mut CallFn<'_>,
) -> Result<Value, (i64, String)> {
    let Some(name) = params
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return Err((INVALID_PARAMS, "name is required.".to_owned()));
    };
    if !mcp_tools::exists(&name) {
        return Err((INVALID_PARAMS, format!("Unknown tool: {name}")));
    }
    let arguments = match params.remove("arguments") {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(arguments)) => {
            // 受け取った行の長さは当てにしない（`1e5` が `100000.0` になるなど、JSON にし直すと長くなる
            // 書き方がある）。ローカル API に送る形で、書き出さずに長さだけ数える
            let mut size = Counter(0);
            let too_large =
                serde_json::to_writer(&mut size, &arguments).is_err() || size.0 > MAX_ARGUMENTS;
            if too_large {
                Err(error::invalid_argument(
                    "The arguments are too large. Send less text at a time.",
                ))
            } else {
                Ok(arguments)
            }
        }
        Some(_) => Err(error::invalid_argument("arguments must be an object.")),
    };
    let result = arguments.and_then(|arguments| call(&name, arguments));
    Ok(match result {
        Ok(value) => tool_result(&value, false),
        // CLI の `--json` と同じ包み方
        Err(e) => tool_result(&json!({"error": e}), true),
    })
}

/// 書き込まれたバイト数だけを数える（中身は捨てる）。
struct Counter(usize);

impl Write for Counter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0 += buf.len();
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn tool_result(value: &Value, is_error: bool) -> Value {
    json!({
        "content": [{"type": "text", "text": value.to_string()}],
        "isError": is_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 呼ばれたツールの名前と引数。
    type Called = (String, Map<String, Value>);

    /// 入力の行 `lines` を処理して、応答（行ごとの JSON）と、呼ばれたツールの一覧を返す。
    fn exchange(lines: &[&str]) -> (Vec<Value>, Vec<Called>) {
        let input = lines.join("\n") + "\n";
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let mut calls = Vec::new();
        let mut call = |method: &str, params: Map<String, Value>| {
            calls.push((method.to_owned(), params));
            match method {
                "get_board" => Ok(json!({"date": "2026-11-02"})),
                "add_todo" => Err(
                    json!({"code": "permission_denied", "required_scope": "todo", "message": "no"}),
                ),
                _ => Ok(json!({})),
            }
        };
        let code = serve(
            &mut input.as_bytes(),
            &mut out,
            &mut err,
            &mut call,
            &mut |_| {},
        );
        assert_eq!(code, 0);
        let responses = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        (responses, calls)
    }

    #[test]
    fn legacy_handshake_answers_the_requested_version() {
        let (r, _) = exchange(&[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"x","version":"1"}}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"initialize","params":{}}"#,
        ]);
        // 通知には答えない
        assert_eq!(r.len(), 3);
        assert_eq!(r[0]["id"], 1);
        assert_eq!(r[0]["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(r[0]["result"]["serverInfo"]["name"], "yunagi");
        assert!(r[0]["result"]["capabilities"]["tools"].is_object());
        assert!(r[0]["result"].get("resultType").is_none());
        // 知らない版には、対応する最新の版で答える
        assert_eq!(r[1]["result"]["protocolVersion"], "2025-11-25");
        assert_eq!(r[2]["error"]["code"], INVALID_PARAMS);
    }

    #[test]
    fn modern_requests_carry_their_version_in_meta() {
        let meta = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":"x","version":"1"},"io.modelcontextprotocol/clientCapabilities":{}}"#;
        let (r, _) = exchange(&[
            &format!(
                r#"{{"jsonrpc":"2.0","id":"d","method":"server/discover","params":{{{meta}}}}}"#
            ),
            &format!(r#"{{"jsonrpc":"2.0","id":"l","method":"tools/list","params":{{{meta}}}}}"#),
            r#"{"jsonrpc":"2.0","id":"v","method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"1900-01-01"}}}"#,
            r#"{"jsonrpc":"2.0","id":"w","method":"tools/list"}"#,
        ]);
        assert_eq!(r[0]["id"], "d");
        assert_eq!(r[0]["result"]["resultType"], "complete");
        assert_eq!(r[0]["result"]["supportedVersions"], json!(["2026-07-28"]));
        assert!(r[0]["result"]["capabilities"]["tools"].is_object());
        assert_eq!(r[0]["result"]["cacheScope"], "public");
        assert_eq!(r[1]["result"]["resultType"], "complete");
        assert_eq!(r[1]["result"]["tools"].as_array().unwrap().len(), 15);
        assert!(r[1]["result"]["ttlMs"].is_number());
        // 対応しない版
        assert_eq!(r[2]["error"]["code"], UNSUPPORTED_VERSION);
        assert_eq!(r[2]["error"]["data"]["supported"], json!(["2026-07-28"]));
        assert_eq!(r[2]["error"]["data"]["requested"], "1900-01-01");
        // 旧い仕様の依頼には resultType もキャッシュの案内も付けない
        assert!(r[3]["result"].get("resultType").is_none());
        assert!(r[3]["result"].get("ttlMs").is_none());
        assert_eq!(r[3]["result"]["tools"].as_array().unwrap().len(), 15);
    }

    #[test]
    fn ids_are_returned_as_they_came() {
        let (r, _) = exchange(&[
            r#"{"jsonrpc":"2.0","id":"abc","method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#,
        ]);
        assert_eq!((&r[0]["id"], &r[1]["id"]), (&json!("abc"), &json!(7)));
        assert_eq!(r[0]["result"], json!({}));
    }

    #[test]
    fn a_tool_result_carries_the_json_or_the_error_as_text() {
        let (r, calls) = exchange(&[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_board","arguments":{"date":"2026-11-02"}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"add_todo","arguments":{"text":"x"}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_board"}}"#,
        ]);
        let text = |i: usize| -> Value {
            serde_json::from_str(r[i]["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
        };
        assert_eq!(r[0]["result"]["isError"], false);
        assert_eq!(text(0), json!({"date": "2026-11-02"}));
        // エラーは isError で、CLI の --json と同じ包み方
        assert_eq!(r[1]["result"]["isError"], true);
        assert_eq!(text(1)["error"]["code"], "permission_denied");
        assert_eq!(text(1)["error"]["required_scope"], "todo");
        // 引数が無ければ空として渡す
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].1["date"], "2026-11-02");
        assert!(calls[2].1.is_empty());
    }

    #[test]
    fn unknown_tools_and_bad_requests_are_protocol_errors() {
        let (r, calls) = exchange(&[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"agent_revert","arguments":{}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"arguments":{}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"nothing/here"}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":[]}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"get_board","arguments":[1]}}"#,
        ]);
        assert_eq!(r[0]["error"]["code"], INVALID_PARAMS);
        assert!(
            r[0]["error"]["message"]
                .as_str()
                .unwrap()
                .contains("agent_revert")
        );
        assert_eq!(r[1]["error"]["code"], INVALID_PARAMS);
        assert_eq!(r[2]["error"]["code"], METHOD_NOT_FOUND);
        assert_eq!(r[3]["error"]["code"], INVALID_PARAMS);
        // 引数がオブジェクトでないのは、ツールのエラー（invalid_argument）
        assert_eq!(r[4]["result"]["isError"], true);
        assert!(calls.is_empty());
    }

    #[test]
    fn broken_lines_get_an_error_and_the_server_keeps_going() {
        let (r, _) = exchange(&[
            "not json",
            "[1,2]",
            "42",
            r#"{"jsonrpc":"2.0","id":{"x":1},"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":1}"#,
            "",
            r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#,
        ]);
        assert_eq!(r[0]["error"]["code"], PARSE_ERROR);
        assert_eq!(r[0]["id"], Value::Null);
        assert_eq!(r[1]["error"]["code"], INVALID_REQUEST);
        assert_eq!(r[2]["error"]["code"], INVALID_REQUEST);
        assert_eq!(r[3]["error"]["code"], INVALID_REQUEST);
        assert_eq!(r[4]["error"]["code"], INVALID_REQUEST);
        assert_eq!(r[5]["id"], 2);
        assert_eq!(r.len(), 6);
    }

    #[test]
    fn a_readable_id_is_echoed_in_a_malformed_request_and_legacy_meta_is_served() {
        let (r, _) = exchange(&[
            r#"{"jsonrpc":"2.0","id":5,"method":123}"#,
            r#"{"jsonrpc":"2.0","id":"six"}"#,
            r#"{"jsonrpc":"2.0","id":7,"method":"ping","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2025-06-18"}}}"#,
        ]);
        assert_eq!(
            (r[0]["id"].clone(), r[0]["error"]["code"].clone()),
            (json!(5), json!(INVALID_REQUEST))
        );
        assert_eq!(r[1]["id"], "six");
        // 旧い仕様の版を名乗る依頼は断らず、旧い仕様の依頼として答える
        assert_eq!(r[2]["result"], json!({}));
    }

    #[test]
    fn a_last_line_without_a_newline_is_still_handled() {
        let input: &[u8] = br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let mut call = |_: &str, _: Map<String, Value>| Ok(json!({}));
        let code = serve(&mut { input }, &mut out, &mut err, &mut call, &mut |_| {});
        assert_eq!(code, 0);
        let response: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(response["id"], 1);
    }

    #[test]
    fn a_line_that_is_too_large_is_skipped() {
        let big = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"ping","x":"{}"}}"#,
            "a".repeat(MAX_LINE)
        );
        let (r, _) = exchange(&[&big, r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#]);
        assert_eq!(r[0]["error"]["code"], INVALID_REQUEST);
        assert_eq!(r[1]["id"], 2);
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn arguments_over_the_api_limit_are_refused_before_calling() {
        let text = "a".repeat(MAX_ARGUMENTS);
        let line = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"append_note","arguments":{"text": text}}}).to_string();
        let (r, calls) = exchange(&[&line]);
        assert!(calls.is_empty());
        assert_eq!(r[0]["result"]["isError"], true);
        let e: Value =
            serde_json::from_str(r[0]["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(e["error"]["code"], "invalid_argument");
    }
}
