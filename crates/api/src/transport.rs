//! 通信路：UNIX ドメインソケット（macOS・Linux）と名前付きパイプ（Windows）。ネットワークの
//! ポートは開かない（docs/release/SECURITY.md 4.2）。
//!
//! どちらも同じ形で使えるよう、待ち受け（[`Listener`]）・つながった相手（[`Stream`]）・
//! 別のスレッドから読み待ちを起こす道具（[`Interrupter`]）・接続（[`connect`]）を揃える。
//! 守りの決まりは docs/design/01-architecture.md 4.6。

use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub use unix::{Interrupter, Listener, Stream, Waker, connect};
#[cfg(windows)]
pub use windows::{Interrupter, Listener, Stream, Waker, connect};

/// ソケットのパス（`sockaddr_un.sun_path`）の長さの上限（バイト。末尾の NUL を含む）。
/// macOS（と BSD 系）は 104、Linux は 108。
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "dragonfly"
))]
pub const SUN_PATH_LIMIT: usize = 104;
/// ソケットのパス（`sockaddr_un.sun_path`）の長さの上限（バイト。末尾の NUL を含む）。
#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "dragonfly"
)))]
pub const SUN_PATH_LIMIT: usize = 108;

/// ソケットのパスが長すぎて、`sun_path` に収まらない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketPathTooLong {
    pub path: PathBuf,
    /// パスの長さ（バイト）。
    pub len: usize,
    /// 使える長さの上限（バイト。NUL の分を除く）。
    pub max: usize,
}

impl std::fmt::Display for SocketPathTooLong {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the socket path {} is {} bytes, over the limit of {}",
            self.path.display(),
            self.len,
            self.max
        )
    }
}

impl std::error::Error for SocketPathTooLong {}

/// ソケットのパス `path` が `sun_path` に収まるかを確かめる。アプリ（待ち受け）とクライアントの両方が、
/// 同じこの判定を使う（収まらないとき、どちらも別の場所に置き直さず、決まった結果を返す。4.6）。
pub fn check_socket_path(path: &Path) -> Result<(), SocketPathTooLong> {
    let len = path_len(path);
    let max = SUN_PATH_LIMIT - 1;
    if len > max {
        return Err(SocketPathTooLong {
            path: path.to_owned(),
            len,
            max,
        });
    }
    Ok(())
}

#[cfg(unix)]
fn path_len(path: &Path) -> usize {
    std::os::unix::ffi::OsStrExt::as_bytes(path.as_os_str()).len()
}

#[cfg(not(unix))]
fn path_len(path: &Path) -> usize {
    path.as_os_str().len()
}

/// 待ち受けを始められなかった理由。どれも、トークンのファイルとソケットを残さない。
#[derive(Debug)]
pub enum ListenError {
    /// ソケットのパスが長すぎる（UNIX 系）。
    SocketPathTooLong(SocketPathTooLong),
    /// ソケットの場所に、ソケットでもシンボリックリンクでもないもの（フォルダ・普通のファイル）がある。
    /// 消さない。
    PathOccupied(PathBuf),
    /// パイプの名前をほかのプロセスが先に作っていた（Windows）。待ち受けを乗っ取られないよう、
    /// 使わずに止める。
    PipeNameTaken(String),
    /// ファイルや OS の操作が失敗した。`action` は何をしようとしたか。
    Io {
        action: &'static str,
        source: io::Error,
    },
}

impl std::fmt::Display for ListenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SocketPathTooLong(e) => e.fmt(f),
            Self::PathOccupied(p) => write!(
                f,
                "{} exists and is not a socket, so the local API did not start",
                p.display()
            ),
            Self::PipeNameTaken(name) => write!(
                f,
                "the pipe name {name} was already taken, so the local API did not start"
            ),
            Self::Io { action, .. } => write!(f, "failed to {action}"),
        }
    }
}

impl std::error::Error for ListenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::SocketPathTooLong(e) => Some(e),
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// 接続できなかった理由の区分（アーキテクチャ設計書 4.4 の手順 1）。
#[derive(Debug)]
pub enum ConnectError {
    /// 待ち受けが無い：ソケット（パイプ）のファイルが無い、`ECONNREFUSED`（異常終了で残ったソケット）、
    /// Windows でパイプが無い。直接モードを試してよい。
    NoListener,
    /// 待ち受けはあるが一時的につながらない：パイプが全部ふさがっている、待ち行列が満杯、割り込まれた、
    /// 時間切れ。直接モードには切り替えず、試し直す。
    Transient(io::Error),
    /// 待ち受けはあるが使えない：権限が無い（`EACCES`）。
    Denied(io::Error),
    /// ソケットのパスが長すぎる（UNIX 系）。
    SocketPathTooLong(SocketPathTooLong),
    /// ほかの失敗。
    Io(io::Error),
}

/// 接続の失敗 `e` を区分する。
pub fn classify(e: io::Error) -> ConnectError {
    use io::ErrorKind::*;
    // Windows の ERROR_PIPE_BUSY（パイプが全部ふさがっている）
    if cfg!(windows) && e.raw_os_error() == Some(231) {
        return ConnectError::Transient(e);
    }
    match e.kind() {
        NotFound | ConnectionRefused | NotADirectory => ConnectError::NoListener,
        PermissionDenied => ConnectError::Denied(e),
        WouldBlock | Interrupted | TimedOut => ConnectError::Transient(e),
        _ => ConnectError::Io(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_paths_over_the_limit_are_refused() {
        let max = SUN_PATH_LIMIT - 1;
        let fits = PathBuf::from(format!("/{}", "a".repeat(max - 1)));
        assert!(check_socket_path(&fits).is_ok());
        let too_long = PathBuf::from(format!("/{}", "a".repeat(max)));
        let e = check_socket_path(&too_long).unwrap_err();
        assert_eq!((e.len, e.max), (max + 1, max));
    }

    #[test]
    fn connection_failures_are_classified_for_the_connection_procedure() {
        let kind = |k: io::ErrorKind| classify(io::Error::from(k));
        assert!(matches!(
            kind(io::ErrorKind::NotFound),
            ConnectError::NoListener
        ));
        assert!(matches!(
            kind(io::ErrorKind::ConnectionRefused),
            ConnectError::NoListener
        ));
        assert!(matches!(
            kind(io::ErrorKind::PermissionDenied),
            ConnectError::Denied(_)
        ));
        assert!(matches!(
            kind(io::ErrorKind::WouldBlock),
            ConnectError::Transient(_)
        ));
        assert!(matches!(
            kind(io::ErrorKind::TimedOut),
            ConnectError::Transient(_)
        ));
        assert!(matches!(kind(io::ErrorKind::Other), ConnectError::Io(_)));
    }
}
