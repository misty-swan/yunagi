//! エラーの表示の道具。決まりは README「エラーの表示（Rust）」。

use std::error::Error;
use std::fmt;

/// エラーの文に、原因（[`source()`](Error::source)）の文を `: ` でつないで最後までたどって
/// 表示する。利用者やログに1行で出すときに使う（README「エラーの表示（Rust）」）。
///
/// このクレートのエラーは Display に原因の文を含めないが、rusqlite などの外部のエラーには、
/// Display に原因の文を含め、同時に `source()` でも返すものがある。二重に出さないよう、
/// 原因の文が1つ前の文と同じか、1つ前の文の末尾と同じときは省く（たどるのは続ける）。
/// 文の途中に同じ言葉があるだけなら省かない（本当の原因を隠さないため）。
///
/// ```
/// use yunagi_core::error::ErrorChain;
/// use yunagi_core::storage::StorageError;
///
/// let err = StorageError::from(std::io::Error::other("disk full"));
/// assert_eq!(err.to_string(), "file error");
/// assert_eq!(ErrorChain(&err).to_string(), "file error: disk full");
/// ```
pub struct ErrorChain<'a>(pub &'a (dyn Error + 'static));

impl fmt::Display for ErrorChain<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut prev = self.0.to_string();
        f.write_str(&prev)?;
        let mut next = self.0.source();
        while let Some(e) = next {
            let text = e.to_string();
            if !text.is_empty() && !prev.ends_with(&text) {
                write!(f, ": {text}")?;
            }
            prev = text;
            next = e.source();
        }
        Ok(())
    }
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
        // 省いた先の原因はたどり続ける
        let deeper = e("a", Some(e("a", Some(e("b", None)))));
        assert_eq!(ErrorChain(&deeper).to_string(), "a: b");
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
