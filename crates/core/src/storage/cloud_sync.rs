//! 保存先がクラウド同期フォルダかの判定（docs/design/03-ui-spec.md 7章、
//! docs/design/01-architecture.md 6章）。
//!
//! 同期フォルダにデータベースを置くと、データベースと WAL のファイルが別々に同期されて
//! 壊れうる。そこで保存先を設定するときに判定し、同期フォルダなら画面で警告する
//! （保存は止めない）。判定は次の2つで行い、どちらにも当たらなければ「判定できない」とする。
//!
//! - 既知の場所：ホームフォルダの下の、各サービスの既定のフォルダ（[`KnownLocations`]）。
//! - 目印：各サービスが同期フォルダの一番上に置くファイル・フォルダ。保存先とその親を順に見る。

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Component, Path, PathBuf};

/// クラウド同期のサービス。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudService {
    ICloudDrive,
    Dropbox,
    OneDrive,
    GoogleDrive,
}

/// 既知の場所を求めるための、ホームフォルダと OneDrive のフォルダ。
#[derive(Debug, Clone, Default)]
pub struct KnownLocations {
    /// ホームフォルダ。
    pub home: Option<PathBuf>,
    /// OneDrive のフォルダ（Windows の環境変数 `OneDrive`・`OneDriveCommercial`・
    /// `OneDriveConsumer` が指す場所）。
    pub onedrive: Vec<PathBuf>,
}

impl KnownLocations {
    /// 環境変数から求める。ホームフォルダは `HOME`、無ければ `USERPROFILE`（Windows）。
    /// 空の値と相対パスは無いものとする。`env` はテストで環境変数を差し替えるために受け取る。
    pub fn from_env(env: impl Fn(&str) -> Option<OsString>) -> Self {
        let absolute = |name: &str| {
            env(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
        };
        Self {
            home: absolute("HOME").or_else(|| absolute("USERPROFILE")),
            onedrive: ["OneDrive", "OneDriveCommercial", "OneDriveConsumer"]
                .into_iter()
                .filter_map(absolute)
                .collect(),
        }
    }
}

/// 保存先 `dir` がクラウド同期フォルダ（の中）かを判定する。判定できなければ `None`。
/// `dir` はまだ無くてもよい（あるところまでの親で判定する）。
pub fn detect_cloud_sync(dir: &Path) -> Option<CloudService> {
    detect_cloud_sync_with(dir, &KnownLocations::from_env(|n| std::env::var_os(n)))
}

/// [`detect_cloud_sync`] の、既知の場所を指定する版。
pub fn detect_cloud_sync_with(dir: &Path, known: &KnownLocations) -> Option<CloudService> {
    let dir = resolve(dir);
    by_location(&dir, known).or_else(|| by_marker(&dir))
}

/// シンボリックリンクを辿った実体のパス（macOS では `~/Dropbox` が `~/Library/CloudStorage/…`
/// へのリンクのことがある）。無い部分は、あるところまでの親の実体に、残りをつなげる。
fn resolve(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut cur = path;
    loop {
        if let Ok(real) = fs::canonicalize(cur) {
            return rest.iter().rev().fold(real, |p, c: &&OsStr| p.join(c));
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

/// 既知の場所で判定する。
fn by_location(dir: &Path, known: &KnownLocations) -> Option<CloudService> {
    // どこにあっても iCloud Drive の中を示す名前（macOS の iCloud Drive の実体のフォルダ）
    if dir
        .components()
        .any(|c| c.as_os_str() == OsStr::new("com~apple~CloudDocs"))
    {
        return Some(CloudService::ICloudDrive);
    }
    for onedrive in &known.onedrive {
        if dir.starts_with(resolve(onedrive)) {
            return Some(CloudService::OneDrive);
        }
    }
    let home = resolve(known.home.as_deref()?);
    let rel = dir.strip_prefix(&home).ok()?;
    let names: Vec<&str> = rel
        .components()
        .take(3)
        .map_while(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    // macOS と Windows のファイル名は大文字と小文字を区別しないので、区別せずに比べる
    let is = |s: &str, name: &str| s.eq_ignore_ascii_case(name);
    let starts = |s: &str, prefix: &str| {
        s.len() >= prefix.len()
            && s.is_char_boundary(prefix.len())
            && is(&s[..prefix.len()], prefix)
    };
    match names.as_slice() {
        // macOS：~/Library/Mobile Documents（iCloud Drive）、
        // ~/Library/CloudStorage/<サービス>-<アカウント>（File Provider を使う同期サービス）
        [lib, md, ..] if is(lib, "Library") && is(md, "Mobile Documents") => {
            Some(CloudService::ICloudDrive)
        }
        [lib, cs, name, ..] if is(lib, "Library") && is(cs, "CloudStorage") => {
            if starts(name, "Dropbox") {
                Some(CloudService::Dropbox)
            } else if starts(name, "OneDrive") {
                Some(CloudService::OneDrive)
            } else if starts(name, "GoogleDrive") {
                Some(CloudService::GoogleDrive)
            } else if starts(name, "iCloud") {
                Some(CloudService::ICloudDrive)
            } else {
                None
            }
        }
        // Windows：~/iCloudDrive
        [first, ..] if is(first, "iCloudDrive") => Some(CloudService::ICloudDrive),
        // ~/Dropbox、~/Dropbox (Personal) など（チームの Dropbox は括弧つきの名前）
        [first, ..] if is(first, "Dropbox") || starts(first, "Dropbox (") => {
            Some(CloudService::Dropbox)
        }
        // ~/OneDrive、~/OneDrive - <組織名>（職場・学校のアカウント）
        [first, ..] if is(first, "OneDrive") || starts(first, "OneDrive - ") => {
            Some(CloudService::OneDrive)
        }
        // ~/Google Drive（旧来の同期アプリ）
        [first, ..] if is(first, "Google Drive") => Some(CloudService::GoogleDrive),
        _ => None,
    }
}

/// 目印で判定する。`dir` とその親を順に見る。
fn by_marker(dir: &Path) -> Option<CloudService> {
    dir.ancestors().find_map(marker_in)
}

/// フォルダ `dir` の直下に、同期フォルダの一番上を示す目印があるか。
fn marker_in(dir: &Path) -> Option<CloudService> {
    let is_file = |name: &str| dir.join(name).is_file();
    let is_dir = |name: &str| dir.join(name).is_dir();
    // Dropbox の同期フォルダの一番上の `.dropbox` はファイル。ホームフォルダの `~/.dropbox` は
    // Dropbox の設定を入れるフォルダなので、ファイルに限る（ホーム全体を同期フォルダとしないため）
    if is_file(".dropbox") || is_dir(".dropbox.cache") {
        return Some(CloudService::Dropbox);
    }
    // OneDrive が同期フォルダの一番上に置く隠しファイル
    if is_file(".849C9593-D756-4E56-8D6E-42412F2A707B") {
        return Some(CloudService::OneDrive);
    }
    // Google ドライブ（パソコン版ドライブ）が同期フォルダの一番上に置く作業用フォルダ
    if is_dir(".tmp.drivedownload") || is_dir(".tmp.driveupload") {
        return Some(CloudService::GoogleDrive);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    fn known(home: &Path) -> KnownLocations {
        KnownLocations {
            home: Some(home.to_path_buf()),
            onedrive: Vec::new(),
        }
    }

    fn detect(home: &Path, rel: &str) -> Option<CloudService> {
        detect_cloud_sync_with(&home.join(rel), &known(home))
    }

    #[test]
    fn known_locations_under_home() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path();
        use CloudService::*;
        let cases = [
            ("Dropbox/Yunagi", Some(Dropbox)),
            ("Dropbox (Personal)/Yunagi", Some(Dropbox)),
            ("dropbox", Some(Dropbox)),
            ("OneDrive/Documents/Yunagi", Some(OneDrive)),
            ("OneDrive - Example Corp/Yunagi", Some(OneDrive)),
            ("Google Drive/Yunagi", Some(GoogleDrive)),
            ("iCloudDrive/Yunagi", Some(ICloudDrive)),
            (
                "Library/Mobile Documents/com~apple~CloudDocs/Yunagi",
                Some(ICloudDrive),
            ),
            (
                "Library/CloudStorage/Dropbox-Personal/Yunagi",
                Some(Dropbox),
            ),
            (
                "Library/CloudStorage/OneDrive-Personal/Yunagi",
                Some(OneDrive),
            ),
            (
                "Library/CloudStorage/GoogleDrive-a@example.com/My Drive/Yunagi",
                Some(GoogleDrive),
            ),
            ("Library/CloudStorage/Box-Box/Yunagi", None),
            ("Library/Application Support/Yunagi", None),
            ("Documents/Yunagi", None),
            ("Dropboxes/Yunagi", None),
            ("OneDriveBackup/Yunagi", None),
            ("", None),
        ];
        for (rel, expected) in cases {
            assert_eq!(detect(home, rel), expected, "{rel}");
        }
    }

    #[test]
    fn outside_home_is_not_a_known_location() {
        let home = TempDir::new().unwrap();
        let other = TempDir::new().unwrap();
        let dir = other.path().join("Dropbox/Yunagi");
        assert_eq!(detect_cloud_sync_with(&dir, &known(home.path())), None);
        assert_eq!(
            detect_cloud_sync_with(&dir, &KnownLocations::default()),
            None
        );
    }

    #[test]
    fn onedrive_environment_variable() {
        let tmp = TempDir::new().unwrap();
        let onedrive = tmp.path().join("CorpSync");
        let locations = KnownLocations {
            home: None,
            onedrive: vec![onedrive.clone()],
        };
        assert_eq!(
            detect_cloud_sync_with(&onedrive.join("Yunagi"), &locations),
            Some(CloudService::OneDrive)
        );
        assert_eq!(
            detect_cloud_sync_with(&tmp.path().join("CorpSyncOther"), &locations),
            None
        );
    }

    #[test]
    fn from_env_reads_variables() {
        let abs = if cfg!(windows) { r"C:\h" } else { "/h" };
        let od = if cfg!(windows) { r"C:\od" } else { "/od" };
        let locations = KnownLocations::from_env(|name| match name {
            "USERPROFILE" => Some(OsString::from(abs)),
            "HOME" => Some(OsString::new()),
            "OneDriveCommercial" => Some(OsString::from(od)),
            "OneDrive" => Some(OsString::from("relative")),
            _ => None,
        });
        assert_eq!(locations.home, Some(PathBuf::from(abs)));
        assert_eq!(locations.onedrive, vec![PathBuf::from(od)]);
    }

    #[test]
    fn markers_in_parent_folders() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let none = KnownLocations::default();
        let check = |marker: &str, is_dir: bool, expected| {
            let sync_root = root.join(format!("root-{}", marker.trim_start_matches('.')));
            fs::create_dir_all(&sync_root).unwrap();
            if is_dir {
                fs::create_dir(sync_root.join(marker)).unwrap();
            } else {
                fs::write(sync_root.join(marker), b"").unwrap();
            }
            // 保存先がまだ無くても、親の目印で判定する
            let dir = sync_root.join("work/Yunagi");
            assert_eq!(
                detect_cloud_sync_with(&dir, &none),
                Some(expected),
                "{marker}"
            );
        };
        check(".dropbox", false, CloudService::Dropbox);
        check(".dropbox.cache", true, CloudService::Dropbox);
        check(
            ".849C9593-D756-4E56-8D6E-42412F2A707B",
            false,
            CloudService::OneDrive,
        );
        check(".tmp.drivedownload", true, CloudService::GoogleDrive);
        check(".tmp.driveupload", true, CloudService::GoogleDrive);
    }

    #[test]
    fn dropbox_settings_folder_in_home_is_not_a_marker() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path();
        fs::create_dir(home.join(".dropbox")).unwrap();
        assert_eq!(detect(home, "Documents/Yunagi"), None);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_to_cloud_storage_is_followed() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path();
        let real = home.join("Library/CloudStorage/Dropbox");
        fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, home.join("Sync")).unwrap();
        assert_eq!(detect(home, "Sync/Yunagi"), Some(CloudService::Dropbox));
    }
}
