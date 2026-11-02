//! 自動バックアップと復元（docs/design/01-architecture.md 6.2）。
//!
//! 保存先の `backups/` に置くバックアップは3種類ある。どれも `-wal` を伴わない1つの
//! SQLite のファイルで、[`list_backups`] の一覧にまとめて並ぶ。
//!
//! | 種類 | ファイル名 | 作るとき | 残す数 |
//! | --- | --- | --- | --- |
//! | 定期 | `daily-<論理日付>-<UNIX 時刻（秒）>.db` | [`Database::ensure_daily_backup`]（1日1回） | 新しい日付から [`DAILY_KEEP`] 個 |
//! | マイグレーション前 | `pre-migration-v<前>-to-v<後>-<UNIX 時刻（秒）>.db` | マイグレーションの適用前（`migration` モジュール） | 新しいものから [`PRE_MIGRATION_KEEP`] 個 |
//! | 復元前 | `pre-restore-<UNIX 時刻（秒）>.db` | [`Database::restore_backup`] の前 | 新しいものから [`PRE_RESTORE_KEEP`] 個 |
//!
//! 古いものは [`Database::ensure_daily_backup`] と [`Database::restore_backup`] のたびに消す。
//! 名前の形に合わないファイル（利用者が置いたものなど）には触れない。

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use jiff::Timestamp;
use rusqlite::backup::NO_PROGRESS;
use rusqlite::{Connection, MAIN_DB, OpenFlags};

use super::data_dir::{self, DataDirError};
use super::migration::{self, Migration};
use super::{Database, StorageError};
use crate::domain::logical_date::LogicalDate;

/// 定期バックアップを残す数（日付の新しいものから）。
///
/// 暦の上の「7日前より古いもの」ではなく、新しい7つを残す。アプリを開かない日が続いても
/// （その日の分は作られない）、バックアップが全部消えることがないようにするため。
pub const DAILY_KEEP: usize = 7;
/// マイグレーション前のバックアップを残す数（新しいものから）。
///
/// 定期バックアップとは別に数える。マイグレーション後の定期バックアップは新しい版の
/// スキーマになり、旧版を再導入したときに開けないので、7日分の定期バックアップに
/// 押し出されないようにする。
pub const PRE_MIGRATION_KEEP: usize = 3;
/// 復元前に退避したバックアップを残す数（新しいものから）。
pub const PRE_RESTORE_KEEP: usize = 3;

const DAILY_PREFIX: &str = "daily-";
const PRE_MIGRATION_PREFIX: &str = "pre-migration-";
const PRE_RESTORE_PREFIX: &str = "pre-restore-";
const EXTENSION: &str = ".db";
/// 書きかけのバックアップの名前の接尾辞（書き終えてから `.db` に名前を変える）。
const TEMP_SUFFIX: &str = ".db.tmp";

/// `backups/` にある1つのバックアップ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backup {
    /// バックアップのファイルのパス。
    pub path: PathBuf,
    /// 種類。
    pub kind: BackupKind,
    /// 作った時刻（ファイル名に書いた UNIX 時刻。秒まで）。
    pub created_at: Timestamp,
}

/// バックアップの種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupKind {
    /// 1日1回の定期バックアップ。`date` は作った日の論理日付。
    Daily { date: LogicalDate },
    /// マイグレーションの適用前のもの。`from` は適用前、`to` は適用後のスキーマの番号。
    PreMigration { from: u32, to: u32 },
    /// 復元の前に、それまでのデータを退避したもの。
    PreRestore,
}

/// 復元の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreReport {
    /// 復元の前に退避した、それまでのデータ。復元を取り消すときは、これから復元する。
    pub saved: Backup,
}

/// 保存先 `data_dir` の `backups/` にあるバックアップの一覧。新しいものが先。
///
/// `backups/` が無ければ空。名前の形に合わないファイルは含めない。
/// 読むだけなので、`yunagi.lock` を持たずに呼んでよい。
pub fn list_backups(data_dir: &Path) -> Result<Vec<Backup>, BackupError> {
    list_in(&data_dir::backups_dir(data_dir))
}

impl Database {
    /// 論理日付 `today` の定期バックアップがまだ無ければ作り、作ったものを返す。
    /// すでにあれば何もせずに `None` を返す。どちらの場合も、残す数を超えた古い
    /// バックアップを消す（消せなかったものはそのまま残し、失敗にはしない）。
    ///
    /// SQLite のオンラインバックアップで写すので、WAL にだけある書き込みも含まれる。
    /// 写している間にほかの接続が書き込んでも、写し直すので、ある時点で整合した内容になる。
    /// 写したものは整合性を確かめ、一時ファイルに書いてから名前を変える（途中で止まっても、
    /// 壊れたバックアップが一覧に並ばない）。
    ///
    /// アプリは起動時と、起動中に論理日付が変わったときに呼ぶ（アーキテクチャ設計書 6.2）。
    pub fn ensure_daily_backup(&self, today: LogicalDate) -> Result<Option<Backup>, BackupError> {
        let db_path = self.file_path()?;
        let dir = backups_dir_of(&db_path);
        let exists = list_in(&dir)?
            .iter()
            .any(|b| b.kind == BackupKind::Daily { date: today });
        let created = if exists {
            None
        } else {
            let secs = now_secs();
            Some(create_backup(
                &db_path,
                &dir,
                &format!("{DAILY_PREFIX}{today}-{secs}"),
            )?)
        };
        prune(&dir)?;
        Ok(created)
    }

    /// バックアップ `backup` の内容でデータベースを置き換える。
    ///
    /// 1. `backup` を確かめる（SQLite のデータベースとして読めて整合性があり、スキーマの番号を
    ///    このアプリが知っている）。通らなければ、何も変えずにエラーを返す。
    /// 2. 今のデータを `backups/` に退避する（`pre-restore-*.db`）。
    /// 3. 開いている接続のまま、`backup` の内容を写し込む。`backup` のスキーマが古ければ、
    ///    続けてマイグレーションを適用する。
    /// 4. 3 で失敗したら、2 で退避したものに戻してからエラーを返す。
    ///
    /// データベースを差し替えるので、`yunagi.lock` を持つ [`Database`] にだけ行える。
    /// 画面などが読み込んで持っている内容は、復元の後に読み直す必要がある。
    pub fn restore_backup(&mut self, backup: &Path) -> Result<RestoreReport, BackupError> {
        self.restore_with_migrations(backup, migration::MIGRATIONS)
    }

    /// マイグレーションの一覧を指定して復元する。テストで失敗を模擬するために分けている。
    fn restore_with_migrations(
        &mut self,
        backup: &Path,
        migrations: &[Migration],
    ) -> Result<RestoreReport, BackupError> {
        let db_path = self.file_path()?;
        check_restorable(backup, &db_path, migrations)?;
        let dir = backups_dir_of(&db_path);
        let saved = create_backup(
            &db_path,
            &dir,
            &format!("{PRE_RESTORE_PREFIX}{}", now_secs()),
        )?;
        if let Err(source) = self.replace_with(backup, &dir, migrations) {
            let source = Box::new(source);
            return Err(match self.replace_with(&saved.path, &dir, migrations) {
                Ok(()) => BackupError::RestoreFailed {
                    source,
                    saved: saved.path,
                },
                Err(rollback) => BackupError::RollbackFailed {
                    source,
                    rollback_error: Box::new(rollback),
                    saved: saved.path,
                },
            });
        }
        prune(&dir)?;
        Ok(RestoreReport { saved })
    }

    /// 接続を保ったまま、`src` の内容をデータベースに写し込み、最新のスキーマにする。
    fn replace_with(
        &mut self,
        src: &Path,
        backup_dir: &Path,
        migrations: &[Migration],
    ) -> Result<(), BackupError> {
        // 開いたままのトランザクションがあると写し込めない
        if !self.conn.is_autocommit() {
            self.conn.execute_batch("ROLLBACK")?;
        }
        self.conn.restore(MAIN_DB, src, NO_PROGRESS)?;
        // 写し込んでも WAL モードのままだが、開くときと同じ設定にそろえておく
        super::configure(&self.conn)?;
        migration::migrate(&mut self.conn, backup_dir, migrations)?;
        Ok(())
    }

    /// データベースのファイルのパス。
    fn file_path(&self) -> Result<PathBuf, BackupError> {
        match self.conn.path() {
            Some(p) if !p.is_empty() => Ok(PathBuf::from(p)),
            _ => Err(BackupError::NoDatabaseFile),
        }
    }
}

/// データベースのファイル `db_path` と同じフォルダの `backups/`（[`Database::open`] と同じ決め方）。
fn backups_dir_of(db_path: &Path) -> PathBuf {
    data_dir::backups_dir(db_path.parent().unwrap_or_else(|| Path::new("")))
}

/// 復元に使えるバックアップかを確かめる。ファイルには書かない。
fn check_restorable(
    backup: &Path,
    db_path: &Path,
    migrations: &[Migration],
) -> Result<(), BackupError> {
    // データベースそのものから写し込むと、自分を読みながら書くことになる
    if let (Ok(a), Ok(b)) = (fs::canonicalize(backup), fs::canonicalize(db_path))
        && a == b
    {
        return Err(BackupError::NotABackup(backup.to_path_buf()));
    }
    // 読み取り専用で開く（無いファイルを作らない。バックアップに書かない）
    let conn = Connection::open_with_flags(backup, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let result: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if result != "ok" {
        return Err(BackupError::Corrupted {
            path: backup.to_path_buf(),
            result,
        });
    }
    migration::check_supported(&conn, migrations)?;
    Ok(())
}

/// データベース `db_path` を `dir` に `<stem>.db` として写し、写したものを返す。
fn create_backup(db_path: &Path, dir: &Path, stem: &str) -> Result<Backup, BackupError> {
    fs::create_dir_all(dir).map_err(|e| BackupError::io("create directory", dir, e))?;
    remove_stale_temp_files(dir);
    let mut name = format!("{stem}{EXTENSION}");
    let mut n = 1;
    while dir.join(&name).symlink_metadata().is_ok() {
        n += 1;
        name = format!("{stem}-{n}{EXTENSION}");
    }
    let path = dir.join(&name);
    let backup = parse(&path).expect("backup names are built to be parsable");

    let tmp = dir.join(format!("{name}.tmp"));
    let result = data_dir::copy_database(db_path, &tmp)
        .map_err(BackupError::Copy)
        .and_then(|()| fs::rename(&tmp, &path).map_err(|e| BackupError::io("rename", &tmp, e)));
    if result.is_err() {
        remove_with_side_files(&tmp);
        return result.map(|()| backup);
    }
    data_dir::sync_dir(dir);
    Ok(backup)
}

/// 書きかけのまま残った定期・復元前のバックアップの一時ファイルを消す（SQLite が横に作る
/// `-journal` などを含む）。マイグレーション前のものは `migration` モジュールが消す。
fn remove_stale_temp_files(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let ours = name.starts_with(DAILY_PREFIX) || name.starts_with(PRE_RESTORE_PREFIX);
        if ours && name.contains(TEMP_SUFFIX) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// ファイルと、SQLite がその横に作るファイルを消す（無ければ何もしない）。
fn remove_with_side_files(path: &Path) {
    let _ = fs::remove_file(path);
    for suffix in ["-journal", "-wal", "-shm"] {
        let mut side = path.as_os_str().to_os_string();
        side.push(suffix);
        let _ = fs::remove_file(PathBuf::from(side));
    }
}

/// `dir` にあるバックアップの一覧。新しいものが先。
fn list_in(dir: &Path) -> Result<Vec<Backup>, BackupError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(BackupError::io("read directory", dir, e)),
    };
    let mut backups = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| BackupError::io("read directory", dir, e))?;
        // フォルダやシンボリックリンクは、名前が合っても一覧に入れない（消すときに辿らないように）
        let is_file = entry.file_type().is_ok_and(|t| t.is_file());
        if let Some(backup) = is_file.then(|| parse(&entry.path())).flatten() {
            backups.push(backup);
        }
    }
    backups.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.path.cmp(&a.path))
    });
    Ok(backups)
}

/// 残す数を超えた古いバックアップを消す。消せなかったものは残す（次の機会にまた消す）。
fn prune(dir: &Path) -> Result<(), BackupError> {
    let backups = list_in(dir)?;
    let mut daily: Vec<_> = backups
        .iter()
        .filter_map(|b| match b.kind {
            BackupKind::Daily { date } => Some((date, b)),
            _ => None,
        })
        .collect();
    // 日付の新しい順（時計が戻って作った時刻の順と食い違っても、日付で決める）
    daily.sort_by(|(da, a), (db, b)| db.cmp(da).then_with(|| b.created_at.cmp(&a.created_at)));
    let pre_migration = backups
        .iter()
        .filter(|b| matches!(b.kind, BackupKind::PreMigration { .. }));
    let pre_restore = backups.iter().filter(|b| b.kind == BackupKind::PreRestore);

    let old = daily
        .into_iter()
        .map(|(_, b)| b)
        .skip(DAILY_KEEP)
        .chain(pre_migration.skip(PRE_MIGRATION_KEEP))
        .chain(pre_restore.skip(PRE_RESTORE_KEEP));
    for b in old {
        let _ = fs::remove_file(&b.path);
    }
    Ok(())
}

/// ファイル名からバックアップの種類と作った時刻を読む。形に合わなければ `None`。
fn parse(path: &Path) -> Option<Backup> {
    let stem = path.file_name()?.to_str()?.strip_suffix(EXTENSION)?;
    let (kind, rest) = if let Some(rest) = stem.strip_prefix(DAILY_PREFIX) {
        // YYYY-MM-DD-<秒>
        let date = rest.get(..10)?.parse().ok()?;
        (
            BackupKind::Daily { date },
            rest.get(10..)?.strip_prefix('-')?,
        )
    } else if let Some(rest) = stem.strip_prefix(PRE_MIGRATION_PREFIX) {
        // v<前>-to-v<後>-<秒>
        let (from, rest) = rest.strip_prefix('v')?.split_once("-to-v")?;
        let (to, rest) = rest.split_once('-')?;
        let kind = BackupKind::PreMigration {
            from: parse_digits(from)?,
            to: parse_digits(to)?,
        };
        (kind, rest)
    } else {
        (
            BackupKind::PreRestore,
            stem.strip_prefix(PRE_RESTORE_PREFIX)?,
        )
    };
    // <秒> の後に、同じ秒に作ったものを見分ける `-<番号>` が付くことがある
    let (secs, counter) = rest.split_once('-').unwrap_or((rest, "1"));
    parse_digits::<u32>(counter)?;
    let created_at = Timestamp::from_second(parse_digits(secs)?).ok()?;
    Some(Backup {
        path: path.to_path_buf(),
        kind,
        created_at,
    })
}

/// 数字だけの文字列を数にする（`+` や空文字列は受け付けない）。
fn parse_digits<T: std::str::FromStr>(s: &str) -> Option<T> {
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}

/// 今の UNIX 時刻（秒）。
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// バックアップ・復元が失敗した理由。
#[derive(Debug)]
pub enum BackupError {
    /// SQLite の操作が失敗した（復元に選んだファイルが SQLite のデータベースでないときを含む）。
    Sqlite(rusqlite::Error),
    /// ファイルの操作が失敗した。`action` は何をしようとしたか。
    Io {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    /// データベースを写すのに失敗した。
    Copy(DataDirError),
    /// 開く・マイグレーションなどのストレージの操作が失敗した。復元に選んだバックアップの
    /// スキーマを知らない（新しい版で作られた）ときは [`StorageError::UnsupportedSchema`]。
    Storage(StorageError),
    /// データベースがファイルではない（メモリ上にある）。
    NoDatabaseFile,
    /// 復元に選んだファイルが、データベースそのもの。
    NotABackup(PathBuf),
    /// 復元に選んだバックアップの整合性の確認（`integrity_check`）が通らなかった。
    /// データベースには触れていない。
    Corrupted { path: PathBuf, result: String },
    /// 復元に失敗し、退避しておいた復元前のデータ（`saved`）に戻した。データは復元前のまま。
    RestoreFailed {
        source: Box<BackupError>,
        saved: PathBuf,
    },
    /// 復元に失敗し、復元前のデータに戻すことにも失敗した。データベースは途中の状態でありうる。
    /// `saved` に復元前のデータが残っている。
    RollbackFailed {
        source: Box<BackupError>,
        rollback_error: Box<BackupError>,
        saved: PathBuf,
    },
}

impl BackupError {
    fn io(action: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            action,
            path: path.to_path_buf(),
            source,
        }
    }
}

impl fmt::Display for BackupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "database error: {e}"),
            Self::Io {
                action,
                path,
                source,
            } => write!(f, "could not {action} {}: {source}", path.display()),
            Self::Copy(e) => write!(f, "could not copy the database: {e}"),
            Self::Storage(e) => write!(f, "{e}"),
            Self::NoDatabaseFile => write!(f, "the database is not stored in a file"),
            Self::NotABackup(path) => {
                write!(f, "{} is the database itself, not a backup", path.display())
            }
            Self::Corrupted { path, result } => write!(
                f,
                "backup {} failed the integrity check: {result}",
                path.display()
            ),
            Self::RestoreFailed { source, saved } => write!(
                f,
                "restore failed and the previous data was put back from {}: {source}",
                saved.display()
            ),
            Self::RollbackFailed {
                source,
                rollback_error,
                saved,
            } => write!(
                f,
                "restore failed: {source}; putting back the previous data from {} also failed: {rollback_error}",
                saved.display()
            ),
        }
    }
}

impl std::error::Error for BackupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(e) => Some(e),
            Self::Io { source, .. } => Some(source),
            Self::Copy(e) => Some(e),
            Self::Storage(e) => Some(e),
            Self::RestoreFailed { source, .. } | Self::RollbackFailed { source, .. } => {
                Some(source.as_ref())
            }
            Self::NoDatabaseFile | Self::NotABackup(_) | Self::Corrupted { .. } => None,
        }
    }
}

impl From<rusqlite::Error> for BackupError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

impl From<StorageError> for BackupError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::Duration;

    use super::super::DbLock;
    use super::*;

    fn day(d: i8) -> LogicalDate {
        LogicalDate::new(2026, 11, d).unwrap()
    }

    /// ロックを取ってから開く。テストでは、ロックファイルをデータベースと同じ一時フォルダに置く。
    fn open(path: &Path) -> Database {
        open_with(path, migration::MIGRATIONS).unwrap()
    }

    fn open_with(path: &Path, migrations: &[Migration]) -> Result<Database, StorageError> {
        let lock = DbLock::acquire(path.parent().unwrap(), Duration::ZERO).unwrap();
        Database::open_with_migrations(lock, path, migrations)
    }

    fn set(db: &Database, key: &str, value: i64) {
        db.connection()
            .execute(
                "INSERT INTO setting (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                (key, value.to_string()),
            )
            .unwrap();
    }

    fn get(conn: &Connection, key: &str) -> Option<i64> {
        conn.query_row("SELECT value FROM setting WHERE key = ?1", [key], |r| {
            r.get::<_, String>(0)
        })
        .ok()
        .map(|v| v.parse().unwrap())
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .map(|es| {
                es.map(|e| e.unwrap().file_name().into_string().unwrap())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    /// 番号 `user_version` の空のデータベースを `path` に作る（バックアップの代わり）。
    fn plain_db(path: &Path, user_version: i64) {
        let conn = Connection::open(path).unwrap();
        conn.pragma_update(None, "user_version", user_version)
            .unwrap();
    }

    struct Env {
        dir: tempfile::TempDir,
    }

    impl Env {
        fn new() -> Self {
            Self {
                dir: tempfile::tempdir().unwrap(),
            }
        }
        fn db_path(&self) -> PathBuf {
            data_dir::database_path(self.dir.path())
        }
        fn backups(&self) -> PathBuf {
            data_dir::backups_dir(self.dir.path())
        }
        fn list(&self) -> Vec<Backup> {
            list_backups(self.dir.path()).unwrap()
        }
    }

    #[test]
    fn creates_one_daily_backup_per_logical_date() {
        let env = Env::new();
        let db = open(&env.db_path());
        set(&db, "a", 1);

        let first = db.ensure_daily_backup(day(3)).unwrap().unwrap();
        assert_eq!(first.kind, BackupKind::Daily { date: day(3) });
        let name = first.path.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("daily-2026-11-03-"), "{name}");
        // 同じ日にもう一度呼んでも作らない
        set(&db, "a", 2);
        assert_eq!(db.ensure_daily_backup(day(3)).unwrap(), None);
        // 翌日は作る
        let second = db.ensure_daily_backup(day(4)).unwrap().unwrap();
        assert_eq!(env.list(), [second, first.clone()]);

        // 中身は作った時点のもの。-wal を伴わない1つのファイル
        let conn =
            Connection::open_with_flags(&first.path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        assert_eq!(get(&conn, "a"), Some(1));
        let mode: String = conn
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "delete");
        drop(conn);
        assert!(names(&env.backups()).iter().all(|n| n.ends_with(".db")));
    }

    #[test]
    fn daily_backup_includes_writes_still_in_the_wal() {
        let env = Env::new();
        let db = open(&env.db_path());
        // 本体への書き戻し（チェックポイント）を止め、書き込みが WAL にだけある状態にする
        db.connection()
            .pragma_update(None, "wal_autocheckpoint", 0)
            .unwrap();
        set(&db, "in_wal", 42);
        let wal = PathBuf::from(format!("{}-wal", env.db_path().display()));
        assert!(fs::metadata(&wal).unwrap().len() > 0);

        let backup = db.ensure_daily_backup(day(3)).unwrap().unwrap();
        let conn = Connection::open(&backup.path).unwrap();
        assert_eq!(get(&conn, "in_wal"), Some(42));
    }

    #[test]
    fn daily_backup_is_consistent_while_another_connection_writes() {
        let env = Env::new();
        let db = open(&env.db_path());
        // 書き込みを増やし、写している途中に書き込みが入るようにする
        for i in 0..2000 {
            set(&db, &format!("seed{i}"), i);
        }

        // 別の接続が、2行を1つのトランザクションで書き続ける。整合していれば、
        // バックアップの中の pair_a と pair_b はいつも同じ値になる
        let stop = Arc::new(AtomicBool::new(false));
        let started = Arc::new(Barrier::new(2));
        let writer = {
            let (path, stop, started) = (env.db_path(), stop.clone(), started.clone());
            thread::spawn(move || {
                let mut conn = Connection::open(path).unwrap();
                conn.busy_timeout(Duration::from_secs(5)).unwrap();
                started.wait();
                let mut n = 0_i64;
                while !stop.load(Ordering::Relaxed) {
                    n += 1;
                    let tx = conn.transaction().unwrap();
                    for key in ["pair_a", "pair_b"] {
                        tx.execute(
                            "INSERT INTO setting (key, value) VALUES (?1, ?2)
                             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                            (key, n.to_string()),
                        )
                        .unwrap();
                    }
                    tx.commit().unwrap();
                }
                n
            })
        };
        started.wait();
        let mut checked = 0;
        for d in 1..=5 {
            let backup = db.ensure_daily_backup(day(d)).unwrap().unwrap();
            let conn = Connection::open(&backup.path).unwrap();
            let ok: String = conn
                .query_row("PRAGMA integrity_check", [], |r| r.get(0))
                .unwrap();
            assert_eq!(ok, "ok");
            assert_eq!(get(&conn, "pair_a"), get(&conn, "pair_b"));
            assert_eq!(get(&conn, "seed1999"), Some(1999));
            checked += 1;
        }
        stop.store(true, Ordering::Relaxed);
        assert!(writer.join().unwrap() > 0);
        assert_eq!(checked, 5);
    }

    #[test]
    fn keeps_only_the_newest_seven_daily_backups() {
        let env = Env::new();
        let db = open(&env.db_path());
        for d in 1..=9 {
            db.ensure_daily_backup(day(d)).unwrap().unwrap();
        }
        let dates: Vec<_> = env
            .list()
            .iter()
            .map(|b| match b.kind {
                BackupKind::Daily { date } => date,
                k => panic!("unexpected {k:?}"),
            })
            .collect();
        assert_eq!(dates, (3..=9).rev().map(day).collect::<Vec<_>>());
        assert_eq!(names(&env.backups()).len(), DAILY_KEEP);
    }

    #[test]
    fn prunes_by_date_even_if_the_clock_went_back() {
        let env = Env::new();
        let db = open(&env.db_path());
        fs::create_dir_all(env.backups()).unwrap();
        // 日付の新しいものほど、作った時刻が古い（時計が戻った）
        for d in 1..=8_i8 {
            let secs = 2_000_000_000 - i64::from(d);
            plain_db(
                &env.backups().join(format!("daily-2026-11-0{d}-{secs}.db")),
                1,
            );
        }
        db.ensure_daily_backup(day(8)).unwrap();
        let left = names(&env.backups());
        assert_eq!(left.len(), 7);
        assert!(
            !left.iter().any(|n| n.starts_with("daily-2026-11-01-")),
            "{left:?}"
        );
    }

    #[test]
    fn pre_migration_backups_are_listed_and_kept_separately() {
        let env = Env::new();
        let db = open(&env.db_path());
        fs::create_dir_all(env.backups()).unwrap();
        for i in 1..=5 {
            plain_db(
                &env.backups()
                    .join(format!("pre-migration-v1-to-v2-{}.db", 1_000 + i)),
                1,
            );
        }
        for d in 1..=9 {
            db.ensure_daily_backup(day(d)).unwrap();
        }
        let list = env.list();
        let pre: Vec<_> = list
            .iter()
            .filter(|b| matches!(b.kind, BackupKind::PreMigration { from: 1, to: 2 }))
            .map(|b| b.created_at.as_second())
            .collect();
        // 新しい3つが残り、定期バックアップの7つに押し出されない
        assert_eq!(pre, [1_005, 1_004, 1_003]);
        assert_eq!(list.len(), DAILY_KEEP + PRE_MIGRATION_KEEP);
        // 定期バックアップより古い（作った時刻が前）ので、一覧では後ろに並ぶ
        assert!(matches!(list[0].kind, BackupKind::Daily { .. }));
        assert!(matches!(list[9].kind, BackupKind::PreMigration { .. }));
    }

    #[test]
    fn list_ignores_unknown_files_and_temp_files() {
        let env = Env::new();
        let backups = env.backups();
        fs::create_dir_all(backups.join("daily-2026-11-03-5.db")).unwrap(); // フォルダ
        for name in [
            "notes.txt",
            "daily-2026-11-03-1.db.tmp",
            "daily-2026-13-03-1.db",
            "daily-2026-11-03-x.db",
            "pre-restore-.db",
            "pre-restore-+1.db",
            "pre-migration-v1-to-v2.db",
        ] {
            fs::write(backups.join(name), b"").unwrap();
        }
        for name in [
            "daily-2026-11-03-1.db",
            "pre-restore-2-3.db",
            "pre-migration-v0-to-v1-3.db",
        ] {
            fs::write(backups.join(name), b"").unwrap();
        }
        let kinds: Vec<_> = env.list().into_iter().map(|b| b.kind).collect();
        assert_eq!(
            kinds,
            [
                BackupKind::PreMigration { from: 0, to: 1 },
                BackupKind::PreRestore,
                BackupKind::Daily { date: day(3) },
            ]
        );
        // 一覧が無いときは空
        let empty = tempfile::tempdir().unwrap();
        assert!(list_backups(empty.path()).unwrap().is_empty());
    }

    #[test]
    fn creating_a_backup_removes_leftover_temp_files_only() {
        let env = Env::new();
        let db = open(&env.db_path());
        let backups = env.backups();
        fs::create_dir_all(&backups).unwrap();
        for name in [
            "daily-2026-11-02-1.db.tmp",
            "daily-2026-11-02-1.db.tmp-journal",
            "pre-restore-1.db.tmp",
            "pre-migration-v1-to-v2-1.db.tmp",
            "mine.db.tmp",
        ] {
            fs::write(backups.join(name), b"").unwrap();
        }
        db.ensure_daily_backup(day(3)).unwrap().unwrap();
        let left = names(&backups);
        assert_eq!(left.len(), 3, "{left:?}");
        assert!(left.contains(&"pre-migration-v1-to-v2-1.db.tmp".to_owned()));
        assert!(left.contains(&"mine.db.tmp".to_owned()));
    }

    #[test]
    fn restores_a_backup_after_saving_the_current_data() {
        let env = Env::new();
        let mut db = open(&env.db_path());
        set(&db, "a", 1);
        let backup = db.ensure_daily_backup(day(3)).unwrap().unwrap();
        set(&db, "a", 2);
        set(&db, "b", 2);

        let report = db.restore_backup(&backup.path).unwrap();
        assert_eq!(get(db.connection(), "a"), Some(1));
        assert_eq!(get(db.connection(), "b"), None);
        // 開いたままの接続で、WAL モードのまま書き続けられる
        let mode: String = db
            .connection()
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        set(&db, "c", 3);

        // 復元前のデータは退避されていて、一覧に並ぶ
        assert_eq!(report.saved.kind, BackupKind::PreRestore);
        assert_eq!(env.list()[0], report.saved);
        let saved = Connection::open(&report.saved.path).unwrap();
        assert_eq!(get(&saved, "a"), Some(2));
        assert_eq!(get(&saved, "b"), Some(2));
        drop(saved);

        // 閉じて開き直しても、復元した内容のまま
        db.close().unwrap();
        let db = open(&env.db_path());
        assert_eq!(get(db.connection(), "a"), Some(1));
        assert_eq!(get(db.connection(), "c"), Some(3));
    }

    #[test]
    fn restore_can_be_undone_from_the_saved_data() {
        let env = Env::new();
        let mut db = open(&env.db_path());
        set(&db, "a", 1);
        let backup = db.ensure_daily_backup(day(3)).unwrap().unwrap();
        set(&db, "a", 2);
        let report = db.restore_backup(&backup.path).unwrap();
        db.restore_backup(&report.saved.path).unwrap();
        assert_eq!(get(db.connection(), "a"), Some(2));
    }

    #[test]
    fn keeps_only_the_newest_pre_restore_backups() {
        let env = Env::new();
        let mut db = open(&env.db_path());
        let backup = db.ensure_daily_backup(day(3)).unwrap().unwrap();
        for _ in 0..5 {
            db.restore_backup(&backup.path).unwrap();
        }
        let pre_restore = env
            .list()
            .iter()
            .filter(|b| b.kind == BackupKind::PreRestore)
            .count();
        assert_eq!(pre_restore, PRE_RESTORE_KEEP);
        // 復元に使ったバックアップは残る
        assert!(backup.path.exists());
    }

    #[test]
    fn restoring_an_older_schema_applies_the_migrations() {
        let env = Env::new();
        let mut db = open(&env.db_path());
        set(&db, "a", 1);
        let old = env.dir.path().join("v0.db");
        plain_db(&old, 0);

        db.restore_backup(&old).unwrap();
        let version: i64 = db
            .connection()
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(
            version,
            i64::from(migration::MIGRATIONS.last().unwrap().version)
        );
        assert_eq!(get(db.connection(), "a"), None);
    }

    /// 復元に使えないファイルは、何も変えず、退避もせずに断る。
    fn assert_rejected(env: &Env, db: &mut Database, backup: &Path) -> BackupError {
        let before = names(&env.backups());
        let err = db.restore_backup(backup).unwrap_err();
        assert_eq!(get(db.connection(), "a"), Some(1), "{err}");
        assert_eq!(names(&env.backups()), before, "{err}");
        err
    }

    #[test]
    fn rejects_unusable_backups_without_changing_anything() {
        let env = Env::new();
        let mut db = open(&env.db_path());
        set(&db, "a", 1);

        let garbage = env.dir.path().join("garbage.db");
        fs::write(&garbage, b"not a database at all, just some text").unwrap();
        let err = assert_rejected(&env, &mut db, &garbage);
        assert!(matches!(err, BackupError::Sqlite(_)), "{err:?}");

        let missing = env.dir.path().join("missing.db");
        assert_rejected(&env, &mut db, &missing);
        assert!(!missing.exists());

        let newer = env.dir.path().join("newer.db");
        plain_db(&newer, 99);
        let err = assert_rejected(&env, &mut db, &newer);
        assert!(
            matches!(
                err,
                BackupError::Storage(StorageError::UnsupportedSchema { found: 99, .. })
            ),
            "{err:?}"
        );

        let err = assert_rejected(&env, &mut db, &env.db_path());
        assert!(matches!(err, BackupError::NotABackup(_)), "{err:?}");
    }

    #[test]
    fn rejects_a_corrupted_backup() {
        let env = Env::new();
        let mut db = open(&env.db_path());
        set(&db, "a", 1);
        for i in 0..200 {
            set(&db, &format!("k{i}"), i);
        }
        let backup = db.ensure_daily_backup(day(3)).unwrap().unwrap();
        // 2ページ目以降の中身を壊す（見出しは残すので、SQLite のファイルとしては開ける）
        let mut bytes = fs::read(&backup.path).unwrap();
        let page_size = usize::from(u16::from_be_bytes([bytes[16], bytes[17]]));
        for b in &mut bytes[page_size + 8..] {
            *b = 0xA5;
        }
        fs::write(&backup.path, &bytes).unwrap();
        let err = assert_rejected(&env, &mut db, &backup.path);
        assert!(
            matches!(err, BackupError::Corrupted { .. } | BackupError::Sqlite(_)),
            "{err:?}"
        );
    }

    #[test]
    fn failed_restore_puts_the_previous_data_back() {
        const V1: Migration = Migration {
            version: 1,
            name: "create memo",
            sql: "CREATE TABLE memo (body TEXT NOT NULL);",
        };
        const V2_FAILS: Migration = Migration {
            version: 2,
            name: "fails",
            sql: "DELETE FROM memo; INSERT INTO no_such_table VALUES (1);",
        };
        let env = Env::new();
        // 今のデータベースは v2 まで適用済み、バックアップは v1
        let old = env.dir.path().join("v1.db");
        {
            let conn = Connection::open(&old).unwrap();
            conn.execute_batch(
                "CREATE TABLE memo (body TEXT NOT NULL);
                 INSERT INTO memo VALUES ('old');
                 PRAGMA user_version = 1;",
            )
            .unwrap();
        }
        let mut db = open_with(
            &env.db_path(),
            &[
                V1,
                Migration {
                    version: 2,
                    name: "ok",
                    sql: "",
                },
            ],
        )
        .unwrap();
        db.connection()
            .execute_batch("INSERT INTO memo VALUES ('current');")
            .unwrap();

        // 復元したバックアップへのマイグレーションが失敗する
        let err = db
            .restore_with_migrations(&old, &[V1, V2_FAILS])
            .unwrap_err();
        let BackupError::RestoreFailed { source, saved } = &err else {
            panic!("{err:?}");
        };
        assert!(
            matches!(
                **source,
                BackupError::Storage(StorageError::MigrationFailed { .. })
            ),
            "{source:?}"
        );
        assert!(saved.exists());
        let bodies: Vec<String> = db
            .connection()
            .prepare("SELECT body FROM memo")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(bodies, ["current"]);
        let version: i64 = db
            .connection()
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 2);
    }

    #[test]
    fn parses_backup_names() {
        let p = |n: &str| parse(Path::new(n)).map(|b| (b.kind, b.created_at.as_second()));
        assert_eq!(
            p("daily-2026-11-03-1762128000.db"),
            Some((BackupKind::Daily { date: day(3) }, 1_762_128_000))
        );
        assert_eq!(
            p("pre-migration-v12-to-v13-7-2.db"),
            Some((BackupKind::PreMigration { from: 12, to: 13 }, 7))
        );
        assert_eq!(p("pre-restore-9.db"), Some((BackupKind::PreRestore, 9)));
        for bad in [
            "daily-2026-11-03.db",
            "daily-2026-11-03-1-x.db",
            "pre-migration-v1-v2-3.db",
            "pre-restore-1.db.tmp",
            "pre-restore-1",
            "other-1.db",
        ] {
            assert_eq!(p(bad), None, "{bad}");
        }
    }
}
