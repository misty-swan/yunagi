//! `agent setup` が使うファイル操作：実行ファイルの検索、バックアップ、壊れにくい書き込み。
//!
//! どれも、渡されたパスだけを触る（環境変数は読まない）。試験は一時フォルダで確かめる。

use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use yunagi_core::storage::data_dir::Platform;

/// `PATH` の中から実行ファイル `name` を探す。Windows は `.exe`・`.cmd`・`.bat` も試す。
pub fn find_in_path(name: &str, path: Option<&OsStr>, platform: Platform) -> Option<PathBuf> {
    let suffixes: &[&str] = if platform == Platform::Windows {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    std::env::split_paths(path?)
        .filter(|dir| dir.is_absolute())
        .flat_map(|dir| suffixes.iter().map(move |s| dir.join(format!("{name}{s}"))))
        .find(|candidate| is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file())
}

/// `path` の隣に、`<ファイル名>.yunagi-backup-<日時>` として `bytes` を書く（05 6章）。すでにある
/// ファイルは上書きせず、同じ名前があれば `-2`・`-3`… を付けて新しく作る。元のファイルの権限を引き継ぐ。
/// 作ったバックアップのパスを返す。
pub fn create_backup(path: &Path, bytes: &[u8], stamp: &str) -> io::Result<PathBuf> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?
        .to_string_lossy()
        .into_owned();
    let permissions = fs::metadata(path).ok().map(|m| m.permissions());
    for n in 1..=999 {
        let suffix = if n == 1 {
            String::new()
        } else {
            format!("-{n}")
        };
        let backup = path.with_file_name(format!("{name}.yunagi-backup-{stamp}{suffix}"));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&backup)
        {
            Ok(mut file) => {
                let written = file.write_all(bytes).and_then(|()| file.sync_all());
                drop(file);
                if let Err(e) = written {
                    let _ = fs::remove_file(&backup);
                    return Err(e);
                }
                if let Some(permissions) = permissions {
                    let _ = fs::set_permissions(&backup, permissions);
                }
                return Ok(backup);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "too many backups with the same timestamp",
    ))
}

/// `target` に `bytes` を書く。同じフォルダの一時ファイルに書いてから名前を変えるので、途中で失敗しても
/// 元のファイルは壊れない。`target` がシンボリックリンクなら、リンクを置き換えず、指す先の実体に書く
/// （dotfiles の管理でよくある形）。すでにあるファイルの権限を引き継ぐ。フォルダが無ければ作る。
pub fn write_atomic(target: &Path, bytes: &[u8]) -> io::Result<()> {
    let real = match fs::canonicalize(target) {
        Ok(real) => real,
        Err(e) if e.kind() == io::ErrorKind::NotFound => target.to_owned(),
        Err(e) => return Err(e),
    };
    let dir = real
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no parent folder"))?;
    fs::create_dir_all(dir)?;
    let name = real
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?
        .to_string_lossy()
        .into_owned();
    let tmp = dir.join(format!(".{name}.yunagi-tmp-{}", std::process::id()));
    let permissions = fs::metadata(&real).ok().map(|m| m.permissions());
    let result = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        if let Some(permissions) = permissions {
            fs::set_permissions(&tmp, permissions)?;
        }
        fs::rename(&tmp, &real)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn finds_only_executable_files_in_absolute_path_entries() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("claude"), "#!/bin/sh\n").unwrap();
        fs::write(b.join("claude"), "#!/bin/sh\n").unwrap();
        make_executable(&b.join("claude"));
        let path = std::env::join_paths([&a, &b]).unwrap();
        // 実行できないものは飛ばし、後ろの実行できるものを返す
        let found = find_in_path("claude", Some(&path), Platform::Linux).unwrap();
        assert_eq!(found, b.join("claude"));
        assert_eq!(find_in_path("codex", Some(&path), Platform::Linux), None);
        assert_eq!(find_in_path("claude", None, Platform::Linux), None);
        // 相対パスの項目（カレントディレクトリ）は探さない
        let relative = std::ffi::OsString::from("a:b");
        assert_eq!(
            find_in_path("claude", Some(&relative), Platform::Linux),
            None
        );
    }

    #[test]
    fn backups_never_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.toml");
        fs::write(&file, "x = 1\n").unwrap();
        let first = create_backup(&file, b"x = 1\n", "20261104-120000").unwrap();
        let second = create_backup(&file, b"x = 2\n", "20261104-120000").unwrap();
        assert_eq!(
            first.file_name().unwrap(),
            "config.toml.yunagi-backup-20261104-120000"
        );
        assert_eq!(
            second.file_name().unwrap(),
            "config.toml.yunagi-backup-20261104-120000-2"
        );
        assert_eq!(fs::read(&first).unwrap(), b"x = 1\n");
        assert_eq!(fs::read(&second).unwrap(), b"x = 2\n");
        assert_eq!(first.parent(), file.parent());
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_keeps_permissions_and_follows_symlinks() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let real_dir = dir.path().join("dotfiles");
        fs::create_dir_all(&real_dir).unwrap();
        let real = real_dir.join("config.toml");
        fs::write(&real, "old\n").unwrap();
        fs::set_permissions(&real, fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.path().join("config.toml");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write_atomic(&link, b"new\n").unwrap();
        // リンクはそのまま、実体が新しくなり、権限も残る
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&real).unwrap(), b"new\n");
        assert_eq!(
            fs::metadata(&real).unwrap().permissions().mode() & 0o777,
            0o600
        );
        // 一時ファイルは残らない
        let names: Vec<_> = fs::read_dir(&real_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["config.toml"]);
    }

    #[test]
    fn atomic_write_creates_missing_folders() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a/b/config.toml");
        write_atomic(&file, b"x\n").unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"x\n");
    }
}
