//! ロックファイル `yunagi.lock`：データベースを開くプロセスを1つに限る
//! （docs/design/01-architecture.md 4章 2・4、docs/design/02-data-model.md 1.1）。
//!
//! - アプリは起動時に取り、終了まで持ち続ける（[`Database`] が持つ）。
//! - 直接モードの CLI・MCP は、操作ごとに取ってから開き、終わったら離す
//!   （[`Database::with_locked`]）。データベースの場所（保存先）は、ロックを取った後で求める
//!   （[`DbLock::database_path`]）。
//! - [`Database::open`] はロック（[`DbLock`]）を受け取らないと呼べない。
//!   これで、ロックを取らずにマイグレーションが走ることを型で防ぐ。パスも受け取らず、
//!   データベースの場所はロックから求める。
//! - ロックは既定ディレクトリ（[`DefaultDir`]）でしか取れない。[`DefaultDir`] は本番では
//!   [`super::data_dir::default_dir`] でしか作れないので、保存先を変えていても、アプリと
//!   CLI・MCP は同じロックファイルを取り合う（アーキテクチャ設計書 4.2・6章）。
//!
//! ロックには OS のファイルロック（Rust 標準の [`File::try_lock`]。Linux・macOS は `flock`、
//! Windows は `LockFileEx`）を使う。プロセスが異常終了しても OS がロックを離すので、
//! 取り残されたロックを手で消す必要がない。ロックファイル自体は消さない
//! （消すと、消す前のファイルを掴んだプロセスと、作り直したファイルを掴んだプロセスが、
//! 両方ともロックを取れてしまうため）。

use std::fmt;
use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use super::data_dir::{self, DataDirError, DefaultDir};
use super::{Database, OpenError, StorageError};

/// ロックファイルの名前。置き場所は既定ディレクトリ（アーキテクチャ設計書 6章。YUN-20）。
pub const LOCK_FILE_NAME: &str = "yunagi.lock";

/// ロックが取れないときに待つ既定の時間（アーキテクチャ設計書 4章 4 の「最大数秒」）。
/// アプリの起動時と、直接モードの CLI・MCP の操作の両方で使う。
pub const DEFAULT_LOCK_WAIT: Duration = Duration::from_secs(5);

/// 待つ間に、取り直しを試みる間隔。
const RETRY_INTERVAL: Duration = Duration::from_millis(50);

/// 取った `yunagi.lock` の排他ロック。手放す（drop する）とロックが離れる。
///
/// 開いているデータベース（[`Database`]）は、このロックを持つ。逆に、ロックが手元にある
/// （`Database` に渡していない）なら、このプロセスがコアを通して開いたデータベースは無い。保存先の変更
/// （[`super::data_dir::change_data_dir`]）は、これを使って「ロックを持ち、データベースを
/// 閉じている」ことを型で求める。
#[derive(Debug)]
pub struct DbLock {
    file: File,
    default_dir: DefaultDir,
}

impl DbLock {
    /// 既定ディレクトリ `default_dir` の `yunagi.lock` を開き（無ければ作り）、排他ロックを取る。
    ///
    /// 受け取るのは保存先（`data_dir`）ではなく、既定ディレクトリ（[`DefaultDir`]。本番では
    /// [`super::data_dir::default_dir`] で求める）だけ（アーキテクチャ設計書 4.2・6章）。
    /// 保存先を変えていても、アプリと CLI・MCP が同じファイルをロックするため。
    /// ファイル名はここで付ける。フォルダは呼び出し側が用意する。
    ///
    /// ほかが持っていれば、最大 `wait` の間、取り直しを試みる。それでも取れなければ
    /// [`LockError::Busy`] を返す。`wait` が 0 なら1回だけ試す。同じプロセスの中で
    /// 二重に取ろうとしても（ロックはファイルを開いたものごとに働くため）`Busy` になる。
    ///
    /// 呼んだスレッドを最大 `wait` 止める。画面を動かすスレッドや非同期の実行環境の中から
    /// 直接呼ばず、止めてよいスレッド（`spawn_blocking` など）で呼ぶ。
    pub fn acquire(default_dir: &DefaultDir, wait: Duration) -> Result<Self, LockError> {
        let path = default_dir.path().join(LOCK_FILE_NAME);
        let io_error = |source| LockError::Io {
            path: path.clone(),
            source,
        };
        // Windows の LockFileEx は読みか書きの権限で開いたファイルにしか使えないため、両方で開く。
        // 中身は使わないので切り詰めない
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        // 本人だけが読み書きできる権限で作る（docs/release/SECURITY.md）。ほかの利用者が
        // ファイルを開けると、ロックを取って本人を締め出せるため。既にあるファイルの権限は変えない
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let file = options.open(&path).map_err(io_error)?;
        // 待ち時間が大きすぎて期限を表せないとき（Duration::MAX など）は、期限なしで待つ
        let deadline = Instant::now().checked_add(wait);
        loop {
            match file.try_lock() {
                Ok(()) => {
                    return Ok(Self {
                        file,
                        default_dir: default_dir.clone(),
                    });
                }
                Err(TryLockError::WouldBlock) => {
                    let sleep = match deadline {
                        None => RETRY_INTERVAL,
                        Some(deadline) => {
                            let now = Instant::now();
                            if now >= deadline {
                                return Err(LockError::Busy { path, waited: wait });
                            }
                            RETRY_INTERVAL.min(deadline - now)
                        }
                    };
                    thread::sleep(sleep);
                }
                Err(TryLockError::Error(e)) => return Err(io_error(e)),
            }
        }
    }

    /// ロックを取った既定ディレクトリ。`config.json` もここにある。
    pub fn default_dir(&self) -> &DefaultDir {
        &self.default_dir
    }

    /// ロックを持ったまま、`config.json` から保存先を求め、そこの `yunagi.db` のパスを返す。
    ///
    /// データベースのパスは、ロックを取った後で求める（アーキテクチャ設計書 4.2）。取る前に
    /// 求めると、待っている間にアプリが保存先を移したとき、古い保存先を開いてしまうため。
    /// [`Database::open`] もこれと同じ決め方で場所を求める。
    pub fn database_path(&self) -> Result<PathBuf, DataDirError> {
        Ok(data_dir::database_path(&data_dir::resolve_data_dir(
            &self.default_dir,
        )?))
    }
}

impl Drop for DbLock {
    fn drop(&mut self) {
        // ファイルを閉じればロックは離れるが、Windows は閉じてから離すまでに時間がかかりうる
        // （LockFileEx の説明）ので、閉じる前に明示して離す。失敗しても閉じれば離れるので無視する
        let _ = self.file.unlock();
    }
}

/// ロックが取れなかった理由。
#[derive(Debug)]
pub enum LockError {
    /// ほか（ふつうは別のプロセス）がロックを持っていて、`waited` だけ待っても取れなかった。
    /// アプリの起動中なら CLI・MCP はローカル API を使う（アーキテクチャ設計書 4章 3）。
    Busy { path: PathBuf, waited: Duration },
    /// ロックファイルを開けない・ロックの操作に失敗した（フォルダが無い、権限が無いなど）。
    Io { path: PathBuf, source: io::Error },
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy { path, waited } if waited.is_zero() => write!(
                f,
                "the Yunagi database is already in use (could not lock {})",
                path.display()
            ),
            Self::Busy { path, waited } => write!(
                f,
                "the Yunagi database is already in use (could not lock {} within {} ms)",
                path.display(),
                waited.as_millis()
            ),
            Self::Io { path, source } => {
                write!(f, "could not lock {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for LockError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Busy { .. } => None,
            Self::Io { source, .. } => Some(source),
        }
    }
}

/// ロックを離さずに閉じようとして（[`Database::close_keeping_lock`]）、閉じるのに失敗した。
///
/// 接続が閉じたとは限らないので、ロックだけを返さず、データベース（`db`）ごと返す。
/// これで「ロックが手元にあれば、データベースは閉じている」が崩れない。呼び出し側は、
/// `db` でもう一度閉じるのを試すか、手放す（接続を閉じてからロックが離れる）。
pub struct CloseError {
    /// 閉じるのに失敗した理由。
    pub error: StorageError,
    /// 閉じられなかったデータベース。ロックも持ったまま。
    pub db: Database,
}

impl fmt::Debug for CloseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CloseError")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for CloseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "could not close the database: {}", self.error)
    }
}

impl std::error::Error for CloseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl Database {
    /// 接続を閉じ、ロックは離さずに返す。保存先の変更
    /// （[`super::data_dir::change_data_dir`]）の前に、アプリがデータベースを閉じるのに使う
    /// （閉じてからロックを取り直すと、その間に CLI・MCP がデータベースを開きうるため）。
    ///
    /// 閉じるのに失敗したら（WAL の書き戻しの失敗など）、閉じられなかったデータベースを
    /// [`CloseError`] で返す（ロックだけは返さない）。
    pub fn close_keeping_lock(self) -> Result<DbLock, Box<CloseError>> {
        let Self {
            conn,
            path,
            _lock: lock,
        } = self;
        match conn.close() {
            Ok(()) => Ok(lock),
            Err((conn, e)) => Err(Box::new(CloseError {
                error: StorageError::from(e),
                db: Database {
                    conn,
                    path,
                    _lock: lock,
                },
            })),
        }
    }

    /// 直接モードの CLI・MCP の1回の操作：既定ディレクトリ `default_dir` の `yunagi.lock` を
    /// 取ってから、保存先の `yunagi.db` を開き、`operation` を行い、終わったら閉じてロックを離す
    /// （アーキテクチャ設計書 4章 2）。
    ///
    /// 保存先は、ロックを取った後で `config.json` から求める（[`Database::open`]）。
    /// 開けなければ（`config.json` が壊れている、保存先にデータベースが無いなど）、
    /// ロックを離してから [`OpenError`] を返す。データベースが無くても、初めて使うとき
    /// 以外は作らない（[`OpenError::NotFound`]）。開く・マイグレーションの失敗は、
    /// [`OpenError::Storage`] に包まず [`StorageError`] のまま返す（ロックの失敗と同じ経路）。
    ///
    /// ロックが取れなければ、最大 `wait` 待ってから [`StorageError::Lock`] を返す。
    /// `operation` が失敗しても、閉じてロックを離してからその失敗を返す（閉じるのにも
    /// 失敗したときは、`operation` の失敗のほうを返す）。`operation` が成功して閉じるのに
    /// 失敗したら、閉じる失敗を返す。呼んだスレッドを止めうる点は
    /// [`DbLock::acquire`] と同じ。
    pub fn with_locked<T, E>(
        default_dir: &DefaultDir,
        wait: Duration,
        operation: impl FnOnce(&mut Database) -> Result<T, E>,
    ) -> Result<T, E>
    where
        E: From<StorageError> + From<OpenError>,
    {
        let lock = DbLock::acquire(default_dir, wait).map_err(StorageError::Lock)?;
        // ストレージの失敗（開く・マイグレーション）は、ロックの失敗と同じく StorageError で渡す
        let mut db = Database::open(lock).map_err(|e| match e {
            OpenError::Storage(e) => E::from(e),
            e => E::from(e),
        })?;
        let result = operation(&mut db);
        let closed = db.close();
        let value = result?;
        closed?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read};
    use std::process::{Child, Command, Stdio};

    use super::*;

    /// 子プロセスとして動くときに、既定ディレクトリとして使うフォルダを受け取る環境変数。
    const CHILD_LOCK_ENV: &str = "YUNAGI_TEST_LOCK_HOLDER";
    /// 子プロセスとして動くときに、データベースを開くかを受け取る環境変数（あれば開く）。
    const CHILD_DB_ENV: &str = "YUNAGI_TEST_LOCK_HOLDER_DB";
    /// 子プロセスがロックを取ったことを親に伝える目印。テストの実行ファイル自身の出力
    /// （「test ... ...」）と同じ行につながって出るので、行に含まれるかで見る。
    const READY: &str = "YUNAGI_TEST_LOCK_HOLDER_READY";
    /// 親が合図をしないまま終わっても子が残り続けないよう、子が自分から終わるまでの時間。
    const CHILD_MAX_LIFETIME: Duration = Duration::from_secs(60);

    /// 別のプロセスとして動く「ロックを持つ側」。
    ///
    /// ふつうのテストの実行では飛ばす（`#[ignore]`）。[`spawn_holder`] が
    /// このテストだけを、`--ignored` と環境変数を付けて別のプロセスで動かす。子はロックを取り
    /// （[`CHILD_DB_ENV`] があればデータベースを開き）、標準出力に [`READY`] を書き、標準入力が
    /// 閉じられるまで（最長 [`CHILD_MAX_LIFETIME`]）ロックを持ち続ける。
    #[test]
    #[ignore = "spawn_holder が子プロセスとして動かす"]
    fn lock_holder_child() {
        // 子として起動されたのでなければ（`--include-ignored` などで走ったとき）何もしない
        let Some(lock_dir) = std::env::var_os(CHILD_LOCK_ENV) else {
            return;
        };
        let lock = DbLock::acquire(&DefaultDir::for_test(lock_dir), Duration::ZERO)
            .expect("child: acquire lock");
        // データベースを開くときは Database がロックを持ち、開かないときはそのまま持つ
        let held: Box<dyn std::any::Any> = match std::env::var_os(CHILD_DB_ENV) {
            Some(_) => Box::new(Database::open(lock).expect("child: open database")),
            None => Box::new(lock),
        };
        println!("{READY}");
        // 親が標準入力を閉じる（EOF）まで待つ
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let mut rest = Vec::new();
            let _ = io::stdin().read_to_end(&mut rest);
            let _ = tx.send(());
        });
        let _ = rx.recv_timeout(CHILD_MAX_LIFETIME);
        drop(held);
    }

    /// ロックを持っている子プロセス。手放すと、子にロックを離して終わるよう伝え、終わるまで待つ
    /// （テストが途中で失敗したときも、子を残さない）。
    struct Holder(Child);

    impl Holder {
        /// 子にロックを離して終わるよう伝え、正常に終わったことを確かめる。
        fn release(mut self) {
            let status = self.finish();
            assert!(status.success(), "child failed: {status}");
        }

        fn finish(&mut self) -> std::process::ExitStatus {
            // 標準入力を閉じる（EOF）のが、子への「終わってよい」の合図
            drop(self.0.stdin.take());
            self.0.wait().unwrap()
        }
    }

    impl Drop for Holder {
        fn drop(&mut self) {
            self.finish();
        }
    }

    /// テストの実行ファイルをもう1つ起動し、ロックを持たせる（`open_db` なら、保存先の
    /// データベースも開かせる）。ロックを取るまで待って返す。
    fn spawn_holder(default_dir: &DefaultDir, open_db: bool) -> Holder {
        // テストの名前は、クレート名を除いたモジュールの場所＋関数名（名前を変えても追従する）
        let (_crate, module) = module_path!().split_once("::").unwrap();
        let test_name = format!("{module}::lock_holder_child");
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args([
            "--ignored",
            "--exact",
            &test_name,
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_LOCK_ENV, default_dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
        if open_db {
            cmd.env(CHILD_DB_ENV, "1");
        }
        let mut holder = Holder(cmd.spawn().unwrap());
        let mut stdout = BufReader::new(holder.0.stdout.take().unwrap());
        let mut line = String::new();
        // テストの実行ファイル自身も出力するので、READY を含む行まで読み飛ばす
        loop {
            line.clear();
            let n = stdout.read_line(&mut line).unwrap();
            assert_ne!(n, 0, "child exited before taking the lock");
            if line.contains(READY) {
                // 残りの出力も読み続ける。受け口を閉じると、子が最後の出力で失敗するため
                thread::spawn(move || io::copy(&mut stdout, &mut io::sink()));
                return holder;
            }
        }
    }

    /// `with_locked` に渡す操作の失敗。本番の CLI・MCP のエラーと同じく、ストレージと
    /// 保存先の両方の失敗を受け取れる。
    #[derive(Debug)]
    enum TestError {
        Storage(StorageError),
        Open(OpenError),
    }

    impl From<StorageError> for TestError {
        fn from(e: StorageError) -> Self {
            Self::Storage(e)
        }
    }

    impl From<OpenError> for TestError {
        fn from(e: OpenError) -> Self {
            Self::Open(e)
        }
    }

    /// 一時フォルダ `dir` を既定ディレクトリとして使う。
    fn dd(dir: &tempfile::TempDir) -> DefaultDir {
        DefaultDir::for_test(dir.path())
    }

    #[test]
    fn creates_lock_file_and_can_reacquire_after_release() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(LOCK_FILE_NAME);
        let lock = DbLock::acquire(&dd(&dir), Duration::ZERO).unwrap();
        assert!(file.exists());
        drop(lock);
        // 離した後は、また取れる。ロックファイルは消さない
        drop(DbLock::acquire(&dd(&dir), Duration::ZERO).unwrap());
        assert!(file.exists());
    }

    #[test]
    fn missing_folder_is_an_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = DbLock::acquire(
            &DefaultDir::for_test(dir.path().join("missing")),
            Duration::ZERO,
        )
        .unwrap_err();
        assert!(matches!(err, LockError::Io { .. }), "{err:?}");
    }

    #[cfg(unix)]
    #[test]
    fn lock_file_is_readable_only_by_the_owner() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        drop(DbLock::acquire(&dd(&dir), Duration::ZERO).unwrap());
        let mode = std::fs::metadata(dir.path().join(LOCK_FILE_NAME))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "mode {mode:o}");
    }

    #[test]
    fn a_huge_wait_does_not_overflow() {
        let dir = tempfile::tempdir().unwrap();
        drop(DbLock::acquire(&dd(&dir), Duration::MAX).unwrap());
    }

    #[test]
    fn a_second_lock_in_the_same_process_is_busy() {
        let dir = tempfile::tempdir().unwrap();
        let _held = DbLock::acquire(&dd(&dir), Duration::ZERO).unwrap();
        let err = DbLock::acquire(&dd(&dir), Duration::ZERO).unwrap_err();
        assert!(matches!(err, LockError::Busy { .. }), "{err:?}");
    }

    #[test]
    fn another_process_holding_the_lock_makes_acquire_fail_after_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let path = &dd(&dir);
        let child = spawn_holder(path, false);

        let wait = Duration::from_millis(300);
        let started = Instant::now();
        let err = DbLock::acquire(path, wait).unwrap_err();
        assert!(started.elapsed() >= wait, "gave up before waiting");
        match err {
            LockError::Busy { waited, .. } => assert_eq!(waited, wait),
            other => panic!("expected Busy, got {other:?}"),
        }

        child.release();
        // 子が離した後は取れる
        drop(DbLock::acquire(path, Duration::ZERO).unwrap());
    }

    #[test]
    fn acquire_succeeds_when_the_other_process_releases_while_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let path = &dd(&dir);
        let child = spawn_holder(path, false);

        // 待っている間に子がロックを離す
        let releaser = thread::spawn(move || {
            thread::sleep(Duration::from_millis(300));
            child.release();
        });
        let lock = DbLock::acquire(path, Duration::from_secs(30)).unwrap();
        releaser.join().unwrap();
        drop(lock);
    }

    #[test]
    fn database_cannot_be_opened_while_another_process_has_it_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = &dd(&dir);
        // 子がロックを取ってデータベースを開いている（アプリの起動中に当たる）
        let child = spawn_holder(path, true);

        let err = Database::with_locked(path, Duration::from_millis(100), |_db| {
            Ok::<_, TestError>(())
        })
        .unwrap_err();
        assert!(
            matches!(
                err,
                TestError::Storage(StorageError::Lock(LockError::Busy { .. }))
            ),
            "{err:?}"
        );

        child.release();
        // 子が閉じた後は開いて操作でき、操作が終わるとロックが離れている
        let mode = Database::with_locked(path, Duration::ZERO, |db| {
            db.connection()
                .pragma_query_value(None, "journal_mode", |r| r.get::<_, String>(0))
                .map_err(|e| TestError::Storage(e.into()))
        })
        .unwrap();
        assert_eq!(mode, "wal");
        drop(DbLock::acquire(path, Duration::ZERO).unwrap());
    }

    #[test]
    fn with_locked_releases_the_lock_when_the_operation_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = &dd(&dir);

        let err = Database::with_locked(path, Duration::ZERO, |_db| {
            Err::<(), _>(TestError::Storage(StorageError::WalUnavailable {
                mode: "test".into(),
            }))
        })
        .unwrap_err();
        assert!(
            matches!(err, TestError::Storage(StorageError::WalUnavailable { .. })),
            "{err:?}"
        );
        drop(DbLock::acquire(path, Duration::ZERO).unwrap());
    }

    #[test]
    fn with_locked_releases_the_lock_when_the_config_is_broken() {
        let dir = tempfile::tempdir().unwrap();
        let path = &dd(&dir);
        std::fs::write(
            dir.path().join(super::super::data_dir::CONFIG_FILE_NAME),
            "{ not json",
        )
        .unwrap();
        let err =
            Database::with_locked(path, Duration::ZERO, |_db| Ok::<_, TestError>(())).unwrap_err();
        assert!(
            matches!(
                err,
                TestError::Open(OpenError::DataDir(DataDirError::InvalidConfig { .. }))
            ),
            "{err:?}"
        );
        // データベースは作らず、ロックは離している
        assert!(!super::super::data_dir::database_path(dir.path()).exists());
        drop(DbLock::acquire(path, Duration::ZERO).unwrap());
    }

    #[test]
    fn with_locked_reports_storage_failures_as_storage_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = &dd(&dir);
        // 新しい版で作られたデータベース（開くと失敗する）
        rusqlite::Connection::open(super::super::data_dir::database_path(dir.path()))
            .unwrap()
            .pragma_update(None, "user_version", 9999)
            .unwrap();
        let err =
            Database::with_locked(path, Duration::ZERO, |_db| Ok::<_, TestError>(())).unwrap_err();
        assert!(
            matches!(
                err,
                TestError::Storage(StorageError::UnsupportedSchema { .. })
            ),
            "{err:?}"
        );
    }

    #[test]
    fn with_locked_does_not_create_a_database_in_a_data_dir_still_syncing() {
        let tmp = tempfile::tempdir().unwrap();
        let default = tmp.path().join("default");
        std::fs::create_dir(&default).unwrap();
        let default_dir = &DefaultDir::for_test(&default);
        // config.json は同期フォルダを指すが、データベースはまだ届いていない
        let sync = tmp.path().join("sync/Yunagi");
        std::fs::create_dir_all(&sync).unwrap();
        std::fs::write(
            default.join(super::super::data_dir::CONFIG_FILE_NAME),
            serde_json::json!({ "data_dir": sync }).to_string(),
        )
        .unwrap();

        let err = Database::with_locked(default_dir, Duration::ZERO, |_db| Ok::<_, TestError>(()))
            .unwrap_err();
        match err {
            TestError::Open(OpenError::NotFound(e)) => {
                assert_eq!(e.path, super::super::data_dir::database_path(&sync));
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
        // どこにもデータベースを作らず、ロックは離している
        assert_eq!(std::fs::read_dir(&sync).unwrap().count(), 0);
        assert!(!super::super::data_dir::database_path(&default).exists());
        drop(DbLock::acquire(default_dir, Duration::ZERO).unwrap());
    }

    #[test]
    fn app_and_cli_contend_for_the_same_lock_after_changing_the_data_dir() {
        use super::super::data_dir::{change_data_dir, database_path, resolve_data_dir};

        let tmp = tempfile::tempdir().unwrap();
        let default = tmp.path().join("default");
        std::fs::create_dir(&default).unwrap();
        let default_dir = &DefaultDir::for_test(&default);
        // 初めて使うときに既定ディレクトリに作ったデータベースを、ロックを離さずに閉じ、
        // 保存先を既定ディレクトリとは別のフォルダ（クラウド同期フォルダなど）に変える
        let db = Database::open(DbLock::acquire(default_dir, Duration::ZERO).unwrap()).unwrap();
        let lock = db.close_keeping_lock().unwrap();
        let data_dir = tmp.path().join("sync/Yunagi");
        change_data_dir(&lock, &data_dir).unwrap();
        drop(lock);
        assert_eq!(resolve_data_dir(default_dir).unwrap(), data_dir);
        let db_path = database_path(&data_dir);

        // アプリ（子プロセス）が、既定ディレクトリのロックを取って、新しい保存先のデータベースを開く
        let app = spawn_holder(default_dir, true);

        // CLI（このプロセス）も、同じ既定ディレクトリのロックを取ろうとし、止まる
        let err = Database::with_locked(default_dir, Duration::ZERO, |_db| Ok::<_, TestError>(()))
            .unwrap_err();
        assert!(
            matches!(
                err,
                TestError::Storage(StorageError::Lock(LockError::Busy { .. }))
            ),
            "{err:?}"
        );
        // ロックファイルは既定ディレクトリにだけあり、保存先には作られていない
        assert!(default.join(LOCK_FILE_NAME).exists());
        assert!(!data_dir.join(LOCK_FILE_NAME).exists());

        // アプリが閉じた後は、CLI が新しい保存先のデータベースを開ける
        app.release();
        let opened = Database::with_locked(default_dir, Duration::ZERO, |db| {
            Ok::<_, TestError>(db.connection().path().map(PathBuf::from))
        })
        .unwrap();
        assert_eq!(
            opened.map(|p| p.canonicalize().unwrap()),
            Some(db_path.canonicalize().unwrap())
        );
    }

    #[test]
    fn with_locked_opens_the_data_dir_recorded_after_waiting_for_the_lock() {
        use super::super::data_dir::{change_data_dir, database_path};

        let tmp = tempfile::tempdir().unwrap();
        let default = tmp.path().join("default");
        std::fs::create_dir(&default).unwrap();
        let default_dir = DefaultDir::for_test(&default);
        let new_dir = tmp.path().join("new");

        // アプリがデータベースを開いている
        let app_db =
            Database::open(DbLock::acquire(&default_dir, Duration::ZERO).unwrap()).unwrap();
        // CLI はロックを待つ
        let cli = {
            let default_dir = default_dir.clone();
            thread::spawn(move || {
                Database::with_locked(&default_dir, Duration::from_secs(10), |db| {
                    Ok::<_, TestError>(db.connection().path().map(PathBuf::from))
                })
                .unwrap()
            })
        };
        // CLI がロックを待つところまで進むのを待つ（時間で待つので、遅い環境では CLI が
        // 移した後に読み始めることもある。そのときも結果は同じなので、テストは失敗しない）
        thread::sleep(Duration::from_millis(300));
        // アプリがロックを持っているので、CLI は終わっていない（ロックを待っている）
        assert!(!cli.is_finished());
        // 待っている間に、アプリがロックを持ったまま閉じて、保存先を移す
        let lock = app_db.close_keeping_lock().unwrap();
        change_data_dir(&lock, &new_dir).unwrap();
        drop(lock);

        // CLI は、ロックを取った後に記録を読むので、新しい保存先を開く。古い保存先を開かない
        let opened = cli.join().unwrap();
        assert_eq!(
            opened.map(|p| p.canonicalize().unwrap()),
            Some(database_path(&new_dir).canonicalize().unwrap())
        );
        assert!(!database_path(&default).exists());
    }

    #[test]
    fn close_keeping_lock_keeps_others_out() {
        let dir = tempfile::tempdir().unwrap();
        let path = &dd(&dir);
        let db = Database::open(DbLock::acquire(path, Duration::ZERO).unwrap()).unwrap();
        let lock = db.close_keeping_lock().unwrap();
        assert_eq!(lock.default_dir(), path);
        // 閉じた後もロックは持ったまま
        let err = DbLock::acquire(path, Duration::ZERO).unwrap_err();
        assert!(matches!(err, LockError::Busy { .. }), "{err:?}");
        drop(lock);
        drop(DbLock::acquire(path, Duration::ZERO).unwrap());
    }
}
