//! 認証のトークン（docs/design/01-architecture.md 4.6）。
//!
//! - 形：OS の乱数 32 バイトを小文字の 16 進にした 64 文字。
//! - 待ち受けを始めるたびに作り直す。ソケット（パイプ）を作るより先に、本人だけが読み書きできる
//!   権限（0600）で書く。
//! - 比較は定数時間（[`matches()`]）。トークンをログ・エラーの文・診断に出さない。

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use yunagi_core::storage::data_dir::DefaultDir;

use crate::TOKEN_FILE_NAME;

/// トークンの元の乱数のバイト数。
pub const TOKEN_BYTES: usize = 32;
/// トークンの文字数（16 進）。
pub const TOKEN_LEN: usize = TOKEN_BYTES * 2;

/// 新しいトークンを作る。
///
/// # Panics
///
/// OS から乱数を得られなかったとき（ふつうは起きない。`Ulid::generate` と同じ）。
pub fn generate() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).expect("failed to get random bytes from the OS");
    hex(&bytes)
}

/// 小文字の 16 進にする。
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 0x0f)]));
    }
    out
}

/// トークンの形（小文字の 16 進 [`TOKEN_LEN`] 文字）か。
pub fn is_well_formed(token: &str) -> bool {
    token.len() == TOKEN_LEN
        && token
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `given` が `expected` と一致するか。定数時間で比べる（一致するまでの長さから中身を推し量らせない）。
/// 長さは秘密ではないので、違えば先に断る。
pub fn matches(expected: &str, given: &str) -> bool {
    if expected.len() != given.len() {
        return false;
    }
    let diff = expected
        .bytes()
        .zip(given.bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b));
    // 最適化で途中で打ち切られないようにする
    std::hint::black_box(diff) == 0
}

/// トークンのファイルの中身：1 行目がトークン。Windows は 2 行目にパイプの名前（起動のたびに
/// 変わる。先に作って待ち構えられないようにするため）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionInfo {
    pub token: String,
    /// パイプの名前（Windows）。UNIX 系では無い。
    pub pipe_name: Option<String>,
}

impl ConnectionInfo {
    fn render(&self) -> String {
        match &self.pipe_name {
            Some(name) => format!("{}\n{name}\n", self.token),
            None => format!("{}\n", self.token),
        }
    }

    fn parse(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        let token = lines.next()?.to_owned();
        if !is_well_formed(&token) {
            return None;
        }
        let pipe_name = lines.next().filter(|l| !l.is_empty()).map(str::to_owned);
        Some(Self { token, pipe_name })
    }
}

/// トークンのファイルを書く。同じフォルダの一時ファイルを本人だけが読み書きできる権限（0600）で
/// 新しく作って書き、名前を変えて置く（途中の内容を読まれず、後から権限を変える隙間も無い）。
pub fn write(dir: &DefaultDir, info: &ConnectionInfo) -> io::Result<()> {
    write_in(dir.path(), info)
}

/// [`write()`] の、フォルダを [`Path`] で受け取る版（既定ディレクトリの型を作れないテストと、
/// [`crate::transport`] の中で使う。公開の入り口は既定ディレクトリの型だけを受け取る）。
pub(crate) fn write_in(dir: &Path, info: &ConnectionInfo) -> io::Result<()> {
    let path = dir.join(TOKEN_FILE_NAME);
    let tmp = dir.join(format!("{TOKEN_FILE_NAME}.tmp"));
    // 前に異常終了して残った一時ファイル。新しく作り直す
    match fs::remove_file(&tmp) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let written = (|| {
        let mut file: File = options.open(&tmp)?;
        file.write_all(info.render().as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, &path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}

/// トークンのファイルを読む。無ければ [`io::ErrorKind::NotFound`]、読めても形が誤っていれば
/// [`io::ErrorKind::InvalidData`]。
pub fn read(dir: &DefaultDir) -> io::Result<ConnectionInfo> {
    read_in(dir.path())
}

pub(crate) fn read_in(dir: &Path) -> io::Result<ConnectionInfo> {
    let text = fs::read_to_string(dir.join(TOKEN_FILE_NAME))?;
    ConnectionInfo::parse(&text).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "the token file is not in the expected form",
        )
    })
}

/// トークンのファイルを消す（無ければ何もしない）。
pub fn remove(dir: &DefaultDir) -> io::Result<()> {
    remove_in(dir.path())
}

/// トークンのファイルが、自分の書いたトークン `token` のときだけ消す。自分が止まるのが遅れて、
/// 次の待ち受け（新しいアプリ）が書き直した後なら、そのファイルを消さない。
pub(crate) fn remove_own_in(dir: &Path, token: &str) {
    if read_in(dir).is_ok_and(|info| info.token == token) {
        let _ = remove_in(dir);
    }
}

pub(crate) fn remove_in(dir: &Path) -> io::Result<()> {
    match fs::remove_file(dir.join(TOKEN_FILE_NAME)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{read_in as read, remove_in as remove, write_in as write, *};

    fn dir(tmp: &tempfile::TempDir) -> &Path {
        tmp.path()
    }

    #[test]
    fn generated_tokens_are_well_formed_and_differ() {
        let a = generate();
        let b = generate();
        assert!(is_well_formed(&a), "{a}");
        assert_ne!(a, b);
    }

    #[test]
    fn well_formed_means_64_lowercase_hex_digits() {
        assert!(is_well_formed(&"a".repeat(64)));
        assert!(!is_well_formed(&"a".repeat(63)));
        assert!(!is_well_formed(&"A".repeat(64)));
        assert!(!is_well_formed(&"g".repeat(64)));
        assert!(!is_well_formed(""));
    }

    #[test]
    fn matches_compares_the_whole_value() {
        let t = generate();
        assert!(matches(&t, &t));
        let mut other = t.clone().into_bytes();
        let last = other.len() - 1;
        other[last] = if other[last] == b'0' { b'1' } else { b'0' };
        assert!(!matches(&t, std::str::from_utf8(&other).unwrap()));
        assert!(!matches(&t, &t[1..]));
        assert!(!matches(&t, ""));
    }

    #[test]
    fn the_token_file_round_trips_and_is_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dir(&tmp);
        let first = ConnectionInfo {
            token: generate(),
            pipe_name: None,
        };
        write(d, &first).unwrap();
        assert_eq!(read(d).unwrap(), first);
        let second = ConnectionInfo {
            token: generate(),
            pipe_name: Some(r"\\.\pipe\yunagi-test".into()),
        };
        write(d, &second).unwrap();
        assert_eq!(read(d).unwrap(), second);
        assert!(!tmp.path().join("api.token.tmp").exists());
        remove(d).unwrap();
        assert_eq!(read(d).unwrap_err().kind(), io::ErrorKind::NotFound);
        remove(d).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn the_token_file_is_readable_only_by_the_owner() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let d = dir(&tmp);
        write(
            d,
            &ConnectionInfo {
                token: generate(),
                pipe_name: None,
            },
        )
        .unwrap();
        let mode = fs::metadata(d.join(TOKEN_FILE_NAME))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_malformed_token_file_is_invalid_data() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dir(&tmp);
        fs::write(d.join(TOKEN_FILE_NAME), "not a token\n").unwrap();
        assert_eq!(read(d).unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_stale_temporary_file_does_not_block_writing() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dir(&tmp);
        fs::write(tmp.path().join("api.token.tmp"), "stale").unwrap();
        let info = ConnectionInfo {
            token: generate(),
            pipe_name: None,
        };
        write(d, &info).unwrap();
        assert_eq!(read(d).unwrap(), info);
    }
}
