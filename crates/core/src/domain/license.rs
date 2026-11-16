//! サポーターライセンスの確認の印（docs/design/02-data-model.md 3章 setting の `license`、画面設計書
//! 7章「ライセンス区分と『このアプリについて』」）。
//!
//! - 保存するのは、キーの末尾 6 文字と、確認できた時刻だけ（キーの全体は保存しない）。`license` がある
//!   ことが「確認済み」の印で、アプリは起動時にこれを読むだけで、通信しない。
//! - 形の壊れた値は、無いものとして扱う（エラーにしない）。
//! - 入力したキーの形の検査（[`check_key`]）もここに置く。形の違う入力は、外へ送らずに断るため。
//!
//! 販売サービスへの問い合わせは、アプリ（`yunagi-app` の `license`）が行う。コアは通信しない。

use jiff::Timestamp;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::storage::StorageError;
use crate::storage::table::{format_time, setting, truncate_time};

/// 設定のキー：サポーターライセンスの確認の印。
pub const LICENSE_KEY: &str = "license";

/// 保存するキーの末尾の文字数。
pub const KEY_TAIL_CHARS: usize = 6;

/// 入力するキーの長さの範囲（前後の空白を除いた文字数）。
pub const KEY_LENGTH: std::ops::RangeInclusive<usize> = 8..=128;

/// 確認の印：キーの末尾と、確認できた時刻。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct License {
    /// キーの末尾 [`KEY_TAIL_CHARS`] 文字（ASCII の英数字・`-`・`_`）。
    pub key_tail: String,
    /// 確認できた時刻（ミリ秒まで）。
    pub verified_at: Timestamp,
}

/// 保存する形（`{"key_tail":"E304DA","verified_at":"2026-11-12T03:04:05.678Z"}`）。
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    key_tail: String,
    verified_at: String,
}

impl License {
    /// 確かめたキー `key`（[`check_key`] を通したもの）と時刻から作る。末尾だけを持つ。
    pub fn new(key: &str, verified_at: Timestamp) -> Self {
        Self {
            key_tail: key_tail(key).to_owned(),
            verified_at: truncate_time(verified_at),
        }
    }
}

/// キーの末尾 [`KEY_TAIL_CHARS`] 文字（短ければ全体）。[`check_key`] を通したキーは ASCII なので、
/// バイトで切ってよい。
fn key_tail(key: &str) -> &str {
    let start = key.len().saturating_sub(KEY_TAIL_CHARS);
    key.get(start..).unwrap_or(key)
}

fn valid_tail(s: &str) -> bool {
    s.len() == KEY_TAIL_CHARS && s.bytes().all(key_byte)
}

fn key_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

/// 入力 `input` がキーの形かを見て、前後の空白を除いたキーを返す。形が違えば `None`（通信せずに断る）。
///
/// 形：前後の空白を除いて、[`KEY_LENGTH`] 文字の ASCII の英数字・`-`・`_` だけの 1 行。`prefix` が
/// 空でなければ、それで始まること（接頭辞の後ろの形までは見ない。画面設計書 7章）。
pub fn check_key<'a>(input: &'a str, prefix: &str) -> Option<&'a str> {
    let key = input.trim();
    (KEY_LENGTH.contains(&key.len()) && key.bytes().all(key_byte) && key.starts_with(prefix))
        .then_some(key)
}

/// 確認の印を読む。無いときと、形の壊れた値は `None`。
pub fn load(conn: &Connection) -> Result<Option<License>, StorageError> {
    let Some(json) = setting::get(conn, LICENSE_KEY)? else {
        return Ok(None);
    };
    let Ok(stored) = serde_json::from_str::<Stored>(&json) else {
        return Ok(None);
    };
    let Ok(verified_at) = stored.verified_at.parse::<Timestamp>() else {
        return Ok(None);
    };
    Ok(valid_tail(&stored.key_tail).then(|| License {
        key_tail: stored.key_tail,
        verified_at: truncate_time(verified_at),
    }))
}

/// 確認の印を書く（あれば置き換える）。
pub fn save(conn: &Connection, license: &License) -> Result<(), StorageError> {
    let stored = Stored {
        key_tail: license.key_tail.clone(),
        verified_at: format_time(license.verified_at),
    };
    let json = serde_json::to_string(&stored).expect("strings are serializable");
    setting::set(conn, LICENSE_KEY, &json)
}

/// 確認の印を消す。消したら `true`、無ければ `false`。
pub fn remove(conn: &Connection) -> Result<bool, StorageError> {
    setting::remove(conn, LICENSE_KEY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::table::test_support::open;

    fn at(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn only_the_tail_and_the_time_are_stored() {
        let (db, _dir) = open();
        let conn = db.connection();
        assert_eq!(load(conn).unwrap(), None);
        // 試験用のキー（秘密の検査に掛からないよう、0 を並べた偽物）
        let entered = "YUNAGI_00000000-0000-4000-8000-0000000E304DA";
        let license = License::new(entered, at("2026-11-12T03:04:05.678912Z"));
        assert_eq!(license.key_tail, "E304DA");
        save(conn, &license).unwrap();
        let json = setting::get(conn, LICENSE_KEY).unwrap().unwrap();
        assert_eq!(
            json,
            r#"{"key_tail":"E304DA","verified_at":"2026-11-12T03:04:05.678Z"}"#
        );
        assert!(!json.contains(entered));
        assert_eq!(load(conn).unwrap(), Some(license));
        assert!(remove(conn).unwrap());
        assert!(!remove(conn).unwrap());
        assert_eq!(load(conn).unwrap(), None);
    }

    #[test]
    fn a_broken_mark_counts_as_none() {
        let (db, _dir) = open();
        let conn = db.connection();
        for bad in [
            "3",
            r#""E304DA""#,
            r#"{"key_tail":"E304DA"}"#,
            r#"{"key_tail":"E304DA","verified_at":"yesterday"}"#,
            r#"{"key_tail":"E304","verified_at":"2026-11-12T03:04:05.678Z"}"#,
            r#"{"key_tail":"E3 4DA","verified_at":"2026-11-12T03:04:05.678Z"}"#,
            r#"{"key_tail":"ＥＥ","verified_at":"2026-11-12T03:04:05.678Z"}"#,
            r#"{"key_tail":"E304DA","verified_at":"2026-11-12T03:04:05.678Z","key":"x"}"#,
        ] {
            setting::set(conn, LICENSE_KEY, bad).unwrap();
            assert_eq!(load(conn).unwrap(), None, "{bad}");
        }
    }

    #[test]
    fn the_key_shape_is_checked_before_sending() {
        let ok = "YUNAGI_00000000-0000-4000-8000-0000000E304DA";
        assert_eq!(check_key(ok, ""), Some(ok));
        assert_eq!(check_key(&format!("  {ok}\n"), ""), Some(ok));
        assert_eq!(check_key(ok, "YUNAGI_"), Some(ok));
        assert_eq!(check_key("abcd_123", ""), Some("abcd_123"));
        let long = "a".repeat(129);
        for bad in [
            "",
            "   ",
            "abc1234",
            long.as_str(),
            "YUNAGI_ 8F1D2C3B",
            "YUNAGI_8F1D\n2C3B",
            "YUNAGI_8F1D\t2C3B",
            "今日のノートの一部です。今日のノート",
            "YUNAGI_８F1D2C3B",
            "YUNAGI_8F1D2C3B.",
        ] {
            assert_eq!(check_key(bad, ""), None, "{bad:?}");
        }
        assert_eq!(check_key("ghp_0123456789abcdef", "YUNAGI_"), None);
        assert_eq!(check_key("yunagi_0123456789", "YUNAGI_"), None);
    }
}
