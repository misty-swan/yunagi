//! `yunagi uninstall` が消す場所を求める：どの導入か（ビルド・配布手順書 4.4）、インストールスクリプトが
//! 置くもの（同 4.1・4.2）、WebView のデータの場所（アーキテクチャ設計書 6.5）。
//!
//! どれも、OS の区分（[`Platform`]）と環境変数の組から求める副作用の無い関数にして、Linux でも
//! macOS・Windows の場所を試験する。

use std::fs;
use std::path::{Path, PathBuf};

use yunagi_core::storage::data_dir::{self, APP_IDENTIFIER, DataDirError, Platform};

use crate::error::{self, ErrorObject};

use super::Setup;

/// どの導入か。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// 正規の置き場所（インストールスクリプトが置いた場所）。
    Standard(Layout),
    /// Homebrew の Cask で入れたもの。
    Homebrew,
    /// どちらでもない。見つけた実行ファイルのパス。
    Unmanaged(PathBuf),
}

/// 正規の置き場所に置かれたもの。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// アプリ本体の置き場所：macOS は `~/Applications/Yunagi.app`、Linux は `~/.local/opt/yunagi`、
    /// Windows は `%LOCALAPPDATA%\Programs\Yunagi`。
    pub root: PathBuf,
    /// `PATH` に置いた `yunagi` へのリンク（macOS・Linux の `~/.local/bin/yunagi`）。
    pub link: Option<PathBuf>,
    /// Linux のデスクトップエントリ。
    pub desktop_entry: Option<PathBuf>,
}

/// 環境変数 `name` の値（空は未設定として扱う）。
fn var(setup: &Setup, name: &str) -> Option<std::ffi::OsString> {
    setup.vars.get(name).filter(|v| !v.is_empty()).cloned()
}

/// 絶対パスの環境変数。無い・相対パスならエラー。
fn absolute_var(setup: &Setup, name: &'static str) -> Result<PathBuf, ErrorObject> {
    var(setup, name)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| {
            error::not_found(format!(
                "Could not find where Yunagi is installed because the environment variable {name} is not set to an absolute path. Nothing was removed."
            ))
        })
}

fn data_dir_error(setup: &Setup, e: &DataDirError) -> ErrorObject {
    match e {
        DataDirError::NoDefaultDir { var } => error::default_dir_unset(&[var]),
        _ => error::default_dir_unset(yunagi_core::storage::dir_mismatch::default_dir_vars(
            setup.platform,
        )),
    }
}

/// 実行ファイルの実体が、どの導入に当たるか。
pub fn classify(setup: &Setup) -> Result<Kind, ErrorObject> {
    let exe = setup.exe.clone().ok_or_else(|| {
        error::internal("Could not find the path of this program. Nothing was removed.")
    })?;
    // リンクを解いた実体で見分ける
    let exe = fs::canonicalize(&exe).unwrap_or(exe);
    let layout = standard_layout(setup)?;
    if is_inside(&exe, &layout.root, setup.platform) {
        return Ok(Kind::Standard(layout));
    }
    if setup.platform == Platform::MacOs && is_homebrew(setup, &exe) {
        return Ok(Kind::Homebrew);
    }
    Ok(Kind::Unmanaged(exe))
}

/// 正規の置き場所（インストールスクリプトが置く場所。ビルド・配布手順書 4.1・4.2）。
pub fn standard_layout(setup: &Setup) -> Result<Layout, ErrorObject> {
    match setup.platform {
        Platform::Linux => {
            let home = absolute_var(setup, "HOME")?;
            let data_root = data_dir::local_data_root_for(Platform::Linux, |n| var(setup, n))
                .map_err(|e| data_dir_error(setup, &e))?;
            Ok(Layout {
                root: home.join(".local/opt/yunagi"),
                link: Some(home.join(".local/bin/yunagi")),
                desktop_entry: Some(
                    data_root
                        .join("applications")
                        .join(format!("{APP_IDENTIFIER}.desktop")),
                ),
            })
        }
        Platform::MacOs => {
            let home = absolute_var(setup, "HOME")?;
            Ok(Layout {
                root: home.join("Applications/Yunagi.app"),
                link: Some(home.join(".local/bin/yunagi")),
                desktop_entry: None,
            })
        }
        Platform::Windows => {
            let local = absolute_var(setup, "LOCALAPPDATA")?;
            Ok(Layout {
                root: local.join("Programs").join("Yunagi"),
                link: None,
                desktop_entry: None,
            })
        }
    }
}

/// Homebrew の Cask か：実体が `/Applications/Yunagi.app/` の中にあり、Caskroom の `yunagi` がある。
fn is_homebrew(setup: &Setup, exe: &Path) -> bool {
    let app = setup.applications_dir.join("Yunagi.app");
    let app = fs::canonicalize(&app).unwrap_or(app);
    is_inside(exe, &app, setup.platform)
        && setup
            .brew_prefixes
            .iter()
            .any(|prefix| fs::symlink_metadata(prefix.join("Caskroom/yunagi")).is_ok())
}

/// `path` が `root` の中（`root` 自身は含まない）にあるか。リンクを解いた実体で比べる。macOS・Windows は
/// 大文字小文字を区別しないファイルシステムが多いので、区別しない。
fn is_inside(path: &Path, root: &Path, platform: Platform) -> bool {
    let root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    if platform == Platform::Linux {
        return path != root && path.starts_with(&root);
    }
    let fold = |p: &Path| PathBuf::from(p.to_string_lossy().to_lowercase());
    let (path, root) = (fold(path), fold(&root));
    path != root && path.starts_with(&root)
}

/// WebView のデータの場所（アーキテクチャ設計書 6.5）。macOS は 5 か所（あるものだけ消す）、
/// Linux・Windows は 1 か所（`<データの親>/<識別子>`。既定ディレクトリと同じ関数から導く）。
pub fn webview_paths(setup: &Setup) -> Result<Vec<PathBuf>, ErrorObject> {
    match setup.platform {
        Platform::MacOs => {
            let lib = absolute_var(setup, "HOME")?.join("Library");
            Ok(vec![
                lib.join("WebKit").join(APP_IDENTIFIER),
                lib.join("Caches").join(APP_IDENTIFIER),
                lib.join("HTTPStorages").join(APP_IDENTIFIER),
                macos_preferences(&lib),
                lib.join("Saved Application State")
                    .join(format!("{APP_IDENTIFIER}.savedState")),
            ])
        }
        platform => data_dir::local_data_root_for(platform, |n| var(setup, n))
            .map(|root| vec![root.join(APP_IDENTIFIER)])
            .map_err(|e| data_dir_error(setup, &e)),
    }
}

fn macos_preferences(library: &Path) -> PathBuf {
    library
        .join("Preferences")
        .join(format!("{APP_IDENTIFIER}.plist"))
}

/// macOS の設定の plist（`defaults delete` を呼んだ後に、また現れていないかを見る）。
pub fn macos_plist(setup: &Setup) -> Result<PathBuf, ErrorObject> {
    Ok(macos_preferences(
        &absolute_var(setup, "HOME")?.join("Library"),
    ))
}

/// 保存先 `data_dir` の、消すデータベースのファイルと `backups/`（保存先のフォルダ自体は消さない）。
pub fn data_files(data_dir: &Path) -> Vec<PathBuf> {
    let db = data_dir::database_path(data_dir);
    let with = |suffix: &str| {
        let mut name = db.clone().into_os_string();
        name.push(suffix);
        PathBuf::from(name)
    };
    vec![
        db.clone(),
        with("-wal"),
        with("-shm"),
        with("-journal"),
        data_dir::backups_dir(data_dir),
    ]
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::uninstall::tests_support::setup_for;

    #[test]
    fn webview_dir_and_default_dir_share_a_parent_for_every_xdg_form() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        for (xdg, parent) in [
            (None, home.join(".local/share")),
            (Some(""), home.join(".local/share")),
            (Some("relative"), home.join(".local/share")),
            (
                Some(tmp.path().join("xdg").to_str().unwrap()),
                tmp.path().join("xdg"),
            ),
        ] {
            let mut vars = vec![("HOME", home.to_str().unwrap())];
            if let Some(x) = xdg {
                vars.push(("XDG_DATA_HOME", x));
            }
            let setup = setup_for(Platform::Linux, &vars);
            let default = data_dir::default_dir_for(Platform::Linux, |n| var(&setup, n)).unwrap();
            assert_eq!(
                webview_paths(&setup).unwrap(),
                vec![parent.join(APP_IDENTIFIER)],
                "XDG_DATA_HOME = {xdg:?}"
            );
            assert_eq!(default, parent.join("yunagi"));
        }
    }

    #[test]
    fn macos_has_five_webview_places_named_by_the_identifier() {
        let setup = setup_for(Platform::MacOs, &[("HOME", "/Users/a")]);
        let places: Vec<String> = webview_paths(&setup)
            .unwrap()
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let id = APP_IDENTIFIER;
        assert_eq!(
            places,
            vec![
                format!("/Users/a/Library/WebKit/{id}"),
                format!("/Users/a/Library/Caches/{id}"),
                format!("/Users/a/Library/HTTPStorages/{id}"),
                format!("/Users/a/Library/Preferences/{id}.plist"),
                format!("/Users/a/Library/Saved Application State/{id}.savedState"),
            ]
        );
    }

    #[test]
    fn windows_webview_dir_is_under_localappdata() {
        let setup = setup_for(Platform::Windows, &[("LOCALAPPDATA", "/c/Local")]);
        assert_eq!(
            webview_paths(&setup).unwrap(),
            vec![PathBuf::from("/c/Local").join(APP_IDENTIFIER)]
        );
        let setup = setup_for(Platform::Windows, &[("HOME", "/h")]);
        assert_eq!(
            error::code(&webview_paths(&setup).unwrap_err()),
            "storage_unavailable"
        );
    }

    #[test]
    fn standard_layouts_follow_the_install_scripts() {
        let linux = setup_for(
            Platform::Linux,
            &[("HOME", "/home/a"), ("XDG_DATA_HOME", "/data")],
        );
        assert_eq!(
            standard_layout(&linux).unwrap(),
            Layout {
                root: PathBuf::from("/home/a/.local/opt/yunagi"),
                link: Some(PathBuf::from("/home/a/.local/bin/yunagi")),
                desktop_entry: Some(PathBuf::from(format!(
                    "/data/applications/{APP_IDENTIFIER}.desktop"
                ))),
            }
        );
        let mac = setup_for(Platform::MacOs, &[("HOME", "/Users/a")]);
        assert_eq!(
            standard_layout(&mac).unwrap(),
            Layout {
                root: PathBuf::from("/Users/a/Applications/Yunagi.app"),
                link: Some(PathBuf::from("/Users/a/.local/bin/yunagi")),
                desktop_entry: None,
            }
        );
        let win = setup_for(Platform::Windows, &[("LOCALAPPDATA", "/c/Local")]);
        assert_eq!(
            standard_layout(&win).unwrap().root,
            PathBuf::from("/c/Local/Programs/Yunagi")
        );
    }

    #[test]
    fn data_files_are_the_database_files_and_backups_only() {
        let files = data_files(Path::new("/d"));
        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "yunagi.db",
                "yunagi.db-wal",
                "yunagi.db-shm",
                "yunagi.db-journal",
                "backups"
            ]
        );
    }

    #[test]
    fn classifies_by_where_the_real_executable_is() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let apps = tmp.path().join("Applications");
        let prefix = tmp.path().join("brew");
        let exe_in = |dir: &Path| {
            fs::create_dir_all(dir).unwrap();
            let exe = dir.join("yunagi");
            fs::write(&exe, "").unwrap();
            exe
        };
        let mut setup = setup_for(Platform::MacOs, &[("HOME", home.to_str().unwrap())]);
        setup.applications_dir = apps.clone();
        setup.brew_prefixes = vec![prefix.clone()];

        // 正規の置き場所
        setup.exe = Some(exe_in(&home.join("Applications/Yunagi.app/Contents/MacOS")));
        assert!(matches!(classify(&setup).unwrap(), Kind::Standard(_)));

        // Homebrew：/Applications/Yunagi.app の中で、Caskroom がある
        setup.exe = Some(exe_in(&apps.join("Yunagi.app/Contents/MacOS")));
        assert!(
            matches!(classify(&setup).unwrap(), Kind::Unmanaged(_)),
            "no Caskroom yet"
        );
        fs::create_dir_all(prefix.join("Caskroom/yunagi")).unwrap();
        assert_eq!(classify(&setup).unwrap(), Kind::Homebrew);

        // どちらでもない
        let elsewhere = exe_in(&tmp.path().join("target/debug"));
        setup.exe = Some(elsewhere.clone());
        assert_eq!(
            classify(&setup).unwrap(),
            Kind::Unmanaged(fs::canonicalize(elsewhere).unwrap())
        );
    }

    #[test]
    fn a_link_into_the_standard_place_counts_as_standard() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let real = home.join(".local/opt/yunagi/bin");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("yunagi"), "").unwrap();
        fs::create_dir_all(home.join(".local/bin")).unwrap();
        let link = home.join(".local/bin/yunagi");
        std::os::unix::fs::symlink(real.join("yunagi"), &link).unwrap();
        let mut setup = setup_for(Platform::Linux, &[("HOME", home.to_str().unwrap())]);
        setup.exe = Some(link);
        assert!(matches!(classify(&setup).unwrap(), Kind::Standard(_)));
    }
}
