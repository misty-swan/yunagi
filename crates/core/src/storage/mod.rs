//! ストレージ：SQLite（WAL）＋ FTS5 全文検索
//! （docs/design/01-architecture.md 2章・3章、docs/design/02-data-model.md 1章）。
//!
//! データベースは [`Database::open`] で開く。開くときに、未適用のマイグレーション
//! （スキーマを番号順に更新する手順）を自動で適用する（[`migration`] モジュール）。

mod error;
mod migration;

use std::path::Path;

use rusqlite::Connection;

pub use error::StorageError;

/// 開いた `yunagi.db`。マイグレーションはすべて適用済み。
pub struct Database {
    conn: Connection,
}

impl Database {
    /// `path` のデータベースを開き（無ければ作り）、未適用のマイグレーションを適用する。
    ///
    /// - WAL モード、`synchronous=NORMAL` で開く（アーキテクチャ設計書 3章）。
    /// - 適用前に、`path` と同じフォルダの `backups/` にバックアップを取る。
    ///   適用に失敗したら、バックアップに戻してから [`StorageError::MigrationFailed`] を返す。
    ///
    /// `path` のフォルダは呼び出し側が用意する（保存先の決め方は YUN-20）。
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        Self::open_with_migrations(path.as_ref(), migration::MIGRATIONS)
    }

    /// マイグレーションの一覧を指定して開く。テストで失敗を模擬するために分けている。
    fn open_with_migrations(
        path: &Path,
        migrations: &[migration::Migration],
    ) -> Result<Self, StorageError> {
        let mut conn = connect(path)?;
        let backup_dir = path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join("backups");
        migration::migrate(&mut conn, &backup_dir, migrations)?;
        Ok(Self { conn })
    }

    /// SQLite への接続。表の読み書きは後続の課題でこの層に作る。
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

/// 接続を開き、WAL モードと `synchronous=NORMAL` にする。
fn connect(path: &Path) -> Result<Connection, StorageError> {
    let conn = Connection::open(path)?;
    // journal_mode は、切り替えられなかったとき（WAL に対応しない場所など）にエラーにならず、
    // 元のモードを返す。黙って別のモードで動かないよう、結果を確かめる
    let mode: String =
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(StorageError::WalUnavailable { mode });
    }
    // synchronous は接続ごとの設定（ファイルには残らない）なので、開くたびに設定する
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(conn)
}
