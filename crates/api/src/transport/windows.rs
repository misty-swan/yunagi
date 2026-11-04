//! 名前付きパイプの通信路（Windows）。
//!
//! **実機では未確認**（Linux の開発機では型検査 `cargo check --target` までしかできない。確かめる項目は
//! docs/release/08-test-plan.md）。守り（docs/design/01-architecture.md 4.6）：
//!
//! - 権限を明示する。パイプの既定の権限は Everyone と匿名に読み取りを許すので、持ち主（作った利用者）
//!   だけに許す権限の記述（`D:P(A;;GA;;;OW)`）を付けて作る。
//! - リモートからの接続は断る（`PIPE_REJECT_REMOTE_CLIENTS`）。
//! - 名前の先取りを防ぐ。名前は起動のたびにランダムに決め、トークンのファイルに書く（前もって作って
//!   待ち構えられないようにする）。最初の実体は `FILE_FLAG_FIRST_PIPE_INSTANCE` で作り、同じ名前が
//!   すでにあれば失敗させて、待ち受けを始めずに止める。

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use windows::Win32::Foundation::{ERROR_PIPE_CONNECTED, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::IO::CancelIoEx;
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::core::PCWSTR;

use super::{ConnectError, ListenError, classify};
use crate::token::{self, ConnectionInfo};

/// 持ち主（作った利用者）だけに、すべてのアクセスを許す権限の記述。継承しない（`P`）。
const PIPE_SDDL: &str = "D:P(A;;GA;;;OW)";
/// 読み書きそれぞれの緩衝の大きさ（バイト）。
const BUFFER_SIZE: u32 = 64 * 1024;
/// 読みを中断されたときの Windows のエラー（`ERROR_OPERATION_ABORTED`）。
const ERROR_OPERATION_ABORTED: i32 = 995;

fn to_wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// `ConvertStringSecurityDescriptorToSecurityDescriptorW` が確保した権限の記述。
struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

// 作った後は読むだけで、確保した領域は手放すときに一度だけ解放する
unsafe impl Send for SecurityDescriptor {}
unsafe impl Sync for SecurityDescriptor {}

impl SecurityDescriptor {
    fn owner_only() -> io::Result<Self> {
        let sddl = to_wide(PIPE_SDDL);
        let mut sd = PSECURITY_DESCRIPTOR::default();
        // SAFETY: `sddl` は NUL で終わる。`sd` は書き込み先として有効
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
        }?;
        Ok(Self(sd))
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: 上で確保した領域を、一度だけ解放する
        unsafe {
            LocalFree(Some(HLOCAL(self.0.0)));
        }
    }
}

/// 待ち受け。作ると、パイプの名前を決め、トークンのファイル（名前を含む）を書いてから最初の実体を
/// 作る。手放すと、トークンのファイルを消す。
pub struct Listener {
    name: String,
    dir: PathBuf,
    sd: SecurityDescriptor,
    /// 書いたトークン。消すときに、自分のものかを確かめる。
    token: String,
    /// 次の接続を待つ実体（接続を渡した直後に作り直す）。
    pending: Mutex<Option<File>>,
}

impl std::fmt::Debug for Listener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listener")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl Listener {
    /// 既定ディレクトリ `dir` で待ち受けを始める。`token` とパイプの名前を、パイプを作るより先に書く。
    pub fn open(
        dir: &yunagi_core::storage::data_dir::DefaultDir,
        token: &str,
    ) -> Result<Self, ListenError> {
        Self::open_in(dir.path(), token)
    }

    pub(crate) fn open_in(dir: &Path, token: &str) -> Result<Self, ListenError> {
        let io = |action| move |source| ListenError::Io { action, source };
        let sd =
            SecurityDescriptor::owner_only().map_err(io("build the pipe security descriptor"))?;
        let name = format!(r"\\.\pipe\yunagi-{}", token::hex(&random_bytes()));
        token::write_in(
            dir,
            &ConnectionInfo {
                token: token.to_owned(),
                pipe_name: Some(name.clone()),
            },
        )
        .map_err(io("write the token file"))?;
        let listener = Self {
            name,
            dir: dir.to_owned(),
            sd,
            token: token.to_owned(),
            pending: Mutex::new(None),
        };
        // 最初の実体。同じ名前がすでにあれば失敗する（先取り）
        match listener.create_instance(true) {
            Ok(first) => {
                *listener
                    .pending
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = Some(first);
                Ok(listener)
            }
            Err(e) => {
                let taken = e.raw_os_error() == Some(5) || e.raw_os_error() == Some(231);
                let name = listener.name.clone();
                // Drop がトークンのファイルを消す
                drop(listener);
                Err(if taken {
                    ListenError::PipeNameTaken(name)
                } else {
                    ListenError::Io {
                        action: "create the named pipe",
                        source: e,
                    }
                })
            }
        }
    }

    fn create_instance(&self, first: bool) -> io::Result<File> {
        let name = to_wide(&self.name);
        let attributes = SECURITY_ATTRIBUTES {
            nLength: u32::try_from(std::mem::size_of::<SECURITY_ATTRIBUTES>())
                .expect("the size fits in u32"),
            lpSecurityDescriptor: self.sd.0.0,
            bInheritHandle: false.into(),
        };
        let mut open_mode = PIPE_ACCESS_DUPLEX;
        if first {
            open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        // SAFETY: `name` は NUL で終わり、`attributes` は呼び出しの間有効
        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(name.as_ptr()),
                FILE_FLAGS_AND_ATTRIBUTES(open_mode.0),
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                BUFFER_SIZE,
                BUFFER_SIZE,
                0,
                Some(&attributes),
            )
        };
        if handle.is_invalid() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: 作ったばかりの有効なハンドルを、ただ 1 つの持ち主（File）に渡す
        Ok(unsafe { File::from_raw_handle(handle.0) })
    }

    /// 次の接続を待つ（止まって待つ）。止めるときは [`Listener::wake`] で起こす。
    pub fn accept(&self) -> io::Result<Stream> {
        let taken = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let pipe = match taken {
            Some(pipe) => pipe,
            None => self.create_instance(false)?,
        };
        // SAFETY: `pipe` は有効なパイプのハンドルで、接続を待つ間も持っている
        let connected = unsafe { ConnectNamedPipe(HANDLE(pipe.as_raw_handle()), None) };
        if let Err(e) = connected {
            // クライアントが、待つより前につないでいた場合は成功
            if e.code() != ERROR_PIPE_CONNECTED.to_hresult() {
                return Err(e.into());
            }
        }
        // 渡した直後に次の実体を作る（その間につなぐクライアントは「ふさがっている」で試し直す）
        if let Ok(next) = self.create_instance(false) {
            *self.pending.lock().unwrap_or_else(PoisonError::into_inner) = Some(next);
        }
        Ok(Stream::new(pipe))
    }

    /// 止まって待っている [`Listener::accept`] を、自分に接続して起こす。
    pub fn wake(&self) {
        self.waker().wake();
    }

    /// [`Listener::wake`] だけができる道具（待ち受けを持つスレッドとは別のスレッドが持つ）。
    pub fn waker(&self) -> Waker {
        Waker(self.name.clone())
    }

    /// 新しいクライアントが「待ち受けが無い」と分かるようにする。パイプは実体を手放すと無くなる
    /// （ファイルは無い）ので、ここでは待ち構えている実体を手放す。何度呼んでもよい。
    pub fn unlink(&self) {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }
}

/// 待ち受けを起こす道具（[`Listener::waker`]）。
#[derive(Debug, Clone)]
pub struct Waker(String);

impl Waker {
    /// 止まって待っている [`Listener::accept`] を、自分に接続して起こす。
    pub fn wake(&self) {
        let _ = OpenOptions::new().read(true).write(true).open(&self.0);
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        token::remove_own_in(&self.dir, &self.token);
    }
}

fn random_bytes() -> [u8; 16] {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("failed to get random bytes from the OS");
    bytes
}

/// 読みの時間切れを起こす見張り（世代が変わっていなければ、期限に読みを中断する）。
#[derive(Debug, Default)]
struct Watch {
    generation: Mutex<u64>,
    changed: Condvar,
}

/// つながった相手。
#[derive(Debug)]
pub struct Stream {
    file: File,
    watch: Arc<Watch>,
    /// 見張りを置いてある読みの期限（[`Stream::set_read_deadline`]）。
    armed: Mutex<Option<std::time::Instant>>,
    /// 書きの待ちの上限（[`Stream::finish`] が相手の読みを待つ上限にも使う）。
    write_timeout: Mutex<Option<Duration>>,
}

impl Stream {
    fn new(file: File) -> Self {
        Self {
            file,
            watch: Arc::default(),
            armed: Mutex::new(None),
            write_timeout: Mutex::new(None),
        }
    }

    /// 読むときの待ちの上限（`None` で無制限）。超えると、読みは中断されて `TimedOut` で失敗する。
    /// 同期のパイプには時間切れが無いので、見張りのスレッドが期限に `CancelIoEx` で読みを中断する。
    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        *self.armed.lock().unwrap_or_else(PoisonError::into_inner) = None;
        self.arm(timeout)
    }

    fn arm(&self, timeout: Option<Duration>) -> io::Result<()> {
        let generation = {
            let mut g = self
                .watch
                .generation
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            *g += 1;
            self.watch.changed.notify_all();
            *g
        };
        let Some(timeout) = timeout else {
            return Ok(());
        };
        let watch = Arc::clone(&self.watch);
        let handle = self.file.as_raw_handle() as usize;
        std::thread::Builder::new()
            .name("yunagi-pipe-watch".into())
            .spawn(move || {
                let guard = watch
                    .generation
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                let (guard, _) = watch
                    .changed
                    .wait_timeout_while(guard, timeout, |g| *g == generation)
                    .unwrap_or_else(PoisonError::into_inner);
                if *guard == generation {
                    // 持ち主が手放す前に（`Drop`）世代を進めるので、ここでは有効なハンドル
                    cancel(handle);
                }
            })?;
        Ok(())
    }

    /// 次の読みを `deadline` までしか待たない（読みごとに呼んでよい）。期限は絶対の時刻なので、同じ
    /// 期限ならすでに見張りを置いてあり、置き直さない（読みのたびにスレッドを作らない）。期限を
    /// 過ぎていれば `TimedOut` で失敗する。
    pub fn set_read_deadline(&self, deadline: std::time::Instant) -> io::Result<()> {
        let mut armed = self.armed.lock().unwrap_or_else(PoisonError::into_inner);
        if *armed == Some(deadline) {
            return Ok(());
        }
        let left = deadline
            .checked_duration_since(std::time::Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "the read deadline passed"))?;
        self.arm(Some(left))?;
        *armed = Some(deadline);
        Ok(())
    }

    /// 書くときの待ちの上限。同期のパイプの書きにも時間切れが無いので、読みと同じ見張りを使う
    /// （[`Stream::set_read_timeout`]。同時に読みの見張りは置けないが、応答を書くときは読みを
    /// 待っていない）。応答の後の [`Stream::finish`] の待ちは、この見張りでは中断できないことがあり、
    /// 実機で確かめる（テスト計画書 4.5）。
    pub fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        *self
            .write_timeout
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = timeout;
        self.set_read_timeout(timeout)
    }

    /// 別のスレッドから、読み待ちを中断する道具。
    pub fn interrupter(&self) -> io::Result<Interrupter> {
        Ok(Interrupter {
            handle: self.file.as_raw_handle() as usize,
            watch: Arc::clone(&self.watch),
        })
    }

    /// 応答を書き終えたことを相手に伝える。相手が読み終えるまで待って（`FlushFileBuffers`）から
    /// 閉じないと、パイプの中の書いたものが捨てられる。
    ///
    /// 相手が読まずに黙っているとき、ここで止まり続けて接続の枠を塞がないよう、別のスレッドが
    /// （複製したハンドルで）待ち、こちらは書きの待ちの上限（[`Stream::set_write_timeout`]）までしか
    /// 待たない。上限を過ぎたら、こちらのハンドルを閉じて先へ進む（待っていたスレッドは、相手が
    /// 読むか閉じるまで残り、パイプの実体もそれまで残る）。
    pub fn finish(&self) {
        let timeout = (*self
            .write_timeout
            .lock()
            .unwrap_or_else(PoisonError::into_inner))
        .unwrap_or(Duration::from_secs(5));
        let Ok(dup) = self.file.try_clone() else {
            let _ = self.file.sync_all();
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("yunagi-pipe-flush".into())
            .spawn(move || {
                let _ = dup.sync_all();
                let _ = tx.send(());
            });
        if spawned.is_ok() {
            let _ = rx.recv_timeout(timeout);
        }
    }
}

fn cancel(handle: usize) {
    // SAFETY: 呼ぶのは、ハンドルの持ち主が生きている間だけ（`Watch` の世代の約束）。中断する
    // 読みが無ければ失敗するが、気にしない
    unsafe {
        let _ = CancelIoEx(HANDLE(handle as RawHandle), None);
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        // 見張りが、閉じたハンドルを中断しようとしないように、世代を進めてから閉じる
        let mut g = self
            .watch
            .generation
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *g += 1;
        self.watch.changed.notify_all();
    }
}

fn map_aborted(e: io::Error) -> io::Error {
    if e.raw_os_error() == Some(ERROR_OPERATION_ABORTED) {
        io::Error::new(io::ErrorKind::TimedOut, "the read was interrupted")
    } else {
        e
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (&self.file).read(buf).map_err(map_aborted)
    }
}

impl Read for &Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (&self.file).read(buf).map_err(map_aborted)
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&self.file).write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        (&self.file).flush()
    }
}

impl Write for &Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&self.file).write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        (&self.file).flush()
    }
}

/// 読み待ちを中断する道具（[`Stream::interrupter`]）。呼ぶのは、持ち主の [`Stream`] が生きている間だけ。
#[derive(Debug)]
pub struct Interrupter {
    handle: usize,
    watch: Arc<Watch>,
}

impl Interrupter {
    /// 読み待ちを中断する。読みは `TimedOut` で終わる。
    pub fn interrupt(&self) {
        // `Stream::drop` と同じ錠を持つ間は、ハンドルが閉じられない
        let guard = self
            .watch
            .generation
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        cancel(self.handle);
        drop(guard);
    }
}

/// 既定ディレクトリ `dir` のトークンのファイルを読み、書かれているパイプの名前に接続する。
/// Windows では名前がファイルにあるので、ファイルを先に読む（戻りの 2 つ目に入れて返す）。
/// ファイルが無ければ待ち受けが無い（`NoListener`）。
///
/// パイプを開く操作は待たずに返る（全部ふさがっていれば `ERROR_PIPE_BUSY` ですぐ失敗する）ので、接続の
/// 時間切れは使わない。開いた後、`hello`（トークンを含む）を送る前に、パイプの持ち主の SID が自分の SID と
/// 同じことを確かめる（違えば何も送らず、`Denied`。アーキテクチャ設計書 4.6。名前を先取りしたほかの
/// プロセスにトークンを渡さないため）。
pub fn connect(
    dir: &yunagi_core::storage::data_dir::DefaultDir,
    _timeout: Duration,
) -> Result<(Stream, Option<ConnectionInfo>), ConnectError> {
    connect_in(dir.path())
}

pub(crate) fn connect_in(dir: &Path) -> Result<(Stream, Option<ConnectionInfo>), ConnectError> {
    let info = token::read_in(dir).map_err(classify)?;
    let Some(name) = info.pipe_name.clone() else {
        return Err(ConnectError::NoListener);
    };
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(name)
        .map_err(classify)?;
    match owner_is_me(&file) {
        Ok(true) => Ok((Stream::new(file), Some(info))),
        Ok(false) => Err(ConnectError::Denied(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the pipe is owned by another user",
        ))),
        // 持ち主を確かめられないパイプには、トークンを渡さない（権限が無いのではなく、確かめる操作の
        // 失敗なので `denied` にはしない。クライアントは `unexpected` にする）
        Err(e) => Err(ConnectError::Io(e)),
    }
}

/// パイプ `file` の持ち主の SID が、この処理の利用者（プロセスのトークンのユーザー、またはトークンの
/// 既定の持ち主）の SID と同じか。サーバーはパイプに持ち主を明示しないので、持ち主は作ったプロセスの
/// トークンで決まる（ふつうはユーザー。管理者権限なら Administrators）。
fn owner_is_me(file: &File) -> io::Result<bool> {
    use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS};
    use windows::Win32::Security::Authorization::{GetSecurityInfo, SE_KERNEL_OBJECT};
    use windows::Win32::Security::{
        EqualSid, GetTokenInformation, OWNER_SECURITY_INFORMATION, PSID, TOKEN_OWNER, TOKEN_QUERY,
        TOKEN_USER, TokenOwner, TokenUser,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    // SAFETY: どの呼び出しにも、この関数の中で確保した有効な領域と、その大きさを渡す。トークンの
    // ハンドルと、権限の記述（持ち主の SID を指す）は、使い終わったら一度だけ解放する。持ち主の SID の
    // 比較は、権限の記述を解放する前に済ませる。
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).map_err(io::Error::from)?;
        let mine = (|| {
            // トークンの情報（大きさを尋ねてから読む。構造体の整列のために u64 の並びで確保する）
            let info = |class| -> io::Result<Vec<u64>> {
                let mut len = 0u32;
                let _ = GetTokenInformation(token, class, None, 0, &mut len);
                if len == 0 {
                    return Err(io::Error::last_os_error());
                }
                let mut buf = vec![0u64; (len as usize).div_ceil(8)];
                GetTokenInformation(token, class, Some(buf.as_mut_ptr().cast()), len, &mut len)
                    .map_err(io::Error::from)?;
                Ok(buf)
            };
            let user_buf = info(TokenUser)?;
            let owner_buf = info(TokenOwner)?;
            let user_sid = (*(user_buf.as_ptr().cast::<TOKEN_USER>())).User.Sid;
            // 管理者権限で作られたオブジェクトの持ち主は、ユーザーではなくトークンの既定の持ち主
            // （Administrators）になりうるので、これも自分として扱う
            let token_owner_sid = (*(owner_buf.as_ptr().cast::<TOKEN_OWNER>())).Owner;
            let mut owner = PSID::default();
            let mut sd = PSECURITY_DESCRIPTOR::default();
            let status = GetSecurityInfo(
                HANDLE(file.as_raw_handle()),
                SE_KERNEL_OBJECT,
                OWNER_SECURITY_INFORMATION,
                Some(&mut owner),
                None,
                None,
                None,
                Some(&mut sd),
            );
            if status != ERROR_SUCCESS {
                return Err(io::Error::from_raw_os_error(status.0 as i32));
            }
            let same =
                EqualSid(user_sid, owner).is_ok() || EqualSid(token_owner_sid, owner).is_ok();
            LocalFree(Some(HLOCAL(sd.0)));
            Ok(same)
        })();
        let _ = CloseHandle(token);
        mine
    }
}
