//! `yunagi uninstall` の削除の層：消すものは必ず [`Report::remove`] などを通す。
//!
//! 通す前に [`Guard`] が、消してよいパスかを確かめる。本番では「絶対パスで、`..` を含まず、ルートや
//! 利用者のホームそのものではない」ことを見る。試験では `confine_to` に一時フォルダを入れ、その外のパスを
//! 消そうとしたら、消さずに panic する（試験が利用者の実際の環境を消すことを防ぐ。YUN-99）。
//!
//! シンボリックリンクは、リンクだけを消して先には触れない（アーキテクチャ設計書 6.5）。

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// 消してよいパスかを確かめる決まり。
#[derive(Debug, Clone, Default)]
pub struct Guard {
    /// 試験で使う：これの下（これ自身は含まない）のパスしか消さない。外のパスは消さずに panic する。
    /// 本番では `None`。
    pub confine_to: Option<PathBuf>,
    /// 消さないフォルダ（利用者のホームなど）。
    pub protected: Vec<PathBuf>,
}

impl Guard {
    /// `path` を消してよいか。だめなら理由を返す。
    ///
    /// # Panics
    ///
    /// `confine_to` を指定していて、`path` がその外にあるとき。
    pub fn check(&self, path: &Path) -> Result<(), String> {
        if !path.is_absolute() {
            return Err("not an absolute path".to_owned());
        }
        if path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err("the path contains . or ..".to_owned());
        }
        if path.parent().is_none() {
            return Err("this is the root of a drive".to_owned());
        }
        // 守るフォルダ自身と、その上のフォルダ（`/home` など）は消さない。macOS・Windows は大文字小文字を
        // 区別しないファイルシステムが多いので、区別せずに比べる
        let key = |p: &Path| {
            if cfg!(any(windows, target_os = "macos")) {
                PathBuf::from(p.to_string_lossy().to_lowercase())
            } else {
                p.to_path_buf()
            }
        };
        let here = key(path);
        if self.protected.iter().any(|p| key(p).starts_with(&here)) {
            return Err(
                "this folder, or a folder that holds a protected folder, is never removed"
                    .to_owned(),
            );
        }
        if let Some(root) = &self.confine_to {
            let root = resolve_existing(root);
            let here = resolved_location(path);
            assert!(
                here != root && here.starts_with(&root),
                "test guard: {} is outside the test folder {}; refusing to remove it",
                path.display(),
                root.display()
            );
        }
        Ok(())
    }
}

/// `.` を除き、`..` を文字の上でたたんだパス（ファイルシステムは見ない）。
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// 存在するところまでの祖先をシンボリックリンクを辿った実体にして、残りをつなげたパス。
fn resolve_existing(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut cur = path;
    loop {
        if let Ok(real) = fs::canonicalize(cur) {
            return rest
                .iter()
                .rev()
                .fold(real, |p, c: &&std::ffi::OsStr| p.join(c));
        }
        match (cur.parent(), cur.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name);
                cur = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// 消す対象の場所。親だけを実体にし、最後の要素は辿らない（それ自身がシンボリックリンクでも、リンクの
/// 場所を指す）。
fn resolved_location(path: &Path) -> PathBuf {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => resolve_existing(parent).join(name),
        _ => path.to_path_buf(),
    }
}

/// 消したものと、消せなかったものの記録。
#[derive(Debug, Default)]
pub struct Report {
    pub removed: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
    /// 一覧の後ろに添える知らせ。
    pub notes: Vec<String>,
}

impl Report {
    /// `path` を消す（フォルダは中身ごと）。無ければ何もしない。消せなくても止まらず、記録する。
    pub fn remove(&mut self, guard: &Guard, path: &Path) {
        if let Err(why) = guard.check(path) {
            self.failed.push((path.to_path_buf(), why));
            return;
        }
        let meta = match fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return,
            Err(e) => {
                self.failed.push((path.to_path_buf(), e.to_string()));
                return;
            }
        };
        let result = if meta.is_dir() {
            fs::remove_dir_all(path)
        } else {
            remove_file_or_link(path)
        };
        self.record(path, result);
    }

    /// 空のフォルダ `path` を消す（中身があれば消さない）。シンボリックリンクはリンクだけを消す。
    pub fn remove_empty_dir(&mut self, guard: &Guard, path: &Path) {
        if let Err(why) = guard.check(path) {
            self.failed.push((path.to_path_buf(), why));
            return;
        }
        let meta = match fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return,
            Err(e) => {
                self.failed.push((path.to_path_buf(), e.to_string()));
                return;
            }
        };
        let result = if meta.is_dir() {
            fs::remove_dir(path)
        } else {
            remove_file_or_link(path)
        };
        self.record(path, result);
    }

    /// フォルダ `dir` の中身を、名前が `keep` のもの以外すべて消す（`dir` 自身は消さない）。`dir` が
    /// シンボリックリンクのときは何もしない（先には触れない）。
    pub fn remove_contents_except(&mut self, guard: &Guard, dir: &Path, keep: &str) {
        if let Err(why) = guard.check(dir) {
            self.failed.push((dir.to_path_buf(), why));
            return;
        }
        match fs::symlink_metadata(dir) {
            Ok(meta) if meta.is_dir() => {}
            _ => return,
        }
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                self.failed.push((dir.to_path_buf(), e.to_string()));
                return;
            }
        };
        for entry in entries {
            match entry {
                Ok(entry) if entry.file_name() == keep => {}
                Ok(entry) => self.remove(guard, &entry.path()),
                Err(e) => self.failed.push((dir.to_path_buf(), e.to_string())),
            }
        }
    }

    /// 消せなかったものを記録する。
    pub fn fail(&mut self, path: &Path, why: impl Into<String>) {
        self.failed.push((path.to_path_buf(), why.into()));
    }

    fn record(&mut self, path: &Path, result: io::Result<()>) {
        match result {
            Ok(()) => self.removed.push(path.to_path_buf()),
            // 消している間に無くなった（別のプロセスが消した）なら、消えているので成功とみなす
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => self.failed.push((path.to_path_buf(), e.to_string())),
        }
    }
}

/// ファイルかシンボリックリンクを消す。Windows のフォルダへのリンクは `remove_file` では消せないので、
/// 失敗したときに `remove_dir` を試す（リンクだけが消え、先には触れない）。
fn remove_file_or_link(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) => {
            fs::remove_dir(path).map_err(|_| e)
        }
        other => other,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn guard(root: &Path) -> Guard {
        Guard {
            confine_to: Some(root.to_path_buf()),
            protected: Vec::new(),
        }
    }

    #[test]
    fn removes_files_folders_and_only_the_link_of_a_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("area");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(root.join("dir/sub")).unwrap();
        fs::write(root.join("dir/sub/f"), "x").unwrap();
        fs::write(root.join("file"), "x").unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep"), "x").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        let guard = guard(&root);
        let mut report = Report::default();
        for name in ["dir", "file", "link", "missing"] {
            report.remove(&guard, &root.join(name));
        }
        assert!(report.failed.is_empty(), "{:?}", report.failed);
        assert_eq!(report.removed.len(), 3, "missing paths are skipped");
        assert!(!root.join("dir").exists() && !root.join("link").exists());
        assert!(
            outside.join("keep").exists(),
            "the link target is untouched"
        );
    }

    #[test]
    fn keeps_the_named_entry_and_does_not_follow_a_linked_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("area");
        let real = tmp.path().join("real");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("f"), "x").unwrap();
        fs::write(root.join("yunagi.lock"), "").unwrap();
        fs::write(root.join("config.json"), "{}").unwrap();
        let link = tmp.path().join("area-link");
        std::os::unix::fs::symlink(&root, &link).unwrap();
        let guard = guard(tmp.path());
        let mut report = Report::default();
        report.remove_contents_except(&guard, &root, "yunagi.lock");
        assert!(root.join("yunagi.lock").exists() && !root.join("config.json").exists());
        // 先のフォルダがシンボリックリンクなら、中身には触れない
        let other = tmp.path().join("other");
        std::os::unix::fs::symlink(&real, &other).unwrap();
        report.remove_contents_except(&guard, &other, "yunagi.lock");
        assert!(real.join("f").exists());
        assert!(report.failed.is_empty(), "{:?}", report.failed);
    }

    #[test]
    fn empty_dir_removal_leaves_a_folder_with_content() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("d");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("f"), "x").unwrap();
        let guard = guard(tmp.path());
        let mut report = Report::default();
        report.remove_empty_dir(&guard, &dir);
        assert!(dir.exists());
        assert_eq!(report.failed.len(), 1);
        fs::remove_file(dir.join("f")).unwrap();
        report.remove_empty_dir(&guard, &dir);
        assert!(!dir.exists());
    }

    #[test]
    #[should_panic(expected = "test guard")]
    fn the_test_guard_panics_before_removing_outside_the_area() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("area");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let mut report = Report::default();
        report.remove(&guard(&root), &outside);
    }

    #[test]
    #[should_panic(expected = "test guard")]
    fn the_test_guard_refuses_the_area_itself() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("area");
        fs::create_dir_all(&root).unwrap();
        let mut report = Report::default();
        report.remove(&guard(&root), &root);
    }

    #[test]
    fn the_production_guard_refuses_unsafe_paths() {
        let guard = Guard {
            confine_to: None,
            protected: vec![PathBuf::from("/home/a")],
        };
        for bad in ["relative/x", "/", "/home", "/home/a", "/home/a/../b"] {
            assert!(guard.check(Path::new(bad)).is_err(), "{bad}");
        }
        assert_eq!(
            normalize(Path::new("/a/b/../c/./d")),
            PathBuf::from("/a/c/d")
        );
        assert!(guard.check(Path::new("/home/a/.local/opt/yunagi")).is_ok());
    }
}
