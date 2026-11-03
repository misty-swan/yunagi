//! 保存先：既定ディレクトリ、設定ファイル `config.json`、保存先（`data_dir`）の決め方と変更
//! （docs/design/01-architecture.md 6章）。
//!
//! - 既定ディレクトリは OS ごとに決まる（[`default_dir`]）。`config.json`・`yunagi.lock`・
//!   ローカル API のトークンとソケットは、保存先の設定に関係なく常にここに置く。
//!   既定ディレクトリは [`DefaultDir`] 型で受け渡す。本番のコードでは [`default_dir`] でしか
//!   作れないので、ほかのフォルダ（保存先など）を既定ディレクトリとして渡せない。
//! - 保存先（`data_dir`）には `yunagi.db` と `backups/` を置く。保存先は `config.json` に
//!   記録する。`config.json` が無ければ、既定ディレクトリを保存先とする（[`resolve_data_dir`]）。
//! - 保存先の変更は [`change_data_dir`] で行う。データベースを移してから `config.json` を
//!   書き換え、途中で失敗しても元のデータが残る。

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

use super::DbLock;
use super::db_copy::{self, CopyError};

/// 設定ファイルの名前。既定ディレクトリに置く。
pub const CONFIG_FILE_NAME: &str = "config.json";
/// データベースのファイルの名前。保存先に置く。
pub const DATABASE_FILE_NAME: &str = "yunagi.db";
/// バックアップを置くフォルダの名前。保存先に置く。
pub const BACKUPS_DIR_NAME: &str = "backups";

/// SQLite が WAL モードでデータベースの横に作るファイルの接尾辞。
const SIDE_FILE_SUFFIXES: [&str; 2] = ["-wal", "-shm"];
/// 移動中のデータベースの一時的な名前（移し終えてから `yunagi.db` に名前を変える）。
const MOVING_FILE_NAME: &str = "yunagi.db.moving";

/// 既定ディレクトリの決め方を選ぶための OS の区分。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    Windows,
    /// Linux。macOS 以外の Unix 系もこれに従う（XDG Base Directory の決まり）。
    Linux,
}

impl Platform {
    /// 動いている OS の区分。
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

/// 既定ディレクトリ（アーキテクチャ設計書 6章）。`config.json`・`yunagi.lock` を置くフォルダ。
///
/// 本番のコードでは [`default_dir`] でしか作れない。これで、ロック（[`super::DbLock`]）や
/// 設定ファイルを、既定ディレクトリ以外のフォルダ（保存先 `data_dir` など）で扱ってしまうことを
/// 型で防ぐ。保存先を変えた利用者のアプリと CLI・MCP が、別々のロックファイルを取り合わずに
/// 済む（アーキテクチャ設計書 4.2）。
///
/// テストでは、一時フォルダを既定ディレクトリとして使うために `DefaultDir::for_test` で作る
/// （このクレートのテストのビルドにだけある）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultDir(PathBuf);

impl DefaultDir {
    /// フォルダのパス。
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// テスト用：`path` を既定ディレクトリとして扱う。`#[cfg(test)]` なので本番のコードからは
    /// 使えない。
    #[cfg(test)]
    pub(crate) fn for_test(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }
}

/// 動いている OS の既定ディレクトリ。環境変数から求める（[`default_dir_for`]）。
///
/// [`DefaultDir`] を作れる、本番で唯一の関数。フォルダは作らない。
pub fn default_dir() -> Result<DefaultDir, DataDirError> {
    default_dir_for(Platform::current(), |name| std::env::var_os(name)).map(DefaultDir)
}

/// `platform` の既定ディレクトリを、`env` で引いた環境変数から求める。
///
/// 求め方を確かめるための関数で、結果は [`DefaultDir`] ではなくただのパスにする
/// （`env` を差し替えれば任意のフォルダを返せるため。既定ディレクトリとして使うのは [`default_dir`]）。
///
/// | OS | 既定ディレクトリ |
/// | --- | --- |
/// | macOS | `$HOME/Library/Application Support/Yunagi` |
/// | Windows | `%LOCALAPPDATA%\Yunagi` |
/// | Linux | `$XDG_DATA_HOME/yunagi`（未設定・空・相対パスのとき `$HOME/.local/share/yunagi`） |
///
/// 空の値は未設定として扱う。使う環境変数が無いか、絶対パスでなければ
/// [`DataDirError::NoDefaultDir`] を返す。`env` はテストで環境変数を差し替えるために受け取る。
pub fn default_dir_for(
    platform: Platform,
    env: impl Fn(&str) -> Option<OsString>,
) -> Result<PathBuf, DataDirError> {
    // XDG Base Directory の決まりでは、相対パスの XDG_DATA_HOME は無視する
    let absolute = |name: &str| absolute_env(&env, name);
    let missing = |var: &'static str| DataDirError::NoDefaultDir { var };
    match platform {
        Platform::MacOs => {
            let home = absolute("HOME").ok_or_else(|| missing("HOME"))?;
            Ok(home.join("Library/Application Support/Yunagi"))
        }
        Platform::Windows => {
            let local = absolute("LOCALAPPDATA").ok_or_else(|| missing("LOCALAPPDATA"))?;
            Ok(local.join("Yunagi"))
        }
        Platform::Linux => {
            if let Some(data_home) = absolute("XDG_DATA_HOME") {
                return Ok(data_home.join("yunagi"));
            }
            let home = absolute("HOME").ok_or_else(|| missing("HOME"))?;
            Ok(home.join(".local/share/yunagi"))
        }
    }
}

/// 環境変数 `name` を `env` で引き、絶対パスなら返す。空の値と相対パスは無いものとする。
pub(super) fn absolute_env(env: impl Fn(&str) -> Option<OsString>, name: &str) -> Option<PathBuf> {
    env(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

/// シンボリックリンクを辿った実体のパス。`path` がまだ無いときは、あるところまでの親の実体に、
/// 残りをつなげる。どこも辿れなければ `path` のまま。
pub(super) fn resolve(path: &Path) -> PathBuf {
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

/// 保存先 `data_dir` の `yunagi.db` のパス。
pub fn database_path(data_dir: &Path) -> PathBuf {
    data_dir.join(DATABASE_FILE_NAME)
}

/// 保存先 `data_dir` の `backups/` のパス。
pub fn backups_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(BACKUPS_DIR_NAME)
}

/// `config.json` の中身。
///
/// このアプリが知らない項目（新しい版で足されたもの）は `other` に入れたまま書き戻し、消さない。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Config {
    /// 保存先。絶対パス。
    pub data_dir: PathBuf,
    #[serde(flatten)]
    other: serde_json::Map<String, serde_json::Value>,
}

/// 読むときの `config.json`。`data_dir` は無くてもよい（保存先を変えていない設定ファイル）。
#[derive(Deserialize)]
struct StoredConfig {
    #[serde(default)]
    data_dir: Option<PathBuf>,
    #[serde(flatten)]
    other: serde_json::Map<String, serde_json::Value>,
}

impl Config {
    /// 既定ディレクトリ `default_dir` の `config.json` を読む。
    ///
    /// ファイルが無いか、`data_dir` の項目が無ければ、保存先を `default_dir` とする
    /// （ファイルは作らない）。
    /// 読めない・JSON として壊れている・`data_dir` が絶対パスでないときはエラーにする
    /// （黙って既定の保存先で開くと、データが消えたように見え、別の場所に新しいデータベースを
    /// 作ってしまうため）。
    pub fn load(default_dir: &DefaultDir) -> Result<Self, DataDirError> {
        let default_dir = default_dir.path();
        let path = default_dir.join(CONFIG_FILE_NAME);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(Self {
                    data_dir: default_dir.to_path_buf(),
                    other: serde_json::Map::new(),
                });
            }
            Err(source) => return Err(DataDirError::io("read", &path, source)),
        };
        let stored: StoredConfig = serde_json::from_str(&text)
            .map_err(|source| DataDirError::InvalidConfig { path, source })?;
        let data_dir = stored.data_dir.unwrap_or_else(|| default_dir.to_path_buf());
        if !data_dir.is_absolute() {
            return Err(DataDirError::NotAbsolute(data_dir));
        }
        Ok(Self {
            data_dir,
            other: stored.other,
        })
    }

    /// 既定ディレクトリ `default_dir` に `config.json` を書く（フォルダが無ければ作る）。
    ///
    /// 一時ファイルに書いてディスクに反映してから名前を変えるので、途中で失敗しても
    /// 元の `config.json` は壊れない。
    pub fn save(&self, default_dir: &DefaultDir) -> Result<(), DataDirError> {
        let default_dir = default_dir.path();
        fs::create_dir_all(default_dir)
            .map_err(|e| DataDirError::io("create directory", default_dir, e))?;
        let path = default_dir.join(CONFIG_FILE_NAME);
        let tmp = default_dir.join(format!("{CONFIG_FILE_NAME}.tmp"));
        let mut text =
            serde_json::to_string_pretty(self).map_err(|source| DataDirError::InvalidConfig {
                path: path.clone(),
                source,
            })?;
        text.push('\n');
        let written = write_synced(&tmp, text.as_bytes())
            .and_then(|()| fs::rename(&tmp, &path))
            .map_err(|e| DataDirError::io("write", &path, e));
        if written.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        written?;
        let _ = db_copy::sync_dir(default_dir);
        Ok(())
    }
}

/// 既定ディレクトリ `default_dir` の `config.json` から保存先を求める。
/// `config.json` が無ければ `default_dir` そのもの。
pub fn resolve_data_dir(default_dir: &DefaultDir) -> Result<PathBuf, DataDirError> {
    Ok(Config::load(default_dir)?.data_dir)
}

/// 保存先の変更の結果。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ChangeReport {
    /// 移動の後に消せなかった、元の保存先のファイル・フォルダ。データは新しい保存先に
    /// 移っていて、`config.json` も新しい保存先を指している。消すかどうかは利用者に任せる。
    pub leftovers: Vec<PathBuf>,
}

/// 保存先を `new_dir` に変える。データベースと `backups/` を移し、`config.json` に記録する。
///
/// 手順（途中で失敗しても元のデータが残るように、元のファイルは最後まで消さない）：
///
/// 1. `new_dir` を確かめる。絶対パスでないとき、`..`・`.` を含むとき、`config.json` に書けない（UTF-8 でない）とき、
///    今の保存先と同じとき、今の保存先の `backups/` の中のとき、すでにデータベース
///    （`yunagi.db` とその `-wal`・`-shm`）か、空でない `backups/` があるときは断る。
/// 2. `new_dir` を作り、`backups/` を写す。
/// 3. データベースを SQLite のオンラインバックアップで一時ファイルに写し（WAL にだけある
///    書き込みも含めて写る）、整合性を確かめてから `yunagi.db` に名前を変える。
/// 4. `config.json` に `new_dir` を記録する。ここで新しい保存先に切り替わる。
/// 5. 元の保存先のデータベースと `backups/` を消す。消せなかったものは
///    [`ChangeReport::leftovers`] で返す（失敗にはしない）。
///
/// 4 より前に失敗したら、2・3 で作ったものを消してからエラーを返す。元の保存先と
/// `config.json` には触れていない。プロセスが途中で止まった場合も元のデータは残り、
/// `config.json` は元の保存先を指したまま（新しい保存先に残ったものは 1 の確認で断られるので、
/// 消してからやり直す）。
///
/// `yunagi.lock` のロック `lock` を持ち、データベースを閉じてから呼ぶ（アーキテクチャ設計書
/// 6章）。移している間に別のプロセスがデータベースに書くと、その書き込みは新しい保存先に
/// 移らないため。これは型で守る：
///
/// - ロック（[`DbLock`]）を渡さないと呼べない。`config.json` は、ロックを取った既定ディレクトリ
///   （[`DbLock::default_dir`]）のものを使う。
/// - 開いているデータベース（[`super::Database`]）はロックを持っているので、ロックが手元に
///   あれば、このプロセスがコアを通して開いたデータベースは閉じている（閉じるのに失敗した
///   ときも、ロックはデータベースと一緒にしか返らない）。アプリは
///   [`super::Database::close_keeping_lock`] で、ロックを離さずに閉じてから呼ぶ。
///   コアを通さずに開いた接続（`rusqlite::Connection` を直接使うもの）までは型で防げないので、
///   データベースはコアの外で開かない。
pub fn change_data_dir(lock: &DbLock, new_dir: &Path) -> Result<ChangeReport, DataDirError> {
    let default_dir = lock.default_dir();
    if !new_dir.is_absolute() {
        return Err(DataDirError::NotAbsolute(new_dir.to_path_buf()));
    }
    // `..` は、まだ無いフォルダの後にあると実体を求められず、下の「同じフォルダ」
    // 「backups/ の中」の確認をすり抜ける（作った後で元の保存先を指し、元のデータを消してしまう）
    if new_dir
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(DataDirError::NotNormalized(new_dir.to_path_buf()));
    }
    // JSON の文字列は UTF-8 なので、写し終えてから記録で失敗しないよう、先に確かめる
    if new_dir.to_str().is_none() {
        return Err(DataDirError::NotUtf8(new_dir.to_path_buf()));
    }
    let mut config = Config::load(default_dir)?;
    let old_dir = config.data_dir.clone();
    let new_real = resolve(new_dir);
    if resolve(&old_dir) == new_real {
        return Err(DataDirError::SameDirectory(new_dir.to_path_buf()));
    }
    // backups/ の中に移すと、backups/ を自分の中へ写し続け、元の backups/ を消すときに
    // 新しい保存先も消してしまう
    if new_real.starts_with(resolve(&backups_dir(&old_dir))) {
        return Err(DataDirError::InsideBackups(new_dir.to_path_buf()));
    }
    check_target(new_dir)?;

    let mut created = Created::default();
    let copied = copy_data(&old_dir, new_dir, &mut created).and_then(|()| {
        config.data_dir = new_dir.to_path_buf();
        config.save(default_dir)
    });
    if let Err(e) = copied {
        created.remove_all();
        return Err(e);
    }

    Ok(ChangeReport {
        leftovers: remove_old_data(&old_dir),
    })
}

/// 移動先に、上書きしてしまうデータが無いことを確かめる。
fn check_target(new_dir: &Path) -> Result<(), DataDirError> {
    for path in database_files(new_dir) {
        if path.symlink_metadata().is_ok() {
            return Err(DataDirError::TargetHasDatabase(path));
        }
    }
    let backups = backups_dir(new_dir);
    match fs::read_dir(&backups) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                return Err(DataDirError::TargetHasBackups(backups));
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        // 読めない（権限が無いなど）ときは、空とみなさない（利用者のファイルを上書きしうる）
        Err(e) => return Err(DataDirError::io("read directory", &backups, e)),
    }
    Ok(())
}

/// `data_dir` のデータベースのファイル（本体と `-wal`・`-shm`）。
fn database_files(data_dir: &Path) -> Vec<PathBuf> {
    with_side_files(&database_path(data_dir))
}

/// データベースのファイル `db` と、その `-wal`・`-shm`。
fn with_side_files(db: &Path) -> Vec<PathBuf> {
    let mut files = vec![db.to_path_buf()];
    for suffix in SIDE_FILE_SUFFIXES {
        let mut name = db.as_os_str().to_os_string();
        name.push(suffix);
        files.push(PathBuf::from(name));
    }
    files
}

/// 移動の途中で作ったもの。失敗したときに、作った順の逆に消す。
#[derive(Default)]
struct Created(Vec<(PathBuf, Kind)>);

/// 作ったものの種類。
#[derive(Clone, Copy)]
enum Kind {
    /// ファイル（シンボリックリンクを含む）。
    File,
    Dir,
}

impl Created {
    fn file(&mut self, path: PathBuf) {
        self.0.push((path, Kind::File));
    }

    fn dir(&mut self, path: PathBuf) {
        self.0.push((path, Kind::Dir));
    }

    /// 作ったものを消す。消せなくても元のデータには関係ないので、続ける。
    /// フォルダは空のときだけ消える（利用者が置いたものは消さない）。
    fn remove_all(self) {
        for (path, kind) in self.0.into_iter().rev() {
            let _ = match kind {
                Kind::File => fs::remove_file(&path),
                Kind::Dir => fs::remove_dir(&path),
            };
        }
    }
}

/// フォルダ `path` を（親も含めて）作り、新しく作ったものを `created` に記録する。
fn create_dirs(path: &Path, created: &mut Created) -> Result<(), DataDirError> {
    let mut missing = Vec::new();
    let mut cur = Some(path);
    while let Some(p) = cur {
        if p.exists() {
            break;
        }
        missing.push(p.to_path_buf());
        cur = p.parent();
    }
    for dir in missing.into_iter().rev() {
        match fs::create_dir(&dir) {
            Ok(()) => created.dir(dir),
            // ほかで同時に作られたものは、こちらが作ったものではない
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && dir.is_dir() => {}
            Err(e) => return Err(DataDirError::io("create directory", &dir, e)),
        }
    }
    Ok(())
}

/// 元の保存先のデータを新しい保存先に写す（元は消さない）。
fn copy_data(old_dir: &Path, new_dir: &Path, created: &mut Created) -> Result<(), DataDirError> {
    create_dirs(new_dir, created)?;

    let old_backups = backups_dir(old_dir);
    if old_backups.is_dir() {
        copy_tree(&old_backups, &backups_dir(new_dir), created)?;
    }

    let old_db = database_path(old_dir);
    if old_db.exists() {
        let moving = new_dir.join(MOVING_FILE_NAME);
        // 写しは1つのファイルにまとめるが、SQLite が途中で横に作る -wal・-shm も、
        // 失敗したときに消すよう記録する。前に止まった移動の残りがあれば消す
        for path in with_side_files(&moving) {
            let _ = fs::remove_file(&path);
            created.file(path);
        }
        copy_database(&old_db, &moving)?;
        let new_db = database_path(new_dir);
        fs::rename(&moving, &new_db).map_err(|e| DataDirError::io("rename", &moving, e))?;
        created.file(new_db);
        let _ = db_copy::sync_dir(new_dir);
    }
    Ok(())
}

/// フォルダ `from` の中身を `to` に写す（中のフォルダも辿る）。
fn copy_tree(from: &Path, to: &Path, created: &mut Created) -> Result<(), DataDirError> {
    create_dirs(to, created)?;
    let entries = fs::read_dir(from).map_err(|e| DataDirError::io("read directory", from, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| DataDirError::io("read directory", from, e))?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        // file_type はシンボリックリンクを辿らない
        let kind = entry
            .file_type()
            .map_err(|e| DataDirError::io("read", &src, e))?;
        // 移動先の backups/ は空であることを確かめてあるので、作ったものを消しても
        // 利用者のファイルは消さない。写す前に記録する（途中まで書いたものも、失敗したら消す）
        if kind.is_symlink() {
            copy_symlink(&src, &dst, created)?;
        } else if kind.is_dir() {
            copy_tree(&src, &dst, created)?;
        } else {
            created.file(dst.clone());
            copy_file_synced(&src, &dst).map_err(|e| DataDirError::io("copy", &src, e))?;
        }
    }
    let _ = db_copy::sync_dir(to);
    Ok(())
}

/// シンボリックリンク `from` を `to` に写す。
///
/// Unix 系では、リンクのまま写す（リンク先の中身は写さない。元の backups/ を消しても、
/// リンク先は消えない）。Windows では、リンクを作るのに権限が要ることがあるので、
/// リンク先を辿って中身を写す。
fn copy_symlink(from: &Path, to: &Path, created: &mut Created) -> Result<(), DataDirError> {
    #[cfg(unix)]
    {
        let target = fs::read_link(from).map_err(|e| DataDirError::io("read link", from, e))?;
        // 相対パスのリンクは、リンク先の実体の絶対パスにする（移した先から辿ると別の場所になり、
        // 元の場所からの道筋（元の backups/ を通る）は、元の backups/ を消すと辿れなくなるため）。
        // リンク先が無いリンクは、元の場所から見たパスにする
        let target = match from.parent() {
            Some(parent) if target.is_relative() => {
                fs::canonicalize(from).unwrap_or_else(|_| parent.join(target))
            }
            _ => target,
        };
        std::os::unix::fs::symlink(&target, to)
            .map_err(|e| DataDirError::io("create link", to, e))?;
        created.file(to.to_path_buf());
        Ok(())
    }
    #[cfg(not(unix))]
    {
        if from.is_dir() {
            copy_tree(from, to, created)
        } else {
            created.file(to.to_path_buf());
            copy_file_synced(from, to).map_err(|e| DataDirError::io("copy", from, e))
        }
    }
}

/// データベース `from` を `to` に写し（[`db_copy::copy`]）、写したものの整合性を確かめる。
pub(super) fn copy_database(from: &Path, to: &Path) -> Result<(), DataDirError> {
    // 元は読み取り専用で開く（元のファイルに書かず、閉じるときに WAL を書き戻すこともしない）
    let src = Connection::open_with_flags(from, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    db_copy::copy(&src, to)?;
    drop(src);
    // 確かめるだけなので、写しも読み取り専用で開く（写しに書かず、横にファイルを作らない）
    let result: String = Connection::open_with_flags(to, OpenFlags::SQLITE_OPEN_READ_ONLY)?
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if result != "ok" {
        return Err(DataDirError::CopyCorrupted { result });
    }
    Ok(())
}

/// 移し終えた後で、元の保存先のデータベースと `backups/` を消す。消せなかったものを返す。
fn remove_old_data(old_dir: &Path) -> Vec<PathBuf> {
    let mut leftovers = Vec::new();
    for path in database_files(old_dir) {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) => leftovers.push(path),
        }
    }
    let backups = backups_dir(old_dir);
    match fs::remove_dir_all(&backups) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(_) => leftovers.push(backups),
    }
    leftovers
}

/// `bytes` を `path` に書き、ディスクに反映する。
fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let mut file = fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// ファイルを写し、写した先をディスクに反映する。
///
/// 写した先は自分で作って書く（`fs::copy` は元の権限を写すので、読み取り専用のファイルだと、
/// 反映のために書き込める形で開き直せない。Windows では書き込める形でないと反映できない）。
/// 反映してから、元の権限をつける。
fn copy_file_synced(from: &Path, to: &Path) -> io::Result<()> {
    let mut src = fs::File::open(from)?;
    let permissions = src.metadata()?.permissions();
    let mut dst = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(to)?;
    io::copy(&mut src, &mut dst)?;
    dst.sync_all()?;
    drop(dst);
    fs::set_permissions(to, permissions)
}

/// 保存先の扱いが失敗した理由。
#[derive(Debug)]
pub enum DataDirError {
    /// 既定ディレクトリを求めるための環境変数 `var` が無い（空・相対パスを含む）。
    NoDefaultDir { var: &'static str },
    /// ファイルの操作が失敗した。`action` は何をしようとしたか。
    Io {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    /// `config.json` を JSON として読めない（書けない）。
    InvalidConfig {
        path: PathBuf,
        source: serde_json::Error,
    },
    /// 保存先が絶対パスでない。
    NotAbsolute(PathBuf),
    /// 保存先に `..`・`.` が含まれる。
    NotNormalized(PathBuf),
    /// 保存先が UTF-8 で表せない（`config.json` に書けない）。
    NotUtf8(PathBuf),
    /// 新しい保存先が、今の保存先と同じ。
    SameDirectory(PathBuf),
    /// 新しい保存先が、今の保存先の `backups/` の中にある。
    InsideBackups(PathBuf),
    /// 新しい保存先に、すでにデータベースのファイルがある（上書きしない）。
    TargetHasDatabase(PathBuf),
    /// 新しい保存先に、空でない `backups/` がある（上書きしない）。
    TargetHasBackups(PathBuf),
    /// データベースを写すときに SQLite の操作が失敗した。
    Sqlite(rusqlite::Error),
    /// 写したデータベースの整合性の確認（`integrity_check`）が通らなかった。
    CopyCorrupted { result: String },
}

impl DataDirError {
    fn io(action: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            action,
            path: path.to_path_buf(),
            source,
        }
    }
}

impl fmt::Display for DataDirError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDefaultDir { var } => write!(
                f,
                "could not determine the default directory: environment variable {var} is not set to an absolute path"
            ),
            Self::Io {
                action,
                path,
                source,
            } => write!(f, "could not {action} {}: {source}", path.display()),
            Self::InvalidConfig { path, source } => {
                write!(f, "invalid config file {}: {source}", path.display())
            }
            Self::NotAbsolute(path) => {
                write!(
                    f,
                    "data directory must be an absolute path: {}",
                    path.display()
                )
            }
            Self::NotNormalized(path) => write!(
                f,
                "data directory must not contain `.` or `..`: {}",
                path.display()
            ),
            Self::NotUtf8(path) => write!(
                f,
                "data directory path is not valid UTF-8: {}",
                path.display()
            ),
            Self::InsideBackups(path) => write!(
                f,
                "the new data directory is inside the current backups folder: {}",
                path.display()
            ),
            Self::SameDirectory(path) => {
                write!(f, "{} is already the data directory", path.display())
            }
            Self::TargetHasDatabase(path) => write!(
                f,
                "the new data directory already has a database file: {}",
                path.display()
            ),
            Self::TargetHasBackups(path) => write!(
                f,
                "the new data directory already has backups: {}",
                path.display()
            ),
            Self::Sqlite(e) => write!(f, "database error while copying: {e}"),
            Self::CopyCorrupted { result } => {
                write!(
                    f,
                    "the copied database failed the integrity check: {result}"
                )
            }
        }
    }
}

impl std::error::Error for DataDirError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidConfig { source, .. } => Some(source),
            Self::Sqlite(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for DataDirError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

impl From<CopyError> for DataDirError {
    fn from(e: CopyError) -> Self {
        match e {
            CopyError::Sqlite(e) => Self::Sqlite(e),
            CopyError::Sync { path, source } => Self::Io {
                action: "sync",
                path,
                source,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::Duration;

    use tempfile::TempDir;

    use super::super::Database;

    fn env_of(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + use<> {
        let map: HashMap<String, OsString> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        move |name| map.get(name).cloned()
    }

    /// テスト用の場所：既定ディレクトリ・元の保存先・新しい保存先の親を、一時フォルダの下に作る。
    struct Dirs {
        _tmp: TempDir,
        /// 既定ディレクトリのパス（ファイルを置いて確かめる用）。
        default: PathBuf,
        /// 同じフォルダを既定ディレクトリとして渡す用。
        dd: DefaultDir,
        root: PathBuf,
    }

    impl Dirs {
        /// 既定ディレクトリのロックを取る。
        fn lock(&self) -> DbLock {
            DbLock::acquire(&self.dd, Duration::ZERO).unwrap()
        }

        /// ロックを取ってから保存先を `new_dir` に変え、ロックを離す。
        fn change(&self, new_dir: &Path) -> Result<ChangeReport, DataDirError> {
            change_data_dir(&self.lock(), new_dir)
        }
    }

    fn dirs() -> Dirs {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        let default = root.join("default");
        fs::create_dir(&default).unwrap();
        Dirs {
            _tmp: tmp,
            dd: DefaultDir::for_test(&default),
            default,
            root,
        }
    }

    /// `dir` に、WAL にだけ書き込みが残ったデータベースを作り、開いたままの接続を返す。
    fn make_db_with_wal(dir: &Path) -> Connection {
        fs::create_dir_all(dir).unwrap();
        let conn = Connection::open(database_path(dir)).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        // 自動の書き戻しを止め、書き込みを WAL に残す
        conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        conn.execute_batch(
            "CREATE TABLE note(body TEXT); INSERT INTO note VALUES ('kept in wal');",
        )
        .unwrap();
        conn
    }

    fn read_note(dir: &Path) -> String {
        let conn = Connection::open(database_path(dir)).unwrap();
        conn.query_row("SELECT body FROM note", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn default_dir_macos() {
        let env = env_of(&[("HOME", "/Users/a")]);
        assert_eq!(
            default_dir_for(Platform::MacOs, env).unwrap(),
            PathBuf::from("/Users/a/Library/Application Support/Yunagi")
        );
    }

    #[test]
    fn default_dir_windows() {
        // Linux 上でテストしても Windows 形式の絶対パスとは判定されないので、
        // 動いている OS で絶対のパスを使う
        let local = if cfg!(windows) {
            r"C:\Users\a\AppData\Local"
        } else {
            "/c/Users/a/AppData/Local"
        };
        let env = env_of(&[("LOCALAPPDATA", local), ("HOME", "/ignored")]);
        assert_eq!(
            default_dir_for(Platform::Windows, env).unwrap(),
            Path::new(local).join("Yunagi")
        );
        assert!(matches!(
            default_dir_for(Platform::Windows, env_of(&[("HOME", "/h")])),
            Err(DataDirError::NoDefaultDir {
                var: "LOCALAPPDATA"
            })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn default_dir_linux_uses_xdg_data_home() {
        let env = env_of(&[("XDG_DATA_HOME", "/data"), ("HOME", "/home/a")]);
        assert_eq!(
            default_dir_for(Platform::Linux, env).unwrap(),
            PathBuf::from("/data/yunagi")
        );
    }

    #[cfg(unix)]
    #[test]
    fn default_dir_linux_falls_back_to_home() {
        for xdg in [None, Some(""), Some("relative/path")] {
            let mut vars = vec![("HOME", "/home/a")];
            if let Some(v) = xdg {
                vars.push(("XDG_DATA_HOME", v));
            }
            assert_eq!(
                default_dir_for(Platform::Linux, env_of(&vars)).unwrap(),
                PathBuf::from("/home/a/.local/share/yunagi"),
                "XDG_DATA_HOME = {xdg:?}"
            );
        }
        assert!(matches!(
            default_dir_for(Platform::Linux, env_of(&[("HOME", "")])),
            Err(DataDirError::NoDefaultDir { var: "HOME" })
        ));
    }

    #[test]
    fn missing_config_means_default_dir() {
        let d = dirs();
        assert_eq!(resolve_data_dir(&d.dd).unwrap(), d.default);
        assert!(!d.default.join(CONFIG_FILE_NAME).exists());
    }

    #[test]
    fn config_round_trip_keeps_unknown_fields() {
        let d = dirs();
        let data = d.root.join("data");
        fs::write(
            d.default.join(CONFIG_FILE_NAME),
            serde_json::json!({ "data_dir": data, "future": { "x": 1 } }).to_string(),
        )
        .unwrap();
        let mut config = Config::load(&d.dd).unwrap();
        assert_eq!(config.data_dir, data);
        config.data_dir = d.root.join("other");
        config.save(&d.dd).unwrap();

        let saved: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(d.default.join(CONFIG_FILE_NAME)).unwrap())
                .unwrap();
        assert_eq!(saved["future"], serde_json::json!({ "x": 1 }));
        assert_eq!(resolve_data_dir(&d.dd).unwrap(), d.root.join("other"));
        assert!(!d.default.join("config.json.tmp").exists());
    }

    #[test]
    fn config_without_data_dir_means_default_dir() {
        let d = dirs();
        fs::write(d.default.join(CONFIG_FILE_NAME), r#"{"theme": "dark"}"#).unwrap();
        assert_eq!(resolve_data_dir(&d.dd).unwrap(), d.default);
        // 保存先を変えても、ほかの項目は残る
        let new_dir = d.root.join("new");
        d.change(&new_dir).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(d.default.join(CONFIG_FILE_NAME)).unwrap())
                .unwrap();
        assert_eq!(saved["theme"], "dark");
        assert_eq!(resolve_data_dir(&d.dd).unwrap(), new_dir);
    }

    #[test]
    fn broken_or_relative_config_is_an_error() {
        let d = dirs();
        let path = d.default.join(CONFIG_FILE_NAME);
        fs::write(&path, "{ not json").unwrap();
        assert!(matches!(
            Config::load(&d.dd),
            Err(DataDirError::InvalidConfig { .. })
        ));
        fs::write(&path, r#"{"data_dir": "relative"}"#).unwrap();
        assert!(matches!(
            Config::load(&d.dd),
            Err(DataDirError::NotAbsolute(_))
        ));
    }

    #[test]
    fn change_moves_database_and_backups() {
        let d = dirs();
        let conn = make_db_with_wal(&d.default);
        fs::create_dir_all(backups_dir(&d.default).join("nested")).unwrap();
        fs::write(backups_dir(&d.default).join("a.db"), b"backup").unwrap();
        fs::write(backups_dir(&d.default).join("nested/b.db"), b"nested").unwrap();

        // 開いたままの接続は、閉じると WAL を書き戻して消すので、閉じずに置く
        // （本番では閉じてから呼ぶ。ここでは WAL にだけある書き込みが写ることを確かめる）
        let new_dir = d.root.join("sync/Yunagi");
        let report = d.change(&new_dir).unwrap();
        drop(conn);

        assert_eq!(report, ChangeReport::default());
        assert_eq!(resolve_data_dir(&d.dd).unwrap(), new_dir);
        assert_eq!(read_note(&new_dir), "kept in wal");
        assert_eq!(
            fs::read(backups_dir(&new_dir).join("nested/b.db")).unwrap(),
            b"nested"
        );
        assert_eq!(
            fs::read(backups_dir(&new_dir).join("a.db")).unwrap(),
            b"backup"
        );
        // 移動先には、データベース本体（1つのファイル）と backups/ だけがある
        let mut names: Vec<_> = fs::read_dir(&new_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        names.sort();
        assert_eq!(names, [BACKUPS_DIR_NAME, DATABASE_FILE_NAME]);
        assert!(!backups_dir(&d.default).exists());
        assert!(!database_path(&d.default).exists());
        // config.json は既定ディレクトリに残る
        assert!(d.default.join(CONFIG_FILE_NAME).exists());
        assert!(!new_dir.join(CONFIG_FILE_NAME).exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_in_backups_are_copied_as_links() {
        let d = dirs();
        let outside = d.root.join("archive");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("old.db"), b"archived").unwrap();
        fs::create_dir(backups_dir(&d.default)).unwrap();
        std::os::unix::fs::symlink(&outside, backups_dir(&d.default).join("archive")).unwrap();
        // 相対パスのリンクは、元の場所から見た先を指したまま写る
        std::os::unix::fs::symlink("../../archive", backups_dir(&d.default).join("rel")).unwrap();

        let new_dir = d.root.join("new");
        d.change(&new_dir).unwrap();
        let link = backups_dir(&new_dir).join("archive");
        assert_eq!(fs::read_link(&link).unwrap(), outside);
        assert_eq!(fs::read(link.join("old.db")).unwrap(), b"archived");
        let rel = backups_dir(&new_dir).join("rel");
        assert_eq!(fs::read(rel.join("old.db")).unwrap(), b"archived");
        // 元の backups/ を消しても、リンク先は残る
        assert_eq!(fs::read(outside.join("old.db")).unwrap(), b"archived");
    }

    #[test]
    fn read_only_backups_are_copied_with_their_permissions() {
        let d = dirs();
        fs::create_dir(backups_dir(&d.default)).unwrap();
        let file = backups_dir(&d.default).join("a.db");
        fs::write(&file, b"backup").unwrap();
        let mut ro = fs::metadata(&file).unwrap().permissions();
        ro.set_readonly(true);
        fs::set_permissions(&file, ro).unwrap();

        let new_dir = d.root.join("new");
        d.change(&new_dir).unwrap();
        let copied = backups_dir(&new_dir).join("a.db");
        assert_eq!(fs::read(&copied).unwrap(), b"backup");
        assert!(fs::metadata(&copied).unwrap().permissions().readonly());
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_target_backups_is_not_treated_as_empty() {
        use std::os::unix::fs::PermissionsExt;
        let d = dirs();
        let new_dir = d.root.join("new");
        let backups = backups_dir(&new_dir);
        fs::create_dir_all(&backups).unwrap();
        fs::write(backups.join("mine.db"), b"mine").unwrap();
        fs::set_permissions(&backups, fs::Permissions::from_mode(0o300)).unwrap();
        let readable = fs::read_dir(&backups).is_ok(); // root で動かすと読めてしまう
        let result = d.change(&new_dir);
        fs::set_permissions(&backups, fs::Permissions::from_mode(0o700)).unwrap();
        if readable {
            assert!(matches!(result, Err(DataDirError::TargetHasBackups(_))));
        } else {
            assert!(matches!(result, Err(DataDirError::Io { .. })), "{result:?}");
        }
        assert_eq!(fs::read(backups.join("mine.db")).unwrap(), b"mine");
        assert_eq!(resolve_data_dir(&d.dd).unwrap(), d.default);
    }

    #[test]
    fn source_database_is_not_modified() {
        let d = dirs();
        let conn = make_db_with_wal(&d.default);
        let wal = with_side_files(&database_path(&d.default))[1].clone();
        let before = fs::read(&wal).unwrap();
        // 記録で失敗させ、元が残る場合に、元の WAL が書き戻されていないことを確かめる
        fs::create_dir(d.default.join("config.json.tmp")).unwrap();
        assert!(d.change(&d.root.join("new")).is_err());
        assert_eq!(fs::read(&wal).unwrap(), before);
        drop(conn);
    }

    #[test]
    fn moved_database_opens_in_wal_mode() {
        let d = dirs();
        // アプリの流れ：開いたデータベースを、ロックを離さずに閉じてから移し、同じロックで開き直す
        let db = Database::open(d.lock(), database_path(&d.default)).unwrap();
        let lock = db.close_keeping_lock().unwrap();
        let new_dir = d.root.join("new");
        change_data_dir(&lock, &new_dir).unwrap();
        let db = Database::open(lock, database_path(&new_dir)).unwrap();
        let mode: String = db
            .connection()
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
    }

    #[test]
    fn change_uses_the_config_of_the_locked_default_dir() {
        let d = dirs();
        let new_dir = d.root.join("new");
        change_data_dir(&d.lock(), &new_dir).unwrap();
        // config.json はロックを取った既定ディレクトリに書かれ、ロックファイルもそこに残る。
        // 保存先には、どちらも置かない
        assert_eq!(resolve_data_dir(&d.dd).unwrap(), new_dir);
        assert!(d.default.join(super::super::LOCK_FILE_NAME).exists());
        assert!(!new_dir.join(super::super::LOCK_FILE_NAME).exists());
        assert!(!new_dir.join(CONFIG_FILE_NAME).exists());
    }

    #[test]
    fn change_twice_moves_from_recorded_data_dir() {
        let d = dirs();
        drop(make_db_with_wal(&d.default));
        let first = d.root.join("first");
        let second = d.root.join("second");
        d.change(&first).unwrap();
        d.change(&second).unwrap();
        assert_eq!(resolve_data_dir(&d.dd).unwrap(), second);
        assert_eq!(read_note(&second), "kept in wal");
        assert!(!database_path(&first).exists());
        // 既定ディレクトリに戻すこともできる
        d.change(&d.default).unwrap();
        assert_eq!(read_note(&d.default), "kept in wal");
    }

    #[test]
    fn change_without_database_only_records() {
        let d = dirs();
        let new_dir = d.root.join("empty");
        d.change(&new_dir).unwrap();
        assert!(new_dir.is_dir());
        assert_eq!(resolve_data_dir(&d.dd).unwrap(), new_dir);
    }

    #[test]
    fn change_refuses_bad_targets() {
        let d = dirs();
        drop(make_db_with_wal(&d.default));

        assert!(matches!(
            d.change(Path::new("relative")),
            Err(DataDirError::NotAbsolute(_))
        ));
        assert!(matches!(
            d.change(&d.default),
            Err(DataDirError::SameDirectory(_))
        ));

        for inside in [
            backups_dir(&d.default),
            backups_dir(&d.default).join("moved"),
        ] {
            assert!(
                matches!(d.change(&inside), Err(DataDirError::InsideBackups(_))),
                "{inside:?}"
            );
        }
        assert!(!backups_dir(&d.default).exists());

        // まだ無いフォルダを通って元の保存先・backups/ に戻るパス
        for sneaky in [
            d.default.join("missing/.."),
            d.default.join("missing/../backups/sub"),
        ] {
            assert!(
                matches!(d.change(&sneaky), Err(DataDirError::NotNormalized(_))),
                "{sneaky:?}"
            );
        }
        assert!(!d.default.join("missing").exists());

        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let bad = d.root.join(std::ffi::OsStr::from_bytes(b"bad\xff"));
            assert!(matches!(d.change(&bad), Err(DataDirError::NotUtf8(_))));
            assert!(!bad.exists());
        }

        let has_wal = d.root.join("has_wal");
        fs::create_dir(&has_wal).unwrap();
        fs::write(has_wal.join("yunagi.db-wal"), b"stale").unwrap();
        assert!(matches!(
            d.change(&has_wal),
            Err(DataDirError::TargetHasDatabase(_))
        ));

        let has_backups = d.root.join("has_backups");
        fs::create_dir_all(backups_dir(&has_backups)).unwrap();
        fs::write(backups_dir(&has_backups).join("x"), b"x").unwrap();
        assert!(matches!(
            d.change(&has_backups),
            Err(DataDirError::TargetHasBackups(_))
        ));

        // どれも元のデータに触れていない
        assert_eq!(resolve_data_dir(&d.dd).unwrap(), d.default);
        assert_eq!(read_note(&d.default), "kept in wal");
        assert_eq!(fs::read(has_wal.join("yunagi.db-wal")).unwrap(), b"stale");
    }

    #[test]
    fn failure_before_recording_keeps_original_and_cleans_up() {
        let d = dirs();
        drop(make_db_with_wal(&d.default));
        fs::create_dir(backups_dir(&d.default)).unwrap();
        fs::write(backups_dir(&d.default).join("a.db"), b"backup").unwrap();
        // config.json の場所をフォルダにして、読めなくする
        fs::create_dir(d.default.join(CONFIG_FILE_NAME)).unwrap();

        let new_dir = d.root.join("a/b/new");
        let err = d.change(&new_dir);
        // config.json が読めないので、移動の前に止まる
        assert!(matches!(err, Err(DataDirError::Io { .. })), "{err:?}");
        assert!(!d.root.join("a").exists());

        // 読めるが書けない場合：config.json.tmp の場所をフォルダにする
        fs::remove_dir_all(d.default.join(CONFIG_FILE_NAME)).unwrap();
        fs::create_dir(d.default.join("config.json.tmp")).unwrap();
        let err = d.change(&new_dir);
        assert!(matches!(err, Err(DataDirError::Io { .. })), "{err:?}");

        // 作ったフォルダ・写したファイルは消え、元のデータと設定はそのまま
        assert!(!d.root.join("a").exists());
        assert_eq!(read_note(&d.default), "kept in wal");
        assert_eq!(
            fs::read(backups_dir(&d.default).join("a.db")).unwrap(),
            b"backup"
        );
        assert_eq!(resolve_data_dir(&d.dd).unwrap(), d.default);
    }

    #[test]
    fn failure_while_copying_keeps_original() {
        let d = dirs();
        fs::write(database_path(&d.default), b"this is not a database").unwrap();
        let new_dir = d.root.join("new");
        let err = d.change(&new_dir);
        assert!(matches!(err, Err(DataDirError::Sqlite(_))), "{err:?}");
        assert!(!new_dir.exists());
        assert_eq!(
            fs::read(database_path(&d.default)).unwrap(),
            b"this is not a database"
        );
        assert_eq!(resolve_data_dir(&d.dd).unwrap(), d.default);
    }

    #[test]
    fn existing_target_dir_is_kept_on_failure() {
        let d = dirs();
        fs::write(database_path(&d.default), b"this is not a database").unwrap();
        let new_dir = d.root.join("existing");
        fs::create_dir(&new_dir).unwrap();
        fs::write(new_dir.join("user-file.txt"), b"mine").unwrap();
        assert!(d.change(&new_dir).is_err());
        assert_eq!(fs::read(new_dir.join("user-file.txt")).unwrap(), b"mine");
        assert_eq!(
            fs::read_dir(&new_dir).unwrap().count(),
            1,
            "only the user's file remains"
        );
    }
}
