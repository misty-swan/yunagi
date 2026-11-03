//! ストレージ：SQLite（WAL）＋ FTS5 全文検索
//! （docs/design/01-architecture.md 2章・3章、docs/design/02-data-model.md 1章）。
//!
//! データベースは [`Database::open`] で開く。開くときに、未適用のマイグレーション
//! （スキーマを番号順に更新する手順）を自動で適用する（[`migration`] モジュール）。
//! 開く前に `yunagi.lock` のロック（[`DbLock`]）を取る（[`lock`] モジュール）。

pub mod cloud_sync;
pub mod data_dir;
mod error;
mod lock;
mod migration;
pub mod table;
mod ulid;

use std::path::Path;

use rusqlite::Connection;

pub use error::StorageError;
pub use lock::{CloseError, DEFAULT_LOCK_WAIT, DbLock, LOCK_FILE_NAME, LockError};
pub use ulid::{ParseUlidError, Ulid};

/// 開いた `yunagi.db`。マイグレーションはすべて適用済み。
///
/// `yunagi.lock` のロックを持っている。閉じる（drop する）と、接続を閉じてからロックを離す。
pub struct Database {
    conn: Connection,
    // フィールドは書いた順に drop される。接続を閉じてからロックを離すよう、conn より後に置く
    _lock: DbLock,
}

impl Database {
    /// `path` のデータベースを開き（無ければ作り）、未適用のマイグレーションを適用する。
    ///
    /// - WAL モード、`synchronous=NORMAL` で開く（アーキテクチャ設計書 3章）。
    /// - 適用前に、`path` と同じフォルダの `backups/` にバックアップを取る。
    ///   適用に失敗したら、バックアップに戻してから [`StorageError::MigrationFailed`] を返す。
    ///
    /// `path` のフォルダは呼び出し側が用意する（保存先の決め方は [`data_dir`]）。
    ///
    /// 同じデータベースを同時に開くのは1つのプロセスだけ、という前提で動く
    /// （アーキテクチャ設計書 4章）。2つが同時にマイグレーションすると、失敗した側の復元で、
    /// もう一方の書き込みが消えうる。そこで、取った `yunagi.lock` のロック（`lock`）を
    /// 受け取らないと開けないようにしている。開いている間は `Database` がロックを持ち、
    /// 開けなかったときはロックを離してからエラーを返す。
    ///
    /// アプリは起動時に [`DbLock::acquire`] でロックを取ってから開く。直接モードの
    /// CLI・MCP は、操作ごとに [`Database::with_locked`] を使う。
    pub fn open(lock: DbLock, path: impl AsRef<Path>) -> Result<Self, StorageError> {
        Self::open_with_migrations(lock, path.as_ref(), migration::MIGRATIONS)
    }

    /// マイグレーションの一覧を指定して開く。テストで失敗を模擬するために分けている。
    fn open_with_migrations(
        lock: DbLock,
        path: &Path,
        migrations: &[migration::Migration],
    ) -> Result<Self, StorageError> {
        let mut conn = Connection::open(path)?;
        // 新しい版で作られたデータベースは、WAL への切り替えなどでファイルを書き換える前に断る
        migration::check_supported(&conn, migrations)?;
        configure(&conn)?;
        let backup_dir = path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(data_dir::BACKUPS_DIR_NAME);
        migration::migrate(&mut conn, &backup_dir, migrations)?;
        Ok(Self { conn, _lock: lock })
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
