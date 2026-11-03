//! データベースを1つのファイルに写す手順と、フォルダへの書き込みの確定。
//!
//! マイグレーション前のバックアップ（`migration`）、保存先の変更（`data_dir`）、
//! 自動バックアップ（`backup`）が共通で使う。手順を食い違わせないよう、ここにだけ書く。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rusqlite::backup::NO_PROGRESS;
use rusqlite::{Connection, MAIN_DB};

use super::{StorageError, db_files};

/// [`copy`] が失敗した理由。どの場合も、作りかけの写しとその横のファイルは消してある。
#[derive(Debug)]
pub(super) enum CopyError {
    /// 写す・1つのファイルにまとめる・整合性を確かめる SQLite の操作が失敗した。
    Sqlite(rusqlite::Error),
    /// 写したファイル `path` の操作（ディスクへの反映）が失敗した。
    Io {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    /// 写したもの（`path` に作ろうとしたもの）の整合性の確認（`integrity_check`）が通らなかった。
    /// ふつうは元のデータベースが壊れている。`result` は確認の結果。
    Corrupted { path: PathBuf, result: String },
}

impl From<rusqlite::Error> for CopyError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

impl From<CopyError> for StorageError {
    fn from(e: CopyError) -> Self {
        match e {
            CopyError::Sqlite(e) => Self::Sqlite(e),
            CopyError::Io {
                action,
                path,
                source,
            } => Self::Io {
                action,
                path,
                source,
            },
            CopyError::Corrupted { path, result } => Self::BackupCorrupted { path, result },
        }
    }
}

/// 接続 `src` のデータベースを `to` に写し、`-wal` を伴わない1つのファイルにして、
/// 整合性を確かめてから、ディスクに反映する。`to` は新しく作るファイル（一時ファイル）にする。
///
/// 失敗したら、作りかけの `to` と、SQLite がその横に作ったファイル（`-journal` など）を消す。
/// `to` かその横のファイルがすでにあれば、何も書かずに断る（自分で作っていないものを消さないように）。
pub(super) fn copy(src: &Connection, to: &Path) -> Result<(), CopyError> {
    // 失敗したときに消すのは、ここで作ったものだけにする。すでにあるもの（とその横のファイル）には
    // 書かずに断る（呼び出し側は空いている名前を選ぶ）
    if db_files::any_exists(to) {
        return Err(CopyError::Io {
            action: "create",
            path: to.to_path_buf(),
            source: io::ErrorKind::AlreadyExists.into(),
        });
    }
    let result = copy_verified(src, to);
    if result.is_err() {
        db_files::remove_with_side_files(to);
    }
    result
}

fn copy_verified(src: &Connection, to: &Path) -> Result<(), CopyError> {
    // SQLite のオンラインバックアップ。WAL にだけある（まだ本体に書き戻していない）直近の
    // 書き込みも含めて写す。写している間にほかの接続が書き込むと、SQLite が写し直す。
    //
    // rusqlite 0.41 の Connection::backup は、SQLite が BUSY・LOCKED（使用中）を返すと、
    // 間を置いて再試行せずにそのままエラーを返す。ロックが取れないときに待つのは、
    // SQLite が元の接続の busy_timeout（rusqlite の既定で5秒）の間だけ。再試行は足さない：
    // 元は WAL モードなので、ほかの接続が書き込み中でも読むためのロックは待たずに取れ
    // （WAL の復旧中などで一時的に取れないときは busy_timeout の間待つ）、
    // ほかのプロセスは yunagi.lock で締め出している
    src.backup(MAIN_DB, to, NO_PROGRESS)?;
    // 写しは元の WAL モードを引き継ぐ。WAL のままだと、閉じた後に開くたび -wal・-shm が
    // 横にでき、名前を変えるのは本体だけなので取り残される。そこで通常の（DELETE）モードにして
    // 1つのファイルにまとめる（開くときに Database::open が WAL に戻す）
    let dst = Connection::open(to)?;
    let mode: String =
        dst.pragma_update_and_check(None, "journal_mode", "DELETE", |row| row.get(0))?;
    debug_assert_eq!(mode, "delete");
    // 写したものが壊れていない（元が壊れていない）ことを確かめる。壊れた写しは、
    // バックアップとしても移した先のデータベースとしても使えない
    let result: String = dst.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    drop(dst);
    if result != "ok" {
        return Err(CopyError::Corrupted {
            path: to.to_path_buf(),
            result,
        });
    }
    // 名前を変える前にディスクに反映する（電源が落ちても、写しが途中にならないように）。
    // Windows では書き込みできる形で開かないと反映できない
    fs::OpenOptions::new()
        .write(true)
        .open(to)
        .and_then(|f| f.sync_all())
        .map_err(|source| CopyError::Io {
            action: "sync",
            path: to.to_path_buf(),
            source,
        })
}

/// フォルダの中の名前の変更・追加をディスクに反映する（電源断で消えないように）。
/// Unix 系だけで行う（Windows はフォルダを開いて反映する方法を std が持たない）。
///
/// 自動バックアップ・保存先の変更・`config.json` の保存は、失敗しても続ける（`let _ =`）。
/// 名前の変更そのものは済んでいて、反映できなくても困るのは直後に電源が落ちたときだけのため。
/// マイグレーション前のバックアップは、これまでどおり失敗にする。
pub(super) fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    fs::File::open(dir)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::test_util::{corrupt_pages, names};
    use super::*;

    /// WAL モードで、本体に書き戻していない書き込みを持つデータベースを作る。
    fn wal_database(path: &Path) -> Connection {
        let conn = Connection::open(path).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        // 自動の書き戻しを止め、書き込みが -wal にだけある状態にする
        conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        conn.execute_batch("CREATE TABLE t (x INTEGER); INSERT INTO t VALUES (1), (2);")
            .unwrap();
        conn
    }

    fn side_files(path: &Path) -> Vec<PathBuf> {
        db_files::side_files(path)
            .into_iter()
            .filter(|p| p.exists())
            .collect()
    }

    /// 2ページ目以降を壊したデータベースを作る（見出しは残すので、SQLite のファイルとしては開ける）。
    fn corrupted_database(path: &Path) -> Connection {
        {
            let conn = Connection::open(path).unwrap();
            conn.execute_batch("CREATE TABLE t (x TEXT);").unwrap();
            for i in 0..200 {
                conn.execute("INSERT INTO t VALUES (?1)", [format!("row {i:0>100}")])
                    .unwrap();
            }
        }
        corrupt_pages(path);
        Connection::open(path).unwrap()
    }

    #[test]
    fn copies_wal_only_writes_into_a_single_file() {
        let dir = tempfile::tempdir().unwrap();
        let src = wal_database(&dir.path().join("src.db"));
        let to = dir.path().join("copy.db");

        copy(&src, &to).unwrap();

        assert_eq!(side_files(&to), Vec::<PathBuf>::new());
        let copied = Connection::open(&to).unwrap();
        let mode: String = copied
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "delete");
        let sum: i64 = copied
            .query_row("SELECT sum(x) FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sum, 3);
    }

    #[test]
    fn a_corrupted_copy_is_removed_with_its_side_files() {
        let dir = tempfile::tempdir().unwrap();
        let src = corrupted_database(&dir.path().join("src.db"));
        let to = dir.path().join("copy.db.tmp");

        // 壊れ方によっては、写す（読む）ところで SQLite が先に気づく。どちらでも写しは残らない
        let err = copy(&src, &to).unwrap_err();
        assert!(
            matches!(err, CopyError::Corrupted { ref path, .. } if *path == to)
                || matches!(err, CopyError::Sqlite(_)),
            "{err:?}"
        );
        assert_eq!(names(dir.path()), ["src.db"]);
    }

    #[test]
    fn a_failed_copy_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let src = wal_database(&dir.path().join("src.db"));
        // 写し先のフォルダが無いので、写すところで失敗する
        let to = dir.path().join("missing/copy.db.tmp");
        assert!(matches!(copy(&src, &to), Err(CopyError::Sqlite(_))));
        assert!(!dir.path().join("missing").exists());
        assert_eq!(names(dir.path()), ["src.db", "src.db-shm", "src.db-wal"]);
    }

    #[test]
    fn an_existing_target_is_not_overwritten_or_removed() {
        let dir = tempfile::tempdir().unwrap();
        let src = wal_database(&dir.path().join("src.db"));
        let to = dir.path().join("copy.db.tmp");
        let journal = db_files::side_files(&to)[2].clone();
        fs::write(&journal, b"someone else's").unwrap();
        let err = copy(&src, &to).unwrap_err();
        assert!(
            matches!(
                err,
                CopyError::Io {
                    action: "create",
                    ..
                }
            ),
            "{err:?}"
        );
        assert_eq!(fs::read(&journal).unwrap(), b"someone else's");
        assert!(!to.exists());
    }

    #[test]
    fn copies_without_waiting_while_another_connection_is_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("src.db");
        drop(wal_database(&path));
        let writer = Connection::open(&path).unwrap();
        writer
            .execute_batch("BEGIN IMMEDIATE; INSERT INTO t VALUES (100);")
            .unwrap();
        // 待たない設定にする。ロックを待つ必要があれば BUSY で失敗する
        let src = Connection::open(&path).unwrap();
        src.busy_timeout(Duration::ZERO).unwrap();
        let to = dir.path().join("copy.db");

        // WAL では書き込み中でも読むためのロックが取れる
        copy(&src, &to).unwrap();

        // 確定していない書き込みは写らない
        let sum: i64 = Connection::open(&to)
            .unwrap()
            .query_row("SELECT sum(x) FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sum, 3);
    }
}
