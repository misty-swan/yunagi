//! ストレージの失敗の種類。

use std::fmt;
use std::io;
use std::path::PathBuf;

use super::LockError;

/// ストレージの操作が失敗した理由。
#[derive(Debug)]
pub enum StorageError {
    /// SQLite の操作が失敗した。
    Sqlite(rusqlite::Error),
    /// ファイルの操作（バックアップ用フォルダの作成など）が失敗した。
    Io(io::Error),
    /// `yunagi.lock` のロックが取れなかった（ほかのプロセスが使っているなど）。
    /// データベースには触れていない。
    Lock(LockError),
    /// WAL モードにできなかった。`mode` は実際のジャーナルモード。
    WalUnavailable { mode: String },
    /// データベースのスキーマの番号を、このアプリが知らない。ふつうは新しい版で作られたもの
    /// （番号が `supported` より大きい）。負の番号もここに入る。ファイルには触れていない。
    UnsupportedSchema { found: i64, supported: u32 },
    /// マイグレーションの適用に失敗し、バックアップに戻した。データは適用前のまま。
    /// 画面では旧版の再導入方法を案内する（ビルド・配布手順書 5章）。
    MigrationFailed {
        version: u32,
        name: &'static str,
        source: rusqlite::Error,
        backup: PathBuf,
    },
    /// マイグレーションの適用に失敗し、バックアップに戻すことにも失敗した。
    /// データベースは途中の状態でありうる。`backup` に適用前のデータが残っている。
    RestoreFailed {
        version: u32,
        name: &'static str,
        source: rusqlite::Error,
        restore_error: Box<StorageError>,
        backup: PathBuf,
    },
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "database error: {e}"),
            Self::Io(e) => write!(f, "file error: {e}"),
            Self::Lock(e) => write!(f, "{e}"),
            Self::WalUnavailable { mode } => {
                write!(f, "could not enable WAL mode (journal mode is {mode})")
            }
            Self::UnsupportedSchema { found, supported } => write!(
                f,
                "database schema version {found} is not supported by this app (supports up to {supported})"
            ),
            Self::MigrationFailed {
                version,
                name,
                source,
                backup,
            } => write!(
                f,
                "migration {version} ({name}) failed and the database was restored from {}: {source}",
                backup.display()
            ),
            Self::RestoreFailed {
                version,
                name,
                source,
                restore_error,
                backup,
            } => write!(
                f,
                "migration {version} ({name}) failed: {source}; restoring from {} also failed: {restore_error}",
                backup.display()
            ),
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(e) => Some(e),
            Self::Io(e) => Some(e),
            Self::Lock(e) => Some(e),
            Self::MigrationFailed { source, .. } | Self::RestoreFailed { source, .. } => {
                Some(source)
            }
            Self::WalUnavailable { .. } | Self::UnsupportedSchema { .. } => None,
        }
    }
}

impl From<rusqlite::Error> for StorageError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

impl From<io::Error> for StorageError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
