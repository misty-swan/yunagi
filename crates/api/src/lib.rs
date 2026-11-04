//! Yunagi のローカル API の取り決め（docs/design/01-architecture.md 4.4・4.6、
//! docs/design/05-mcp-cli-spec.md 4.4）。
//!
//! アプリ（待ち受け。`src-tauri`）と、コンソール用バイナリ `yunagi`（接続。CLI・MCP サーバー）が
//! 同じものを使うので、両方から依存できるこのクレートに置く。コアは接続の方式を知らないままにする
//! （4.4）ので、コアには置かない。
//!
//! - [`token`]：認証のトークンの生成・保存・読み取り・比較（定数時間）。
//! - [`wire`]：プロトコル（JSON-RPC 2.0 を 1 行 1 メッセージで送る）の型と、行の読み方。
//! - [`transport`]：UNIX ドメインソケット（macOS・Linux）と名前付きパイプ（Windows）。
//! - [`client`]：接続して認証し、依頼を 1 つ送る（接続の手順 4.4 の 1 回分）。
//!
//! ネットワークのポートは開かない（docs/release/SECURITY.md 4.2）。

pub mod client;
pub mod token;
pub mod transport;
pub mod wire;

use std::path::PathBuf;
use std::time::Duration;

use yunagi_core::storage::data_dir::DefaultDir;

/// プロトコルの版。互換のない変更をするときに上げる。`hello` は版をまたいで形を変えない
/// （版の食い違いを、必ず `hello` で見分けられるようにするため。4.6）。
pub const PROTOCOL_VERSION: u32 = 1;

/// 認証のトークン（と、Windows ではパイプの名前）を置くファイルの名前。既定ディレクトリに置く。
pub const TOKEN_FILE_NAME: &str = "api.token";
/// UNIX ドメインソケットのファイルの名前。既定ディレクトリに置く。
pub const SOCKET_FILE_NAME: &str = "api.sock";

/// `hello` の 1 行の大きさの上限（バイト）。
pub const MAX_HELLO_LINE: usize = 4 * 1024;
/// 依頼の 1 行の大きさの上限（バイト）。ノートの本文が入る大きさ。
pub const MAX_REQUEST_LINE: usize = 4 * 1024 * 1024;
/// 応答の 1 行の大きさの上限（バイト）。クライアントが読むときの上限。
pub const MAX_RESPONSE_LINE: usize = 64 * 1024 * 1024;

/// クライアントが、接続して `hello` の応答を受けるまでに待つ時間の上限（4.4 の 1 秒）。
pub const CLIENT_CONNECT_TIMEOUT: Duration = Duration::from_secs(1);

/// ソケットのパス（UNIX 系）。長さの上限は [`transport::check_socket_path`]。
pub fn socket_path(dir: &DefaultDir) -> PathBuf {
    dir.path().join(SOCKET_FILE_NAME)
}

/// トークンのファイルのパス。
pub fn token_path(dir: &DefaultDir) -> PathBuf {
    dir.path().join(TOKEN_FILE_NAME)
}
