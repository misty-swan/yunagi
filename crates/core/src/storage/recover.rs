//! 開けないデータベースを、バックアップで置き換えて開く（docs/design/01-architecture.md 6.2
//! 「開けないデータベースからの回復」・6.3）。
//!
//! [`Database::restore_backup`] は開いたデータベースにしか行えない。壊れていて開けない
//! （マイグレーション前のバックアップの整合性が通らない、SQLite のデータベースとして読めない）とき、
//! または保存先にデータベースが無いときに、選んだバックアップから戻すのに使う。
//!
//! - 今の `yunagi.db` と横のファイルは消さず、`backups/` に `damaged-<UNIX 時刻（秒）>.db`
//!   （横のファイルは同じ名前に `-wal` などを付けたもの）として退避する。バックアップの名前の形に
//!   合わないので、一覧に並ばず、古いものとして消されることもない。
//! - 選んだバックアップは、復元と同じ確認（[`Database::restore_backup`]）を通してから写す。
//! - 途中で失敗したら、退避したものを元の場所に戻してからエラーを返す。

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::backup::{self, BackupError};
use super::data_dir::{self, DataDirError};
use super::db_copy::{self, CopyError};
use super::migration::{self, Migration};
use super::{Database, DbLock, StorageError, db_files};
use crate::error::ErrorChain;

/// 退避したデータベースの名前の頭（`backups/damaged-<秒>.db`）。
const DAMAGED_PREFIX: &str = "damaged-";

/// [`Database::open_from_backup`] の結果。
pub struct Recovered {
    /// 戻して開いたデータベース。
    pub database: Database,
    /// 退避した、それまでのデータベース（`backups/damaged-<秒>.db`。横のファイルは、この名前に
    /// `-wal` などを付けたもの）。退避するものが無かった（データベースが無かった）ときは `None`。
    /// 本体が無く横のファイルだけを退避したときも、この名前を返す（本体のファイルは無い）。
    pub set_aside: Option<PathBuf>,
}

impl Database {
    /// 保存先の `yunagi.db` を、バックアップ `backup` の内容で置き換えてから開く。
    ///
    /// 開けないデータベース（壊れている、SQLite のデータベースとして読めない）や、見つからない
    /// データベース（[`super::OpenError::NotFound`]）から、利用者が選んだバックアップで回復するのに
    /// 使う（アーキテクチャ設計書 6.2・6.3）。開けるデータベースへの復元は、退避と取り消しのできる
    /// [`Database::restore_backup`] を使う。
    ///
    /// 1. 保存先は、[`Database::open`] と同じくロック `lock` の `config.json` から求める。保存先の
    ///    フォルダが無ければ [`RecoverError::NoDataDir`]。`yunagi.db` がシンボリックリンクなら
    ///    [`RecoverError::Symlink`]（リンクだけを動かすと、リンク先のデータベースと横のファイルが
    ///    離れるため）。
    /// 2. `backup` を、[`Database::restore_backup`] と同じく確かめる（SQLite として読めて整合性があり、
    ///    スキーマの番号が 1 以上でアプリの知る範囲にある）。通らなければ [`RecoverError::Backup`]。
    ///    ここまでは何も変えない。
    /// 3. 確かめたのと同じ接続から、保存先の一時ファイルに写す。
    /// 4. 今の `yunagi.db` と横のファイルを `backups/damaged-<秒>.db`（と横のファイル）へ退避し、
    ///    写したものを `yunagi.db` に置く。
    /// 5. ふつうに開く（スキーマが古ければマイグレーションを適用する。そのときマイグレーションが作る
    ///    バックアップは、選んだバックアップと同じ内容なので消す）。
    ///
    /// 3〜5 で失敗したら、置いたものを消し、退避したものを元の場所に戻してからエラーを返す。戻すのにも
    /// 失敗したら [`RecoverError::PutBackFailed`]（退避したものは `backups/` に残る）。
    pub fn open_from_backup(lock: DbLock, backup: &Path) -> Result<Recovered, RecoverError> {
        Self::open_from_backup_with(lock, backup, migration::MIGRATIONS)
    }

    /// マイグレーションの一覧を指定して [`Database::open_from_backup`] を行う。テストで失敗を
    /// 模擬するために分けている。
    fn open_from_backup_with(
        lock: DbLock,
        backup: &Path,
        migrations: &[Migration],
    ) -> Result<Recovered, RecoverError> {
        let data_dir = data_dir::resolve_data_dir(lock.default_dir())?;
        let path = data_dir::database_path(&data_dir);
        if !data_dir.is_dir() {
            return Err(RecoverError::NoDataDir(data_dir));
        }
        if fs::symlink_metadata(&path).is_ok_and(|m| m.is_symlink()) {
            return Err(RecoverError::Symlink(path));
        }
        let src = backup::open_restorable(backup, &path, migrations)?;
        let backups_dir = data_dir::backups_dir(&data_dir);
        fs::create_dir_all(&backups_dir)
            .map_err(|e| RecoverError::io("create directory", &backups_dir, e))?;

        // 写したものを置くまで、今のデータベースには触れない
        let tmp = free_name(&data_dir, "yunagi-restore-", db_files::TEMP_SUFFIX);
        db_copy::copy(&src, &tmp).map_err(|e| RecoverError::from_copy(e, backup))?;
        drop(src);

        let set_aside = match SetAside::run(&path, &backups_dir) {
            Ok(s) => s,
            Err(e) => {
                db_files::remove_with_side_files(&tmp);
                return Err(e);
            }
        };
        if let Err(e) = fs::rename(&tmp, &path) {
            db_files::remove_with_side_files(&tmp);
            return Err(set_aside.put_back(RecoverError::io("rename", &tmp, e)));
        }
        let _ = db_copy::sync_dir(&data_dir);

        let pre_migration_before = backup::pre_migration_paths(&backups_dir);
        let result = super::connect(&path, migrations, false);
        // 古いスキーマから戻すと、マイグレーションが戻したものの写しを作る。選んだバックアップと
        // 同じ内容なので消す（旧版の再導入に備えた、本来のマイグレーション前のものを押し出さない）
        for p in backup::pre_migration_paths(&backups_dir) {
            if !pre_migration_before.contains(&p) {
                let _ = fs::remove_file(&p);
            }
        }
        match result {
            Ok(conn) => Ok(Recovered {
                database: Database {
                    conn,
                    path,
                    _lock: lock,
                },
                set_aside: set_aside.name,
            }),
            Err(e) => {
                // 置いたもの（とその横のファイル）を消してから、退避したものを戻す。ロックは
                // まだ持っているので、その間にほかのプロセスが開くことはない
                db_files::remove_with_side_files(&path);
                Err(set_aside.put_back(RecoverError::Open(e)))
            }
        }
    }
}

/// `dir` の中で、`<prefix><秒>[-<番号>]<suffix>` の形の、横のファイルまで含めて空いている名前。
fn free_name(dir: &Path, prefix: &str, suffix: &str) -> PathBuf {
    let stem = format!("{prefix}{}", backup::now_secs());
    let mut path = dir.join(format!("{stem}{suffix}"));
    let mut n = 1;
    while db_files::any_exists(&path) {
        n += 1;
        path = dir.join(format!("{stem}-{n}{suffix}"));
    }
    path
}

/// 退避した、それまでのデータベースのファイル。
struct SetAside {
    /// 退避した名前（本体の名前。[`Recovered::set_aside`]）。何も退避しなかったら `None`。
    name: Option<PathBuf>,
    /// 動かしたもの（元の場所, 退避した場所）。動かした順。
    moved: Vec<(PathBuf, PathBuf)>,
}

impl SetAside {
    /// `db` と横のファイルのうち、あるもの（シンボリックリンクは辿らない）を `backups_dir` へ動かす。
    /// 途中で失敗したら、動かした分を戻してからエラーを返す。
    fn run(db: &Path, backups_dir: &Path) -> Result<Self, RecoverError> {
        let present: Vec<PathBuf> = db_files::with_side_files(db)
            .into_iter()
            .filter(|p| {
                !matches!(fs::symlink_metadata(p), Err(e) if e.kind() == io::ErrorKind::NotFound)
            })
            .collect();
        let mut set_aside = Self {
            name: None,
            moved: Vec::new(),
        };
        if present.is_empty() {
            return Ok(set_aside);
        }
        let name = free_name(backups_dir, DAMAGED_PREFIX, ".db");
        for from in present {
            // 横のファイルは、退避した名前に同じ接尾辞を付ける（SQLite が組にして読めるように）
            let suffix = from
                .to_string_lossy()
                .strip_prefix(&*db.to_string_lossy())
                .unwrap_or_default()
                .to_owned();
            let to = PathBuf::from(format!("{}{suffix}", name.display()));
            if let Err(e) = fs::rename(&from, &to) {
                set_aside.name = Some(name);
                return Err(set_aside.put_back(RecoverError::io("move", &from, e)));
            }
            set_aside.moved.push((from, to));
        }
        let _ = db_copy::sync_dir(backups_dir);
        set_aside.name = Some(name);
        Ok(set_aside)
    }

    /// 動かしたものを元の場所に戻し、`error` を返す。戻すのに失敗したら
    /// [`RecoverError::PutBackFailed`] にして返す。
    fn put_back(self, error: RecoverError) -> RecoverError {
        for (from, to) in self.moved.iter().rev() {
            if let Err(e) = fs::rename(to, from) {
                return RecoverError::PutBackFailed {
                    source: Box::new(error),
                    put_back_error: io::Error::new(
                        e.kind(),
                        format!("could not move {} back: {e}", to.display()),
                    ),
                    set_aside: self.name.unwrap_or_else(|| to.clone()),
                };
            }
        }
        error
    }
}

/// [`Database::open_from_backup`] が失敗した理由。
#[derive(Debug)]
pub enum RecoverError {
    /// `config.json` から保存先を求められなかった（壊れている、読めないなど）。何も変えていない。
    DataDir(DataDirError),
    /// 保存先のフォルダが無い。何も変えていない。
    NoDataDir(PathBuf),
    /// `yunagi.db` がシンボリックリンク。何も変えていない。
    Symlink(PathBuf),
    /// 選んだバックアップを使えない（読めない、整合性の確認が通らない、Yunagi のデータベースでない、
    /// スキーマの番号を知らない（[`StorageError::UnsupportedSchema`] を包む））。何も変えていない。
    Backup(BackupError),
    /// ファイルの操作が失敗した。`action` は何をしようとしたか。退避したものは元に戻した。
    Io {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    /// 戻したデータベースを開けなかった（マイグレーションの失敗など）。退避したものは元に戻した。
    Open(StorageError),
    /// 失敗し（`source`）、退避したものを元の場所に戻すことにも失敗した。退避したものは
    /// `set_aside`（とその横のファイル）に残っている。
    PutBackFailed {
        source: Box<RecoverError>,
        put_back_error: io::Error,
        set_aside: PathBuf,
    },
}

impl RecoverError {
    fn io(action: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            action,
            path: path.to_path_buf(),
            source,
        }
    }

    /// バックアップ `backup` を写すのに失敗したとき。写しの整合性が通らなければ、バックアップが
    /// 壊れている。
    fn from_copy(e: CopyError, backup: &Path) -> Self {
        match e {
            CopyError::Sqlite(e) => Self::Backup(BackupError::Sqlite(e)),
            CopyError::Corrupted { result, .. } => Self::Backup(BackupError::Corrupted {
                path: backup.to_path_buf(),
                result,
            }),
            CopyError::Io {
                action,
                path,
                source,
            } => Self::Io {
                action,
                path,
                source,
            },
        }
    }
}

impl fmt::Display for RecoverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 原因のエラーの文は含めない（source() で返す。README「エラーの表示」）
        match self {
            // 包むだけなので、中身をそのまま見せる（透過）
            Self::DataDir(e) => write!(f, "{e}"),
            Self::Backup(e) => write!(f, "{e}"),
            Self::Open(e) => write!(f, "{e}"),
            Self::NoDataDir(path) => {
                write!(f, "the data directory {} does not exist", path.display())
            }
            Self::Symlink(path) => write!(
                f,
                "{} is a symbolic link, so it was not replaced",
                path.display()
            ),
            Self::Io { action, path, .. } => write!(f, "could not {action} {}", path.display()),
            // 原因が2つある。source() は戻す処理の失敗を返すので、元に戻す失敗はその原因まで
            // ここに書き、戻す処理の失敗を最後に書く
            Self::PutBackFailed {
                put_back_error,
                set_aside,
                ..
            } => write!(
                f,
                "putting the previous database back failed ({}), so it is left at {}; restoring from the backup failed",
                ErrorChain(put_back_error),
                set_aside.display()
            ),
        }
    }
}

impl std::error::Error for RecoverError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            // Display と同じく透過。中身の原因を返す
            Self::DataDir(e) => e.source(),
            Self::Backup(e) => e.source(),
            Self::Open(e) => e.source(),
            Self::Io { source, .. } => Some(source),
            Self::PutBackFailed { source, .. } => Some(source.as_ref()),
            Self::NoDataDir(_) | Self::Symlink(_) => None,
        }
    }
}

impl From<DataDirError> for RecoverError {
    fn from(e: DataDirError) -> Self {
        Self::DataDir(e)
    }
}

impl From<BackupError> for RecoverError {
    fn from(e: BackupError) -> Self {
        Self::Backup(e)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rusqlite::Connection;

    use super::super::data_dir::{CONFIG_FILE_NAME, DefaultDir};
    use super::super::test_util::{corrupt_pages, names};
    use super::*;

    /// 一時フォルダの既定ディレクトリ（そのまま保存先）と、バックアップを作る別のフォルダ。
    struct Env {
        _tmp: tempfile::TempDir,
        dir: PathBuf,
        other: PathBuf,
        dd: DefaultDir,
    }

    impl Env {
        fn new() -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let dir = tmp.path().join("data");
            let other = tmp.path().join("other");
            fs::create_dir(&dir).unwrap();
            fs::create_dir(&other).unwrap();
            Self {
                dd: DefaultDir::for_test(&dir),
                _tmp: tmp,
                dir,
                other,
            }
        }

        fn lock(&self) -> DbLock {
            DbLock::acquire(&self.dd, Duration::ZERO).unwrap()
        }

        fn db(&self) -> PathBuf {
            data_dir::database_path(&self.dir)
        }

        fn backups(&self) -> PathBuf {
            data_dir::backups_dir(&self.dir)
        }

        /// `memo` の値 `value` を持つ、`migrations` を適用したデータベースを、`backups/` に
        /// 定期バックアップの名前で置く。
        fn backup(&self, value: &str, migrations: &[Migration]) -> PathBuf {
            let src = self.other.join(format!("{value}.db"));
            let mut conn = Connection::open(&src).unwrap();
            migration::migrate(&mut conn, &self.other.join("b"), migrations).unwrap();
            conn.execute("INSERT INTO memo (body) VALUES (?1)", [value])
                .unwrap();
            fs::create_dir_all(self.backups()).unwrap();
            let to = self.backups().join(format!("daily-2026-11-01-{}.db", 100));
            db_copy::copy(&conn, &to).unwrap();
            to
        }

        fn recover(
            &self,
            backup: &Path,
            migrations: &[Migration],
        ) -> Result<Recovered, RecoverError> {
            Database::open_from_backup_with(self.lock(), backup, migrations)
        }
    }

    const V1: Migration = Migration {
        version: 1,
        name: "create memo",
        sql: "CREATE TABLE memo (id INTEGER PRIMARY KEY, body TEXT NOT NULL);",
    };
    const V2: Migration = Migration {
        version: 2,
        name: "add memo.done",
        sql: "ALTER TABLE memo ADD COLUMN done INTEGER NOT NULL DEFAULT 0;",
    };
    const V2_BROKEN: Migration = Migration {
        version: 2,
        name: "broken",
        sql: "ALTER TABLE missing ADD COLUMN x INTEGER;",
    };

    fn memo(db: &Database) -> Vec<String> {
        let mut stmt = db
            .connection()
            .prepare("SELECT body FROM memo ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    /// SQLite のデータベースとして読めない `yunagi.db` と、中身のある `-wal` を置く。
    fn damage(env: &Env) -> (Vec<u8>, Vec<u8>) {
        let db = b"this is not a database".to_vec();
        let wal = b"some wal".to_vec();
        fs::write(env.db(), &db).unwrap();
        fs::write(format!("{}-wal", env.db().display()), &wal).unwrap();
        (db, wal)
    }

    #[test]
    fn replaces_a_damaged_database_and_keeps_it_in_backups() {
        let env = Env::new();
        let backup = env.backup("from backup", &[V1]);
        let (db_bytes, wal_bytes) = damage(&env);

        let recovered = env.recover(&backup, &[V1]).unwrap();
        assert_eq!(memo(&recovered.database), ["from backup"]);
        assert_eq!(recovered.database.path(), env.db());

        // 壊れたものは消さずに、横のファイルと組のまま backups/ に退避している
        let set_aside = recovered.set_aside.clone().unwrap();
        assert_eq!(set_aside.parent().unwrap(), env.backups());
        let name = set_aside.file_name().unwrap().to_str().unwrap();
        assert!(
            name.starts_with("damaged-") && name.ends_with(".db"),
            "{name}"
        );
        assert_eq!(fs::read(&set_aside).unwrap(), db_bytes);
        assert_eq!(
            fs::read(format!("{}-wal", set_aside.display())).unwrap(),
            wal_bytes
        );
        // 退避したものはバックアップの一覧に並ばない。選んだバックアップは残る
        let listed: Vec<PathBuf> = backup::list_backups(&env.dir)
            .unwrap()
            .into_iter()
            .map(|b| b.path)
            .collect();
        assert_eq!(listed, [backup]);
        // 一時ファイルは残らない
        assert!(
            !names(&env.dir).iter().any(|n| n.contains(".tmp")),
            "{:?}",
            names(&env.dir)
        );
    }

    #[test]
    fn restores_when_there_is_no_database() {
        let env = Env::new();
        let backup = env.backup("from backup", &[V1]);
        let recovered = env.recover(&backup, &[V1]).unwrap();
        assert_eq!(memo(&recovered.database), ["from backup"]);
        assert_eq!(recovered.set_aside, None);
    }

    #[test]
    fn an_older_backup_is_migrated_without_leaving_a_pre_migration_copy() {
        let env = Env::new();
        let backup = env.backup("old", &[V1]);
        damage(&env);
        let recovered = env.recover(&backup, &[V1, V2]).unwrap();
        let done: i64 = recovered
            .database
            .connection()
            .query_row("SELECT done FROM memo", [], |r| r.get(0))
            .unwrap();
        assert_eq!(done, 0);
        assert!(
            !names(&env.backups())
                .iter()
                .any(|n| n.starts_with("pre-migration-")),
            "{:?}",
            names(&env.backups())
        );
    }

    #[test]
    fn unusable_backups_change_nothing() {
        let env = Env::new();
        let (db_bytes, _) = damage(&env);
        let before = names(&env.dir);

        // 壊れたバックアップ
        let corrupted = env.backup("corrupted", &[V1]);
        corrupt_pages(&corrupted);
        // 新しい版で作られたもの
        let newer = env.other.join("newer.db");
        let conn = Connection::open(&newer).unwrap();
        conn.pragma_update(None, "user_version", 9).unwrap();
        drop(conn);
        // Yunagi のデータベースでないもの（番号 0）
        let foreign = env.other.join("foreign.db");
        Connection::open(&foreign)
            .unwrap()
            .execute_batch("CREATE TABLE t(x);")
            .unwrap();

        assert!(matches!(
            env.recover(&corrupted, &[V1]),
            Err(RecoverError::Backup(BackupError::Corrupted { .. }))
        ));
        assert!(matches!(
            env.recover(&newer, &[V1]),
            Err(RecoverError::Backup(BackupError::Storage(
                StorageError::UnsupportedSchema { found: 9, .. }
            )))
        ));
        assert!(matches!(
            env.recover(&foreign, &[V1]),
            Err(RecoverError::Backup(BackupError::NotYunagiDatabase(_)))
        ));
        assert!(matches!(
            env.recover(&env.other.join("missing.db"), &[V1]),
            Err(RecoverError::Backup(BackupError::Sqlite(_)))
        ));
        assert_eq!(fs::read(env.db()).unwrap(), db_bytes);
        // ロックファイルと backups/ は、このテストの準備で作るもの
        let ours = |n: &String| n != "backups" && n != "yunagi.lock";
        let mut after = names(&env.dir);
        after.retain(ours);
        let mut expected = before;
        expected.retain(ours);
        assert_eq!(after, expected);
    }

    #[test]
    fn a_failed_open_puts_the_damaged_database_back() {
        let env = Env::new();
        let backup = env.backup("old", &[V1]);
        let (db_bytes, wal_bytes) = damage(&env);

        let Err(err) = env.recover(&backup, &[V1, V2_BROKEN]) else {
            panic!("recovered with a broken migration");
        };
        assert!(
            matches!(
                err,
                RecoverError::Open(StorageError::MigrationFailed { .. })
            ),
            "{err:?}"
        );
        assert_eq!(fs::read(env.db()).unwrap(), db_bytes);
        assert_eq!(
            fs::read(format!("{}-wal", env.db().display())).unwrap(),
            wal_bytes
        );
        let left = names(&env.backups());
        assert!(
            !left
                .iter()
                .any(|n| n.starts_with("damaged-") || n.starts_with("pre-migration-")),
            "{left:?}"
        );
        assert!(
            !names(&env.dir).iter().any(|n| n.contains(".tmp")),
            "{:?}",
            names(&env.dir)
        );
    }

    #[test]
    fn a_missing_data_directory_is_refused() {
        let env = Env::new();
        let backup = env.backup("b", &[V1]);
        let gone = env.other.join("gone");
        fs::write(
            env.dir.join(CONFIG_FILE_NAME),
            serde_json::json!({ "data_dir": gone }).to_string(),
        )
        .unwrap();
        assert!(matches!(
            env.recover(&backup, &[V1]),
            Err(RecoverError::NoDataDir(p)) if p == gone
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_database_is_not_replaced() {
        let env = Env::new();
        let backup = env.backup("b", &[V1]);
        let target = env.other.join("real.db");
        fs::write(&target, b"not a database").unwrap();
        std::os::unix::fs::symlink(&target, env.db()).unwrap();
        assert!(matches!(
            env.recover(&backup, &[V1]),
            Err(RecoverError::Symlink(_))
        ));
        assert!(fs::symlink_metadata(env.db()).unwrap().is_symlink());
    }

    #[test]
    fn put_back_failure_keeps_both_causes() {
        let err = RecoverError::PutBackFailed {
            source: Box::new(RecoverError::io(
                "rename",
                Path::new("/d/yunagi-restore-1.db.tmp"),
                io::Error::new(io::ErrorKind::PermissionDenied, "permission denied"),
            )),
            put_back_error: io::Error::other("could not move /d/backups/damaged-1.db back: busy"),
            set_aside: "/d/backups/damaged-1.db".into(),
        };
        assert_eq!(
            ErrorChain(&err).to_string(),
            "putting the previous database back failed (could not move /d/backups/damaged-1.db back: busy), so it is left at /d/backups/damaged-1.db; restoring from the backup failed: could not rename /d/yunagi-restore-1.db.tmp: permission denied"
        );
    }
}
