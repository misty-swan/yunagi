//! クライアント：接続して認証し、依頼を 1 つ送る（docs/design/01-architecture.md 4.4 の手順 1 の
//! 1 回分と、4.6）。接続の切り替え（直接モードとの選び分け・試し直し・期限）は、CLI・MCP が
//! このクレートの上に作る（YUN-90）。ここは「1 回の接続と認証」と「依頼と応答」までを持つ。
//!
//! 順：接続 → トークンを読む → `hello`。トークンを読むのを接続の後にするのは、サーバーが
//! トークンをソケットより先に書くため、つながった時点のファイルがその待ち受けのものだからである
//! （先に読むと、アプリの再起動と重なったとき、古いトークンで誤って認証に失敗する）。

use std::io::{self, BufReader, Read, Write};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use yunagi_core::storage::data_dir::DefaultDir;

use crate::token;
use crate::transport::{self, ConnectError, SocketPathTooLong, Stream};
use crate::wire::{self, HelloParams, HelloResult, LineError};
use crate::{MAX_HELLO_LINE, MAX_RESPONSE_LINE, PROTOCOL_VERSION};

/// 接続と認証ができなかった理由（4.4 の手順 1 の結果に当たる）。
#[derive(Debug)]
pub enum HelloError {
    /// 待ち受けが無い。直接モードを試してよい。
    NoListener,
    /// 待ち受けはあるが一時的につながらない（直接モードには切り替えず、試し直す）。
    Transient(io::Error),
    /// 接続を受けたが、認証に答えない（固まっている。まだ何も送っていない依頼は無いので、試し直す）。
    NotAnswering,
    /// ソケットを開く権限が無い（`api_error` の `denied`）。
    Denied(io::Error),
    /// ソケットのパスが長すぎる（UNIX 系）。
    SocketPathTooLong(SocketPathTooLong),
    /// トークンが合わない、またはトークンのファイルを読めない（`api_error` の `auth_failed`）。
    AuthFailed,
    /// プロトコルの版が合わない（`api_error` の `version_mismatch`）。`server` はアプリの版（分かれば）。
    VersionMismatch { server: Option<u32> },
    /// ほかの失敗。
    Io(io::Error),
}

impl From<ConnectError> for HelloError {
    fn from(e: ConnectError) -> Self {
        match e {
            ConnectError::NoListener => Self::NoListener,
            ConnectError::Transient(e) => Self::Transient(e),
            ConnectError::Denied(e) => Self::Denied(e),
            ConnectError::SocketPathTooLong(e) => Self::SocketPathTooLong(e),
            ConnectError::Io(e) => Self::Io(e),
        }
    }
}

/// サーバーが返した失敗（JSON-RPC の `error`）。
#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl RpcError {
    /// アプリのエラー（05 4.3 の `{"code": ..., "message": ...}`）。標準のコードのときは `None`。
    pub fn app_error(&self) -> Option<&Value> {
        self.data.as_ref()?.get("error")
    }
}

/// 依頼が失敗した理由。
#[derive(Debug)]
pub enum CallError {
    /// 依頼を送る前に失敗した。何も送っていない。
    NotSent(io::Error),
    /// サーバーが失敗を返した。
    Rpc(RpcError),
    /// 依頼を送った後、応答を受け取る前に接続が切れた。反映されたかが分からない
    /// （`api_error` の `result_unknown`）。
    ConnectionLost,
    /// 応答の形が誤っている。
    Protocol(String),
}

struct Shared(Arc<Stream>);

impl Read for Shared {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (&*self.0).read(buf)
    }
}

/// 認証を済ませた接続。依頼を 1 つ送って閉じる（[`Session::call`]）。
pub struct Session {
    stream: Arc<Stream>,
    reader: BufReader<Shared>,
    /// アプリのプロトコルの版とアプリの版。
    pub server: HelloResult,
}

/// 既定ディレクトリ `dir` の待ち受けに接続して認証する。`client` はクライアントの識別子
/// （無ければ `unknown`）。通信路への接続そのもの（UNIX 系の `connect`）と、つながってから `hello` の
/// 応答までの合計に、`timeout` の上限をかける（4.4 の 1 秒。アーキテクチャ設計書 4.6）。
pub fn connect(
    dir: &DefaultDir,
    client: Option<&str>,
    timeout: Duration,
) -> Result<Session, HelloError> {
    let started = std::time::Instant::now();
    let (stream, info) = transport::connect(dir, timeout)?;
    // つながったのにトークンが読めない：古い（消えた）待ち受けか、壊れている
    let info = info.or_else(|| token::read(dir).ok());
    let Some(info) = info else {
        return Err(HelloError::AuthFailed);
    };
    // 接続に使った時間を引き、1 回の接続と認証の合計が `timeout` を超えないようにする
    hello(
        stream,
        &info.token,
        client,
        timeout.saturating_sub(started.elapsed()),
    )
}

/// つながった `stream` で `hello` を行う。
pub fn hello(
    stream: Stream,
    token: &str,
    client: Option<&str>,
    timeout: Duration,
) -> Result<Session, HelloError> {
    let stream = Arc::new(stream);
    // 0 の時間切れは設定できないので、1 ミリ秒を下限にする（期限の残りが尽きかけているとき）
    let timeout = timeout.max(Duration::from_millis(1));
    stream
        .set_read_timeout(Some(timeout))
        .map_err(HelloError::Io)?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(HelloError::Io)?;
    let params = HelloParams {
        protocol: PROTOCOL_VERSION,
        token: token.to_owned(),
        client: client.map(str::to_owned),
    };
    let mut line = json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": wire::METHOD_HELLO,
        "params": params,
    })
    .to_string();
    line.push('\n');
    let mut writer: &Stream = &stream;
    // 書けない：まだ何も送っていない。認証に答えないのと同じに扱う
    writer
        .write_all(line.as_bytes())
        .map_err(|_| HelloError::NotAnswering)?;
    let mut reader = BufReader::new(Shared(Arc::clone(&stream)));
    let reply = match wire::read_line(&mut reader, MAX_HELLO_LINE.max(64 * 1024)) {
        Ok(Some(reply)) => reply,
        // 時間切れ・切断・何も返さずに閉じた
        Ok(None) | Err(LineError::Io(_) | LineError::Truncated) => {
            return Err(HelloError::NotAnswering);
        }
        Err(LineError::TooLong) => {
            return Err(HelloError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "the hello reply is too long",
            )));
        }
    };
    let value: Value = serde_json::from_slice(&reply)
        .map_err(|e| HelloError::Io(io::Error::new(io::ErrorKind::InvalidData, e)))?;
    if let Some(error) = value.get("error") {
        let reason = error.pointer("/data/error/reason").and_then(Value::as_str);
        return Err(match reason {
            Some("auth_failed") => HelloError::AuthFailed,
            Some("version_mismatch") => HelloError::VersionMismatch {
                server: error
                    .pointer("/data/error/server_protocol")
                    .and_then(Value::as_u64)
                    .and_then(|v| u32::try_from(v).ok()),
            },
            _ => HelloError::Io(io::Error::other("the hello was refused")),
        });
    }
    let server: HelloResult = value
        .get("result")
        .cloned()
        .and_then(|r| serde_json::from_value(r).ok())
        .ok_or_else(|| {
            HelloError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "the hello reply has no result",
            ))
        })?;
    // 依頼を送った後の応答の待ちには、上限を設けない（上限で切って送り直すと、反映済みの書き込みが
    // 二重になる。4.4）
    stream.set_read_timeout(None).map_err(HelloError::Io)?;
    Ok(Session {
        stream,
        reader,
        server,
    })
}

impl Session {
    /// 依頼を 1 つ送り、応答を受け取る。接続はこれで閉じる。
    pub fn call(mut self, method: &str, params: &Value) -> Result<Value, CallError> {
        let mut line = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        })
        .to_string();
        line.push('\n');
        let mut writer: &Stream = &self.stream;
        writer
            .write_all(line.as_bytes())
            .and_then(|()| writer.flush())
            .map_err(CallError::NotSent)?;
        let reply = match wire::read_line(&mut self.reader, MAX_RESPONSE_LINE) {
            Ok(Some(reply)) => reply,
            Ok(None) | Err(LineError::Io(_) | LineError::Truncated) => {
                return Err(CallError::ConnectionLost);
            }
            Err(LineError::TooLong) => {
                return Err(CallError::Protocol("the response is too long".into()));
            }
        };
        let value: Value = serde_json::from_slice(&reply)
            .map_err(|e| CallError::Protocol(format!("the response is not JSON: {e}")))?;
        if let Some(error) = value.get("error") {
            return Err(CallError::Rpc(RpcError {
                code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                message: error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                data: error.get("data").cloned(),
            }));
        }
        value
            .get("result")
            .cloned()
            .ok_or_else(|| CallError::Protocol("the response has no result".into()))
    }
}
