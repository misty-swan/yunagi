//! ノート・振り返りの本文の扱い：行、時刻付き見出しとその区間、末尾への追記、行単位の書き換え
//! （docs/design/02-data-model.md 3章 note / reflection）。
//!
//! データベースには触れない。本文の文字列を受け取って、結果の文字列や見出しの一覧を返すだけ。
//! 読み書きと変更履歴の記録は [`super::text_doc`] が行う。
//!
//! # 行の数え方
//!
//! 本文を改行（LF。`\n`）で区切ったものを行とし、1行目から数える。改行で終わる本文は、
//! 最後に空の行がある（`"a\n"` は `"a"` と `""` の2行）。空の本文は空の1行。エディタの行の数え方
//! と同じにして、画面が知らせる行番号（アーキテクチャ設計書 4.1）とエージェントが `edit_note`
//! で指定する行番号をそろえる。CR（`\r`）は区切りに数えず、行の文字に含める。

use std::ops::Range;

use jiff::civil::Time;

/// 本文の行。1行目から順に返す。空の本文は空の1行。
pub fn lines(body: &str) -> impl Iterator<Item = &str> {
    body.split('\n')
}

/// 本文の行数（1以上）。
pub fn line_count(body: &str) -> usize {
    body.matches('\n').count() + 1
}

/// 時刻付き見出しの行（`## 10:00 定例`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeHeading {
    /// 見出しの行の番号（1始まり）。
    pub line: usize,
    pub hour: u8,
    pub minute: u8,
    /// 件名。前後の空白を除いたもの。件名の無い見出し（`## 10:00`）では空。
    pub title: String,
    /// 見出しの文字（`## ` の後ろ。前後の空白を除き、時刻は書いたままの形）。`get_board` の
    /// `headings` に入れる値（例：`10:00 定例`）。
    pub text: String,
}

/// 1行が時刻付き見出しなら、時・分・件名を返す。
///
/// 見出しの形：行頭の `## `（`#` 2つと半角の空白1つ）に続けて時刻（`H:MM` か `HH:MM`。
/// 時は 0〜23、分は 00〜59）、その後は行末か、空白（全角の空白も含む）に続く件名。
/// `## 良かったこと` のような時刻の無い見出しや、`### 10:00` は時刻付き見出しではない。
fn parse_time_heading(line: &str) -> Option<(u8, u8, &str, &str)> {
    let rest = line.strip_prefix("## ")?;
    let (hour_digits, after_hour) = rest.split_once(':')?;
    if !(1..=2).contains(&hour_digits.len()) || !hour_digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let minute_digits = after_hour.get(..2)?;
    if !minute_digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let after_time = &after_hour[2..];
    if !after_time.chars().next().is_none_or(char::is_whitespace) {
        return None;
    }
    let hour: u8 = hour_digits.parse().ok()?;
    let minute: u8 = minute_digits.parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some((hour, minute, after_time.trim(), rest.trim()))
}

/// 本文の時刻付き見出しを、上から順に返す。
pub fn headings(body: &str) -> Vec<TimeHeading> {
    lines(body)
        .enumerate()
        .filter_map(|(i, line)| {
            let (hour, minute, title, text) = parse_time_heading(line)?;
            Some(TimeHeading {
                line: i + 1,
                hour,
                minute,
                title: title.to_owned(),
                text: text.to_owned(),
            })
        })
        .collect()
}

/// 時刻付き見出しの区間。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// 区間の見出し。最初の見出しより前の区間（前置き）では `None`。
    pub heading: Option<TimeHeading>,
    /// 区間の中身の行の番号（1始まり。終わりは含まない）。見出しの行は含めず、見出しの次の行から
    /// 次の見出しの前までとする（操作仕様書 2章）。中身が無ければ空の範囲。
    pub lines: Range<usize>,
}

impl Section {
    /// 区間の中身の行（上から順）。`body` は区間を求めた本文。
    pub fn lines_in<'a>(&self, body: &'a str) -> impl Iterator<Item = &'a str> {
        lines(body)
            .skip(self.lines.start - 1)
            .take(self.lines.len())
    }

    /// 区間の中身の行を改行でつないだもの。
    pub fn text(&self, body: &str) -> String {
        self.lines_in(body).collect::<Vec<_>>().join("\n")
    }
}

/// 本文を時刻付き見出しで区切った区間を、上から順に返す。最初は前置き（最初の見出しより前。
/// 見出しが無ければ本文全体）で、本文が見出しで始まるときは中身が空になる。
pub fn sections(body: &str) -> Vec<Section> {
    let end = line_count(body) + 1;
    let headings = headings(body);
    let mut starts: Vec<usize> = headings.iter().map(|h| h.line).collect();
    starts.push(end);
    let mut result = vec![Section {
        heading: None,
        lines: 1..starts[0],
    }];
    for (heading, next) in headings.into_iter().zip(&starts[1..]) {
        result.push(Section {
            lines: heading.line + 1..*next,
            heading: Some(heading),
        });
    }
    result
}

/// 行 `line`（1始まり）が属する区間。見出しの行は、その見出しの区間に属する。
/// 行が本文に無ければ `None`。
pub fn section_of_line(body: &str, line: usize) -> Option<Section> {
    let sections = sections(body);
    // 最後の区間の終わりは、行数＋1
    if line == 0 || line >= sections.last().expect("at least one section").lines.end {
        return None;
    }
    sections.into_iter().rev().find(|s| match &s.heading {
        Some(h) => h.line <= line,
        None => true,
    })
}

/// 引数が誤っている理由（MCP・CLI 仕様書 4.3 の `invalid_argument`）。同じ引数でやり直しても
/// 成功しない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidArgument {
    /// 追記する本文が空（空白と改行だけを含む）。
    EmptyText,
    /// 見出しの件名に改行が含まれる。
    MultilineTitle,
    /// 行番号（書き換える行か移動先）が 0（行番号は1始まり）。
    LineZero,
    /// `expected_text` に改行が含まれる（1行の本文と一致することはない）。
    MultilineExpectedText,
    /// 書き換えた後の行に改行（LF）が含まれる。書き換えは1行を1行にする。CR は行の文字として
    /// 受け付ける（行の数え方と同じ）。
    MultilineReplacement,
}

impl std::fmt::Display for InvalidArgument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::EmptyText => "The text is empty.",
            Self::MultilineTitle => "The heading title must be a single line.",
            Self::LineZero => "Line numbers start at 1.",
            Self::MultilineExpectedText => "expected_text must be a single line.",
            Self::MultilineReplacement => "The replacement must be a single line.",
        })
    }
}

impl std::error::Error for InvalidArgument {}

/// 追記する本文を整える：改行を LF にそろえ（CRLF と単独の CR を LF に）、末尾の空白と改行を除く。
/// 標準入力から読んだ本文（`git log -1 --format=%s | yunagi reflection append -`）の末尾の改行や
/// 空白だけの行が、本文に残らないようにするため。空白と改行だけなら誤り。
fn normalize_text(text: &str) -> Result<String, InvalidArgument> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let text = text.trim_end();
    if text.is_empty() {
        return Err(InvalidArgument::EmptyText);
    }
    Ok(text.to_owned())
}

/// 時刻 `time` の時刻付き見出しの行（`## HH:MM 件名`。件名が無ければ `## HH:MM`）。
/// 秒より細かい部分は使わない。件名は前後の空白を除く。
pub fn time_heading_line(time: Time, title: &str) -> Result<String, InvalidArgument> {
    if title.contains(['\n', '\r']) {
        return Err(InvalidArgument::MultilineTitle);
    }
    let (hour, minute) = (time.hour(), time.minute());
    let title = title.trim();
    Ok(if title.is_empty() {
        format!("## {hour:02}:{minute:02}")
    } else {
        format!("## {hour:02}:{minute:02} {title}")
    })
}

/// 本文の末尾に `text` を追記した本文。
///
/// - 最後の行が空なら（空の本文や、改行で終わる本文）その行から書き、そうでなければ次の行から書く。
/// - `heading`（[`time_heading_line`] で作った見出しの行）があれば、本文の前に見出しの行を入れる。
///   最後の行が空ならその行に、そうでなければ空行を1行あけて入れる（画面の時刻付き見出しの挿入と
///   同じ。操作仕様書 2章）。
/// - `text` は改行を LF にそろえ、末尾の空白と改行を除いてから書く。
pub fn append(body: &str, text: &str, heading: Option<&str>) -> Result<String, InvalidArgument> {
    let text = normalize_text(text)?;
    let last_line_empty = body.is_empty() || body.ends_with('\n');
    let mut result = body.to_owned();
    if let Some(heading) = heading {
        if !last_line_empty {
            result.push_str("\n\n");
        }
        result.push_str(heading);
        result.push('\n');
    } else if !last_line_empty {
        result.push('\n');
    }
    result.push_str(&text);
    Ok(result)
}

/// 行単位の書き換えの内容（`edit_note`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineOp {
    /// 行の本文を置き換える（1行を1行に）。
    Replace(String),
    /// 行を消す。
    Delete,
    /// 行を動かし、動かした後の本文で `M` 行目（1始まり）になるようにする。
    MoveTo(usize),
}

/// 行単位の書き換え。`expected_text` は書き換え前の行の本文で、いまの本文の `line` 行目と
/// 完全に一致しなければ書き換えない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineEdit {
    /// 書き換える行の番号（1始まり）。
    pub line: usize,
    pub expected_text: String,
    pub op: LineOp,
}

/// 行単位の書き換えが失敗した理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineEditError {
    InvalidArgument(InvalidArgument),
    /// 本文がエージェントの読んだ後に変わっている（`conflict`、`reason: changed`）。
    /// 最新の本文を読み直してから、操作を組み立て直す。
    Changed(Changed),
}

/// どこが食い違ったか。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Changed {
    /// 指定した行の本文が `expected_text` と違う。`actual` はいまのその行の本文。
    LineMismatch { line: usize, actual: String },
    /// 指定した行（または移動先の行）が、いまの本文に無い。`line_count` はいまの行数。
    LineOutOfRange { line: usize, line_count: usize },
}

impl std::fmt::Display for Changed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LineMismatch { line, .. } => write!(
                f,
                "Line {line} does not match expected_text. The note was changed. Read it again."
            ),
            Self::LineOutOfRange { line, line_count } => write!(
                f,
                "The note has {line_count} lines and has no line {line}. The note was changed. Read it again."
            ),
        }
    }
}

/// 本文に行単位の書き換えを当てた結果の本文。同じ本文への置き換えや同じ位置への移動では、
/// もとと同じ本文になる（何も変えなかったかは、呼び出し側が本文を比べて決める）。
///
/// 確かめる順：引数の誤り（[`InvalidArgument`]）、`line` 行目が本文にあり `expected_text` と
/// 一致するか、移動先が本文にあるか（[`Changed`]）。
pub fn apply_line_edit(body: &str, edit: &LineEdit) -> Result<String, LineEditError> {
    let invalid = |e| Err(LineEditError::InvalidArgument(e));
    if edit.line == 0 {
        return invalid(InvalidArgument::LineZero);
    }
    if edit.expected_text.contains('\n') {
        return invalid(InvalidArgument::MultilineExpectedText);
    }
    match &edit.op {
        LineOp::Replace(text) if text.contains('\n') => {
            return invalid(InvalidArgument::MultilineReplacement);
        }
        LineOp::MoveTo(0) => return invalid(InvalidArgument::LineZero),
        _ => {}
    }

    let mut lines: Vec<&str> = lines(body).collect();
    let line_count = lines.len();
    let index = edit.line - 1;
    let changed = |c| Err(LineEditError::Changed(c));
    let Some(&actual) = lines.get(index) else {
        return changed(Changed::LineOutOfRange {
            line: edit.line,
            line_count,
        });
    };
    if actual != edit.expected_text {
        return changed(Changed::LineMismatch {
            line: edit.line,
            actual: actual.to_owned(),
        });
    }
    match &edit.op {
        LineOp::Replace(text) => lines[index] = text,
        LineOp::Delete => {
            lines.remove(index);
        }
        LineOp::MoveTo(to) => {
            if *to > line_count {
                return changed(Changed::LineOutOfRange {
                    line: *to,
                    line_count,
                });
            }
            let moved = lines.remove(index);
            lines.insert(to - 1, moved);
        }
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_lines_like_the_editor() {
        assert_eq!(line_count(""), 1);
        assert_eq!(lines("").collect::<Vec<_>>(), [""]);
        assert_eq!(line_count("a"), 1);
        assert_eq!(line_count("a\n"), 2);
        assert_eq!(lines("a\n\nb").collect::<Vec<_>>(), ["a", "", "b"]);
        // CR は区切りに数えない
        assert_eq!(lines("a\r\nb").collect::<Vec<_>>(), ["a\r", "b"]);
    }

    #[test]
    fn finds_time_headings() {
        let body = "前置き\n## 10:00 定例\n- 議題\n## 9:30\n## 14:00　設計 レビュー \n## 良かったこと\n### 15:00 小見出し\n";
        assert_eq!(
            headings(body),
            [
                TimeHeading {
                    line: 2,
                    hour: 10,
                    minute: 0,
                    title: "定例".into(),
                    text: "10:00 定例".into(),
                },
                TimeHeading {
                    line: 4,
                    hour: 9,
                    minute: 30,
                    title: "".into(),
                    text: "9:30".into(),
                },
                TimeHeading {
                    line: 5,
                    hour: 14,
                    minute: 0,
                    title: "設計 レビュー".into(),
                    text: "14:00　設計 レビュー".into(),
                },
            ]
        );
    }

    #[test]
    fn rejects_lines_that_are_not_time_headings() {
        for line in [
            "## 良かったこと",
            "##10:00 定例",
            "##  10:00 定例",
            " ## 10:00",
            "# 10:00",
            "### 10:00",
            "## 24:00",
            "## 10:60",
            "## 100:00",
            "## 10:0",
            "## 10:000",
            "## 10:00定例",
            "## :00",
            "## １０:００",
            "## 10：00",
        ] {
            assert_eq!(headings(line), [], "{line:?}");
        }
        // 末尾の CR は空白として扱う
        assert_eq!(headings("## 23:59\r")[0].text, "23:59");
        assert_eq!(headings("## 0:00")[0].hour, 0);
    }

    #[test]
    fn splits_into_sections() {
        let body = "前置き\n\n## 10:00 定例\n- a\n- b\n## 14:00 設計\n- c";
        let s = sections(body);
        assert_eq!(s.len(), 3);
        assert_eq!(s[0].heading, None);
        assert_eq!(s[0].lines, 1..3);
        assert_eq!(s[0].text(body), "前置き\n");
        assert_eq!(s[1].heading.as_ref().unwrap().text, "10:00 定例");
        assert_eq!(s[1].lines, 4..6);
        assert_eq!(s[1].text(body), "- a\n- b");
        assert_eq!(s[2].lines, 7..8);
        assert_eq!(s[2].text(body), "- c");
    }

    #[test]
    fn sections_without_headings_or_starting_with_one() {
        let s = sections("a\nb");
        assert_eq!(
            s,
            [Section {
                heading: None,
                lines: 1..3
            }]
        );
        let body = "## 10:00\n## 11:00";
        let s = sections(body);
        assert_eq!(s.len(), 3);
        assert_eq!(s[0].lines, 1..1);
        assert_eq!(s[0].text(body), "");
        assert_eq!(s[1].lines, 2..2);
        assert_eq!(s[2].lines, 3..3);
        assert_eq!(s[2].text(body), "");
    }

    #[test]
    fn finds_the_section_of_a_line() {
        let body = "前置き\n## 10:00 定例\n- a\n## 14:00\n- b";
        assert_eq!(section_of_line(body, 1).unwrap().heading, None);
        for (line, heading_line) in [(2, 2), (3, 2), (4, 4), (5, 4)] {
            assert_eq!(
                section_of_line(body, line).unwrap().heading.unwrap().line,
                heading_line,
                "line {line}"
            );
        }
        assert_eq!(section_of_line(body, 0), None);
        assert_eq!(section_of_line(body, 6), None);
    }

    #[test]
    fn appends_on_the_last_line_if_empty_or_the_next_line() {
        assert_eq!(append("", "x", None).unwrap(), "x");
        assert_eq!(append("a", "x", None).unwrap(), "a\nx");
        assert_eq!(append("a\n", "x", None).unwrap(), "a\nx");
        assert_eq!(append("a\n\n", "x", None).unwrap(), "a\n\nx");
        assert_eq!(append("a", "x\ny", None).unwrap(), "a\nx\ny");
    }

    #[test]
    fn appends_with_a_time_heading() {
        let h = time_heading_line(Time::constant(9, 5, 59, 999), " 定例 ").unwrap();
        assert_eq!(h, "## 09:05 定例");
        assert_eq!(
            time_heading_line(Time::constant(14, 0, 0, 0), "").unwrap(),
            "## 14:00"
        );
        assert_eq!(append("", "x", Some(&h)).unwrap(), "## 09:05 定例\nx");
        assert_eq!(append("a", "x", Some(&h)).unwrap(), "a\n\n## 09:05 定例\nx");
        assert_eq!(append("a\n", "x", Some(&h)).unwrap(), "a\n## 09:05 定例\nx");
        // 入れた見出しは時刻付き見出しとして読める
        let body = append("a", "x", Some(&h)).unwrap();
        assert_eq!(headings(&body)[0].text, "09:05 定例");
        assert_eq!(
            time_heading_line(Time::constant(9, 5, 0, 0), "a\nb"),
            Err(InvalidArgument::MultilineTitle)
        );
    }

    #[test]
    fn normalizes_appended_text() {
        assert_eq!(append("a", "x\r\ny\r\n", None).unwrap(), "a\nx\ny");
        assert_eq!(append("a", "x\ry\n\n", None).unwrap(), "a\nx\ny");
        // 末尾の空白だけの行と、最後の行の末尾の空白も除く
        assert_eq!(append("a", "x\n  \n\u{3000}", None).unwrap(), "a\nx");
        assert_eq!(append("a", "x  ", None).unwrap(), "a\nx");
        // 先頭と途中の空白・空行は残す
        assert_eq!(append("a", "  x\n\ny", None).unwrap(), "a\n  x\n\ny");
        for empty in ["", "\n", " \n\u{3000}", "\r\n"] {
            assert_eq!(
                append("a", empty, None),
                Err(InvalidArgument::EmptyText),
                "{empty:?}"
            );
        }
    }

    fn edit(line: usize, expected: &str, op: LineOp) -> LineEdit {
        LineEdit {
            line,
            expected_text: expected.into(),
            op,
        }
    }

    #[test]
    fn replaces_deletes_and_moves_lines() {
        let body = "a\nb\nc";
        let apply = |e| Some(apply_line_edit(body, &e).unwrap());
        assert_eq!(
            apply(edit(2, "b", LineOp::Replace("B".into()))).as_deref(),
            Some("a\nB\nc")
        );
        assert_eq!(apply(edit(2, "b", LineOp::Delete)).as_deref(), Some("a\nc"));
        assert_eq!(
            apply(edit(1, "a", LineOp::MoveTo(3))).as_deref(),
            Some("b\nc\na")
        );
        assert_eq!(
            apply(edit(3, "c", LineOp::MoveTo(1))).as_deref(),
            Some("c\na\nb")
        );
        assert_eq!(
            apply(edit(1, "a", LineOp::MoveTo(2))).as_deref(),
            Some("b\na\nc")
        );
        // 空の行も書き換えられる。行を消し切ると空の本文
        assert_eq!(
            apply_line_edit("a\n", &edit(2, "", LineOp::Replace("b".into()))).unwrap(),
            "a\nb"
        );
        assert_eq!(
            apply_line_edit("a", &edit(1, "a", LineOp::Delete)).unwrap(),
            ""
        );
        // CR は行の文字なので、CR を含む行を CR を残したまま書き換えられる
        assert_eq!(
            apply_line_edit("a\r\nb", &edit(1, "a\r", LineOp::Replace("A\r".into()))).unwrap(),
            "A\r\nb"
        );
    }

    #[test]
    fn edits_that_change_nothing_return_the_same_body() {
        let body = "a\nb";
        assert_eq!(
            apply_line_edit(body, &edit(1, "a", LineOp::Replace("a".into()))).unwrap(),
            body
        );
        assert_eq!(
            apply_line_edit(body, &edit(2, "b", LineOp::MoveTo(2))).unwrap(),
            body
        );
    }

    #[test]
    fn mismatched_or_missing_lines_are_changed() {
        let body = "a\nb";
        assert_eq!(
            apply_line_edit(body, &edit(2, "a", LineOp::Delete)),
            Err(LineEditError::Changed(Changed::LineMismatch {
                line: 2,
                actual: "b".into()
            }))
        );
        // 前後の空白も比べる
        assert!(matches!(
            apply_line_edit(body, &edit(1, "a ", LineOp::Delete)),
            Err(LineEditError::Changed(Changed::LineMismatch { .. }))
        ));
        assert_eq!(
            apply_line_edit(body, &edit(3, "", LineOp::Delete)),
            Err(LineEditError::Changed(Changed::LineOutOfRange {
                line: 3,
                line_count: 2
            }))
        );
        assert_eq!(
            apply_line_edit(body, &edit(1, "a", LineOp::MoveTo(3))),
            Err(LineEditError::Changed(Changed::LineOutOfRange {
                line: 3,
                line_count: 2
            }))
        );
    }

    #[test]
    fn invalid_arguments_are_checked_before_the_body() {
        let body = "a\nb";
        let invalid = |e: LineEdit| match apply_line_edit(body, &e) {
            Err(LineEditError::InvalidArgument(i)) => i,
            other => panic!("{other:?}"),
        };
        // 本文と一致しない（changed にも当たる）ときも、引数の誤りを先に返す
        assert_eq!(
            invalid(edit(0, "x", LineOp::Delete)),
            InvalidArgument::LineZero
        );
        assert_eq!(
            invalid(edit(1, "a\nb", LineOp::Delete)),
            InvalidArgument::MultilineExpectedText
        );
        for text in ["x\ny", "x\r\n"] {
            assert_eq!(
                invalid(edit(9, "x", LineOp::Replace(text.into()))),
                InvalidArgument::MultilineReplacement
            );
        }
        assert_eq!(
            invalid(edit(1, "a", LineOp::MoveTo(0))),
            InvalidArgument::LineZero
        );
    }
}
