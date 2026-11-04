//! 既定ディレクトリのずれの検出（docs/design/01-architecture.md 4.5）。
//!
//! アプリと、エージェントが起動する CLI・MCP で環境変数が違うと、既定ディレクトリが別の場所になる。
//! クライアントはアプリの環境変数を知る手段が無いので、環境変数に頼らず OS に尋ねた標準の場所を
//! 「別の候補」として調べ、そこでアプリ（または直接モードの操作）が動いているかを見る。
//!
//! - [`alternative`]：別の候補を求める（最大 1 つ。自分の既定ディレクトリと実体が同じなら無し）。
//! - [`Candidate::probe`]：候補を調べる。読むだけで、フォルダもファイルも作らず、ソケットにも
//!   つながない。候補の `yunagi.lock` を、作らずに既存のものだけ開いて 1 回だけ取ってみて、すぐ離す。
//! - [`own_storage`]：自分の側の保存先に使えるデータベースがあるか。
//! - [`judge`]：4.5 の判定（直接モードに入る前に、操作ごとに 1 回）。
//!
//! CLI・MCP・`doctor`・`agent setup` が同じものを使う（作ったのは YUN-90）。

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use super::data_dir::{self, Config, DefaultDir, Platform};
use super::{LOCK_FILE_NAME, db_files};

/// 既定ディレクトリを求めるのに使う環境変数の名前（`platform` のもの）。値は出さない
/// （`storage_unavailable` の案内と `doctor` に、名前だけを出す。4.5・MCP・CLI 仕様書 3.1）。
pub fn default_dir_vars(platform: Platform) -> &'static [&'static str] {
    match platform {
        Platform::MacOs => &["HOME"],
        Platform::Windows => &["LOCALAPPDATA"],
        Platform::Linux => &["XDG_DATA_HOME", "HOME"],
    }
}

/// 別の候補の既定ディレクトリを使っているか（調べた結果）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateStatus {
    /// 候補のフォルダが無い、またはフォルダはあるがロックもデータベースも無い。
    Absent,
    /// 使っているプロセスはいないが、データベースがある（アプリを閉じている間はふつうの状態）。
    HasData,
    /// 使っているプロセスがいる（`yunagi.lock` を取れなかった。アプリか直接モードの操作かは区別しない）。
    InUse,
    /// 調べられない（権限が無いなど）。エラーにも「いる」にもせず、ずれの判定には使わない。
    Unknown,
}

/// 別の候補の既定ディレクトリ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    dir: PathBuf,
}

/// [`Candidate::probe`] の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateInfo {
    /// 候補の既定ディレクトリ。
    pub dir: PathBuf,
    /// 使っているプロセスがいるか。
    pub status: CandidateStatus,
    /// 候補の保存先（候補の `config.json` の `data_dir`。無ければ候補そのもの）。`config.json` を
    /// 読めなかったときは `None`。
    pub data_dir: Option<PathBuf>,
}

impl Candidate {
    /// 候補の既定ディレクトリ。
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 候補を調べる（読むだけ）。
    ///
    /// 候補の `yunagi.lock` を、無ければ作らずに既存のものだけ読み書きの形で開き（Windows の
    /// `LockFileEx` は読みか書きで開いたファイルにしか使えないため）、1 回だけ取ってみる。取れなければ
    /// 使っているプロセスがいる。取れたらすぐ離す。ファイルが無ければ、いない。開けない・調べられない
    /// （権限が無いなど）ときは [`CandidateStatus::Unknown`]。あわせて候補の保存先と `yunagi.db` の有無を
    /// 読み取りだけで調べる。
    pub fn probe(&self) -> CandidateInfo {
        let data_dir = Config::load_from(&self.dir).ok().map(|c| c.data_dir);
        let lock_state = lock_state(&self.dir.join(LOCK_FILE_NAME));
        let status = match lock_state {
            LockState::Held => CandidateStatus::InUse,
            LockState::Unknown => CandidateStatus::Unknown,
            LockState::Free | LockState::NoFile => {
                let has_data = data_dir
                    .as_deref()
                    .unwrap_or(&self.dir)
                    .join(data_dir::DATABASE_FILE_NAME)
                    .metadata()
                    .is_ok_and(|m| m.is_file() && m.len() > 0);
                // 設定を読めないときは、データがあるとも無いとも言えない
                if data_dir.is_none() && lock_state == LockState::NoFile {
                    CandidateStatus::Unknown
                } else if has_data {
                    CandidateStatus::HasData
                } else {
                    CandidateStatus::Absent
                }
            }
        };
        CandidateInfo {
            dir: self.dir.clone(),
            status,
            data_dir,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LockState {
    /// 別のプロセスが持っている。
    Held,
    /// 取れた（すぐ離した）。
    Free,
    /// ファイルもフォルダも無い。
    NoFile,
    /// 調べられない。
    Unknown,
}

fn lock_state(path: &Path) -> LockState {
    use std::fs::TryLockError;
    let file = match OpenOptions::new().read(true).write(true).open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LockState::NoFile,
        // フォルダの途中がフォルダでない場合も、無いものとして扱う
        Err(e) if e.kind() == std::io::ErrorKind::NotADirectory => return LockState::NoFile,
        Err(_) => return LockState::Unknown,
    };
    match file.try_lock() {
        Ok(()) => {
            let _ = file.unlock();
            LockState::Free
        }
        Err(TryLockError::WouldBlock) => LockState::Held,
        Err(TryLockError::Error(_)) => LockState::Unknown,
    }
}

/// 自分の既定ディレクトリ `own` に対する別の候補を、OS に尋ねて求める。環境変数は見ない。
/// 自分と実体（シンボリックリンクを辿ったもの）が同じなら、ずれていないので `None`。
/// OS が答えられないときも `None`。
pub fn alternative(own: &DefaultDir) -> Option<Candidate> {
    alternative_from_home(own, Platform::current(), os_home_dir())
}

/// [`alternative`] の、ホームを渡す版（試験が、本物のホームではなく一時フォルダで確かめるため）。
/// `home` は、macOS・Linux はパスワードデータベースのホーム、Windows は LocalAppData。
pub fn alternative_from_home(
    own: &DefaultDir,
    platform: Platform,
    home: Option<PathBuf>,
) -> Option<Candidate> {
    let home = home.filter(|h| h.is_absolute())?;
    let dir = match platform {
        Platform::MacOs => home.join("Library/Application Support/Yunagi"),
        Platform::Windows => home.join("Yunagi"),
        Platform::Linux => home.join(".local/share/yunagi"),
    };
    if data_dir::resolve(&dir) == data_dir::resolve(own.path()) {
        return None;
    }
    Some(Candidate { dir })
}

/// 自分の側の保存先の状態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnStorage {
    /// 自分の既定ディレクトリがあるか。
    pub default_dir_exists: bool,
    /// 自分の保存先（自分の既定ディレクトリの `config.json` の `data_dir`。無ければ既定ディレクトリ
    /// そのもの）。既定ディレクトリが無い、または `config.json` を読めなくて求められないときは `None`。
    pub data_dir: Option<PathBuf>,
    /// 保存先に使えるデータベース（中身のある `yunagi.db`）があるか。
    pub has_database: bool,
}

/// 自分の保存先の状態を、読み取りだけで調べる。
pub fn own_storage(own: &DefaultDir) -> OwnStorage {
    let default_dir_exists = own.path().is_dir();
    let data_dir = default_dir_exists
        .then(|| data_dir::resolve_data_dir(own).ok())
        .flatten();
    let has_database = data_dir
        .as_deref()
        .is_some_and(|d| db_files::has_database(&data_dir::database_path(d)).unwrap_or(false));
    OwnStorage {
        default_dir_exists,
        data_dir,
        has_database,
    }
}

/// 4.5 の判定の結果（調べたのが「使っているプロセスがいる」候補のときだけ、ずれとなりうる）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Judgement {
    /// ずれとは扱わない（1：候補が無い・調べられない・使っているプロセスがいない）。
    Normal,
    /// ずれ（2）：直接モードに入らず、`api_error`（`default_dir_mismatch`）を返す。
    /// `same_storage` は、自分の保存先が候補の保存先と同じ場合（そうでなければ、自分の保存先に
    /// 使えるデータベースが無い場合）。
    Mismatch { same_storage: bool },
    /// 環境を分けている形（3）：止めず、警告を 1 行出して直接モードで続ける。
    Separate,
}

/// 4.5 の判定。`candidate` は [`Candidate::probe`] の結果（候補が無ければ `None`）。
pub fn judge(own: &OwnStorage, candidate: Option<&CandidateInfo>) -> Judgement {
    let Some(c) = candidate.filter(|c| c.status == CandidateStatus::InUse) else {
        return Judgement::Normal;
    };
    let same_storage = match (&own.data_dir, &c.data_dir) {
        (Some(a), Some(b)) => data_dir::resolve(a) == data_dir::resolve(b),
        _ => false,
    };
    if same_storage {
        Judgement::Mismatch { same_storage: true }
    } else if !own.has_database {
        Judgement::Mismatch {
            same_storage: false,
        }
    } else {
        Judgement::Separate
    }
}

/// OS が答える、利用者のホーム（macOS・Linux：パスワードデータベース。`$HOME` ではない）か
/// LocalAppData（Windows：既知のフォルダの API）。
fn os_home_dir() -> Option<PathBuf> {
    os::home_dir()
}

#[cfg(unix)]
mod os {
    use std::ffi::{CStr, OsStr};
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;

    pub fn home_dir() -> Option<PathBuf> {
        // SAFETY: getpwuid_r に、自分で確保した領域と、その大きさを渡す。結果の文字列は、この関数の中で
        // 領域が生きている間にだけ読み、所有する PathBuf に写してから返す。
        unsafe {
            let uid = libc::geteuid();
            let mut size = match libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) {
                n if n > 0 => usize::try_from(n).unwrap_or(1024),
                _ => 1024,
            };
            loop {
                let mut buf = vec![0u8; size];
                let mut pwd: libc::passwd = std::mem::zeroed();
                let mut result: *mut libc::passwd = std::ptr::null_mut();
                let rc = libc::getpwuid_r(
                    uid,
                    &mut pwd,
                    buf.as_mut_ptr().cast(),
                    buf.len(),
                    &mut result,
                );
                if rc == libc::ERANGE && size < 1 << 20 {
                    size *= 2;
                    continue;
                }
                if rc != 0 || result.is_null() || pwd.pw_dir.is_null() {
                    return None;
                }
                let dir = CStr::from_ptr(pwd.pw_dir).to_bytes();
                return (!dir.is_empty()).then(|| PathBuf::from(OsStr::from_bytes(dir)));
            }
        }
    }
}

#[cfg(windows)]
mod os {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;

    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{
        FOLDERID_LocalAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
    };

    pub fn home_dir() -> Option<PathBuf> {
        // SAFETY: SHGetKnownFolderPath が返す、NUL で終わる文字列を、解放する前に写し取る。
        unsafe {
            let path =
                SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DEFAULT, None).ok()?;
            let os = OsString::from_wide(path.as_wide());
            CoTaskMemFree(Some(path.as_ptr().cast()));
            Some(PathBuf::from(os))
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod os {
    use std::path::PathBuf;

    pub fn home_dir() -> Option<PathBuf> {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};

    use super::*;

    fn own(dir: &Path) -> DefaultDir {
        DefaultDir::for_test(dir)
    }

    fn candidate_dir(home: &Path) -> PathBuf {
        home.join(".local/share/yunagi")
    }

    fn write_db(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(data_dir::DATABASE_FILE_NAME), b"x").unwrap();
    }

    #[test]
    fn the_candidate_is_the_standard_location_under_the_given_home() {
        let tmp = tempfile::tempdir().unwrap();
        let mine = tmp.path().join("mine");
        let home = tmp.path().join("home");
        let c = alternative_from_home(&own(&mine), Platform::Linux, Some(home.clone())).unwrap();
        assert_eq!(c.dir(), candidate_dir(&home));
        let c = alternative_from_home(&own(&mine), Platform::MacOs, Some(home.clone())).unwrap();
        assert_eq!(c.dir(), home.join("Library/Application Support/Yunagi"));
        let c = alternative_from_home(&own(&mine), Platform::Windows, Some(home.clone())).unwrap();
        assert_eq!(c.dir(), home.join("Yunagi"));
    }

    #[test]
    fn there_is_no_candidate_when_it_is_the_same_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let mine = candidate_dir(&home);
        fs::create_dir_all(&mine).unwrap();
        assert!(alternative_from_home(&own(&mine), Platform::Linux, Some(home.clone())).is_none());
        // シンボリックリンクを辿った実体が同じでも、ずれていない
        #[cfg(unix)]
        {
            let link = tmp.path().join("link");
            std::os::unix::fs::symlink(&mine, &link).unwrap();
            assert!(alternative_from_home(&own(&link), Platform::Linux, Some(home)).is_none());
        }
    }

    #[test]
    fn there_is_no_candidate_without_an_absolute_home() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(alternative_from_home(&own(tmp.path()), Platform::Linux, None).is_none());
        assert!(
            alternative_from_home(&own(tmp.path()), Platform::Linux, Some("rel".into())).is_none()
        );
    }

    #[test]
    fn probing_never_creates_anything() {
        let tmp = tempfile::tempdir().unwrap();
        let c = Candidate {
            dir: tmp.path().join("nothing/here"),
        };
        let info = c.probe();
        assert_eq!(info.status, CandidateStatus::Absent);
        assert!(!tmp.path().join("nothing").exists());
        // フォルダはあるがロックファイルは無い：ロックファイルを作らない
        let c = Candidate {
            dir: tmp.path().to_path_buf(),
        };
        assert_eq!(c.probe().status, CandidateStatus::Absent);
        assert!(!tmp.path().join(LOCK_FILE_NAME).exists());
    }

    #[test]
    fn probing_sees_a_lock_held_by_someone_else_and_releases_its_own() {
        let tmp = tempfile::tempdir().unwrap();
        let c = Candidate {
            dir: tmp.path().to_path_buf(),
        };
        let path = tmp.path().join(LOCK_FILE_NAME);
        let held = File::create(&path).unwrap();
        held.lock().unwrap();
        assert_eq!(c.probe().status, CandidateStatus::InUse);
        held.unlock().unwrap();
        // 調べ終わったら、すぐ別のプロセスが取れる
        assert_eq!(c.probe().status, CandidateStatus::Absent);
        held.try_lock().unwrap();
    }

    #[test]
    fn a_candidate_with_data_and_no_process_is_has_data() {
        let tmp = tempfile::tempdir().unwrap();
        write_db(tmp.path());
        File::create(tmp.path().join(LOCK_FILE_NAME)).unwrap();
        let info = Candidate {
            dir: tmp.path().to_path_buf(),
        }
        .probe();
        assert_eq!(info.status, CandidateStatus::HasData);
        assert_eq!(info.data_dir.as_deref(), Some(tmp.path()));
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_lock_file_is_unknown() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(LOCK_FILE_NAME);
        File::create(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        // root は権限に関係なく開けるので、開けないときだけ確かめる
        if File::open(&path).is_err() {
            let info = Candidate {
                dir: tmp.path().to_path_buf(),
            }
            .probe();
            assert_eq!(info.status, CandidateStatus::Unknown);
        }
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn in_use(data_dir: Option<&str>) -> CandidateInfo {
        CandidateInfo {
            dir: "/c".into(),
            status: CandidateStatus::InUse,
            data_dir: data_dir.map(PathBuf::from),
        }
    }

    fn mine(has_database: bool, data_dir: Option<&str>) -> OwnStorage {
        OwnStorage {
            default_dir_exists: data_dir.is_some(),
            data_dir: data_dir.map(PathBuf::from),
            has_database,
        }
    }

    #[test]
    fn the_judgement_follows_the_decision_order() {
        // 1：候補が無い、調べられない、使っているプロセスがいない
        assert_eq!(judge(&mine(false, None), None), Judgement::Normal);
        for status in [
            CandidateStatus::Absent,
            CandidateStatus::HasData,
            CandidateStatus::Unknown,
        ] {
            let c = CandidateInfo {
                status,
                ..in_use(None)
            };
            assert_eq!(judge(&mine(false, None), Some(&c)), Judgement::Normal);
        }
        // 2：使っているプロセスがいて、自分の側に使えるデータベースが無い（既定ディレクトリが無い場合も）
        assert_eq!(
            judge(&mine(false, None), Some(&in_use(Some("/c")))),
            Judgement::Mismatch {
                same_storage: false
            }
        );
        assert_eq!(
            judge(&mine(false, Some("/m")), Some(&in_use(Some("/c")))),
            Judgement::Mismatch {
                same_storage: false
            }
        );
        // 2：自分の保存先が候補の保存先と同じ（使えるデータベースがあっても）
        assert_eq!(
            judge(&mine(true, Some("/s")), Some(&in_use(Some("/s")))),
            Judgement::Mismatch { same_storage: true }
        );
        // 3：自分の側に別のデータベースがある
        assert_eq!(
            judge(&mine(true, Some("/m")), Some(&in_use(Some("/c")))),
            Judgement::Separate
        );
        // 候補の保存先が分からなくても、自分にデータベースがあれば止めない
        assert_eq!(
            judge(&mine(true, Some("/m")), Some(&in_use(None))),
            Judgement::Separate
        );
    }

    #[test]
    fn own_storage_reads_the_config_and_the_database() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = own_storage(&own(&tmp.path().join("none")));
        assert_eq!(
            missing,
            OwnStorage {
                default_dir_exists: false,
                data_dir: None,
                has_database: false
            }
        );
        let s = own_storage(&own(tmp.path()));
        assert!(s.default_dir_exists && !s.has_database);
        assert_eq!(s.data_dir.as_deref(), Some(tmp.path()));
        write_db(tmp.path());
        assert!(own_storage(&own(tmp.path())).has_database);
    }

    #[test]
    fn the_default_dir_variables_are_named_per_platform() {
        assert_eq!(default_dir_vars(Platform::Linux), ["XDG_DATA_HOME", "HOME"]);
        assert_eq!(default_dir_vars(Platform::MacOs), ["HOME"]);
        assert_eq!(default_dir_vars(Platform::Windows), ["LOCALAPPDATA"]);
    }

    #[cfg(unix)]
    #[test]
    fn the_os_home_is_an_absolute_path() {
        // 本物のホーム（パスワードデータベース）が求められる。中身には触れない
        assert!(os_home_dir().is_none_or(|h| h.is_absolute()));
    }
}
