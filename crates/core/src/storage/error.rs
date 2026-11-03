//! ストレージの失敗の種類。

use std::fmt;
use std::io;
use std::path::PathBuf;

use super::LockError;
use crate::error::ErrorChain;

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
        // 原因のエラーの文は含めない（source() で返す。README「エラーの表示」）
        match self {
            Self::Sqlite(_) => write!(f, "database error"),
            Self::Io(_) => write!(f, "file error"),
            // 包むだけなので、中身をそのまま見せる（透過）
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
                backup,
                ..
            } => write!(
                f,
                "migration {version} ({name}) failed and the database was restored from {}",
                backup.display()
            ),
            // 原因が2つある。source() はマイグレーションの失敗を返すので、戻す失敗は
            // その原因までここに書く。source() の文が後ろにつながったとき、マイグレーションの
            // 原因と読めるよう、マイグレーションの失敗を最後に書く
            Self::RestoreFailed {
                version,
                name,
                restore_error,
                backup,
                ..
            } => write!(
                f,
                "restoring from {} failed ({}); migration {version} ({name}) failed",
                backup.display(),
                ErrorChain(restore_error.as_ref())
            ),
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(e) => Some(e),
            Self::Io(e) => Some(e),
            // Display と同じく透過。中身の原因を返す
            Self::Lock(e) => e.source(),
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

#[cfg(test)]
mod tests {
    //! エラーの表示の決まり（README「エラーの表示」）を、storage のエラー型ごとに確かめる。
    //! - Display には原因の文を含めない。原因は source() で返す
    //! - ErrorChain でつなぐと、原因まで1回ずつ出る

    use std::error::Error;
    use std::time::Duration;

    use super::*;
    use crate::error::ErrorChain;
    use crate::storage::backup::BackupError;
    use crate::storage::data_dir::{DataDirError, DefaultDir};
    use crate::storage::{CloseError, Database, DbLock};

    fn io_err() -> io::Error {
        io::Error::new(io::ErrorKind::PermissionDenied, "permission denied")
    }

    /// SQLite が説明の文を付けない失敗。rusqlite は Display に原因の文を含め、
    /// source() でも同じ原因を返す。
    fn sqlite_err() -> rusqlite::Error {
        rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL), None)
    }

    /// Display・source() の文・ErrorChain の文をまとめて確かめる。原因があれば、その文が
    /// Display に含まれていないことも確かめる。
    fn check(err: &(dyn Error + 'static), display: &str, source: Option<&str>, chain: &str) {
        assert_eq!(err.to_string(), display);
        let src = err.source().map(ToString::to_string);
        assert_eq!(src.as_deref(), source);
        if let Some(src) = src {
            assert!(!display.contains(&src), "{display:?} contains {src:?}");
        }
        assert_eq!(ErrorChain(err).to_string(), chain);
    }

    #[test]
    fn storage_error_display_does_not_repeat_its_source() {
        check(
            &StorageError::Io(io_err()),
            "file error",
            Some("permission denied"),
            "file error: permission denied",
        );
        let sqlite = sqlite_err().to_string();
        check(
            &StorageError::Sqlite(sqlite_err()),
            "database error",
            Some(&sqlite),
            &format!("database error: {sqlite}"),
        );
        check(
            &StorageError::WalUnavailable {
                mode: "memory".into(),
            },
            "could not enable WAL mode (journal mode is memory)",
            None,
            "could not enable WAL mode (journal mode is memory)",
        );
        check(
            &StorageError::UnsupportedSchema {
                found: 9,
                supported: 3,
            },
            "database schema version 9 is not supported by this app (supports up to 3)",
            None,
            "database schema version 9 is not supported by this app (supports up to 3)",
        );
        check(
            &StorageError::MigrationFailed {
                version: 2,
                name: "add_notes",
                source: rusqlite::Error::InvalidQuery,
                backup: "/b/pre-2.db".into(),
            },
            "migration 2 (add_notes) failed and the database was restored from /b/pre-2.db",
            Some("Query is not read-only"),
            "migration 2 (add_notes) failed and the database was restored from /b/pre-2.db: Query is not read-only",
        );
    }

    #[test]
    fn restore_failed_shows_both_causes_once() {
        let err = StorageError::RestoreFailed {
            version: 2,
            name: "add_notes",
            source: rusqlite::Error::InvalidQuery,
            restore_error: Box::new(StorageError::Io(io_err())),
            backup: "/b/pre-2.db".into(),
        };
        // 戻す失敗は source() で返さないので、その原因（permission denied）まで Display に書く
        check(
            &err,
            "restoring from /b/pre-2.db failed (file error: permission denied); migration 2 (add_notes) failed",
            Some("Query is not read-only"),
            "restoring from /b/pre-2.db failed (file error: permission denied); migration 2 (add_notes) failed: Query is not read-only",
        );
    }

    #[test]
    fn restore_failed_keeps_main_cause_with_the_same_text() {
        // ディスクがいっぱいで、マイグレーションも戻すのも同じ失敗になったとき。
        // 戻す失敗の文に同じ文が入っていても、マイグレーションの原因を省かない
        let sqlite = sqlite_err().to_string();
        let err = StorageError::RestoreFailed {
            version: 2,
            name: "add_notes",
            source: sqlite_err(),
            restore_error: Box::new(StorageError::Sqlite(sqlite_err())),
            backup: "/b/pre-2.db".into(),
        };
        assert_eq!(
            ErrorChain(&err).to_string(),
            format!(
                "restoring from /b/pre-2.db failed (database error: {sqlite}); migration 2 (add_notes) failed: {sqlite}"
            )
        );
    }

    #[test]
    fn wrapping_variants_are_transparent() {
        // StorageError::Lock は LockError をそのまま見せ、source() も LockError の原因を返す
        let lock = || LockError::Io {
            path: "/d/yunagi.lock".into(),
            source: io_err(),
        };
        check(
            &lock(),
            "could not lock /d/yunagi.lock",
            Some("permission denied"),
            "could not lock /d/yunagi.lock: permission denied",
        );
        check(
            &StorageError::Lock(lock()),
            "could not lock /d/yunagi.lock",
            Some("permission denied"),
            "could not lock /d/yunagi.lock: permission denied",
        );
        let busy = StorageError::Lock(LockError::Busy {
            path: "/d/yunagi.lock".into(),
            waited: Duration::ZERO,
        });
        check(
            &busy,
            "the Yunagi database is already in use (could not lock /d/yunagi.lock)",
            None,
            "the Yunagi database is already in use (could not lock /d/yunagi.lock)",
        );
        // BackupError::Storage も同じ
        check(
            &BackupError::Storage(StorageError::Lock(lock())),
            "could not lock /d/yunagi.lock",
            Some("permission denied"),
            "could not lock /d/yunagi.lock: permission denied",
        );
    }

    #[test]
    fn close_error_display_does_not_repeat_its_source() {
        let dir = tempfile::tempdir().unwrap();
        let lock = DbLock::acquire(&DefaultDir::for_test(dir.path()), Duration::ZERO).unwrap();
        let db = Database::open(lock, dir.path().join("yunagi.db")).unwrap();
        let err = CloseError {
            error: StorageError::Io(io_err()),
            db,
        };
        check(
            &err,
            "could not close the database",
            Some("file error"),
            "could not close the database: file error: permission denied",
        );
    }

    #[test]
    fn data_dir_error_display_does_not_repeat_its_source() {
        check(
            &DataDirError::Io {
                action: "create",
                path: "/d".into(),
                source: io_err(),
            },
            "could not create /d",
            Some("permission denied"),
            "could not create /d: permission denied",
        );
        let json = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
        let json_text = json.to_string();
        check(
            &DataDirError::InvalidConfig {
                path: "/d/config.json".into(),
                source: json,
            },
            "invalid config file /d/config.json",
            Some(&json_text),
            &format!("invalid config file /d/config.json: {json_text}"),
        );
        let sqlite = sqlite_err().to_string();
        check(
            &DataDirError::Sqlite(sqlite_err()),
            "database error while copying",
            Some(&sqlite),
            &format!("database error while copying: {sqlite}"),
        );
    }

    #[test]
    fn backup_error_display_does_not_repeat_its_source() {
        check(
            &BackupError::Io {
                action: "remove",
                path: "/b/x.db".into(),
                source: io_err(),
            },
            "could not remove /b/x.db",
            Some("permission denied"),
            "could not remove /b/x.db: permission denied",
        );
        check(
            &BackupError::Copy(DataDirError::Sqlite(rusqlite::Error::InvalidQuery)),
            "could not copy the database",
            Some("database error while copying"),
            "could not copy the database: database error while copying: Query is not read-only",
        );
        let restore = || BackupError::Sqlite(rusqlite::Error::InvalidQuery);
        check(
            &BackupError::RestoreFailed {
                source: Box::new(restore()),
                saved: "/b/pre-restore.db".into(),
            },
            "restore failed and the previous data was put back from /b/pre-restore.db",
            Some("database error"),
            "restore failed and the previous data was put back from /b/pre-restore.db: database error: Query is not read-only",
        );
        check(
            &BackupError::RollbackFailed {
                source: Box::new(restore()),
                rollback_error: Box::new(BackupError::Io {
                    action: "rename",
                    path: "/d/yunagi.db".into(),
                    source: io_err(),
                }),
                saved: "/b/pre-restore.db".into(),
            },
            "putting back the previous data from /b/pre-restore.db failed (could not rename /d/yunagi.db: permission denied); the restore failed",
            Some("database error"),
            "putting back the previous data from /b/pre-restore.db failed (could not rename /d/yunagi.db: permission denied); the restore failed: database error: Query is not read-only",
        );
    }

    #[test]
    fn chain_skips_text_already_shown_by_external_errors() {
        // rusqlite の SqliteFailure(_, None) は、Display と source() の両方で同じ原因の文を出す
        let inner = sqlite_err();
        let text = inner.to_string();
        assert_eq!(inner.source().unwrap().to_string(), text);
        assert_eq!(ErrorChain(&inner).to_string(), text);
        assert_eq!(
            ErrorChain(&StorageError::Sqlite(sqlite_err())).to_string(),
            format!("database error: {text}")
        );
    }
}
