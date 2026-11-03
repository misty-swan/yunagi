//! マイグレーション：スキーマを番号順に更新する仕組み（docs/design/02-data-model.md 1章）。
//!
//! - 適用済みの番号は、SQLite のファイルの見出しにある `user_version` に記録する。
//!   0 は「まだ何も適用していない」。
//! - 起動時（[`super::Database::open`]）に、記録より大きい番号のものを順に適用する。
//! - 適用の前に、保存先の `backups/` にバックアップを取る。1つでも失敗したら、
//!   それまでに適用したものも含めてバックアップに戻し、データを適用前のままにする。
//! - 記録がこのアプリの知る範囲にない（新しい版で作られた）データベースは、
//!   ファイルに書かずにエラーとする（WAL への切り替えより前に確かめる）。
//! - 適用中は外部キーの検査を止め（表の作り直しのため）、各マイグレーションの確定前に
//!   `foreign_key_check` で参照が壊れていないことを確かめる（SQLite の推奨する手順）。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::backup::NO_PROGRESS;
use rusqlite::{Connection, MAIN_DB, ffi};

use super::StorageError;

/// 1つのマイグレーション。
pub(crate) struct Migration {
    /// 番号。1 から始めて、1 ずつ増やす。
    pub version: u32,
    /// 何をするかの短い説明（失敗したときの表示に使う）。
    pub name: &'static str,
    /// 実行する SQL。番号の記録と同じトランザクションの中で、外部キーの検査を止めて実行する。
    /// 確定の前に参照の壊れた行が見つかれば失敗にする。
    pub sql: &'static str,
}

/// 本番のマイグレーションの一覧。番号の順に並べる。
///
/// 一度配布した版に入ったものは書き換えない（利用者のデータベースには適用済みのため、
/// 書き換えても反映されない）。スキーマを変えるときは、新しい番号を末尾に足す。
///
/// 表の構造を変えなくても、保存する値の種類（item の `status`、agent_client の `scopes` など）を
/// 足すときは、新しい番号を足す（SQL は空でもよい）。旧版は新しい番号のデータベースを開かずに
/// 断るので、知らない値を読んで途中で止まることがない（設計書 1.1）。
pub(crate) const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "create initial tables",
    sql: include_str!("migrations/0001_initial.sql"),
}];

// 番号の並びの誤りは、起動時ではなくコンパイル時に見つける
const _: () = assert!(
    is_sequential(MIGRATIONS),
    "MIGRATIONS must be numbered 1, 2, 3, ... in order"
);

/// 未適用のマイグレーションを適用する。`backup_dir` はバックアップを置くフォルダ。
pub(super) fn migrate(
    conn: &mut Connection,
    backup_dir: &Path,
    migrations: &[Migration],
) -> Result<(), StorageError> {
    // 本番の一覧はコンパイル時に確かめている。ここはテスト用の一覧のため
    debug_assert!(
        is_sequential(migrations),
        "migration versions must be 1, 2, 3, ... in order"
    );
    let current = check_supported(conn, migrations)?;
    let latest = migrations.last().map_or(0, |m| m.version);
    // 番号は 1 からの連番なので、先頭から current 個が適用済み
    let pending = &migrations[current as usize..];
    if pending.is_empty() {
        return Ok(());
    }

    // 番号 0 で表も無いデータベースは、新しく作ったもので、戻す意味のあるデータが無い
    let was_empty = current == 0 && schema_is_empty(conn)?;
    let backup = create_backup(conn, backup_dir, current, latest)?;

    // 外部キーの検査はトランザクションの中では切り替えられないので、外で止めて、最後に戻す
    conn.pragma_update(None, "foreign_keys", "OFF")?;
    let result = apply_all(conn, pending, &backup);
    let reenabled = conn.pragma_update(None, "foreign_keys", "ON");
    result?;
    reenabled?;

    // 新しく作ったデータベースのバックアップは、成功したら消す（消せなくても害はない）
    if was_empty {
        let _ = fs::remove_file(&backup);
    }
    Ok(())
}

/// 順に適用し、失敗したらバックアップに戻す。
fn apply_all(
    conn: &mut Connection,
    pending: &[Migration],
    backup: &Path,
) -> Result<(), StorageError> {
    for m in pending {
        if let Err(source) = apply(conn, m) {
            return Err(match restore(conn, backup) {
                Ok(()) => StorageError::MigrationFailed {
                    version: m.version,
                    name: m.name,
                    source,
                    backup: backup.to_path_buf(),
                },
                Err(restore_error) => StorageError::RestoreFailed {
                    version: m.version,
                    name: m.name,
                    source,
                    restore_error: Box::new(restore_error),
                    backup: backup.to_path_buf(),
                },
            });
        }
    }
    Ok(())
}

/// 番号が 1, 2, 3, … と並んでいるか。
const fn is_sequential(migrations: &[Migration]) -> bool {
    let mut i = 0;
    while i < migrations.len() {
        if migrations[i].version as usize != i + 1 {
            return false;
        }
        i += 1;
    }
    true
}

/// 表・索引などが1つも無いか。
fn schema_is_empty(conn: &Connection) -> Result<bool, StorageError> {
    let count: i64 = conn.query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))?;
    Ok(count == 0)
}

/// 適用済みの番号が、このアプリの知る範囲（0 から最新まで）にあることを確かめ、その番号を返す。
/// 範囲外なら [`StorageError::UnsupportedSchema`]。読むだけで、ファイルには書かない。
pub(super) fn check_supported(
    conn: &Connection,
    migrations: &[Migration],
) -> Result<u32, StorageError> {
    let found = schema_version(conn)?;
    let supported = migrations.last().map_or(0, |m| m.version);
    if found < 0 || found > i64::from(supported) {
        return Err(StorageError::UnsupportedSchema { found, supported });
    }
    Ok(found as u32)
}

/// 適用済みの番号（`user_version`）。SQLite では符号付きの 32 ビットの値なので、そのまま読む。
fn schema_version(conn: &Connection) -> Result<i64, StorageError> {
    Ok(conn.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

/// 1つを適用する。SQL と番号の記録を1つのトランザクションで行う。
fn apply(conn: &mut Connection, m: &Migration) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute_batch(m.sql)?;
    let broken: Option<String> = tx
        .query_row("PRAGMA foreign_key_check", [], |r| r.get(0))
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(e),
        })?;
    if let Some(table) = broken {
        return Err(rusqlite::Error::SqliteFailure(
            ffi::Error::new(ffi::SQLITE_CONSTRAINT_FOREIGNKEY),
            Some(format!("foreign key violation in table {table}")),
        ));
    }
    tx.pragma_update(None, "user_version", m.version)?;
    tx.commit()
}

/// `backup_dir` にデータベースのバックアップを作り、そのパスを返す。
///
/// 名前は `pre-migration-v<適用前の番号>-to-v<適用後の番号>-<UNIX 時刻（秒）>.db`。
/// 書き終わる前に止まっても壊れたバックアップが残らないよう、一時ファイルに書いてから名前を変える。
/// 前回に中断して残った一時ファイルは、ここで消す。
fn create_backup(
    conn: &Connection,
    backup_dir: &Path,
    from: u32,
    to: u32,
) -> Result<PathBuf, StorageError> {
    fs::create_dir_all(backup_dir)?;
    remove_stale_temp_files(backup_dir)?;
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let stem = format!("pre-migration-v{from}-to-v{to}-{secs}");
    let mut path = backup_dir.join(format!("{stem}.db"));
    let mut n = 1;
    while path.exists() {
        n += 1;
        path = backup_dir.join(format!("{stem}-{n}.db"));
    }

    let tmp = path.with_extension("db.tmp");
    let result = (|| {
        // SQLite のオンラインバックアップ。WAL にだけある直近の書き込みも含めて写す
        conn.backup(MAIN_DB, &tmp, NO_PROGRESS)?;
        // 写したファイルは元と同じく WAL の印を持ち、開くたびに横に -wal・-shm ができる。
        // バックアップは1つのファイルで完結させたいので、通常の（DELETE）モードにする
        let mode: String = Connection::open(&tmp)?.pragma_update_and_check(
            None,
            "journal_mode",
            "DELETE",
            |row| row.get(0),
        )?;
        debug_assert_eq!(mode, "delete");
        fs::rename(&tmp, &path)?;
        sync_dir(backup_dir)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map(|()| path)
}

/// バックアップの内容でデータベースを置き換える。
fn restore(conn: &mut Connection, backup: &Path) -> Result<(), StorageError> {
    // 失敗したトランザクションが閉じていなければ閉じる（開いたままだと書き戻せない）
    if !conn.is_autocommit() {
        conn.execute_batch("ROLLBACK")?;
    }
    conn.restore(MAIN_DB, backup, NO_PROGRESS)?;
    Ok(())
}

/// 書きかけのまま残ったバックアップの一時ファイル（`pre-migration-*.db.tmp`）を消す。
fn remove_stale_temp_files(backup_dir: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(backup_dir)? {
        let path = entry?.path();
        let is_temp = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("pre-migration-") && n.ends_with(".db.tmp"));
        if is_temp {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}

/// 名前の変更を確実にディスクに書く（電源断で消えないように）。Windows ではフォルダを
/// 開けないため行わない。
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    fs::File::open(dir)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::data_dir::DefaultDir;
    use super::super::{Database, DbLock};
    use super::*;

    /// テスト用のマイグレーション。本番の一覧（MIGRATIONS）とは別。
    const V1: Migration = Migration {
        version: 1,
        name: "create memo",
        sql: "CREATE TABLE memo (id INTEGER PRIMARY KEY, body TEXT NOT NULL);",
    };
    const V2: Migration = Migration {
        version: 2,
        name: "add memo.done and rewrite rows",
        sql: "ALTER TABLE memo ADD COLUMN done INTEGER NOT NULL DEFAULT 0;
              UPDATE memo SET body = body || ' (migrated)', done = 1;",
    };
    /// 失敗を模擬するマイグレーション：途中まで書いてから、存在しない表を参照して失敗する。
    const V3_FAILS: Migration = Migration {
        version: 3,
        name: "fails halfway",
        sql: "DELETE FROM memo;
              CREATE TABLE tag (id INTEGER PRIMARY KEY);
              INSERT INTO no_such_table VALUES (1);",
    };
    const V3_OK: Migration = Migration {
        version: 3,
        name: "create tag",
        sql: "CREATE TABLE tag (id INTEGER PRIMARY KEY);",
    };

    fn db_path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("yunagi.db")
    }

    /// ロックを取ってから、マイグレーションの一覧を指定して開く。テストでは、既定ディレクトリの
    /// 代わりにデータベースと同じ一時フォルダにロックファイルを置く（本番では別のフォルダ）。
    fn open_with(path: &Path, migrations: &[Migration]) -> Result<Database, StorageError> {
        let lock = DbLock::acquire(
            &DefaultDir::for_test(path.parent().unwrap()),
            Duration::ZERO,
        )
        .unwrap();
        Database::open_with_migrations(lock, path, migrations)
    }

    /// ロックを取ってから、本番の一覧で開く（ロックの置き場所は [`open_with`] と同じ）。
    fn open_default(path: &Path) -> Result<Database, StorageError> {
        open_with(path, MIGRATIONS)
    }

    fn backups(dir: &tempfile::TempDir) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(dir.path().join("backups")) else {
            return Vec::new();
        };
        let mut paths: Vec<_> = entries.map(|e| e.unwrap().path()).collect();
        paths.sort();
        paths
    }

    /// スキーマと全行を文字列にしたもの。前後で比べて「何も変わっていない」を確かめる。
    fn snapshot(conn: &Connection) -> Vec<String> {
        let mut out = vec![format!("user_version={}", schema_version(conn).unwrap())];
        let mut stmt = conn
            .prepare("SELECT type, name, sql FROM sqlite_schema ORDER BY name")
            .unwrap();
        let schema: Vec<(String, String, Option<String>)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for (kind, name, sql) in schema {
            out.push(format!("{kind} {name}: {sql:?}"));
            if kind == "table" {
                let mut rows = conn
                    .prepare(&format!("SELECT * FROM \"{name}\" ORDER BY rowid"))
                    .unwrap();
                let n = rows.column_count();
                let lines: Vec<String> = rows
                    .query_map([], |r| {
                        let cols: Vec<String> = (0..n)
                            .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
                            .collect();
                        Ok(cols.join(","))
                    })
                    .unwrap()
                    .map(Result::unwrap)
                    .collect();
                out.extend(lines.into_iter().map(|l| format!("  {name}: {l}")));
            }
        }
        out
    }

    /// v1 まで適用し、行を入れたデータベースを作る（旧版で作ったデータの代わり）。
    fn create_v1_with_rows(path: &Path) {
        let db = open_with(path, &[V1]).unwrap();
        db.connection()
            .execute_batch("INSERT INTO memo (body) VALUES ('first'), ('second');")
            .unwrap();
    }

    #[test]
    fn opens_in_wal_mode_with_synchronous_normal() {
        let dir = tempfile::tempdir().unwrap();
        let db = open_default(&db_path(&dir)).unwrap();
        let conn = db.connection();
        let mode: String = conn
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        // synchronous の値：0=OFF, 1=NORMAL, 2=FULL, 3=EXTRA
        let sync: i64 = conn
            .pragma_query_value(None, "synchronous", |r| r.get(0))
            .unwrap();
        assert_eq!(sync, 1);
    }

    #[test]
    fn production_migrations_are_sequential() {
        assert!(is_sequential(MIGRATIONS));
        assert!(MIGRATIONS.iter().all(|m| !m.name.is_empty()));
    }

    #[test]
    fn fts5_is_available() {
        // 全文検索（FTS5）が同梱の SQLite に入っていること（アーキテクチャ設計書 3章）
        let dir = tempfile::tempdir().unwrap();
        let db = open_default(&db_path(&dir)).unwrap();
        db.connection()
            .execute_batch("CREATE VIRTUAL TABLE t USING fts5(body);")
            .unwrap();
    }

    #[test]
    fn new_database_gets_all_migrations_without_leaving_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let db = open_with(&db_path(&dir), &[V1, V2]).unwrap();
        assert_eq!(schema_version(db.connection()).unwrap(), 2);
        assert!(backups(&dir).is_empty());
    }

    #[test]
    fn applies_only_pending_migrations_after_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(&dir);
        create_v1_with_rows(&path);

        let db = open_with(&path, &[V1, V2]).unwrap();
        assert_eq!(schema_version(db.connection()).unwrap(), 2);
        let bodies: Vec<String> = db
            .connection()
            .prepare("SELECT body FROM memo ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(bodies, ["first (migrated)", "second (migrated)"]);

        // 適用前（v1）の内容のバックアップが残る
        let saved = backups(&dir);
        assert_eq!(saved.len(), 1);
        let name = saved[0].file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("pre-migration-v1-to-v2-"), "{name}");
        assert!(name.ends_with(".db"), "{name}");
        let backup = Connection::open(&saved[0]).unwrap();
        assert_eq!(schema_version(&backup).unwrap(), 1);
        let count: i64 = backup
            .query_row("SELECT count(*) FROM memo", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn up_to_date_database_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(&dir);
        create_v1_with_rows(&path);
        drop(open_with(&path, &[V1, V2]).unwrap());
        let before = backups(&dir);

        let db = open_with(&path, &[V1, V2]).unwrap();
        assert_eq!(schema_version(db.connection()).unwrap(), 2);
        assert_eq!(backups(&dir), before, "no new backup when nothing to apply");
    }

    #[test]
    fn failed_migration_restores_the_backup_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(&dir);
        create_v1_with_rows(&path);
        let before = snapshot(&Connection::open(&path).unwrap());

        // v2 は成功して書き込まれ、v3 が途中で失敗する。v2 の分も戻ること
        let err = open_with(&path, &[V1, V2, V3_FAILS])
            .err()
            .expect("migration should fail");
        let StorageError::MigrationFailed {
            version, backup, ..
        } = &err
        else {
            panic!("unexpected error: {err}");
        };
        assert_eq!(*version, 3);
        assert!(backup.exists());

        // 開き直して、適用前と全く同じであることを確かめる
        let conn = Connection::open(&path).unwrap();
        assert_eq!(snapshot(&conn), before);
        let mode: String = conn
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        drop(conn);

        // 直した版（v3 を修正）なら、同じデータベースに適用できる
        let db = open_with(&path, &[V1, V2, V3_OK]).unwrap();
        assert_eq!(schema_version(db.connection()).unwrap(), 3);
    }

    #[test]
    fn failed_first_migration_leaves_a_new_database_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(&dir);
        let failing_v1 = Migration {
            version: 1,
            name: "broken",
            sql: "CREATE TABLE a (id INTEGER); NOT VALID SQL;",
        };
        let err = open_with(&path, &[failing_v1])
            .err()
            .expect("migration should fail");
        assert!(matches!(
            err,
            StorageError::MigrationFailed { version: 1, .. }
        ));
        let conn = Connection::open(&path).unwrap();
        assert_eq!(snapshot(&conn), ["user_version=0"]);
    }

    #[test]
    fn newer_database_is_not_touched() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(&dir);
        create_v1_with_rows(&path);
        drop(open_with(&path, &[V1, V2]).unwrap());
        let before = snapshot(&Connection::open(&path).unwrap());
        let backups_before = backups(&dir);

        // v2 のデータベースを、v1 までしか知らない（古い版の）アプリで開く
        let err = open_with(&path, &[V1])
            .err()
            .expect("should refuse a newer schema");
        assert!(matches!(
            err,
            StorageError::UnsupportedSchema {
                found: 2,
                supported: 1
            }
        ));
        assert_eq!(snapshot(&Connection::open(&path).unwrap()), before);
        assert_eq!(backups(&dir), backups_before);
    }

    #[test]
    fn foreign_keys_are_enforced_after_open() {
        let dir = tempfile::tempdir().unwrap();
        let db = open_default(&db_path(&dir)).unwrap();
        let on: i64 = db
            .connection()
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        assert_eq!(on, 1);
    }

    const PARENT_CHILD: Migration = Migration {
        version: 1,
        name: "parent and child",
        sql: "CREATE TABLE parent (id INTEGER PRIMARY KEY, label TEXT);
              CREATE TABLE child (id INTEGER PRIMARY KEY,
                                  parent_id INTEGER NOT NULL REFERENCES parent(id) ON DELETE CASCADE);",
    };

    fn create_parent_child_with_rows(path: &Path) {
        let db = open_with(path, &[PARENT_CHILD]).unwrap();
        db.connection()
            .execute_batch("INSERT INTO parent VALUES (1, 'a'); INSERT INTO child VALUES (10, 1);")
            .unwrap();
    }

    #[test]
    fn migration_can_rebuild_a_referenced_table() {
        // 参照されている表の作り直し（新しい表に写して、古い表を消し、名前を変える）。
        // 外部キーの検査が効いたままだと、古い表を消したときに child の行が消える
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(&dir);
        create_parent_child_with_rows(&path);
        let rebuild = Migration {
            version: 2,
            name: "rebuild parent",
            sql: "CREATE TABLE parent_new (id INTEGER PRIMARY KEY, label TEXT NOT NULL DEFAULT '');
                  INSERT INTO parent_new SELECT id, label FROM parent;
                  DROP TABLE parent;
                  ALTER TABLE parent_new RENAME TO parent;",
        };
        let db = open_with(&path, &[PARENT_CHILD, rebuild]).unwrap();
        let children: i64 = db
            .connection()
            .query_row("SELECT count(*) FROM child", [], |r| r.get(0))
            .unwrap();
        assert_eq!(children, 1);
        let on: i64 = db
            .connection()
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        assert_eq!(on, 1);
    }

    #[test]
    fn migration_that_breaks_references_is_rolled_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(&dir);
        create_parent_child_with_rows(&path);
        let before = snapshot(&Connection::open(&path).unwrap());
        let breaks = Migration {
            version: 2,
            name: "orphan a child",
            sql: "DELETE FROM parent;",
        };
        let err = open_with(&path, &[PARENT_CHILD, breaks])
            .err()
            .expect("migration should fail");
        assert!(
            matches!(err, StorageError::MigrationFailed { version: 2, .. }),
            "{err}"
        );
        assert_eq!(snapshot(&Connection::open(&path).unwrap()), before);
    }

    #[test]
    fn keeps_the_backup_of_a_non_empty_database_at_version_0() {
        // 番号の記録が無くても中身のあるデータベース（別の道具で作られたものなど）は、
        // 新しく作ったものとはみなさず、バックアップを残す
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(&dir);
        Connection::open(&path)
            .unwrap()
            .execute_batch("CREATE TABLE other (x); INSERT INTO other VALUES (1);")
            .unwrap();
        open_with(&path, &[V1]).unwrap();
        assert_eq!(backups(&dir).len(), 1);
    }

    #[test]
    fn newer_database_file_is_not_rewritten() {
        // 新しい版が別のジャーナルモードで書いたファイルでも、WAL に切り替える前に断る
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(&dir);
        Connection::open(&path)
            .unwrap()
            .execute_batch("CREATE TABLE x (a); PRAGMA user_version = 9;")
            .unwrap();
        let bytes = fs::read(&path).unwrap();
        let err = open_with(&path, &[V1])
            .err()
            .expect("should refuse a newer schema");
        assert!(matches!(
            err,
            StorageError::UnsupportedSchema { found: 9, .. }
        ));
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(!dir.path().join("yunagi.db-wal").exists());
    }

    #[test]
    fn negative_schema_version_is_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(&dir);
        Connection::open(&path)
            .unwrap()
            .execute_batch("PRAGMA user_version = -1;")
            .unwrap();
        let err = open_with(&path, &[V1]).err().expect("should refuse");
        assert!(
            matches!(err, StorageError::UnsupportedSchema { found: -1, .. }),
            "{err}"
        );
    }

    #[test]
    fn backups_are_single_files_and_stale_temp_files_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(&dir);
        create_v1_with_rows(&path);
        let stale = dir.path().join("backups/pre-migration-v1-to-v2-1.db.tmp");
        fs::create_dir_all(stale.parent().unwrap()).unwrap();
        fs::write(&stale, b"half written").unwrap();

        // 失敗して復元する（バックアップを開いて読む）ところまで通す
        open_with(&path, &[V1, V2, V3_FAILS])
            .err()
            .expect("migration should fail");

        let saved = backups(&dir);
        assert_eq!(saved.len(), 1, "{saved:?}");
        assert!(saved[0].extension().is_some_and(|e| e == "db"));
        let backup = Connection::open(&saved[0]).unwrap();
        let mode: String = backup
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "delete");
    }

    #[test]
    #[should_panic(expected = "migration versions")]
    fn rejects_out_of_order_migrations() {
        let dir = tempfile::tempdir().unwrap();
        let _ = open_with(&db_path(&dir), &[V2, V1]);
    }
}
