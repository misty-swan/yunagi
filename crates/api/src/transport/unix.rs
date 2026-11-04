//! UNIX ドメインソケットの通信路（macOS・Linux）。
//!
//! 守り（docs/design/01-architecture.md 4.6）：
//!
//! - 守りの境目は、既定ディレクトリが本人だけの権限（0700）であること。待ち受けを始めるとき、
//!   ゆるければ 0700 に狭める。ソケットのファイルも bind の後に 0600 にする（二重の守り。bind から
//!   権限を変えるまでの隙間があり、接続のときにソケットの権限を確かめない OS もあるため、
//!   ソケットの権限だけには頼らない）。
//! - `umask` はプロセス全体に効くので触らない。
//! - Linux の抽象名前空間のソケットは、ファイルの権限が無く誰でも接続できるので使わない。

use std::fs;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use yunagi_core::storage::data_dir::DefaultDir;

use super::{ConnectError, ListenError, check_socket_path, classify};
use crate::SOCKET_FILE_NAME;
use crate::token::{self, ConnectionInfo};

/// 待ち受け。作ると、トークンのファイルを書いてからソケットを作る。手放すと、ソケットとトークンの
/// ファイルを消す。
#[derive(Debug)]
pub struct Listener {
    inner: UnixListener,
    dir: PathBuf,
    path: PathBuf,
    /// 書いたトークン。消すときに、自分のものかを確かめる。
    token: String,
    /// 作ったソケットのファイルの（デバイス, i-node）。消すときに、自分のものかを確かめる。
    identity: Option<(u64, u64)>,
}

impl Listener {
    /// 既定ディレクトリ `dir` で待ち受けを始める。`token` を、ソケットを作るより先に書く。
    ///
    /// 順：パスの長さを確かめる → フォルダを 0700 にする → トークンを書く → 古いソケット（ロックを
    /// 持つアプリだけが消す。呼び出し側が持っている）を消す → bind → 0600 にする。
    /// ソケットの場所にソケット・シンボリックリンク以外のものがあれば、消さずに断る。
    pub fn open(dir: &DefaultDir, token: &str) -> Result<Self, ListenError> {
        Self::open_in(dir.path(), token)
    }

    pub(crate) fn open_in(dir: &Path, token: &str) -> Result<Self, ListenError> {
        let io = |action| move |source| ListenError::Io { action, source };
        let path = dir.join(SOCKET_FILE_NAME);
        check_socket_path(&path).map_err(ListenError::SocketPathTooLong)?;
        narrow_dir(dir).map_err(io("restrict the permissions of the default directory"))?;
        token::write_in(
            dir,
            &ConnectionInfo {
                token: token.to_owned(),
                pipe_name: None,
            },
        )
        .map_err(io("write the token file"))?;
        let opened = (|| {
            remove_stale_socket(&path)?;
            let inner = UnixListener::bind(&path).map_err(io("bind the socket"))?;
            let listener = Self {
                inner,
                dir: dir.to_owned(),
                path: path.clone(),
                token: token.to_owned(),
                identity: identity(&path),
            };
            // bind の後に権限を狭める。失敗したら使わない（Drop が消す）
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                .map_err(io("restrict the permissions of the socket"))?;
            Ok(listener)
        })();
        if opened.is_err() {
            let _ = token::remove_in(dir);
        }
        opened
    }

    /// 次の接続を待つ（止まって待つ）。止めるときは [`Listener::wake`] で起こす。
    pub fn accept(&self) -> io::Result<Stream> {
        self.inner.accept().map(|(s, _)| Stream(s))
    }

    /// 止まって待っている [`Listener::accept`] を、自分に接続して起こす。
    pub fn wake(&self) {
        self.waker().wake();
    }

    /// [`Listener::wake`] だけができる道具（待ち受けを持つスレッドとは別のスレッドが持つ）。
    pub fn waker(&self) -> Waker {
        Waker(self.path.clone())
    }

    /// ソケットのファイルを消す（新しいクライアントが「待ち受けが無い」と分かるように）。
    /// 何度呼んでもよい。
    pub fn unlink(&self) {
        remove_socket(&self.path, self.identity);
    }
}

/// ソケットのファイルの（デバイス, i-node）。
fn identity(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    fs::symlink_metadata(path).ok().map(|m| (m.dev(), m.ino()))
}

/// 待ち受けを起こす道具（[`Listener::waker`]）。
#[derive(Debug, Clone)]
pub struct Waker(PathBuf);

impl Waker {
    /// 止まって待っている [`Listener::accept`] を、自分に接続して起こす。
    pub fn wake(&self) {
        let _ = UnixStream::connect(&self.0);
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        remove_socket(&self.path, self.identity);
        token::remove_own_in(&self.dir, &self.token);
    }
}

/// 自分が作ったソケットのときだけ消す。止まるのが遅れて、次の待ち受け（新しいアプリ）が作り直した
/// 後なら、そのソケットを消さない。
fn remove_socket(path: &Path, own: Option<(u64, u64)>) {
    if own.is_some()
        && identity(path) == own
        && fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_socket())
    {
        let _ = fs::remove_file(path);
    }
}

/// フォルダに本人以外が入れるなら 0700 にする。
fn narrow_dir(dir: &Path) -> io::Result<()> {
    let mode = fs::metadata(dir)?.permissions().mode();
    if mode & 0o077 != 0 {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// 異常終了で残ったソケット（とシンボリックリンク）を消す。ほかのものがあれば断る。
fn remove_stale_socket(path: &Path) -> Result<(), ListenError> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_socket() || m.file_type().is_symlink() => fs::remove_file(path)
            .map_err(|source| ListenError::Io {
                action: "remove the stale socket",
                source,
            }),
        Ok(_) => Err(ListenError::PathOccupied(path.to_owned())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(ListenError::Io {
            action: "inspect the socket path",
            source,
        }),
    }
}

/// つながった相手。
#[derive(Debug)]
pub struct Stream(UnixStream);

impl Stream {
    /// 読むときの待ちの上限（`None` で無制限）。超えると読みは `WouldBlock`・`TimedOut` で失敗する。
    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.0.set_read_timeout(timeout)
    }

    /// 次の読みを `deadline` までしか待たない（読みごとに呼ぶ。残りの時間を読みの時間切れにする）。
    /// 期限を過ぎていれば `TimedOut` で失敗する。
    pub fn set_read_deadline(&self, deadline: std::time::Instant) -> io::Result<()> {
        match deadline
            .checked_duration_since(std::time::Instant::now())
            .filter(|d| !d.is_zero())
        {
            Some(left) => self.0.set_read_timeout(Some(left)),
            None => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the read deadline passed",
            )),
        }
    }

    /// 書くときの待ちの上限（`None` で無制限）。
    pub fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.0.set_write_timeout(timeout)
    }

    /// 別のスレッドから、読み待ちを起こして接続を閉じさせる道具。
    pub fn interrupter(&self) -> io::Result<Interrupter> {
        self.0.try_clone().map(Interrupter)
    }

    /// 応答を書き終えたことを相手に伝える（書く側を閉じる）。
    pub fn finish(&self) {
        let _ = self.0.shutdown(Shutdown::Write);
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl Read for &Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (&self.0).read(buf)
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl Write for &Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&self.0).write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        (&self.0).flush()
    }
}

/// 読み待ちを起こす道具（[`Stream::interrupter`]）。
#[derive(Debug)]
pub struct Interrupter(UnixStream);

impl Interrupter {
    /// 読み待ちを終わらせる。読み待ちのスレッドは、読み終わり（EOF）で起きる。読む側だけを閉じる
    /// ので、起きたスレッドは、依頼を受け取れなかったことを伝える応答を書ける。
    pub fn interrupt(&self) {
        let _ = self.0.shutdown(Shutdown::Read);
    }
}

/// 既定ディレクトリ `dir` のソケットに接続する。UNIX 系では、トークンのファイルは接続した後に読む
/// （サーバーはトークンをソケットより先に書くので、つながった時点のファイルがその待ち受けのもの。
/// 先に読むと、アプリの再起動と重なったとき古いトークンで誤って認証に失敗する。4.6）ので、
/// 戻りの 2 つ目は常に `None`。
pub fn connect(dir: &DefaultDir) -> Result<(Stream, Option<ConnectionInfo>), ConnectError> {
    connect_in(dir.path())
}

pub(crate) fn connect_in(dir: &Path) -> Result<(Stream, Option<ConnectionInfo>), ConnectError> {
    let path = dir.join(SOCKET_FILE_NAME);
    check_socket_path(&path).map_err(ConnectError::SocketPathTooLong)?;
    UnixStream::connect(&path)
        .map(|s| (Stream(s), None))
        .map_err(classify)
}

#[cfg(test)]
mod tests {
    use std::io::BufRead;
    use std::os::unix::fs::symlink;

    use super::*;

    fn token() -> String {
        token::generate()
    }

    #[test]
    fn opening_writes_the_token_first_and_restricts_permissions() {
        let tmp = tempfile::tempdir().unwrap();
        fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let t = token();
        let listener = Listener::open_in(tmp.path(), &t).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(tmp.path()), 0o700);
        assert_eq!(mode(&tmp.path().join(SOCKET_FILE_NAME)), 0o600);
        assert_eq!(mode(&tmp.path().join(crate::TOKEN_FILE_NAME)), 0o600);
        assert_eq!(token::read_in(tmp.path()).unwrap().token, t);
        drop(listener);
        assert!(!tmp.path().join(SOCKET_FILE_NAME).exists());
        assert!(!tmp.path().join(crate::TOKEN_FILE_NAME).exists());
    }

    #[test]
    fn a_stale_socket_is_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(SOCKET_FILE_NAME);
        // 異常終了で残ったソケット（待ち受けていない）
        drop(UnixListener::bind(&path).unwrap());
        assert!(path.exists());
        let _listener = Listener::open_in(tmp.path(), &token()).unwrap();
        assert!(UnixStream::connect(&path).is_ok());
    }

    #[test]
    fn a_late_listener_does_not_remove_a_newer_ones_files() {
        let tmp = tempfile::tempdir().unwrap();
        let old = Listener::open_in(tmp.path(), &token()).unwrap();
        // 古い待ち受けの止まりが遅れる間に、次の待ち受け（新しいアプリ）がソケットとトークンを作り直す
        let newer_token = token();
        let newer = Listener::open_in(tmp.path(), &newer_token).unwrap();
        old.unlink();
        drop(old);
        assert!(UnixStream::connect(tmp.path().join(SOCKET_FILE_NAME)).is_ok());
        assert_eq!(token::read_in(tmp.path()).unwrap().token, newer_token);
        drop(newer);
        assert!(!tmp.path().join(SOCKET_FILE_NAME).exists());
        assert!(!tmp.path().join(crate::TOKEN_FILE_NAME).exists());
    }

    #[test]
    fn a_stale_symlink_is_replaced_without_touching_its_target() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        fs::write(&target, "keep").unwrap();
        symlink(&target, tmp.path().join(SOCKET_FILE_NAME)).unwrap();
        let _listener = Listener::open_in(tmp.path(), &token()).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "keep");
    }

    #[test]
    fn something_else_at_the_socket_path_is_not_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(SOCKET_FILE_NAME);
        fs::write(&path, "data").unwrap();
        let e = Listener::open_in(tmp.path(), &token()).unwrap_err();
        assert!(matches!(e, ListenError::PathOccupied(_)), "{e}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "data");
        // 失敗したときはトークンのファイルも残さない
        assert!(!tmp.path().join(crate::TOKEN_FILE_NAME).exists());
    }

    #[test]
    fn a_too_long_path_is_refused_before_anything_is_written() {
        let tmp = tempfile::tempdir().unwrap();
        let long = tmp.path().join("a".repeat(120));
        fs::create_dir(&long).unwrap();
        let e = Listener::open_in(&long, &token()).unwrap_err();
        assert!(matches!(e, ListenError::SocketPathTooLong(_)), "{e}");
        assert!(!long.join(crate::TOKEN_FILE_NAME).exists());
        assert!(matches!(
            connect_in(&long).unwrap_err(),
            ConnectError::SocketPathTooLong(_)
        ));
    }

    #[test]
    fn connect_reports_no_listener_for_a_missing_or_stale_socket() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(matches!(
            connect_in(tmp.path()).unwrap_err(),
            ConnectError::NoListener
        ));
        drop(UnixListener::bind(tmp.path().join(SOCKET_FILE_NAME)).unwrap());
        assert!(matches!(
            connect_in(tmp.path()).unwrap_err(),
            ConnectError::NoListener
        ));
    }

    #[test]
    fn accept_and_wake_and_exchange_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let listener = Listener::open_in(tmp.path(), &token()).unwrap();
        let (mut client, _) = connect_in(tmp.path()).unwrap();
        let server = listener.accept().unwrap();
        client.write_all(b"ping\n").unwrap();
        let mut reader = std::io::BufReader::new(&server);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line, "ping\n");
        (&server).write_all(b"pong\n").unwrap();
        server.finish();
        let mut got = String::new();
        client.read_to_string(&mut got).unwrap();
        assert_eq!(got, "pong\n");
        // 起こされた accept は接続を 1 つ返す
        let waker = std::thread::scope(|s| {
            let h = s.spawn(|| listener.accept().is_ok());
            std::thread::sleep(Duration::from_millis(50));
            listener.wake();
            h.join().unwrap()
        });
        assert!(waker);
    }

    #[test]
    fn the_interrupter_ends_a_waiting_read() {
        let tmp = tempfile::tempdir().unwrap();
        let listener = Listener::open_in(tmp.path(), &token()).unwrap();
        let (_client, _) = connect_in(tmp.path()).unwrap();
        let server = listener.accept().unwrap();
        let interrupter = server.interrupter().unwrap();
        std::thread::scope(|s| {
            let h = s.spawn(|| {
                let mut buf = [0u8; 8];
                (&server).read(&mut buf).unwrap()
            });
            std::thread::sleep(Duration::from_millis(50));
            interrupter.interrupt();
            assert_eq!(h.join().unwrap(), 0);
        });
    }

    #[test]
    fn a_read_timeout_ends_a_waiting_read() {
        let tmp = tempfile::tempdir().unwrap();
        let listener = Listener::open_in(tmp.path(), &token()).unwrap();
        let (_client, _) = connect_in(tmp.path()).unwrap();
        let server = listener.accept().unwrap();
        server
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let e = (&server).read(&mut [0u8; 8]).unwrap_err();
        assert!(matches!(
            e.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
        ));
    }
}
