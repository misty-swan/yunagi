//! データベースのファイルと、SQLite がその横に作るファイル（`-wal`・`-shm`・`-journal`）。
//!
//! 横のファイルの名前の一覧と「データベースの跡があるか」の判定は、ここにだけ書く。
//! 開く（`open`）・保存先の変更（`data_dir`）・写す（`db_copy`）・マイグレーション（`migration`）・
//! バックアップ（`backup`）が使い、片方だけ直して食い違わないようにする。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// SQLite がデータベースの横に作るファイルの接尾辞。
///
/// - `-wal`：WAL モードの、本体にまだ書き戻していない書き込み。
/// - `-shm`：WAL の索引。SQLite が作り直せるので、データは持たない。
/// - `-journal`：通常の（DELETE）モードの、書き込みの途中で取り消すための元の内容。
pub(super) const SIDE_SUFFIXES: [&str; 3] = ["-wal", "-shm", "-journal"];

/// 横のファイルのうち、書き込みの途中の内容を持つもの（中身があれば、SQLite が本体に写し込む）。
const JOURNAL_SUFFIXES: [&str; 2] = ["-wal", "-journal"];

/// データベースのファイル `db` の横のファイル（`-wal`・`-shm`・`-journal`）のパス。
pub(super) fn side_files(db: &Path) -> [PathBuf; 3] {
    SIDE_SUFFIXES.map(|suffix| side_file(db, suffix))
}

/// データベースのファイル `db` と、その横のファイル。
pub(super) fn with_side_files(db: &Path) -> [PathBuf; 4] {
    let [wal, shm, journal] = side_files(db);
    [db.to_path_buf(), wal, shm, journal]
}

fn side_file(db: &Path, suffix: &str) -> PathBuf {
    let mut name = db.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// `db` に中身のあるデータベースがあるか（シンボリックリンクは辿る）。
///
/// 0 バイトのファイルは無いものとみなす（同期の途中で中身がまだ届いていないファイルや、
/// 初めて作る途中で止まったもの）。SQLite は 0 バイトのファイルを空のデータベースとして開くため、
/// 区別しないと空のデータベースを作るのと同じことになる。
/// 調べられない（権限が無いなど）ときは、そのエラーを返す（あるとも無いとも決めない）。
pub(super) fn has_database(db: &Path) -> io::Result<bool> {
    match fs::metadata(db) {
        Ok(m) => Ok(!(m.is_file() && m.len() == 0)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// `db` にデータベースを置いたことがある跡を探し、最初に見つけたもののパスを返す。
///
/// 跡とみなすもの（アーキテクチャ設計書 6章・6.3）：
///
/// - 中身のある `db`（0 バイトのものは [`has_database`] と同じく除く）。
/// - `db` のシンボリックリンク（宛先が無くても。作ると、SQLite がリンク先に作ってしまう）。
/// - 中身のある `-wal`・`-journal`（作ると、SQLite が新しいデータベースに写し込んでしまう。
///   0 バイトのものは SQLite が使わないので除く。`-shm` は SQLite が作り直すので見ない）。
/// - 調べられないもの（権限が無いなど。黙って作ったり上書きしたりしない）。
pub(super) fn find_trace(db: &Path) -> Option<PathBuf> {
    match fs::symlink_metadata(db) {
        Ok(m) if m.is_symlink() => return Some(db.to_path_buf()),
        Ok(m) if m.is_file() && m.len() == 0 => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        _ => return Some(db.to_path_buf()),
    }
    JOURNAL_SUFFIXES.iter().find_map(|suffix| {
        let side = side_file(db, suffix);
        match fs::symlink_metadata(&side) {
            Ok(m) if m.is_file() && m.len() == 0 => None,
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            _ => Some(side),
        }
    })
}

/// ファイル `db` と、その横のファイルを消す（無いものは飛ばす。消せなくても続ける）。
/// 作りかけの写しを片付けるときに使う。
pub(super) fn remove_with_side_files(db: &Path) {
    for path in with_side_files(db) {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_file_names() {
        let names = with_side_files(Path::new("/d/yunagi.db"));
        assert_eq!(
            names.map(|p| p.to_str().unwrap().to_owned()),
            [
                "/d/yunagi.db",
                "/d/yunagi.db-wal",
                "/d/yunagi.db-shm",
                "/d/yunagi.db-journal",
            ]
        );
    }

    #[test]
    fn traces_of_a_database() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("yunagi.db");
        assert_eq!(find_trace(&db), None);
        assert!(!has_database(&db).unwrap());

        // 0 バイトのものと -shm は跡ではない
        for path in with_side_files(&db) {
            fs::write(&path, b"").unwrap();
        }
        fs::write(side_file(&db, "-shm"), b"shm").unwrap();
        assert_eq!(find_trace(&db), None);
        assert!(!has_database(&db).unwrap());

        for suffix in ["-wal", "-journal"] {
            let side = side_file(&db, suffix);
            fs::write(&side, b"x").unwrap();
            assert_eq!(find_trace(&db), Some(side.clone()));
            fs::write(&side, b"").unwrap();
        }

        fs::write(&db, b"x").unwrap();
        assert_eq!(find_trace(&db), Some(db.clone()));
        assert!(has_database(&db).unwrap());

        remove_with_side_files(&db);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_a_trace_even_if_it_points_nowhere() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("yunagi.db");
        std::os::unix::fs::symlink(dir.path().join("missing/yunagi.db"), &db).unwrap();
        assert_eq!(find_trace(&db), Some(db.clone()));
        assert!(!has_database(&db).unwrap());
    }

    #[test]
    fn a_folder_in_place_of_the_database_is_a_trace() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("yunagi.db");
        fs::create_dir(&db).unwrap();
        assert_eq!(find_trace(&db), Some(db.clone()));
    }
}
