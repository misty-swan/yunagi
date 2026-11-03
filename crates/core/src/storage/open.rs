//! データベースの場所を求め、開くか作るかを決める（docs/design/01-architecture.md 6.3）。
//!
//! - 場所は、ロック（[`DbLock`]）の既定ディレクトリの `config.json` から求める。
//!   呼び出し側はパスを渡せないので、保存先を移した後に古いパスで開くことはない。
//! - [`Database::open`] は、データベースが無いとき、初めて使うとき（[`is_first_use`]）に
//!   だけ新しく作る。それ以外は [`OpenError::NotFound`] を返し、空のデータベースを作らない
//!   （クラウド同期の途中でファイルがまだ届いていない、外付けの保存先を繋いでいないなど。
//!   黙って作ると、利用者には「データが消えた」ように見える）。
//! - 新しく作るのは [`Database::create`] で明示する。

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::data_dir::{self, DataDirError};
use super::migration::Migration;
use super::{Database, DbLock, StorageError};

/// 開くのか、新しく作るのか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Intent {
    /// 今あるものを開く。無ければ、初めて使うときだけ作る。
    Open,
    /// 新しく作る。すでにあれば断る。
    Create,
}

/// データベースの場所で見つかったもの。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Found {
    /// 中身のある `yunagi.db` がある（読めないものも含む。開くときに SQLite が確かめる）。
    Database,
    /// `yunagi.db` が無いか、空（0 バイト）のファイル。
    Missing,
}

impl Database {
    /// ロックの既定ディレクトリの `config.json` から場所を求め、`intent` に従って開く・作る。
    pub(super) fn open_located(
        lock: DbLock,
        intent: Intent,
        migrations: &[Migration],
    ) -> Result<Self, OpenError> {
        let default_dir = lock.default_dir().path().to_path_buf();
        let data_dir = data_dir::resolve_data_dir(lock.default_dir())?;
        let path = data_dir::database_path(&data_dir);
        match (intent, find(&path)) {
            (Intent::Open, Found::Database) => {}
            (Intent::Open, Found::Missing) => {
                if !is_first_use(&default_dir, &data_dir, &path) {
                    let configured_in = (data_dir != default_dir)
                        .then(|| default_dir.join(data_dir::CONFIG_FILE_NAME));
                    return Err(OpenError::NotFound(DatabaseNotFound {
                        data_dir_exists: data_dir.is_dir(),
                        path,
                        data_dir,
                        configured_in,
                    }));
                }
            }
            (Intent::Create, Found::Database) => return Err(OpenError::AlreadyExists(path)),
            (Intent::Create, Found::Missing) => {
                // 取り残された -wal があると、SQLite が新しいデータベースに写し込んでしまう
                if let Some(wal) = wal_of(&path).filter(|w| w.symlink_metadata().is_ok()) {
                    return Err(OpenError::AlreadyExists(wal));
                }
                // 宛先の無いシンボリックリンクは、SQLite がリンク先に作ってしまう
                if path.is_symlink() {
                    return Err(OpenError::AlreadyExists(path));
                }
            }
        }
        Ok(Self::connect(lock, &path, migrations)?)
    }
}

/// `path` に中身のあるデータベースがあるか。
///
/// 0 バイトのファイルは無いものとみなす（同期の途中で中身がまだ届いていないファイルや、
/// 初めて作る途中で止まったもの）。SQLite は 0 バイトのファイルを空のデータベースとして開くため、
/// 区別しないと空のデータベースを作るのと同じことになる。
/// 調べられない（権限が無いなど）ときは「ある」とし、開くときの SQLite の失敗に任せる。
fn find(path: &Path) -> Found {
    match fs::metadata(path) {
        Ok(m) if m.is_file() && m.len() == 0 => Found::Missing,
        Ok(_) => Found::Database,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Found::Missing,
        Err(_) => Found::Database,
    }
}

/// 初めて使うときか：保存先が既定ディレクトリのまま（`config.json` が無い・`data_dir` の項目が
/// 無い・既定ディレクトリを指す）で、そこにデータベースの跡（`yunagi.db` のシンボリックリンク、
/// `yunagi.db` の `-wal`、`backups/` の中身）も無い。`yunagi.db` が無いことは呼び出し側が確かめる。
///
/// 保存先を変えていれば、元の場所でデータベースを作ったことがある（保存先の変更はアプリの
/// 設定画面で、データベースを開いてから行う）ので、初めてではない。`backups/` に何かあれば、
/// データベースが消えたのであって初めてではない（黙って作らず、復元を案内する）。
fn is_first_use(default_dir: &Path, data_dir: &Path, db_path: &Path) -> bool {
    if data_dir != default_dir {
        return false;
    }
    // 宛先の無いシンボリックリンク（リンク先のドライブを繋いでいないなど）は、データベースを
    // 置いたことがある跡。作ると SQLite がリンク先に作ってしまう
    if db_path.is_symlink() {
        return false;
    }
    if wal_of(db_path).is_some_and(|w| w.symlink_metadata().is_ok()) {
        return false;
    }
    match fs::read_dir(data_dir::backups_dir(data_dir)) {
        Ok(mut entries) => entries.next().is_none(),
        Err(e) => e.kind() == io::ErrorKind::NotFound,
    }
}

/// データベースのファイル `db` の `-wal`。
fn wal_of(db: &Path) -> Option<PathBuf> {
    let mut name = db.file_name()?.to_os_string();
    name.push("-wal");
    Some(db.with_file_name(name))
}

/// データベースを開けなかった・作れなかった理由。
#[derive(Debug)]
pub enum OpenError {
    /// `config.json` から保存先を求められなかった（壊れている、読めないなど）。
    DataDir(DataDirError),
    /// 保存先にデータベースが無く、初めて使うときでもない。何も作っていない。
    NotFound(DatabaseNotFound),
    /// 新しく作ろうとした場所に、すでにデータベース（またはその `-wal`）がある。
    /// 上書きしないので、何も変えていない。
    AlreadyExists(PathBuf),
    /// 開く・マイグレーションを適用するのに失敗した。
    Storage(StorageError),
}

/// 保存先にデータベースが見つからなかったときの、画面・CLI が案内に使う情報。
///
/// 案内の例（アーキテクチャ設計書 6.3）：同期が終わるのを待つ・保存先のドライブを繋いでから
/// やり直す、保存先を選び直す、バックアップから戻す、新しく作る（[`Database::create`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseNotFound {
    /// 探したデータベースのファイル（`<保存先>/yunagi.db`）。
    pub path: PathBuf,
    /// 保存先。
    pub data_dir: PathBuf,
    /// 保存先を既定ディレクトリ以外に変えているときの、それを記録した `config.json`。
    /// 既定ディレクトリのままなら `None`。
    pub configured_in: Option<PathBuf>,
    /// 保存先のフォルダそのものがあるか（無ければ、ドライブを繋いでいない・同期の途中などが
    /// 考えられる）。
    pub data_dir_exists: bool,
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DataDir(e) => write!(f, "{e}"),
            Self::NotFound(e) => write!(f, "{e}"),
            Self::AlreadyExists(path) => write!(
                f,
                "a database file already exists, so a new database was not created: {}",
                path.display()
            ),
            Self::Storage(e) => write!(f, "{e}"),
        }
    }
}

impl fmt::Display for DatabaseNotFound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "no Yunagi database found at {}", self.path.display())?;
        if let Some(config) = &self.configured_in {
            write!(f, " (data directory set in {})", config.display())?;
        }
        if !self.data_dir_exists {
            write!(
                f,
                "; the data directory {} does not exist",
                self.data_dir.display()
            )?;
        }
        write!(
            f,
            "; wait for syncing to finish or connect the drive, choose the data directory again, \
             restore from a backup, or create a new database"
        )
    }
}

impl std::error::Error for OpenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::DataDir(e) => Some(e),
            Self::Storage(e) => Some(e),
            Self::NotFound(_) | Self::AlreadyExists(_) => None,
        }
    }
}

impl std::error::Error for DatabaseNotFound {}

impl From<DataDirError> for OpenError {
    fn from(e: DataDirError) -> Self {
        Self::DataDir(e)
    }
}

impl From<StorageError> for OpenError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::data_dir::{
        CONFIG_FILE_NAME, DefaultDir, backups_dir, change_data_dir, database_path, resolve_data_dir,
    };
    use super::*;

    /// 一時フォルダの下の既定ディレクトリ。
    struct Env {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        default: PathBuf,
        dd: DefaultDir,
    }

    impl Env {
        fn new() -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path().to_path_buf();
            let default = root.join("default");
            fs::create_dir(&default).unwrap();
            Self {
                _tmp: tmp,
                dd: DefaultDir::for_test(&default),
                default,
                root,
            }
        }

        fn lock(&self) -> DbLock {
            DbLock::acquire(&self.dd, Duration::ZERO).unwrap()
        }

        fn open(&self) -> Result<Database, OpenError> {
            Database::open(self.lock())
        }

        fn create(&self) -> Result<Database, OpenError> {
            Database::create(self.lock())
        }

        /// `config.json` に保存先 `data_dir` を書く（データは移さない）。
        fn point_config_to(&self, data_dir: &Path) {
            fs::write(
                self.default.join(CONFIG_FILE_NAME),
                serde_json::json!({ "data_dir": data_dir }).to_string(),
            )
            .unwrap();
        }
    }

    fn write_note(db: &Database, body: &str) {
        db.connection()
            .execute_batch(&format!(
                "CREATE TABLE IF NOT EXISTS t(body TEXT); INSERT INTO t VALUES ('{body}');"
            ))
            .unwrap();
    }

    fn read_note(db: &Database) -> String {
        db.connection()
            .query_row("SELECT body FROM t", [], |r| r.get(0))
            .unwrap()
    }

    /// `dir` の中の名前の一覧（並べたもの）。
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    fn not_found(result: Result<Database, OpenError>) -> DatabaseNotFound {
        match result {
            Err(OpenError::NotFound(e)) => e,
            Err(other) => panic!("expected NotFound, got {other:?}"),
            Ok(_) => panic!("expected NotFound, but the database was opened"),
        }
    }

    #[test]
    fn first_use_creates_the_database_in_the_default_dir() {
        let env = Env::new();
        let db = env.open().unwrap();
        write_note(&db, "first");
        drop(db);
        assert!(database_path(&env.default).exists());
        // config.json は作らない（保存先は既定ディレクトリのまま）
        assert!(!env.default.join(CONFIG_FILE_NAME).exists());
        assert_eq!(read_note(&env.open().unwrap()), "first");
    }

    #[test]
    fn config_without_data_dir_is_still_first_use() {
        let env = Env::new();
        fs::write(env.default.join(CONFIG_FILE_NAME), r#"{"theme": "dark"}"#).unwrap();
        drop(env.open().unwrap());
        assert!(database_path(&env.default).exists());
    }

    #[test]
    fn empty_file_left_by_an_interrupted_first_use_is_initialized() {
        let env = Env::new();
        fs::write(database_path(&env.default), b"").unwrap();
        let db = env.open().unwrap();
        write_note(&db, "ok");
        assert_eq!(read_note(&db), "ok");
    }

    #[test]
    fn missing_database_after_moving_the_data_dir_is_not_created() {
        let env = Env::new();
        drop(env.open().unwrap());
        let new_dir = env.root.join("moved");
        change_data_dir(&env.lock(), &new_dir).unwrap();
        // 移した後、新しい保存先のデータベースが消えた（利用者が手で動かしたなど）
        fs::remove_file(database_path(&new_dir)).unwrap();

        let e = not_found(env.open());
        assert_eq!(e.path, database_path(&new_dir));
        assert_eq!(e.data_dir, new_dir);
        assert_eq!(e.configured_in, Some(env.default.join(CONFIG_FILE_NAME)));
        assert!(e.data_dir_exists);
        // 新しい保存先にも、元の保存先（既定ディレクトリ）にも、データベースを作らない
        assert!(!database_path(&new_dir).exists());
        assert!(!database_path(&env.default).exists());
        assert_eq!(resolve_data_dir(&env.dd).unwrap(), new_dir);
    }

    #[test]
    fn data_dir_still_syncing_is_not_filled_with_an_empty_database() {
        let env = Env::new();
        // 別の端末で作った保存先が、同期でまだ届いていない：フォルダと backups/ だけがある
        let sync = env.root.join("sync/Yunagi");
        fs::create_dir_all(backups_dir(&sync)).unwrap();
        fs::write(backups_dir(&sync).join("daily-2026-11-02-1.db"), b"x").unwrap();
        env.point_config_to(&sync);
        let before = names(&sync);

        let e = not_found(env.open());
        assert_eq!(e.path, database_path(&sync));
        assert!(e.data_dir_exists);
        assert_eq!(names(&sync), before, "nothing is created in the data dir");

        // 同期で中身の無いファイル（置き場所だけ）が先に届いた
        fs::write(database_path(&sync), b"").unwrap();
        not_found(env.open());
        assert_eq!(fs::metadata(database_path(&sync)).unwrap().len(), 0);

        // 中身が届けば開ける
        fs::remove_file(database_path(&sync)).unwrap();
        {
            let other = tempfile::tempdir().unwrap();
            let other_dd = DefaultDir::for_test(other.path());
            let db = Database::open(DbLock::acquire(&other_dd, Duration::ZERO).unwrap()).unwrap();
            write_note(&db, "from the other device");
            db.close().unwrap();
            fs::copy(database_path(other.path()), database_path(&sync)).unwrap();
        }
        assert_eq!(read_note(&env.open().unwrap()), "from the other device");
    }

    #[test]
    fn data_dir_that_does_not_exist_is_reported_and_not_created() {
        let env = Env::new();
        let unmounted = env.root.join("external/Yunagi");
        env.point_config_to(&unmounted);
        let e = not_found(env.open());
        assert!(!e.data_dir_exists);
        assert!(e.to_string().contains("does not exist"), "{e}");
        assert!(!env.root.join("external").exists());
        // 明示して作るときも、フォルダは作らない（繋いでいないドライブの代わりに作らないように）
        assert!(matches!(env.create(), Err(OpenError::Storage(_))));
        assert!(!env.root.join("external").exists());
    }

    #[test]
    fn missing_database_with_backups_in_the_default_dir_is_not_first_use() {
        let env = Env::new();
        let db = env.open().unwrap();
        write_note(&db, "kept in the backup");
        db.ensure_daily_backup(crate::domain::logical_date::LogicalDate::new(2026, 11, 3).unwrap())
            .unwrap()
            .unwrap();
        drop(db);
        // データベースだけが消えた
        fs::remove_file(database_path(&env.default)).unwrap();

        let e = not_found(env.open());
        assert_eq!(e.configured_in, None);
        assert!(!database_path(&env.default).exists());

        // 新しく作ってから、バックアップから戻せる
        let backup = super::super::backup::list_backups(&env.default).unwrap()[0]
            .path
            .clone();
        let mut db = env.create().unwrap();
        db.restore_backup(&backup).unwrap();
        assert_eq!(read_note(&db), "kept in the backup");
    }

    #[test]
    fn leftover_wal_is_not_first_use_and_blocks_create() {
        let env = Env::new();
        let wal = env.default.join("yunagi.db-wal");
        fs::write(&wal, b"stale").unwrap();
        not_found(env.open());
        assert!(matches!(env.create(), Err(OpenError::AlreadyExists(p)) if p == wal));
        assert!(!database_path(&env.default).exists());
        assert_eq!(fs::read(&wal).unwrap(), b"stale");
    }

    #[test]
    fn create_makes_a_new_database_where_it_is_missing() {
        let env = Env::new();
        let sync = env.root.join("sync");
        fs::create_dir(&sync).unwrap();
        env.point_config_to(&sync);
        not_found(env.open());

        let db = env.create().unwrap();
        write_note(&db, "new");
        drop(db);
        assert_eq!(read_note(&env.open().unwrap()), "new");
        // 同期で置き場所だけ届いた空のファイルも、明示すれば作り直せる
        fs::write(database_path(&sync), b"").unwrap();
        drop(env.create().unwrap());
    }

    #[test]
    fn create_refuses_when_a_database_exists() {
        let env = Env::new();
        let db = env.open().unwrap();
        write_note(&db, "mine");
        drop(db);
        assert!(matches!(
            env.create(),
            Err(OpenError::AlreadyExists(p)) if p == database_path(&env.default)
        ));
        assert_eq!(read_note(&env.open().unwrap()), "mine");
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_is_not_found_and_not_followed_by_create() {
        let env = Env::new();
        let target = env.root.join("unmounted/yunagi.db");
        std::os::unix::fs::symlink(&target, database_path(&env.default)).unwrap();
        not_found(env.open());
        assert!(matches!(env.create(), Err(OpenError::AlreadyExists(_))));
        assert!(!env.root.join("unmounted").exists());
    }

    #[test]
    fn broken_config_is_a_data_dir_error_and_nothing_is_created() {
        let env = Env::new();
        fs::write(env.default.join(CONFIG_FILE_NAME), "{ not json").unwrap();
        assert!(matches!(
            env.open(),
            Err(OpenError::DataDir(DataDirError::InvalidConfig { .. }))
        ));
        assert!(matches!(
            env.create(),
            Err(OpenError::DataDir(DataDirError::InvalidConfig { .. }))
        ));
        assert!(!database_path(&env.default).exists());
    }

    #[test]
    fn the_lock_is_released_when_opening_fails() {
        let env = Env::new();
        let missing = env.root.join("missing");
        env.point_config_to(&missing);
        not_found(env.open());
        drop(env.lock());
    }
}
