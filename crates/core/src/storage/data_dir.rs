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
//!   書き換え、途中で失敗しても元のデータが残る。今の保存先にデータベースが無ければ断る。
//! - データベースのある既存のフォルダを、データを移さずに保存先にする（選び直す）のは
//!   [`use_existing_data_dir`] で行う。中身を確かめてから `config.json` に記録する。

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;

use super::db_copy::{self, CopyError};
use super::{DbLock, StorageError, db_files, migration};

/// 設定ファイルの名前。既定ディレクトリに置く。
pub const CONFIG_FILE_NAME: &str = "config.json";
/// データベースのファイルの名前。保存先に置く。
pub const DATABASE_FILE_NAME: &str = "yunagi.db";
/// バックアップを置くフォルダの名前。保存先に置く。
pub const BACKUPS_DIR_NAME: &str = "backups";

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

/// 既定ディレクトリを作る（無ければ）。UNIX 系では利用者本人だけが入れる権限（0700）で作る
/// （ローカル API のソケットとトークンを置くフォルダなので。docs/release/SECURITY.md 4.2。
/// アーキテクチャ設計書 4.4）。親のフォルダは通常の権限で作る。すでにあるフォルダの権限は変えない
/// （狭めるのはローカル API の待ち受けを始めるとき）。
pub fn create_default_dir(dir: &DefaultDir) -> io::Result<()> {
    let path = dir.path();
    if path.is_dir() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    match builder.create(path) {
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => Ok(()),
        other => other,
    }
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
    let name = match platform {
        Platform::MacOs | Platform::Windows => "Yunagi",
        Platform::Linux => "yunagi",
    };
    Ok(local_data_root_for(platform, env)?.join(name))
}

/// アプリの識別子（bundle identifier。アーキテクチャ設計書 6.1）。`src-tauri/tauri.conf.json` の
/// `identifier` と同じ（試験で固定する）。WebView が自分のデータを置くフォルダの名前にも使われる
/// （アーキテクチャ設計書 6.5）。
pub const APP_IDENTIFIER: &str = "io.github.misty-swan.yunagi";

/// `platform` の「利用者ごとのアプリのデータを置く親のフォルダ」を、`env` で引いた環境変数から求める。
/// 既定ディレクトリ（[`default_dir_for`]）の親で、Linux・Windows では Tauri が WebView のデータを置く
/// `<親>/<識別子>/` の親でもある（アーキテクチャ設計書 6.5。別々に環境変数を読むと食い違うので、
/// どちらもここから導く）。
///
/// | OS | 親のフォルダ |
/// | --- | --- |
/// | macOS | `$HOME/Library/Application Support` |
/// | Windows | `%LOCALAPPDATA%` |
/// | Linux | `$XDG_DATA_HOME`（未設定・空・相対パスのとき `$HOME/.local/share`） |
pub fn local_data_root_for(
    platform: Platform,
    env: impl Fn(&str) -> Option<OsString>,
) -> Result<PathBuf, DataDirError> {
    // XDG Base Directory の決まりでは、相対パスの XDG_DATA_HOME は無視する
    let absolute = |name: &str| absolute_env(&env, name);
    let missing = |var: &'static str| DataDirError::NoDefaultDir { var };
    match platform {
        Platform::MacOs => {
            let home = absolute("HOME").ok_or_else(|| missing("HOME"))?;
            Ok(home.join("Library/Application Support"))
        }
        Platform::Windows => absolute("LOCALAPPDATA").ok_or_else(|| missing("LOCALAPPDATA")),
        Platform::Linux => {
            if let Some(data_home) = absolute("XDG_DATA_HOME") {
                return Ok(data_home);
            }
            let home = absolute("HOME").ok_or_else(|| missing("HOME"))?;
            Ok(home.join(".local/share"))
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
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// 保存先。絶対パス。
    pub data_dir: PathBuf,
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
        Self::load_from(default_dir.path())
    }

    /// [`Config::load`] の、フォルダのパスを受け取る版。別の候補の既定ディレクトリを読むときに使う
    /// （候補は [`DefaultDir`] にできない。`dir_mismatch`）。
    pub(super) fn load_from(default_dir: &Path) -> Result<Self, DataDirError> {
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
    /// 保存先が既定ディレクトリ（シンボリックリンクを辿った実体どうしで比べる）なら、`data_dir` の
    /// 項目を書かない（無ければ既定ディレクトリを保存先とする。絶対パスを書くと、既定ディレクトリ
    /// ごと移したとき（新しい端末への引っ越しなど）に古いパスを指してしまうため。
    /// アーキテクチャ設計書 6.3）。
    ///
    /// 一時ファイルに書いてディスクに反映してから名前を変えるので、途中で失敗しても
    /// 元の `config.json` は壊れない。
    pub fn save(&self, default_dir: &DefaultDir) -> Result<(), DataDirError> {
        let default_dir = default_dir.path();
        fs::create_dir_all(default_dir)
            .map_err(|e| DataDirError::io("create directory", default_dir, e))?;
        let path = default_dir.join(CONFIG_FILE_NAME);
        let tmp = default_dir.join(format!("{CONFIG_FILE_NAME}.tmp"));
        let invalid = |source| DataDirError::InvalidConfig {
            path: path.clone(),
            source,
        };
        let mut stored = self.other.clone();
        if resolve(&self.data_dir) == resolve(default_dir) {
            stored.remove("data_dir");
        } else {
            stored.insert(
                "data_dir".to_owned(),
                serde_json::to_value(&self.data_dir).map_err(invalid)?,
            );
        }
        let mut text = serde_json::to_string_pretty(&stored).map_err(invalid)?;
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
/// 1. `new_dir` を確かめる。絶対パスでないとき、`..`・`.` を含むとき、`config.json` に書けない
///    （UTF-8 でない）とき、今の保存先と同じとき、今の保存先の `backups/` の中の
///    とき、すでにデータベースの跡（中身のある `yunagi.db`・`-wal`・`-journal`、`yunagi.db` の
///    シンボリックリンク）か、空でない `backups/` があるときは断る。
///    今の保存先に中身のあるデータベースが無いとき（[`DataDirError::NoDatabase`]）も、何も変えずに断る。
/// 2. `new_dir` を作り、`backups/` を写す。
/// 3. データベースを SQLite のオンラインバックアップで一時ファイルに写し（WAL にだけある
///    書き込みも含めて写る）、整合性を確かめてから `yunagi.db` に名前を変える。
/// 4. `config.json` に `new_dir` を記録する。ここで新しい保存先に切り替わる。
/// 5. 元の保存先のデータベース（とその横のファイル）と `backups/` を消す。消せなかったものは
///    [`ChangeReport::leftovers`] で返す（失敗にはしない）。
///
/// 4 より前に失敗したら、2・3 で作ったものを消してからエラーを返す。元の保存先と
/// `config.json` には触れていない。プロセスが途中で止まった場合も元のデータは残り、
/// `config.json` は元の保存先を指したまま（新しい保存先に残ったものは 1 の確認で断られるので、
/// 消してからやり直す）。
///
/// 今の保存先にデータベースが無いとき（[`super::OpenError::NotFound`] のとき）に、データベースのある
/// 別のフォルダを保存先にするのは [`use_existing_data_dir`]、新しく作るのは [`super::Database::create`]
/// で行う（アーキテクチャ設計書 6章）。移すものが無いのに記録だけを変えると、元の保存先に残った
/// `-wal`（同期の途中で本体がまだ届いていないものなど）を、5 で消してしまうため。
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
    check_new_dir(new_dir)?;
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
    let old_db = database_path(&old_dir);
    let has_db =
        db_files::has_database(&old_db).map_err(|e| DataDirError::io("read", &old_db, e))?;
    if !has_db {
        return Err(DataDirError::NoDatabase(old_db));
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

/// 保存先を、データベースのある既存のフォルダ `dir` に変える。データは移さず、中身を確かめてから
/// `config.json` に記録するだけ（アーキテクチャ設計書 6章・6.3「保存先を選び直す」）。
///
/// 今の保存先にデータベースが見つからないとき（[`super::OpenError::NotFound`]）に、データベースの
/// ある別のフォルダ（同期フォルダの場所が変わった、ドライブの名前が変わったなど）を指し直すのに使う。
/// [`change_data_dir`] は移動先にデータベースがあると断るので、こちらを使う。今の保存先のファイルには
/// 触れない（データベースが残っていても消さない）。
///
/// 確かめること（通らなければ、何も変えずにエラーを返す）：
///
/// 1. `dir` が保存先として書ける形（絶対パスで、`..`・`.` を含まず、UTF-8 で表せる）で、フォルダである。
/// 2. `dir` に中身のある `yunagi.db` がある（[`DataDirError::NoDatabase`]）。
/// 3. それを読み取り専用で開け、整合性の確認が通り（[`DataDirError::DatabaseCorrupted`]）、
///    スキーマの番号が 1 以上（[`DataDirError::NotYunagiDatabase`]）で、このアプリの知る範囲にある
///    （[`DataDirError::UnsupportedSchema`]）。ファイルには書かない。
///
/// `dir` が既定ディレクトリ（の実体）なら、`config.json` から `data_dir` の項目を消す（既定ディレクトリの
/// 絶対パスを書くと、既定ディレクトリごと移したときに古いパスを指してしまうため。[`Config::save`]）。
///
/// [`change_data_dir`] と同じく、`yunagi.lock` のロック `lock` を持ち、データベースを閉じてから呼ぶ
/// （記録した後は、同じロックで [`super::Database::open`] を呼べば `dir` のデータベースを開く）。
pub fn use_existing_data_dir(lock: &DbLock, dir: &Path) -> Result<(), DataDirError> {
    let default_dir = lock.default_dir();
    check_new_dir(dir)?;
    if !dir.is_dir() {
        return Err(DataDirError::NotADirectory(dir.to_path_buf()));
    }
    let db = database_path(dir);
    if !db_files::has_database(&db).map_err(|e| DataDirError::io("read", &db, e))? {
        return Err(DataDirError::NoDatabase(db));
    }
    inspect_database(&db)?;
    let mut config = Config::load(default_dir)?;
    config.data_dir = dir.to_path_buf();
    config.save(default_dir)
}

/// 保存先にするパス `dir` が、`config.json` に書ける形かを確かめる：絶対パスで、`..`・`.` を含まず、
/// UTF-8 で表せる。
fn check_new_dir(dir: &Path) -> Result<(), DataDirError> {
    if !dir.is_absolute() {
        return Err(DataDirError::NotAbsolute(dir.to_path_buf()));
    }
    // `..` は、まだ無いフォルダの後にあると実体を求められず、change_data_dir の「同じフォルダ」
    // 「backups/ の中」の確認をすり抜ける（作った後で元の保存先を指し、元のデータを消してしまう）
    if dir
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(DataDirError::NotNormalized(dir.to_path_buf()));
    }
    // JSON の文字列は UTF-8 なので、写し終えてから記録で失敗しないよう、先に確かめる
    if dir.to_str().is_none() {
        return Err(DataDirError::NotUtf8(dir.to_path_buf()));
    }
    Ok(())
}

/// `db` を開き、Yunagi のデータベースとして開けるかを確かめる。
///
/// まず読み取り専用で開いて確かめる（断るときにも、選んだデータベースに書かない。別の端末の新しい版が
/// 使っているもの（同期の途中の `-wal` があるもの）を書き換えないように）。読み取り専用で開くと
/// SQLite が WAL モードのデータベースの横に空の `-wal`・`-shm` を作ることがあるので、もともと無く、
/// 閉じた後も `-wal` が空なら消す（同期フォルダに残さない）。
///
/// 書き込みの途中で止まったデータベース（中身のある `-journal` が残ったもの）は、読み取り専用では
/// SQLite が巻き戻せずに読めない（[`super::Database::open`] なら開ける正しいデータベースを断ってしまう）。
/// そのときだけ、作らない形の読み書きで開き直して確かめる。このとき SQLite は開くときに `-journal` を
/// 巻き戻す（どの版の SQLite も、開けば必ず行う復旧）が、表とスキーマの番号には書かない。
fn inspect_database(db: &Path) -> Result<(), DataDirError> {
    let [wal, shm, _] = db_files::side_files(db);
    let existed = wal.symlink_metadata().is_ok() || shm.symlink_metadata().is_ok();
    let result = inspect_with(db, OpenFlags::SQLITE_OPEN_READ_ONLY);
    if !existed && fs::metadata(&wal).map_or(true, |m| m.len() == 0) {
        let _ = fs::remove_file(&wal);
        let _ = fs::remove_file(&shm);
    }
    match result {
        Err(DataDirError::Inspect { source, .. }) if needs_rollback(&source) => {
            let mut flags = OpenFlags::default();
            flags.remove(OpenFlags::SQLITE_OPEN_CREATE);
            inspect_with(db, flags)
        }
        result => result,
    }
}

/// 読み取り専用の接続では、残った `-journal` を巻き戻せずに読めなかった失敗か。
fn needs_rollback(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(f, _) if f.extended_code == rusqlite::ffi::SQLITE_READONLY_ROLLBACK
    )
}

/// `db` を `flags` で開き、整合性とスキーマの番号を確かめる（[`inspect_database`]）。
fn inspect_with(db: &Path, flags: OpenFlags) -> Result<(), DataDirError> {
    let read_error = |source| DataDirError::Inspect {
        path: db.to_path_buf(),
        source,
    };
    let conn = Connection::open_with_flags(db, flags).map_err(read_error)?;
    let result: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(read_error)?;
    if result != "ok" {
        return Err(DataDirError::DatabaseCorrupted {
            path: db.to_path_buf(),
            result,
        });
    }
    // 開く・復元するときと同じ範囲の確認（migration::check_supported）
    match migration::check_supported(&conn, migration::MIGRATIONS) {
        // 空のデータベースや別のアプリのもの。開くと、そこに空の Yunagi のデータベースを作ってしまう
        Ok(0) => Err(DataDirError::NotYunagiDatabase(db.to_path_buf())),
        Ok(_) => Ok(()),
        Err(StorageError::UnsupportedSchema { found, supported }) => {
            Err(DataDirError::UnsupportedSchema {
                path: db.to_path_buf(),
                found,
                supported,
            })
        }
        Err(StorageError::Sqlite(e)) => Err(read_error(e)),
        Err(other) => unreachable!("check_supported only reads the schema version: {other:?}"),
    }
}

/// 移動先に、上書きしてしまうデータが無いことを確かめる。
fn check_target(new_dir: &Path) -> Result<(), DataDirError> {
    // データベースの跡（中身のある yunagi.db・-wal・-journal、シンボリックリンク、調べられないもの）
    if let Some(trace) = db_files::find_trace(&database_path(new_dir)) {
        return Err(DataDirError::TargetHasDatabase(trace));
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

    // データベースがあることは change_data_dir が確かめてある
    let moving = new_dir.join(MOVING_FILE_NAME);
    // 写しは1つのファイルにまとめるが、SQLite が途中で横に作るファイルも、
    // 失敗したときに消すよう記録する。前に止まった移動の残りがあれば消す
    for path in db_files::with_side_files(&moving) {
        let _ = fs::remove_file(&path);
        created.file(path);
    }
    copy_database(&database_path(old_dir), &moving)?;
    let new_db = database_path(new_dir);
    fs::rename(&moving, &new_db).map_err(|e| DataDirError::io("rename", &moving, e))?;
    created.file(new_db);
    let _ = db_copy::sync_dir(new_dir);
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

/// データベースのファイル `from` を `to` に写す（[`db_copy::copy`]。写したものの整合性も確かめ、
/// 失敗したら作りかけの `to` とその横のファイルを消す）。`from` を開けなければ、そのパスを持つ
/// [`DataDirError::Inspect`] を返す。
pub(super) fn copy_database(from: &Path, to: &Path) -> Result<(), DataDirError> {
    // 元は読み取り専用で開く（元のファイルに書かず、閉じるときに WAL を書き戻すこともしない）。
    // 開けないときは、どのファイルかを知らせる
    let src =
        Connection::open_with_flags(from, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|source| {
            DataDirError::Inspect {
                path: from.to_path_buf(),
                source,
            }
        })?;
    Ok(db_copy::copy(&src, to)?)
}

/// 移し終えた後で、元の保存先のデータベースと `backups/` を消す。消せなかったものを返す。
fn remove_old_data(old_dir: &Path) -> Vec<PathBuf> {
    let mut leftovers = Vec::new();
    for path in db_files::with_side_files(&database_path(old_dir)) {
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
    /// 写したデータベースの整合性の確認（`integrity_check`）が通らなかった。ふつうは元が壊れている。
    /// `path` は写した元のデータベース（元が分からないときは、作ろうとした写し）。
    CopyCorrupted { path: PathBuf, result: String },
    /// `path` に中身のあるデータベースが無い（保存先の変更では今の保存先に、選び直しでは選んだ
    /// フォルダに）。何も変えていない。
    NoDatabase(PathBuf),
    /// 選んだ保存先が無いか、フォルダでない。
    NotADirectory(PathBuf),
    /// データベース `path` を読めなかった（SQLite のデータベースでない、開けないなど）。
    /// 選んだ保存先のデータベースを確かめるときと、写すときの元を開くときに使う。
    Inspect {
        path: PathBuf,
        source: rusqlite::Error,
    },
    /// 選んだ保存先のデータベース `path` の整合性の確認（`integrity_check`）が通らなかった。
    DatabaseCorrupted { path: PathBuf, result: String },
    /// 選んだ保存先のデータベース `path` が、Yunagi のデータベースではない（スキーマの番号が 0。
    /// 空のデータベースや別のアプリのもの）。
    NotYunagiDatabase(PathBuf),
    /// 選んだ保存先のデータベース `path` のスキーマの番号 `found` を、このアプリが知らない
    /// （ふつうは新しい版で作られたもの。`supported` まで知っている）。
    UnsupportedSchema {
        path: PathBuf,
        found: i64,
        supported: u32,
    },
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
        // 原因のエラーの文は含めない（source() で返す。README「エラーの表示」）
        match self {
            Self::NoDefaultDir { var } => write!(
                f,
                "could not determine the default directory: environment variable {var} is not set to an absolute path"
            ),
            Self::Io { action, path, .. } => write!(f, "could not {action} {}", path.display()),
            Self::InvalidConfig { path, .. } => {
                write!(f, "invalid config file {}", path.display())
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
            Self::Sqlite(_) => write!(f, "database error while copying"),
            Self::CopyCorrupted { path, result } => write!(
                f,
                "the copy of the database {} failed the integrity check: {result}",
                path.display()
            ),
            Self::NoDatabase(path) => {
                write!(f, "no Yunagi database found at {}", path.display())
            }
            Self::NotADirectory(path) => {
                write!(f, "{} does not exist or is not a folder", path.display())
            }
            Self::Inspect { path, .. } => {
                write!(f, "could not read the database {}", path.display())
            }
            Self::DatabaseCorrupted { path, result } => write!(
                f,
                "the database {} failed the integrity check: {result}",
                path.display()
            ),
            Self::NotYunagiDatabase(path) => {
                write!(f, "{} is not a Yunagi database", path.display())
            }
            Self::UnsupportedSchema {
                path,
                found,
                supported,
            } => write!(
                f,
                "the database {} has schema version {found}, which this app does not support (supports up to {supported})",
                path.display()
            ),
        }
    }
}

impl std::error::Error for DataDirError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidConfig { source, .. } => Some(source),
            Self::Sqlite(e) => Some(e),
            Self::Inspect { source, .. } => Some(source),
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
            CopyError::Io {
                action,
                path,
                source,
            } => Self::Io {
                action,
                path,
                source,
            },
            CopyError::Corrupted { path, result } => Self::CopyCorrupted { path, result },
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

    /// 既定ディレクトリにデータベース（閉じたもの）を作った [`dirs`]。
    fn dirs_with_db() -> Dirs {
        let d = dirs();
        drop(make_db_with_wal(&d.default));
        d
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
        // Windows 上でテストしても Unix 形式のパスは絶対と判定されないので、動いている OS で絶対のパスを使う
        let home = if cfg!(windows) {
            r"C:\Users\a"
        } else {
            "/Users/a"
        };
        let env = env_of(&[("HOME", home)]);
        assert_eq!(
            default_dir_for(Platform::MacOs, env).unwrap(),
            Path::new(home).join("Library/Application Support/Yunagi")
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

    /// アーキテクチャ設計書 6.5：既定ディレクトリと WebView の場所は、同じ親（`local_data_root_for`）の
    /// 下になる。`XDG_DATA_HOME` の未設定・空・相対・絶対の 4 通りで確かめる（環境変数は書き換えず、
    /// 引く関数を差し替える）。
    #[cfg(unix)]
    #[test]
    fn linux_default_dir_and_webview_dir_share_a_parent() {
        let cases = [
            (None, "/home/a/.local/share"),
            (Some(""), "/home/a/.local/share"),
            (Some("relative/path"), "/home/a/.local/share"),
            (Some("/data"), "/data"),
        ];
        for (xdg, root) in cases {
            let mut vars = vec![("HOME", "/home/a")];
            if let Some(v) = xdg {
                vars.push(("XDG_DATA_HOME", v));
            }
            let default = default_dir_for(Platform::Linux, env_of(&vars)).unwrap();
            let parent = local_data_root_for(Platform::Linux, env_of(&vars)).unwrap();
            assert_eq!(parent, PathBuf::from(root), "XDG_DATA_HOME = {xdg:?}");
            assert_eq!(default, parent.join("yunagi"), "XDG_DATA_HOME = {xdg:?}");
            assert_eq!(default.parent(), Some(parent.as_path()));
        }
    }

    /// 識別子は `src-tauri/tauri.conf.json` の `identifier` と同じ（アーキテクチャ設計書 6.1）。
    #[test]
    fn app_identifier_matches_tauri_conf() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../../../../src-tauri/tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"], APP_IDENTIFIER);
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
        let d = dirs_with_db();
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

        if cfg!(windows) {
            // Windows は開いたままのファイルを消せないので、閉じていない接続の元のデータベース
            // （と WAL・共有メモリ）が「消せなかったもの」として返る（backups/ は消えている）。
            // 移した後の確認のため、閉じた今、自分で消す
            let mut expected = vec![
                database_path(&d.default),
                db_files::side_files(&database_path(&d.default))[0].clone(),
                db_files::side_files(&database_path(&d.default))[1].clone(),
            ];
            let mut leftovers = report.leftovers.clone();
            expected.sort();
            leftovers.sort();
            assert_eq!(leftovers, expected);
            // 接続を閉じると、SQLite が WAL と共有メモリを自分で消すことがある（もう無くてよい）
            for path in &report.leftovers {
                let _ = fs::remove_file(path);
            }
        } else {
            assert_eq!(report, ChangeReport::default());
        }
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
        let d = dirs_with_db();
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
        let d = dirs_with_db();
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
        let d = dirs_with_db();
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
        let wal = db_files::with_side_files(&database_path(&d.default))[1].clone();
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
        let db = Database::open(d.lock()).unwrap();
        let lock = db.close_keeping_lock().unwrap();
        let new_dir = d.root.join("new");
        change_data_dir(&lock, &new_dir).unwrap();
        // 開き直すと、config.json に記録した新しい保存先を開く
        let db = Database::open(lock).unwrap();
        assert_eq!(
            db.connection()
                .path()
                .map(|p| Path::new(p).canonicalize().unwrap()),
            Some(database_path(&new_dir).canonicalize().unwrap())
        );
        let mode: String = db
            .connection()
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
    }

    #[test]
    fn change_uses_the_config_of_the_locked_default_dir() {
        let d = dirs_with_db();
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
    fn change_without_database_is_refused_and_keeps_its_traces() {
        let d = dirs();
        // 本体はまだ届いていないが、-wal と backups/ がある（同期の途中など）
        let wal = db_files::side_files(&database_path(&d.default))[0].clone();
        fs::write(&wal, b"kept in wal").unwrap();
        fs::create_dir(backups_dir(&d.default)).unwrap();
        fs::write(backups_dir(&d.default).join("a.db"), b"backup").unwrap();
        // 中身の無い yunagi.db も、移すデータベースではない
        for body in [None, Some(&b""[..])] {
            if let Some(body) = body {
                fs::write(database_path(&d.default), body).unwrap();
            }
            let new_dir = d.root.join("empty");
            let err = d.change(&new_dir);
            assert!(
                matches!(&err, Err(DataDirError::NoDatabase(p)) if *p == database_path(&d.default)),
                "{err:?}"
            );
            assert!(!new_dir.exists());
            assert_eq!(resolve_data_dir(&d.dd).unwrap(), d.default);
            assert!(!d.default.join(CONFIG_FILE_NAME).exists());
            assert_eq!(fs::read(&wal).unwrap(), b"kept in wal");
            assert_eq!(
                fs::read(backups_dir(&d.default).join("a.db")).unwrap(),
                b"backup"
            );
        }
    }

    #[test]
    fn changing_back_to_the_default_dir_removes_the_entry() {
        let d = dirs_with_db();
        fs::write(d.default.join(CONFIG_FILE_NAME), r#"{"theme": "dark"}"#).unwrap();
        let other = d.root.join("other");
        d.change(&other).unwrap();
        d.change(&d.default).unwrap();
        // 既定ディレクトリの絶対パスは書かない（既定ディレクトリごと移しても、古いパスを指さない）
        let saved: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(d.default.join(CONFIG_FILE_NAME)).unwrap())
                .unwrap();
        assert_eq!(saved, serde_json::json!({ "theme": "dark" }));
        assert_eq!(read_note(&d.default), "kept in wal");
    }

    // --- 保存先を選び直す（use_existing_data_dir） ---

    /// `dir` に、番号 `user_version` で表のあるデータベースを作る。
    fn plain_db(dir: &Path, user_version: i64) {
        fs::create_dir_all(dir).unwrap();
        let conn = Connection::open(database_path(dir)).unwrap();
        conn.execute_batch("CREATE TABLE note(body TEXT); INSERT INTO note VALUES ('elsewhere');")
            .unwrap();
        conn.pragma_update(None, "user_version", user_version)
            .unwrap();
    }

    #[test]
    fn use_existing_data_dir_records_a_folder_with_a_database() {
        let d = dirs();
        // 今の保存先は見つからない（同期フォルダの場所が変わったなど）
        let missing = d.root.join("old-sync/Yunagi");
        fs::write(
            d.default.join(CONFIG_FILE_NAME),
            serde_json::json!({ "data_dir": missing, "theme": "dark" }).to_string(),
        )
        .unwrap();
        // 別の場所で作った本物のデータベースを、選ぶフォルダに置く
        let found = d.root.join("new-sync/Yunagi");
        fs::create_dir_all(&found).unwrap();
        {
            let other = TempDir::new().unwrap();
            let lock =
                DbLock::acquire(&DefaultDir::for_test(other.path()), Duration::ZERO).unwrap();
            let db = Database::open(lock).unwrap();
            db.connection()
                .execute(
                    r#"INSERT INTO setting (key, value) VALUES ('found', '"elsewhere"')"#,
                    [],
                )
                .unwrap();
            db.close().unwrap();
            fs::copy(database_path(other.path()), database_path(&found)).unwrap();
        }
        let before = fs::read(database_path(&found)).unwrap();

        use_existing_data_dir(&d.lock(), &found).unwrap();
        assert_eq!(resolve_data_dir(&d.dd).unwrap(), found);
        let saved: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(d.default.join(CONFIG_FILE_NAME)).unwrap())
                .unwrap();
        assert_eq!(saved["theme"], "dark");
        // 確かめただけで、選んだデータベースには書いておらず、横にファイルも残さない
        assert_eq!(fs::read(database_path(&found)).unwrap(), before);
        assert_eq!(super::super::test_util::names(&found), [DATABASE_FILE_NAME]);
        // 記録したので、同じロックで開くと選んだデータベースを開く
        let db = Database::open(d.lock()).unwrap();
        let value: String = db
            .connection()
            .query_row("SELECT value FROM setting WHERE key = 'found'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(value, "\"elsewhere\"");
    }

    #[test]
    fn use_existing_data_dir_accepts_a_database_left_with_a_hot_journal() {
        // 書き込みの途中で止まったデータベース：本体には書きかけのページがあり、中身のある
        // -journal が残っている。SQLite は開くときに巻き戻すので、読み取り専用では開けない
        let d = dirs();
        let src = d.root.join("src");
        plain_db(&src, 1);
        let found = d.root.join("found");
        fs::create_dir(&found).unwrap();
        {
            let conn = Connection::open(database_path(&src)).unwrap();
            // 書きかけのページを、確定の前に本体へ書き出させる
            conn.execute_batch("PRAGMA cache_size = 1; BEGIN;").unwrap();
            for i in 0..500 {
                conn.execute("INSERT INTO note VALUES (?1)", [format!("{i:0>200}")])
                    .unwrap();
            }
            let journal = db_files::side_files(&database_path(&src))[2].clone();
            assert!(fs::metadata(&journal).unwrap().len() > 0);
            fs::copy(database_path(&src), database_path(&found)).unwrap();
            fs::copy(
                &journal,
                db_files::side_files(&database_path(&found))[2].clone(),
            )
            .unwrap();
        }
        use_existing_data_dir(&d.lock(), &found).unwrap();
        assert_eq!(resolve_data_dir(&d.dd).unwrap(), found);
        // 巻き戻され、確定していた行だけが残る
        assert_eq!(read_note(&found), "elsewhere");
        let count: i64 = Connection::open(database_path(&found))
            .unwrap()
            .query_row("SELECT count(*) FROM note", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn use_existing_data_dir_reads_writes_still_in_the_wal() {
        let d = dirs();
        let found = d.root.join("found");
        // 開いたままの接続：本体には表が無く、WAL にだけある
        let conn = make_db_with_wal(&found);
        conn.pragma_update(None, "user_version", 1).unwrap();
        use_existing_data_dir(&d.lock(), &found).unwrap();
        drop(conn);
        assert_eq!(resolve_data_dir(&d.dd).unwrap(), found);
    }

    #[test]
    fn use_existing_data_dir_of_the_default_dir_removes_the_entry() {
        let d = dirs_with_db();
        fs::write(
            d.default.join(CONFIG_FILE_NAME),
            serde_json::json!({ "data_dir": d.root.join("gone") }).to_string(),
        )
        .unwrap();
        // 既定ディレクトリのデータベースは番号 0 の表なので、Yunagi の番号を付ける
        Connection::open(database_path(&d.default))
            .unwrap()
            .pragma_update(None, "user_version", 1)
            .unwrap();
        use_existing_data_dir(&d.lock(), &d.default).unwrap();
        assert_eq!(
            fs::read_to_string(d.default.join(CONFIG_FILE_NAME)).unwrap(),
            "{}\n"
        );
        assert_eq!(resolve_data_dir(&d.dd).unwrap(), d.default);
    }

    #[test]
    fn use_existing_data_dir_refuses_folders_it_cannot_open() {
        let d = dirs();
        let config = d.default.join(CONFIG_FILE_NAME);
        let recorded = d.root.join("recorded");
        fs::write(
            &config,
            serde_json::json!({ "data_dir": recorded }).to_string(),
        )
        .unwrap();
        let before = fs::read(&config).unwrap();
        let refuse = |dir: &Path| {
            let err = use_existing_data_dir(&d.lock(), dir).unwrap_err();
            assert_eq!(fs::read(&config).unwrap(), before, "{err:?}");
            err
        };

        assert!(matches!(
            refuse(Path::new("relative")),
            DataDirError::NotAbsolute(_)
        ));
        assert!(matches!(
            refuse(&d.root.join("a/../b")),
            DataDirError::NotNormalized(_)
        ));
        let missing = d.root.join("missing");
        assert!(matches!(refuse(&missing), DataDirError::NotADirectory(_)));
        assert!(!missing.exists());

        // フォルダはあるが、データベースが無い（-wal・backups/ だけ、中身の無い yunagi.db）
        let empty = d.root.join("empty");
        fs::create_dir_all(backups_dir(&empty)).unwrap();
        fs::write(empty.join("yunagi.db-wal"), b"wal").unwrap();
        assert!(matches!(refuse(&empty), DataDirError::NoDatabase(_)));
        fs::write(database_path(&empty), b"").unwrap();
        assert!(matches!(refuse(&empty), DataDirError::NoDatabase(_)));

        let garbage = d.root.join("garbage");
        fs::create_dir(&garbage).unwrap();
        fs::write(
            database_path(&garbage),
            b"not a database at all, just some text",
        )
        .unwrap();
        assert!(matches!(refuse(&garbage), DataDirError::Inspect { .. }));

        let other_app = d.root.join("other-app");
        plain_db(&other_app, 0);
        assert!(matches!(
            refuse(&other_app),
            DataDirError::NotYunagiDatabase(_)
        ));

        let newer = d.root.join("newer");
        plain_db(&newer, 99);
        assert!(matches!(
            refuse(&newer),
            DataDirError::UnsupportedSchema { found: 99, .. }
        ));
        // WAL モードのデータベースを断っても、横に -wal・-shm を残さない（同期フォルダに置かせない）
        Connection::open(database_path(&newer))
            .unwrap()
            .pragma_update(None, "journal_mode", "WAL")
            .unwrap();
        assert_eq!(super::super::test_util::names(&newer), [DATABASE_FILE_NAME]);
        assert!(matches!(
            refuse(&newer),
            DataDirError::UnsupportedSchema { found: 99, .. }
        ));
        assert_eq!(super::super::test_util::names(&newer), [DATABASE_FILE_NAME]);
        // 別の端末の新しい版が書いている途中（-wal に中身がある）でも、断るときに書き換えない
        let writer = Connection::open(database_path(&newer)).unwrap();
        writer.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        writer
            .execute("INSERT INTO note VALUES ('in the wal')", [])
            .unwrap();
        let wal = db_files::side_files(&database_path(&newer))[0].clone();
        let (db_before, wal_before) = (
            fs::read(database_path(&newer)).unwrap(),
            fs::read(&wal).unwrap(),
        );
        assert!(!wal_before.is_empty());
        assert!(matches!(
            refuse(&newer),
            DataDirError::UnsupportedSchema { found: 99, .. }
        ));
        assert_eq!(fs::read(database_path(&newer)).unwrap(), db_before);
        assert_eq!(fs::read(&wal).unwrap(), wal_before);
        drop(writer);

        let corrupted = d.root.join("corrupted");
        plain_db(&corrupted, 1);
        {
            let conn = Connection::open(database_path(&corrupted)).unwrap();
            for i in 0..200 {
                conn.execute("INSERT INTO note VALUES (?1)", [format!("row {i:0>100}")])
                    .unwrap();
            }
        }
        super::super::test_util::corrupt_pages(&database_path(&corrupted));
        assert!(matches!(
            refuse(&corrupted),
            DataDirError::DatabaseCorrupted { .. } | DataDirError::Inspect { .. }
        ));
        assert_eq!(resolve_data_dir(&d.dd).unwrap(), recorded);
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
    fn corrupted_database_is_not_moved() {
        let d = dirs_with_db();
        // 索引の定義を書き換えて、整合性の確認だけが通らないようにする（ページはそのまま写る）
        {
            let conn = Connection::open(database_path(&d.default)).unwrap();
            conn.execute_batch(
                "CREATE INDEX note_body ON note (body);
                 PRAGMA writable_schema = ON;
                 UPDATE sqlite_schema SET sql = 'CREATE INDEX note_body ON note (length(body))'
                     WHERE name = 'note_body';
                 PRAGMA writable_schema = OFF;",
            )
            .unwrap();
        }
        let new_dir = d.root.join("new");
        let err = d.change(&new_dir);
        assert!(
            matches!(err, Err(DataDirError::CopyCorrupted { .. })),
            "{err:?}"
        );
        assert!(!new_dir.exists());
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
