//! データベースを1つのファイルに写す手順と、フォルダへの書き込みの確定。
//!
//! マイグレーション前のバックアップ（`migration`）、保存先の変更（`data_dir`）、
//! 自動バックアップ（`backup`）が共通で使う。手順を食い違わせないよう、ここにだけ書く。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rusqlite::backup::NO_PROGRESS;
use rusqlite::{Connection, MAIN_DB};

use super::StorageError;

/// [`copy`] が失敗した理由。
#[derive(Debug)]
pub(super) enum CopyError {
    /// 写す・1つのファイルにまとめる SQLite の操作が失敗した。
    Sqlite(rusqlite::Error),
    /// 写したファイル `path` をディスクに反映できなかった。
    Sync { path: PathBuf, source: io::Error },
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
            CopyError::Sync { source, .. } => Self::Io(source),
        }
    }
}

/// 接続 `src` のデータベースを `to` に写し、`-wal` を伴わない1つのファイルにして、
/// ディスクに反映する。`to` は新しく作るファイル（一時ファイル）にする。
pub(super) fn copy(src: &Connection, to: &Path) -> Result<(), CopyError> {
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
    drop(dst);
    // 名前を変える前にディスクに反映する（電源が落ちても、写しが途中にならないように）。
    // Windows では書き込みできる形で開かないと反映できない
    fs::OpenOptions::new()
        .write(true)
        .open(to)
        .and_then(|f| f.sync_all())
        .map_err(|source| CopyError::Sync {
            path: to.to_path_buf(),
            source,
        })
}

/// フォルダの中の名前の変更・追加をディスクに反映する（電源断で消えないように）。
/// Unix 系だけで行う（Windows はフォルダを開いて反映する方法を std が持たない）。
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
        ["-wal", "-shm", "-journal"]
            .iter()
            .map(|s| PathBuf::from(format!("{}{s}", path.display())))
            .filter(|p| p.exists())
            .collect()
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
