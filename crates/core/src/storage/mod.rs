//! ストレージ：SQLite（WAL）＋ FTS5 全文検索
//! （docs/design/01-architecture.md 2章・3章、docs/design/02-data-model.md 1章）。
//!
//! データベースは [`Database::open`] で開く。開くときに、未適用のマイグレーション
//! （スキーマを番号順に更新する手順）を自動で適用する（[`migration`] モジュール）。
//! 開く前に `yunagi.lock` のロック（[`DbLock`]）を取る（[`lock`] モジュール）。
//! 場所はロックと `config.json` から求め、データベースが無いときに黙って作らない（`open` モジュール）。

pub mod backup;
pub mod cloud_sync;
pub mod data_dir;
mod error;
mod lock;
mod migration;
mod open;
pub mod search;
pub mod table;
mod ulid;

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

pub use error::StorageError;
pub use lock::{CloseError, DEFAULT_LOCK_WAIT, DbLock, LOCK_FILE_NAME, LockError};
pub use open::{DatabaseNotFound, OpenError};
pub use ulid::{ParseUlidError, Ulid};

/// 開いた `yunagi.db`。マイグレーションはすべて適用済み。
///
/// `yunagi.lock` のロックを持っている。閉じる（drop する）と、接続を閉じてからロックを離す。
pub struct Database {
    conn: Connection,
    /// 開いたデータベースのパス（バックアップの置き場所を、マイグレーションと同じに決めるため）
    path: PathBuf,
    // フィールドは書いた順に drop される。接続を閉じてからロックを離すよう、conn より後に置く
    _lock: DbLock,
}

impl Database {
    /// 保存先の `yunagi.db` を開き、未適用のマイグレーションを適用する。
    ///
    /// - 保存先は、ロック `lock` を取った既定ディレクトリの `config.json` から求める
    ///   （[`DbLock::database_path`]。アーキテクチャ設計書 4.2）。パスは受け取らない。
    /// - データベースが無ければ、初めて使うとき（保存先が既定ディレクトリのままで、
    ///   データベースの跡も無いとき）だけ新しく作る。それ以外は何も作らずに
    ///   [`OpenError::NotFound`] を返す（同期の途中・保存先のドライブを繋いでいないときなどに、
    ///   空のデータベースを作らない。アーキテクチャ設計書 6.3）。新しく作るのは
    ///   [`Database::create`] で明示する。
    /// - WAL モード、`synchronous=NORMAL` で開く（アーキテクチャ設計書 3章）。
    /// - 適用前に、データベースと同じフォルダの `backups/` にバックアップを取る。
    ///   適用に失敗したら、バックアップに戻してから [`StorageError::MigrationFailed`] を返す。
    ///
    /// 同じデータベースを同時に開くのは1つのプロセスだけ、という前提で動く
    /// （アーキテクチャ設計書 4章）。2つが同時にマイグレーションすると、失敗した側の復元で、
    /// もう一方の書き込みが消えうる。そこで、取った `yunagi.lock` のロック（`lock`）を
    /// 受け取らないと開けないようにしている。開いている間は `Database` がロックを持ち、
    /// 開けなかったときはロックを離してからエラーを返す。
    ///
    /// アプリは起動時に [`DbLock::acquire`] でロックを取ってから開く。直接モードの
    /// CLI・MCP は、操作ごとに [`Database::with_locked`] を使う。
    pub fn open(lock: DbLock) -> Result<Self, OpenError> {
        Self::open_located(lock, open::Intent::Open, migration::MIGRATIONS)
    }

    /// 保存先に、新しいデータベースを作って開く（[`Database::open`] がデータベースを
    /// 見つけられず、利用者が新しく作ることを選んだときに使う）。
    ///
    /// 保存先の決め方・ロック・マイグレーションは [`Database::open`] と同じ。すでにデータベース
    /// （中身のある `yunagi.db`、その `-wal`、`yunagi.db` のシンボリックリンク）があれば、
    /// 上書きせずに [`OpenError::AlreadyExists`] を返す。中身の無い（0 バイトの）`yunagi.db` は
    /// 作り直す。保存先のフォルダは作らない（繋いでいないドライブの代わりに作らないように）ので、
    /// 無ければ [`OpenError::NotFound`]（`data_dir_exists` が `false`）を返す。
    ///
    /// バックアップから戻すときは、作ってから [`Database::restore_backup`] を呼ぶ。
    pub fn create(lock: DbLock) -> Result<Self, OpenError> {
        Self::open_located(lock, open::Intent::Create, migration::MIGRATIONS)
    }

    /// テスト用：パス `path` を、マイグレーションの一覧を指定して開く（無ければ作る）。
    /// マイグレーションの失敗を模擬するテストで使う。
    #[cfg(test)]
    fn open_with_migrations(
        lock: DbLock,
        path: &Path,
        migrations: &[migration::Migration],
    ) -> Result<Self, StorageError> {
        Self::connect(lock, path, migrations, true)
    }

    /// `path` のデータベースを開き、マイグレーションを適用する。`create` なら、無ければ
    /// SQLite が作る。`create` でなければ作らず、無ければ失敗する。
    /// 開いてよい場所かは、呼び出し側（[`Database::open_located`]）が確かめる。
    fn connect(
        lock: DbLock,
        path: &Path,
        migrations: &[migration::Migration],
        create: bool,
    ) -> Result<Self, StorageError> {
        let mut flags = OpenFlags::default();
        if !create {
            flags.remove(OpenFlags::SQLITE_OPEN_CREATE);
        }
        let mut conn = Connection::open_with_flags(path, flags)?;
        // 新しい版で作られたデータベースは、WAL への切り替えなどでファイルを書き換える前に断る
        migration::check_supported(&conn, migrations)?;
        configure(&conn)?;
        let backup_dir = path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(data_dir::BACKUPS_DIR_NAME);
        migration::migrate(&mut conn, &backup_dir, migrations)?;
        Ok(Self {
            conn,
            path: path.to_path_buf(),
            _lock: lock,
        })
    }

    /// 接続を閉じ、ロックを離す。閉じるのに失敗したら（WAL の書き戻しの失敗など）エラーを返す。
    /// 閉じずに手放した（drop した）ときは、閉じるときの失敗は捨てられる。
    pub fn close(self) -> Result<(), StorageError> {
        // 閉じたらロックを離す。失敗したときは、返ってきたデータベースを手放す
        // （接続を閉じてからロックが離れる）
        self.close_keeping_lock().map(drop).map_err(|e| e.error)
    }

    /// SQLite への接続。表ごとの基本の読み書きは [`table`] の関数に渡して行う。
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

/// 接続を WAL モード・`synchronous=NORMAL`・外部キー有効にする。
fn configure(conn: &Connection) -> Result<(), StorageError> {
    // journal_mode は、切り替えられなかったとき（WAL に対応しない場所など）にエラーにならず、
    // 元のモードを返す。黙って別のモードで動かないよう、結果を確かめる
    let mode: String =
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(StorageError::WalUnavailable { mode });
    }
    // synchronous と foreign_keys は接続ごとの設定（ファイルには残らない）なので、開くたびに設定する
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    // SQLite は、接続ごとに有効にしない限り外部キー（REFERENCES）を確かめない
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}
