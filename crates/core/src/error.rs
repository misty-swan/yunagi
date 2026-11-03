//! エラーの表示の道具。決まりは README「エラーの表示（Rust）」。

use std::error::Error;
use std::fmt;

/// エラーの文に、原因（[`source()`](Error::source)）の文を `: ` でつないで最後までたどって
/// 表示する。利用者やログに1行で出すときに使う（README「エラーの表示（Rust）」）。
///
/// このクレートのエラーは Display に原因の文を含めないが、rusqlite などの外部のエラーには、
/// Display に原因の文を含め、同時に `source()` でも返すものがある。二重に出さないよう、
/// 原因の文が1つ前の文と同じとき、1つ前の文の末尾に `: `・`, ` で区切って付いているとき、
/// 1つ前の文の先頭に空白で区切って付いているとき（rusqlite の `Utf8Error` など）は省く
/// （たどるのは続ける）。それ以外で同じ言葉があるだけなら省かない（本当の原因を隠さないため）。
///
/// ```
/// use yunagi_core::error::ErrorChain;
/// use yunagi_core::storage::StorageError;
///
/// let err = StorageError::Io {
///     action: "write",
///     path: "/d/yunagi.db".into(),
///     source: std::io::Error::other("disk full"),
/// };
/// assert_eq!(err.to_string(), "could not write /d/yunagi.db");
/// assert_eq!(ErrorChain(&err).to_string(), "could not write /d/yunagi.db: disk full");
/// ```
pub struct ErrorChain<'a>(pub &'a (dyn Error + 'static));

impl fmt::Display for ErrorChain<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut prev = self.0.to_string();
        f.write_str(&prev)?;
        let mut next = self.0.source();
        while let Some(e) = next {
            let text = e.to_string();
            next = e.source();
            if text.is_empty() {
                // 文の無い原因は飛ばし、その先の原因は、文のある1つ前と比べる
                continue;
            }
            if !already_shown(&prev, &text) {
                write!(f, ": {text}")?;
            }
            prev = text;
        }
        Ok(())
    }
}

/// 原因の文 `cause` が、1つ前の文 `prev` にすでに含まれているか。
fn already_shown(prev: &str, cause: &str) -> bool {
    if prev == cause {
        return true;
    }
    if let Some(head) = prev.strip_suffix(cause)
        && (head.ends_with(": ") || head.ends_with(", "))
    {
        return true;
    }
    prev.strip_prefix(cause)
        .is_some_and(|tail| tail.starts_with(' '))
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;

    /// Display と source() を決めて作れるエラー。
    #[derive(Debug)]
    struct E(&'static str, Option<Box<E>>);

    impl fmt::Display for E {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.0)
        }
    }

    impl Error for E {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            self.1.as_deref().map(|e| e as &(dyn Error + 'static))
        }
    }

    fn e(text: &'static str, source: Option<E>) -> E {
        E(text, source.map(Box::new))
    }

    #[test]
    fn joins_every_cause() {
        let err = e("a", Some(e("b", Some(e("c", None)))));
        assert_eq!(ErrorChain(&err).to_string(), "a: b: c");
        let io = io::Error::other("disk full");
        assert_eq!(ErrorChain(&io).to_string(), "disk full");
    }

    #[test]
    fn skips_a_cause_already_at_the_end_of_the_previous_text() {
        // 外部のエラーが Display の末尾に原因の文を含め、source() でも返すとき
        let err = e(
            "x",
            Some(e(
                "conversion failed, bad value",
                Some(e("bad value", None)),
            )),
        );
        assert_eq!(
            ErrorChain(&err).to_string(),
            "x: conversion failed, bad value"
        );
        let same = e("disk full", Some(e("disk full", None)));
        assert_eq!(ErrorChain(&same).to_string(), "disk full");
        // 外部のエラーが Display の先頭に原因の文を含め、source() でも返すとき
        let head = e(
            "x",
            Some(e("bad byte at index 2", Some(e("bad byte", None)))),
        );
        assert_eq!(ErrorChain(&head).to_string(), "x: bad byte at index 2");
        // 省いた先の原因はたどり続ける
        let deeper = e("a", Some(e("a", Some(e("b", None)))));
        assert_eq!(ErrorChain(&deeper).to_string(), "a: b");
        // 文の無い原因は飛ばし、その先は文のある1つ前と比べる
        let empty = e("x: foo", Some(e("", Some(e("foo", None)))));
        assert_eq!(ErrorChain(&empty).to_string(), "x: foo");
    }

    #[test]
    fn skips_rusqlite_utf8_error_shown_at_the_start() {
        let utf8 = String::from_utf8(vec![0xff]).unwrap_err().utf8_error();
        let err = rusqlite::Error::Utf8Error(2, utf8);
        assert_eq!(ErrorChain(&err).to_string(), format!("{utf8} at index 2"));
    }

    #[test]
    fn keeps_a_cause_that_only_matches_part_of_a_word_or_path() {
        // パスの末尾が原因の文とたまたま同じでも、区切りが無ければ省かない
        let err = e("could not read /x/disk full", Some(e("full", None)));
        assert_eq!(
            ErrorChain(&err).to_string(),
            "could not read /x/disk full: full"
        );
    }

    #[test]
    fn keeps_a_cause_found_only_in_the_middle_of_the_previous_text() {
        let err = e(
            "could not copy the database to /d",
            Some(e("database", None)),
        );
        assert_eq!(
            ErrorChain(&err).to_string(),
            "could not copy the database to /d: database"
        );
    }
}
