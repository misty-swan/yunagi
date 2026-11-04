use std::fs;

use super::*;

fn touch(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, b"").unwrap();
}

#[test]
fn linux_finds_the_app_image_next_to_the_installed_command() {
    let root = tempfile::tempdir().unwrap();
    let opt = root.path().join(".local/opt/yunagi");
    let exe = opt.join("bin/yunagi");
    touch(&exe);
    // まだ無い
    assert_eq!(locate(&exe, None, "linux"), None);
    touch(&opt.join("Yunagi.AppImage"));
    assert_eq!(
        locate(&exe, None, "linux"),
        Some(Target::Program(opt.join("Yunagi.AppImage")))
    );
}

#[test]
fn linux_finds_a_build_next_to_the_command() {
    let root = tempfile::tempdir().unwrap();
    let exe = root.path().join("debug/yunagi");
    touch(&exe);
    touch(&root.path().join("debug/yunagi-app"));
    assert_eq!(
        locate(&exe, None, "linux"),
        Some(Target::Program(root.path().join("debug/yunagi-app")))
    );
}

#[test]
fn windows_finds_the_app_above_the_bin_folder() {
    let root = tempfile::tempdir().unwrap();
    let exe = root.path().join("Yunagi/bin/yunagi.exe");
    touch(&exe);
    touch(&root.path().join("Yunagi/Yunagi.exe"));
    assert_eq!(
        locate(&exe, None, "windows"),
        Some(Target::Program(root.path().join("Yunagi/Yunagi.exe")))
    );
}

#[test]
fn macos_finds_the_app_bundle_that_holds_the_command() {
    let root = tempfile::tempdir().unwrap();
    let exe = root.path().join("Yunagi.app/Contents/MacOS/yunagi");
    touch(&exe);
    assert_eq!(
        locate(&exe, None, "macos"),
        Some(Target::MacApp(root.path().join("Yunagi.app")))
    );
    // .app の外（開発のビルド）は、同じ場所の yunagi-app
    let build = root.path().join("debug/yunagi");
    touch(&build);
    assert_eq!(locate(&build, None, "macos"), None);
    touch(&root.path().join("debug/yunagi-app"));
    assert_eq!(
        locate(&build, None, "macos"),
        Some(Target::Program(root.path().join("debug/yunagi-app")))
    );
}

#[test]
fn the_environment_variable_wins_and_must_exist() {
    let root = tempfile::tempdir().unwrap();
    let exe = root.path().join("opt/bin/yunagi");
    touch(&exe);
    touch(&root.path().join("opt/Yunagi.AppImage"));
    let other = root.path().join("elsewhere/App");
    // 指した先が無ければ、置き場所の探索には戻らない
    assert_eq!(locate(&exe, Some(other.as_os_str()), "linux"), None);
    touch(&other);
    assert_eq!(
        locate(&exe, Some(other.as_os_str()), "linux"),
        Some(Target::Program(other.clone()))
    );
    // 空の値は無いのと同じ
    assert_eq!(
        locate(&exe, Some(OsStr::new("")), "linux"),
        Some(Target::Program(root.path().join("opt/Yunagi.AppImage")))
    );
    // フォルダ（macOS の .app 以外）を指していても、見つからないものとする
    let folder = root.path().join("folder");
    fs::create_dir_all(&folder).unwrap();
    assert_eq!(locate(&exe, Some(folder.as_os_str()), "linux"), None);
    // macOS の .app を指したら open で起動する
    let app = root.path().join("Apps/Yunagi.app");
    fs::create_dir_all(&app).unwrap();
    assert_eq!(
        locate(&exe, Some(app.as_os_str()), "macos"),
        Some(Target::MacApp(app))
    );
}
