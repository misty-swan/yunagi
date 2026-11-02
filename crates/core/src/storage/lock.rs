//! ロックファイル `yunagi.lock`：データベースを開くプロセスを1つに限る
//! （docs/design/01-architecture.md 4章 2・4、docs/design/02-data-model.md 1.1）。
//!
//! - アプリは起動時に取り、終了まで持ち続ける（[`Database`] が持つ）。
//! - 直接モードの CLI・MCP は、操作ごとに取ってから開き、終わったら離す
//!   （[`Database::with_locked`]）。
//! - [`Database::open`] はロック（[`DbLock`]）を受け取らないと呼べない。
//!   これで、ロックを取らずにマイグレーションが走ることを型で防ぐ。
//!
//! ロックには OS のファイルロック（Rust 標準の [`File::try_lock`]。Linux・macOS は `flock`、
//! Windows は `LockFileEx`）を使う。プロセスが異常終了しても OS がロックを離すので、
//! 取り残されたロックを手で消す必要がない。ロックファイル自体は消さない
//! （消すと、消す前のファイルを掴んだプロセスと、作り直したファイルを掴んだプロセスが、
//! 両方ともロックを取れてしまうため）。

use std::fmt;
use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use super::{Database, StorageError};

/// ロックファイルの名前。置き場所は既定ディレクトリ（アーキテクチャ設計書 6章。YUN-20）。
pub const LOCK_FILE_NAME: &str = "yunagi.lock";

/// ロックが取れないときに待つ既定の時間（アーキテクチャ設計書 4章 4 の「最大数秒」）。
/// アプリの起動時と、直接モードの CLI・MCP の操作の両方で使う。
pub const DEFAULT_LOCK_WAIT: Duration = Duration::from_secs(5);

/// 待つ間に、取り直しを試みる間隔。
const RETRY_INTERVAL: Duration = Duration::from_millis(50);

/// 取った `yunagi.lock` の排他ロック。手放す（drop する）とロックが離れる。
#[derive(Debug)]
pub struct DbLock {
    file: File,
    path: PathBuf,
}

impl DbLock {
    /// `path` のロックファイルを開き（無ければ作り）、排他ロックを取る。
    ///
    /// ほかのプロセスが持っていれば、最大 `wait` の間、取り直しを試みる。
    /// それでも取れなければ [`LockError::Busy`] を返す。`wait` が 0 なら1回だけ試す。
    ///
    /// `path` のフォルダは呼び出し側が用意する。
    pub fn acquire(path: impl AsRef<Path>, wait: Duration) -> Result<Self, LockError> {
        let path = path.as_ref();
        let io_error = |source| LockError::Io {
            path: path.to_path_buf(),
            source,
        };
        // Windows の LockFileEx は読みか書きの権限で開いたファイルにしか使えないため、両方で開く。
        // 中身は使わないので切り詰めない
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(io_error)?;
        let deadline = Instant::now() + wait;
        loop {
            match file.try_lock() {
                Ok(()) => {
                    return Ok(Self {
                        file,
                        path: path.to_path_buf(),
                    });
                }
                Err(TryLockError::WouldBlock) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(LockError::Busy {
                            path: path.to_path_buf(),
                            waited: wait,
                        });
                    }
                    thread::sleep(RETRY_INTERVAL.min(deadline - now));
                }
                Err(TryLockError::Error(e)) => return Err(io_error(e)),
            }
        }
    }

    /// ロックファイルの場所。
    pub fn path(&self) -> &Path {
        &self.path
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
    /// ほかのプロセスがロックを持っていて、`waited` だけ待っても取れなかった。
    /// アプリの起動中なら CLI・MCP はローカル API を使う（アーキテクチャ設計書 4章 3）。
    Busy { path: PathBuf, waited: Duration },
    /// ロックファイルを開けない・ロックの操作に失敗した（フォルダが無い、権限が無いなど）。
    Io { path: PathBuf, source: io::Error },
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy { path, waited } => write!(
                f,
                "another Yunagi process is using the database (could not lock {} within {} ms)",
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

impl Database {
    /// 直接モードの CLI・MCP の1回の操作：`lock_path` のロックを取ってから `db_path` を開き、
    /// `operation` を行い、終わったら閉じてロックを離す（アーキテクチャ設計書 4章 2）。
    ///
    /// ロックが取れなければ、最大 `wait` 待ってから [`StorageError::Lock`] を返す。
    /// `operation` が失敗しても、閉じてロックを離してからその失敗を返す。
    pub fn with_locked<T, E>(
        lock_path: impl AsRef<Path>,
        db_path: impl AsRef<Path>,
        wait: Duration,
        operation: impl FnOnce(&mut Database) -> Result<T, E>,
    ) -> Result<T, E>
    where
        E: From<StorageError>,
    {
        let lock = DbLock::acquire(lock_path, wait).map_err(StorageError::Lock)?;
        let mut db = Database::open(lock, db_path)?;
        // db は関数の終わりで閉じ、持っているロックもそこで離れる
        operation(&mut db)
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read};
    use std::process::{Child, Command, Stdio};

    use super::*;

    /// 子プロセスとして動くときに、ロックファイルの場所を受け取る環境変数。
    const CHILD_LOCK_ENV: &str = "YUNAGI_TEST_LOCK_HOLDER";
    /// 子プロセスとして動くときに、データベースの場所を受け取る環境変数（あれば開く）。
    const CHILD_DB_ENV: &str = "YUNAGI_TEST_LOCK_HOLDER_DB";
    /// 子プロセスがロックを取ったことを親に伝える目印。テストの実行ファイル自身の出力
    /// （「test ... ...」）と同じ行につながって出るので、行に含まれるかで見る。
    const READY: &str = "YUNAGI_TEST_LOCK_HOLDER_READY";
    /// 親が合図をしないまま終わっても子が残り続けないよう、子が自分から終わるまでの時間。
    const CHILD_MAX_LIFETIME: Duration = Duration::from_secs(60);

    /// 別のプロセスとして動く「ロックを持つ側」。
    ///
    /// ふつうのテストの実行では、環境変数が無いので何もしない。[`spawn_holder`] が
    /// このテストだけを、環境変数を付けて別のプロセスで動かす。子はロックを取り
    /// （データベースの場所があれば開き）、標準出力に [`READY`] を書き、標準入力が
    /// 閉じられるまで（最長 [`CHILD_MAX_LIFETIME`]）ロックを持ち続ける。
    #[test]
    fn lock_holder_child() {
        let Some(lock_path) = std::env::var_os(CHILD_LOCK_ENV) else {
            return;
        };
        let lock = DbLock::acquire(&lock_path, Duration::ZERO).expect("child: acquire lock");
        // データベースを開くときは Database がロックを持ち、開かないときはそのまま持つ
        let held: (Option<DbLock>, Option<Database>) = match std::env::var_os(CHILD_DB_ENV) {
            Some(db_path) => (
                None,
                Some(Database::open(lock, db_path).expect("child: open database")),
            ),
            None => (Some(lock), None),
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

    /// テストの実行ファイルをもう1つ起動し、ロックを持たせる。ロックを取るまで待って返す。
    fn spawn_holder(lock_path: &Path, db_path: Option<&Path>) -> Holder {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args([
            "--exact",
            "storage::lock::tests::lock_holder_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_LOCK_ENV, lock_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
        if let Some(db_path) = db_path {
            cmd.env(CHILD_DB_ENV, db_path);
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

    fn lock_path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join(LOCK_FILE_NAME)
    }

    #[test]
    fn creates_lock_file_and_can_reacquire_after_release() {
        let dir = tempfile::tempdir().unwrap();
        let path = lock_path(&dir);
        let lock = DbLock::acquire(&path, Duration::ZERO).unwrap();
        assert!(path.exists());
        assert_eq!(lock.path(), path);
        drop(lock);
        // 離した後は、また取れる。ロックファイルは消さない
        drop(DbLock::acquire(&path, Duration::ZERO).unwrap());
        assert!(path.exists());
    }

    #[test]
    fn missing_folder_is_an_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join(LOCK_FILE_NAME);
        let err = DbLock::acquire(&path, Duration::ZERO).unwrap_err();
        assert!(matches!(err, LockError::Io { .. }), "{err:?}");
    }

    #[test]
    fn another_process_holding_the_lock_makes_acquire_fail_after_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let path = lock_path(&dir);
        let child = spawn_holder(&path, None);

        let wait = Duration::from_millis(300);
        let started = Instant::now();
        let err = DbLock::acquire(&path, wait).unwrap_err();
        assert!(started.elapsed() >= wait, "gave up before waiting");
        match err {
            LockError::Busy { waited, .. } => assert_eq!(waited, wait),
            other => panic!("expected Busy, got {other:?}"),
        }

        child.release();
        // 子が離した後は取れる
        drop(DbLock::acquire(&path, Duration::ZERO).unwrap());
    }

    #[test]
    fn acquire_succeeds_when_the_other_process_releases_while_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let path = lock_path(&dir);
        let child = spawn_holder(&path, None);

        // 待っている間に子がロックを離す
        let releaser = thread::spawn(move || {
            thread::sleep(Duration::from_millis(300));
            child.release();
        });
        let lock = DbLock::acquire(&path, Duration::from_secs(30)).unwrap();
        releaser.join().unwrap();
        drop(lock);
    }

    #[test]
    fn database_cannot_be_opened_while_another_process_has_it_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = lock_path(&dir);
        let db_path = dir.path().join("yunagi.db");
        // 子がロックを取ってデータベースを開いている（アプリの起動中に当たる）
        let child = spawn_holder(&path, Some(&db_path));

        let err = Database::with_locked(&path, &db_path, Duration::from_millis(100), |_db| {
            Ok::<_, StorageError>(())
        })
        .unwrap_err();
        assert!(
            matches!(err, StorageError::Lock(LockError::Busy { .. })),
            "{err:?}"
        );

        child.release();
        // 子が閉じた後は開いて操作でき、操作が終わるとロックが離れている
        let mode = Database::with_locked(&path, &db_path, Duration::ZERO, |db| {
            db.connection()
                .pragma_query_value(None, "journal_mode", |r| r.get::<_, String>(0))
                .map_err(StorageError::from)
        })
        .unwrap();
        assert_eq!(mode, "wal");
        drop(DbLock::acquire(&path, Duration::ZERO).unwrap());
    }

    #[test]
    fn with_locked_releases_the_lock_when_the_operation_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = lock_path(&dir);
        let db_path = dir.path().join("yunagi.db");

        let err = Database::with_locked(&path, &db_path, Duration::ZERO, |_db| {
            Err::<(), _>(StorageError::WalUnavailable {
                mode: "test".into(),
            })
        })
        .unwrap_err();
        assert!(matches!(err, StorageError::WalUnavailable { .. }));
        drop(DbLock::acquire(&path, Duration::ZERO).unwrap());
    }
}
