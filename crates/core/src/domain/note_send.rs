//! 行送り：ノートの行を TODO・障害へ送り、送った行に送信済みの印を付ける（データモデル設計書 3章
//! note_send、操作仕様書 2章、MCP・CLI 仕様書 5章）。
//!
//! - [`send`]：画面の行送り（Mod+Shift+T・Mod+Shift+B）。1行ごとに項目と note_send を1件ずつ作る。
//! - [`marks`]：本文の各行を note_send の指紋と照合して、送信済みの印を付ける行を求める。
//! - [`source_line`]：項目がどのノートの行から来たかを、持ち越し・分割の元へさかのぼって求める。
//! - [`request_context`]：依頼文の書き出し（A-06）で使う、送り元の行の見出しの区間の行と、
//!   そこから送った未解消の障害。
//!
//! 行送りは人の操作なので change_log には記録しない。
//!
//! # 指紋と照合（2026-11-03 決定）
//!
//! 指紋は、行の本文と、前後の「空白だけでない最も近い行」の本文のハッシュを
//! 並べた文字列。照合では、本文のハッシュが一致する行だけを候補にし（本文を変えると印が外れる）、
//! 候補が複数あるときは前後の行も一致する数の多い行を選ぶ（行を動かしても、同じ本文の別の行と
//! 取り違えにくくするため）。形は `v1:<本文のハッシュ>:<前の行のハッシュ>:<後の行のハッシュ>`
//! で、前・後の行が無ければ `-`。ハッシュは、行の末尾の空白（CR を含む）を除いた本文の 64 ビット
//! FNV-1a（16 桁の小文字の 16 進数）。

use std::collections::{HashMap, HashSet};
use std::ops::RangeInclusive;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use rusqlite::Connection;

use super::item;
use super::note_body::{self, Section};
use super::text_doc::{self, ScreenSave, Snapshot, TextDoc};
use crate::domain::logical_date::LogicalDate;
use crate::storage::table::item::{self as item_table, Item, ItemKind, ItemStatus};
use crate::storage::table::note_send::{self as note_send_table, NoteSend};
use crate::storage::{StorageError, Ulid, in_read_transaction, in_transaction};

// ---- 指紋 ----

/// 指紋の形の版。形を変えるときは版を上げ、古い版の指紋も読めるようにする（保存済みの印を
/// 外さないため）。
const FINGERPRINT_VERSION: &str = "v1";

/// 指紋の中で、前後に行が無い（本文の先頭・末尾）ことを表す印。
const NO_LINE: &str = "-";

/// 行の本文のハッシュ。比べる本文は、行の末尾の空白（CR を含む）を除いたもの（見えない末尾の空白を
/// 足し引きしただけで印が外れないようにするため。行頭の字下げは比べる）。ハッシュは 64 ビットの
/// FNV-1a を 16 桁の 16 進数（小文字）にしたもの。版をまたいで同じ値になるよう、ライブラリの
/// ハッシュ（版で変わりうる）を使わずにここで計算する。
fn line_hash(line: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in line.trim_end().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// 空白だけの行か（前後の行に数えない）。
fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

/// 行ごとの、本文と前後の行のハッシュ。
#[derive(Debug, Clone, PartialEq, Eq)]
struct LineKey {
    text: String,
    prev: String,
    next: String,
}

impl LineKey {
    /// 指紋の文字列（note_send の `line_fingerprint`）。形はモジュールの説明のとおり。
    fn fingerprint(&self) -> String {
        format!(
            "{FINGERPRINT_VERSION}:{}:{}:{}",
            self.text, self.prev, self.next
        )
    }

    /// 指紋を読む。形が違えば `None`（どの行とも一致しない）。
    fn parse(fingerprint: &str) -> Option<Self> {
        let mut parts = fingerprint.split(':');
        if parts.next()? != FINGERPRINT_VERSION {
            return None;
        }
        let (text, prev, next) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            text: text.to_owned(),
            prev: prev.to_owned(),
            next: next.to_owned(),
        })
    }

    /// 前後の行のうち、`other` と一致する数（0〜2）。
    fn neighbor_score(&self, other: &Self) -> u8 {
        u8::from(self.prev == other.prev) + u8::from(self.next == other.next)
    }
}

/// 本文の各行の [`LineKey`]（1行目から順）。前・後の行は、上からと下からの1回ずつの走査で、
/// 直前に見た空白だけでない行を覚えて求める。
fn line_keys(lines: &[&str]) -> Vec<LineKey> {
    let hashes: Vec<String> = lines.iter().map(|l| line_hash(l)).collect();
    let nearest = |order: &mut dyn Iterator<Item = usize>| {
        let mut found = vec![NO_LINE.to_owned(); lines.len()];
        let mut last: Option<usize> = None;
        for i in order {
            if let Some(j) = last {
                found[i].clone_from(&hashes[j]);
            }
            if !is_blank(lines[i]) {
                last = Some(i);
            }
        }
        found
    };
    let prev = nearest(&mut (0..lines.len()));
    let next = nearest(&mut (0..lines.len()).rev());
    hashes
        .into_iter()
        .zip(prev)
        .zip(next)
        .map(|((text, prev), next)| LineKey { text, prev, next })
        .collect()
}

/// 送った記録 `sends` のそれぞれが、各行の [`LineKey`] が `keys` の本文のどの行（1始まり）に
/// 印を付けるか（付けなければ `None`）。
///
/// 本文のハッシュが一致する行だけが候補。はじめに、記録と候補の行の組を、前後の行の一致する数の
/// 多い順（同じなら記録の順、行の上の順）に見て、記録にも行にもまだ相手が無ければ組にする（同じ
/// 本文の行を2つ送ったとき、2つの印が1つの行に重なって、もう1つの行が印の無いまま送り直せる
/// ことが無いようにするため）。候補の行より記録が多くて相手の無い記録は、前後の行の一致する数の
/// 多い行（同じなら上の行）に、ほかの記録と重ねて付ける。
fn assign(sends: &[&NoteSend], keys: &[LineKey]) -> Vec<Option<usize>> {
    let mut by_text: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, key) in keys.iter().enumerate() {
        by_text.entry(key.text.as_str()).or_default().push(i);
    }
    // 記録ごとの候補の行と、前後の行の一致する数
    let candidates: Vec<Vec<(u8, usize)>> = sends
        .iter()
        .map(|send| {
            LineKey::parse(&send.line_fingerprint)
                .and_then(|sent| {
                    let lines = by_text.get(sent.text.as_str())?;
                    Some(
                        lines
                            .iter()
                            .map(|&i| (sent.neighbor_score(&keys[i]), i))
                            .collect(),
                    )
                })
                .unwrap_or_default()
        })
        .collect();
    let mut pairs: Vec<(u8, usize, usize)> = candidates
        .iter()
        .enumerate()
        .flat_map(|(s, c)| c.iter().map(move |&(score, line)| (score, s, line)))
        .collect();
    pairs.sort_by_key(|&(score, s, line)| (std::cmp::Reverse(score), s, line));
    let mut result: Vec<Option<usize>> = vec![None; sends.len()];
    let mut claimed: HashSet<usize> = HashSet::new();
    for (_, s, line) in pairs {
        if result[s].is_none() && !claimed.contains(&line) {
            result[s] = Some(line);
            claimed.insert(line);
        }
    }
    for (s, c) in candidates.iter().enumerate() {
        if result[s].is_none() {
            result[s] = c
                .iter()
                .max_by_key(|&&(score, line)| (score, std::cmp::Reverse(line)))
                .map(|&(_, line)| line);
        }
    }
    result.into_iter().map(|line| line.map(|l| l + 1)).collect()
}

// ---- 行から項目の本文へ ----

/// 送らなかった行の理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// 見出しの行（`#` 1〜6個と空白で始まる行。時刻付き見出しも含む）。
    Heading,
    /// リストの印などを除くと本文が無い（空の行、`- ` だけの行、区切り線、コードの囲みの行など）。
    NoText,
    /// すでに送信済みの印が付いている（同じ行から2つ目の項目を作らない）。印を付けている記録の
    /// 送り先の項目。
    AlreadySent { item_id: Ulid },
}

/// 見出しの行か：行頭の空白（全角の空白・タブを含む）を除いて、`#` 1〜6 個、その後が行末か空白。
/// 行頭の空白は、項目の本文を作るときにも除くので、数を問わない（字下げした見出しを項目に
/// しないため）。
fn is_heading(line: &str) -> bool {
    let rest = line.trim_start();
    let hashes = rest.len() - rest.trim_start_matches('#').len();
    (1..=6).contains(&hashes)
        && rest[hashes..]
            .chars()
            .next()
            .is_none_or(char::is_whitespace)
}

/// `rest` が `marker` で始まり、その後が行末か空白なら、その後ろ。
fn strip_marker<'a>(rest: &'a str, marker: &str) -> Option<&'a str> {
    let after = rest.strip_prefix(marker)?;
    after
        .chars()
        .next()
        .is_none_or(char::is_whitespace)
        .then_some(after)
}

/// 区切り線（`-`・`*`・`_` のどれか1種を3つ以上と空白だけの行）か、コードの囲み（`` ``` `` か
/// `~~~` で始まる行）か。本文の無い、書式だけの行。行頭の空白は除いてから見る。
fn is_markup(rest: &str) -> bool {
    if rest.starts_with("```") || rest.starts_with("~~~") {
        return true;
    }
    ['-', '*', '_'].iter().any(|&mark| {
        rest.chars().filter(|&c| c == mark).count() >= 3
            && rest.chars().all(|c| c == mark || c.is_whitespace())
    })
}

/// ノートの1行を、項目の本文にする。行頭の空白とリストの印（`-`・`*`・`+`・`1.`・`1)`）を除き、
/// 印の後にチェック欄（`[ ]`・`[x]`・`[X]`）があればそれも除き、前後の空白を除く。行の中の CR は
/// 空白にする。見出しの行、区切り線・コードの囲みの行、本文が残らない行は送らない。
fn item_text(line: &str) -> Result<String, SkipReason> {
    if is_heading(line) {
        return Err(SkipReason::Heading);
    }
    let line = line.replace('\r', " ");
    let mut rest = line.trim_start();
    if is_markup(rest) {
        return Err(SkipReason::NoText);
    }
    let ordered = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if let Some(after) = ["-", "*", "+"]
        .iter()
        .find_map(|m| strip_marker(rest, m))
        .or_else(|| {
            (1..=9).contains(&ordered).then_some(())?;
            [".", ")"]
                .iter()
                .find_map(|m| strip_marker(&rest[ordered..], m))
        })
    {
        rest = after.trim_start();
        if let Some(after) = ["[ ]", "[x]", "[X]"]
            .iter()
            .find_map(|m| strip_marker(rest, m))
        {
            rest = after;
        }
    }
    item::one_line_text(rest).map_err(|_| SkipReason::NoText)
}

// ---- 行送り ----

/// 行送りの引数の誤り。何も書いていない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidArgument {
    /// 行番号が 0（行番号は1始まり）。
    LineZero,
    /// 範囲が空（始まりが終わりより後か、使い切った範囲）。
    EmptyRange,
    /// 行が本文に無い。`line_count` は本文の行数。
    LineOutOfRange { line: usize, line_count: usize },
}

impl std::fmt::Display for InvalidArgument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LineZero => f.write_str("Line numbers start at 1."),
            Self::EmptyRange => f.write_str("The range of lines is empty."),
            Self::LineOutOfRange { line, line_count } => {
                write!(f, "The note has {line_count} lines and has no line {line}.")
            }
        }
    }
}

impl std::error::Error for InvalidArgument {}

/// 行送りが失敗した理由。どれも、何も書いていない（トランザクションは取り消してある）。
#[derive(Debug)]
pub enum SendError {
    InvalidArgument(InvalidArgument),
    /// 画面の本文の、もとにした版が古い（その間にエージェントがノートを変えた）。画面は `latest` を
    /// 取り込んでから送り直す（保存の行き違いと同じ。アーキテクチャ設計書 4.1）。
    Stale {
        latest: Snapshot,
    },
    /// 読み書きに失敗した。
    Storage(StorageError),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidArgument(e) => e.fmt(f),
            Self::Stale { latest } => write!(
                f,
                "the note was changed in the meantime (now at rev {})",
                latest.rev
            ),
            Self::Storage(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for SendError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidArgument(e) => Some(e),
            Self::Storage(e) => Some(e),
            Self::Stale { .. } => None,
        }
    }
}

impl From<StorageError> for SendError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

impl From<rusqlite::Error> for SendError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Storage(e.into())
    }
}

impl From<InvalidArgument> for SendError {
    fn from(e: InvalidArgument) -> Self {
        Self::InvalidArgument(e)
    }
}

/// 送った1行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentLine {
    /// 行の番号（1始まり）。
    pub line: usize,
    /// 作った項目。
    pub item: Item,
    /// 作った記録。
    pub send: NoteSend,
}

/// 送らなかった1行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedLine {
    pub line: usize,
    pub reason: SkipReason,
}

/// [`send`] の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    /// 本文を保存した後の版の番号（画面は次の保存のもとにする）。
    pub rev: i64,
    /// 送った行（上から順）。
    pub sent: Vec<SentLine>,
    /// 送らなかった行（上から順）。
    pub skipped: Vec<SkippedLine>,
}

/// 行送りで送る行（画面の行送りの引数）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendLines<'a> {
    /// ノートの日。項目もこの日に作る（過去の日のノートから送った場合も、その日に作る）。
    pub date: LogicalDate,
    /// 画面のエディタの本文。
    pub body: &'a str,
    /// `body` のもとにした版の番号（まだ書いていない日は 0）。
    pub base_rev: i64,
    /// 送る行（1始まり。両端を含む）。現在行だけなら `n..=n`。
    pub lines: RangeInclusive<usize>,
    /// 送り先（TODO か障害）。
    pub kind: ItemKind,
}

/// 画面の行送り：その日のノートの行を、1行ごとに項目として送る（操作仕様書 2章）。
///
/// - はじめに画面の保存（[`text_doc::save`]）と同じく `body` を保存し、もとにした版が古ければ
///   何も書かずに [`SendError::Stale`] を返す。送信済みの印は保存した本文と照合するため、本文の
///   保存と項目の作成を1つのトランザクションで行う。
/// - 項目はノートと同じ日の末尾に、上の行から順に作る（`origin = note`）。本文は、行から行頭の空白・
///   リストの印・チェック欄を除いたもの。各行に note_send を1件作り、行の指紋（モジュールの説明）を入れる。
/// - 見出しの行、区切り線・コードの囲みの行、本文が残らない行、すでに送信済みの印が付いた行は送らずに、`skipped` に理由を返す。
///   1行も送らなくても、本文は保存する。
/// - 行番号の誤り（0、範囲が空、本文に無い行）は [`SendError::InvalidArgument`] で、何も書かない。
pub fn send(
    conn: &Connection,
    request: &SendLines<'_>,
    tz: &TimeZone,
    now: Timestamp,
) -> Result<Sent, SendError> {
    let SendLines {
        date,
        body,
        base_rev,
        ref lines,
        kind,
    } = *request;
    // 使い切った範囲も空とみなす（`start`・`end` だけでは分からないため）
    if lines.is_empty() {
        return Err(InvalidArgument::EmptyRange.into());
    }
    let (first, last) = (*lines.start(), *lines.end());
    if first == 0 {
        return Err(InvalidArgument::LineZero.into());
    }
    let texts: Vec<&str> = note_body::lines(body).collect();
    let line_count = texts.len();
    if last > line_count {
        return Err(InvalidArgument::LineOutOfRange {
            line: last,
            line_count,
        }
        .into());
    }
    in_transaction(conn, || {
        let rev =
            match text_doc::save_screen_body(conn, TextDoc::Note, date, body, base_rev, tz, now)? {
                ScreenSave::Saved(written) => written.rev,
                ScreenSave::Stale(latest) => return Err(SendError::Stale { latest }),
            };
        let keys = line_keys(&texts);
        let already: HashMap<usize, Ulid> = marks_for(conn, date, &keys, None)?
            .into_iter()
            .map(|m| (m.line, m.item_id))
            .collect();
        let mut to_send = Vec::new();
        let mut skipped = Vec::new();
        for line in first..=last {
            let text = match already.get(&line) {
                Some(&item_id) => Err(SkipReason::AlreadySent { item_id }),
                None => item_text(texts[line - 1]),
            };
            match text {
                Ok(text) => to_send.push((line, text)),
                Err(reason) => skipped.push(SkippedLine { line, reason }),
            }
        }
        let items = item::add_from_note(
            conn,
            date,
            kind,
            to_send.iter().map(|(_, text)| text.clone()).collect(),
            tz,
            now,
        )?;
        let mut sent = Vec::with_capacity(items.len());
        for ((line, _), item) in to_send.into_iter().zip(items) {
            let send = NoteSend {
                id: Ulid::generate(),
                date,
                line_text: texts[line - 1].to_owned(),
                line_fingerprint: keys[line - 1].fingerprint(),
                item_id: item.id,
                created_at: now,
            };
            note_send_table::insert(conn, &send)?;
            sent.push(SentLine { line, item, send });
        }
        Ok(Sent { rev, sent, skipped })
    })
}

// ---- 送信済みの印 ----

/// 送信済みの印を付ける行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    /// 行の番号（1始まり）。
    pub line: usize,
    /// 送り先の項目の種類（「→ TODO」「→ BLOCKER」）。
    pub kind: ItemKind,
    /// 送り先の項目。
    pub item_id: Ulid,
    /// 印の元の記録。
    pub send_id: Ulid,
}

/// その日のノートの本文 `body` で、送信済みの印を付ける行を、行の順に返す（同じ行に印が2つ
/// 付くこともある）。`body` は画面のエディタの本文（まだ保存していない入力を含んでよい）でも、
/// 保存済みの本文でもよい。
///
/// 送り先の項目が削除された記録は印を付けない（データモデル設計書 3章 note_send）。行の照合の
/// 決まりは、このモジュールの説明の「指紋と照合」。
pub fn marks(conn: &Connection, date: LogicalDate, body: &str) -> Result<Vec<Mark>, StorageError> {
    let lines: Vec<&str> = note_body::lines(body).collect();
    marks_for(conn, date, &line_keys(&lines), None)
}

/// [`marks`] の中身。本文は、各行の [`LineKey`] で受け取る。
///
/// `extra` は、送り先の項目が削除されていても照合に加える記録と、その項目（[`source_with_marks`] が
/// さかのぼった先の記録に使う）。ほかの記録の後に照合するので、ほかの記録の印の行は変えない。
fn marks_for(
    conn: &Connection,
    date: LogicalDate,
    keys: &[LineKey],
    extra: Option<(&NoteSend, &Item)>,
) -> Result<Vec<Mark>, StorageError> {
    let mut live: HashMap<Ulid, Item> = item_table::list_by_date(conn, date)?
        .into_iter()
        .map(|i| (i.id, i))
        .collect();
    let mut sends: Vec<NoteSend> = note_send_table::list_by_date(conn, date)?
        .into_iter()
        .filter(|s| live.contains_key(&s.item_id))
        .collect();
    if let Some((send, item)) = extra
        && !sends.iter().any(|s| s.id == send.id)
    {
        live.insert(item.id, item.clone());
        sends.push(send.clone());
    }
    let refs: Vec<&NoteSend> = sends.iter().collect();
    let mut marks: Vec<Mark> = assign(&refs, keys)
        .into_iter()
        .zip(&sends)
        .filter_map(|(line, send)| {
            Some(Mark {
                line: line?,
                kind: live[&send.item_id].kind,
                item_id: send.item_id,
                send_id: send.id,
            })
        })
        .collect();
    marks.sort_by_key(|m| (m.line, m.send_id));
    Ok(marks)
}

// ---- 項目から送り元の行へ ----

/// 項目の送り元の行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLine {
    /// 送った記録（`send.date` が送り元のノートの日）。
    pub send: NoteSend,
    /// 送り元のノートのいまの本文。
    pub body: String,
    /// いまの本文で、送り元の行（1始まり）。
    pub line: usize,
}

/// 項目 `item_id` がどのノートの行から来たか（データモデル設計書 3章 note_send）。
///
/// その項目を送り先にする記録を探し、無ければ `carried_from_id`・`split_from_id` で元の項目へ
/// さかのぼって同じく探す。最初に見つかった記録の行が、いまのそのノートの本文に無ければ（本文が
/// 変わった）、ノートから来ていないものとして `None`。項目が無いときも `None`。
///
/// 見つかった記録の送り先の項目（さかのぼった先の元の項目を含む）が削除されていても、その記録の
/// 行は照合する（持ち越した後に元の日の項目を消して整理しても、持ち越し先の依頼文の文脈を失わない
/// ため）。その場合も、送信済みの印（[`marks`]）は付かない。
pub fn source_line(conn: &Connection, item_id: Ulid) -> Result<Option<SourceLine>, StorageError> {
    Ok(source_with_marks(conn, item_id)?.map(|(source, _)| source))
}

/// [`source_line`] と、送り元のノートのいまの本文の印（[`request_context`] で使い回す）。印には、
/// 削除された項目への見つかった記録も含む（[`marks_for`] の `extra`）。
fn source_with_marks(
    conn: &Connection,
    item_id: Ulid,
) -> Result<Option<(SourceLine, Vec<Mark>)>, StorageError> {
    let mut visited = HashSet::new();
    let mut current = item_id;
    loop {
        // 壊れたデータで元をたどる輪ができても止まるように
        if !visited.insert(current) {
            return Ok(None);
        }
        let Some(item) = item_table::get(conn, current)? else {
            return Ok(None);
        };
        let sends = note_send_table::list_by_item(conn, current)?;
        if let Some(send) = sends.into_iter().next() {
            let body = text_doc::get(conn, TextDoc::Note, send.date)?.body;
            let lines: Vec<&str> = note_body::lines(&body).collect();
            let marked = marks_for(conn, send.date, &line_keys(&lines), Some((&send, &item)))?;
            let Some(line) = marked.iter().find(|m| m.send_id == send.id).map(|m| m.line) else {
                return Ok(None);
            };
            return Ok(Some((SourceLine { line, send, body }, marked)));
        }
        match item.carried_from_id.or(item.split_from_id) {
            Some(from) => current = from,
            None => return Ok(None),
        }
    }
}

/// 依頼文の書き出し（A-06。MCP・CLI 仕様書 5章）で使う、ノートの文脈。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestContext {
    /// 送り元のノートの日。
    pub date: LogicalDate,
    /// 送り元の行が属する時刻付き見出しの区間（見出しの次の行から次の見出しの前まで。見出しが
    /// 無ければ先頭から最初の見出しの前まで）。
    pub section: Section,
    /// 区間の中身の行（上から順。空白だけの行も含む。書き出すときにどう扱うかは書き出す側が決める）。
    pub lines: Vec<String>,
    /// 区間の行から送った障害のうち未解消のもの（行の順。同じ障害は1件）。持ち越した障害は、
    /// 最後の持ち越し先の項目で数える（持ち越し元の状態は持ち越した後に変わらないため。
    /// データモデル設計書 3章 item）。
    pub blockers: Vec<Item>,
}

/// 項目 `item_id` の依頼文の書き出しの文脈。ノートから来ていない項目（[`source_line`] が `None`）
/// は `None`（「Context」と「Related blockers」の節を省く）。
///
/// 送り元・印・項目・持ち越し先は、同じ時点の内容を読む。
pub fn request_context(
    conn: &Connection,
    item_id: Ulid,
) -> Result<Option<RequestContext>, StorageError> {
    in_read_transaction(conn, || read_request_context(conn, item_id))
}

fn read_request_context(
    conn: &Connection,
    item_id: Ulid,
) -> Result<Option<RequestContext>, StorageError> {
    let Some((source, marked)) = source_with_marks(conn, item_id)? else {
        return Ok(None);
    };
    let date = source.send.date;
    let body = &source.body;
    let section =
        note_body::section_of_line(body, source.line).expect("the marked line is in the body");
    let lines = section.lines_in(body).map(str::to_owned).collect();
    // 区間の行から送った、削除されていない障害（行の順）
    let mut sent = Vec::new();
    for mark in marked {
        if mark.kind != ItemKind::Blocker || !section.lines.contains(&mark.line) {
            continue;
        }
        if let Some(item) = item_table::get(conn, mark.item_id)?.filter(|i| i.deleted_at.is_none())
        {
            sent.push(item);
        }
    }
    let ids: Vec<Ulid> = sent.iter().map(|i| i.id).collect();
    let latest = latest_carried(conn, &ids)?;
    let mut blockers = Vec::new();
    let mut seen = HashSet::new();
    for item in sent {
        let item = match latest.get(&item.id) {
            Some(&last) => match item_table::get(conn, last)? {
                Some(last) => last,
                None => continue,
            },
            None => item,
        };
        if item.status == ItemStatus::Open && seen.insert(item.id) {
            blockers.push(item);
        }
    }
    Ok(Some(RequestContext {
        date,
        section,
        lines,
        blockers,
    }))
}

/// 項目 `ids` それぞれの、最後の持ち越し先の id。持ち越していない項目は入れない。持ち越しの1段ごとに
/// まとめて読む（[`item_table::carried_to_many`]）。持ち越し先はいつも削除されていない項目。
fn latest_carried(conn: &Connection, ids: &[Ulid]) -> Result<HashMap<Ulid, Ulid>, StorageError> {
    let mut latest: HashMap<Ulid, Ulid> = HashMap::new();
    // 壊れたデータで輪になっていても止まるように、元ごとに一度通った項目は通らない
    let mut visited: HashSet<(Ulid, Ulid)> = ids.iter().map(|&id| (id, id)).collect();
    // （元, 今いる項目）
    let mut frontier: Vec<(Ulid, Ulid)> = ids.iter().map(|&id| (id, id)).collect();
    while !frontier.is_empty() {
        let nodes: Vec<Ulid> = frontier.iter().map(|&(_, node)| node).collect();
        let next = item_table::carried_to_many(conn, &nodes)?;
        frontier = frontier
            .into_iter()
            .filter_map(|(origin, node)| {
                let to = next.get(&node)?;
                visited.insert((origin, to.id)).then(|| {
                    latest.insert(origin, to.id);
                    (origin, to.id)
                })
            })
            .collect();
    }
    Ok(latest)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::domain::carryover::{self, Decision, Review, Selection};
    use crate::domain::item::{Origin, Position, add, delete, set_closed};
    use crate::storage::Database;
    use crate::storage::table::day as day_table;
    use crate::storage::table::test_support::{at, date};

    const T0: &str = "2026-11-03T01:00:00Z";
    const T1: &str = "2026-11-03T02:00:00Z";
    const T2: &str = "2026-11-03T03:00:00Z";

    fn tokyo() -> TimeZone {
        TimeZone::get("Asia/Tokyo").unwrap()
    }

    fn open() -> (Database, tempfile::TempDir) {
        crate::storage::table::test_support::open()
    }

    /// その日の保存済みのノートの版を使って、`lines` を送る。
    fn send_at(
        conn: &Connection,
        d: &str,
        body: &str,
        lines: RangeInclusive<usize>,
        kind: ItemKind,
        now: &str,
    ) -> Result<Sent, SendError> {
        let base_rev = text_doc::get(conn, TextDoc::Note, date(d)).unwrap().rev;
        send(
            conn,
            &SendLines {
                date: date(d),
                body,
                base_rev,
                lines,
                kind,
            },
            &tokyo(),
            at(now),
        )
    }

    fn send_ok(
        conn: &Connection,
        d: &str,
        body: &str,
        lines: RangeInclusive<usize>,
        kind: ItemKind,
    ) -> Sent {
        send_at(conn, d, body, lines, kind, T1).unwrap()
    }

    fn marked_lines(conn: &Connection, d: &str, body: &str) -> Vec<usize> {
        marks(conn, date(d), body)
            .unwrap()
            .into_iter()
            .map(|m| m.line)
            .collect()
    }

    /// 本文 `body` の `line` 行目（1始まり）の指紋。行が本文に無ければ `None`。
    fn fingerprint(body: &str, line: usize) -> Option<String> {
        let lines: Vec<&str> = note_body::lines(body).collect();
        line_keys(&lines)
            .get(line.checked_sub(1)?)
            .map(LineKey::fingerprint)
    }

    // ---- 行から項目の本文へ ----

    #[test]
    fn item_text_strips_list_markers_and_check_boxes() {
        assert_eq!(item_text("- API を調べる").unwrap(), "API を調べる");
        assert_eq!(item_text("  * 字下げした行  ").unwrap(), "字下げした行");
        assert_eq!(item_text("+ プラス").unwrap(), "プラス");
        assert_eq!(item_text("12. 番号").unwrap(), "番号");
        assert_eq!(item_text("3) 番号").unwrap(), "番号");
        assert_eq!(item_text("- [ ] 未完").unwrap(), "未完");
        assert_eq!(item_text("- [x] 済み").unwrap(), "済み");
        // チェック欄は、リストの印の後にあるときだけ除く
        assert_eq!(item_text("[X] 印だけ").unwrap(), "[X] 印だけ");
        assert_eq!(item_text("1. [ ] 番号の後").unwrap(), "番号の後");
        assert_eq!(item_text("ただの文").unwrap(), "ただの文");
        // 印の後に空白が無ければ印ではない
        assert_eq!(item_text("-5度").unwrap(), "-5度");
        assert_eq!(item_text("2026.11.03 の件").unwrap(), "2026.11.03 の件");
        // 行の中の CR は空白にする
        assert_eq!(item_text("a\rb\r").unwrap(), "a b");
    }

    #[test]
    fn item_text_skips_headings_and_lines_without_text() {
        for line in [
            "## 10:00 定例",
            "# 見出し",
            "### 小見出し",
            "##",
            "   ## 字下げ",
            "        # 深い字下げ",
            "\t# タブ",
            "\u{3000}## 10:00 全角の空白",
        ] {
            assert_eq!(item_text(line), Err(SkipReason::Heading), "{line}");
        }
        // 区切り線に似ても、ほかの文字があれば本文
        assert_eq!(item_text("--- 区切り").unwrap(), "--- 区切り");
        assert_eq!(item_text("-*-").unwrap(), "-*-");
        // `#` が7つ以上、`#` の後に空白が無いものは見出しではない
        assert_eq!(item_text("####### 7").unwrap(), "####### 7");
        assert_eq!(item_text("#tag").unwrap(), "#tag");
        for line in [
            "", "   ", "- ", "-", "1. ", "- [ ] ", "\u{3000}", "---", " * * * ", "___", "```",
            "```rust", "~~~",
        ] {
            assert_eq!(item_text(line), Err(SkipReason::NoText), "{line:?}");
        }
    }

    // ---- 指紋 ----

    #[test]
    fn fingerprint_uses_the_nearest_non_blank_neighbors() {
        let body = "a\n\nb\n  \nc";
        let h = line_hash;
        assert_eq!(
            fingerprint(body, 3).unwrap(),
            format!("v1:{}:{}:{}", h("b"), h("a"), h("c"))
        );
        assert_eq!(
            fingerprint(body, 1).unwrap(),
            format!("v1:{}:-:{}", h("a"), h("b"))
        );
        assert_eq!(
            fingerprint(body, 5).unwrap(),
            format!("v1:{}:{}:-", h("c"), h("b"))
        );
        assert_eq!(fingerprint(body, 0), None);
        assert_eq!(fingerprint(body, 6), None);
        // 末尾の空白は比べない。行頭の字下げは比べる
        assert_eq!(h("x \r"), h("x"));
        assert_ne!(h("  x"), h("x"));
        // FNV-1a 64 の決まった値（版をまたいで変わらないこと）
        assert_eq!(h(""), "cbf29ce484222325");
        assert_eq!(h("a"), "af63dc4c8601ec8c");
    }

    #[test]
    fn a_malformed_fingerprint_matches_no_line() {
        let send = |fp: &str| NoteSend {
            id: Ulid::generate(),
            date: date("2026-11-03"),
            line_text: "a".into(),
            line_fingerprint: fp.into(),
            item_id: Ulid::generate(),
            created_at: at(T0),
        };
        let a = line_hash("a");
        for fp in [
            String::new(),
            format!("v2:{a}:-:-"),
            format!("v1:{a}:-"),
            format!("v1:{a}:-:-:-"),
        ] {
            assert_eq!(assign(&[&send(&fp)], &line_keys(&["a"])), [None], "{fp}");
        }
        assert_eq!(
            assign(&[&send(&format!("v1:{a}:-:-"))], &line_keys(&["a"])),
            [Some(1)]
        );
    }

    // ---- 行送り ----

    #[test]
    fn sending_a_line_saves_the_body_and_creates_an_item_and_a_record() {
        let (db, _dir) = open();
        let conn = db.connection();
        let body = "## 10:00 定例\n- レート制限を調べる\n- 次の話";
        let sent = send_ok(conn, "2026-11-03", body, 2..=2, ItemKind::Todo);

        assert_eq!(sent.rev, 1);
        assert_eq!(
            text_doc::get(conn, TextDoc::Note, date("2026-11-03"))
                .unwrap()
                .body,
            body
        );
        assert!(sent.skipped.is_empty());
        let [line] = sent.sent.as_slice() else {
            panic!("{sent:?}")
        };
        assert_eq!(line.line, 2);
        let item = &line.item;
        assert_eq!(item.date, date("2026-11-03"));
        assert_eq!(item.kind, ItemKind::Todo);
        assert_eq!(item.text, "レート制限を調べる");
        assert_eq!(item.status, ItemStatus::Open);
        assert_eq!(item.origin, Origin::Note);
        assert_eq!(item.agent_client_id, None);
        assert_eq!(item.created_at, at(T1));
        assert_eq!(item_table::get(conn, item.id).unwrap().as_ref(), Some(item));

        let record = &line.send;
        assert_eq!(record.date, date("2026-11-03"));
        assert_eq!(record.line_text, "- レート制限を調べる");
        assert_eq!(record.line_fingerprint, fingerprint(body, 2).unwrap());
        assert_eq!(record.item_id, item.id);
        assert_eq!(record.created_at, at(T1));
        assert_eq!(
            note_send_table::list_by_item(conn, item.id).unwrap(),
            std::slice::from_ref(record)
        );
        // 人の操作なので change_log に記録しない
        let logs: i64 = conn
            .query_row("SELECT count(*) FROM change_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(logs, 0);

        assert_eq!(
            marks(conn, date("2026-11-03"), body).unwrap(),
            [Mark {
                line: 2,
                kind: ItemKind::Todo,
                item_id: item.id,
                send_id: record.id,
            }]
        );
    }

    #[test]
    fn sending_several_lines_creates_one_item_and_record_per_line() {
        let (db, _dir) = open();
        let conn = db.connection();
        let earlier = add(
            conn,
            date("2026-11-03"),
            ItemKind::Blocker,
            "前からある障害",
            Position::Last,
            &tokyo(),
            at(T0),
        )
        .unwrap();
        let body = "- 一つ目\n\n## 10:00 定例\n- \n- 二つ目\n- 二つ目";
        let sent = send_ok(conn, "2026-11-03", body, 1..=6, ItemKind::Blocker);

        let lines: Vec<(usize, &str)> = sent
            .sent
            .iter()
            .map(|s| (s.line, s.item.text.as_str()))
            .collect();
        // 同じ本文の行も1行ずつ送る
        assert_eq!(lines, [(1, "一つ目"), (5, "二つ目"), (6, "二つ目")]);
        assert_eq!(
            sent.skipped,
            [
                SkippedLine {
                    line: 2,
                    reason: SkipReason::NoText
                },
                SkippedLine {
                    line: 3,
                    reason: SkipReason::Heading
                },
                SkippedLine {
                    line: 4,
                    reason: SkipReason::NoText
                },
            ]
        );
        // 項目は末尾に、上の行から順に並ぶ
        let order: Vec<Ulid> = item_table::list_by_date(conn, date("2026-11-03"))
            .unwrap()
            .into_iter()
            .map(|i| i.id)
            .collect();
        let mut expected = vec![earlier.id];
        expected.extend(sent.sent.iter().map(|s| s.item.id));
        assert_eq!(order, expected);
        for s in &sent.sent {
            assert_eq!(
                note_send_table::list_by_item(conn, s.item.id).unwrap(),
                std::slice::from_ref(&s.send)
            );
        }
        assert_eq!(
            note_send_table::list_by_date(conn, date("2026-11-03"))
                .unwrap()
                .len(),
            3
        );
        // 同じ本文の2行には、別々の印が付く
        assert_eq!(marked_lines(conn, "2026-11-03", body), [1, 5, 6]);
    }

    #[test]
    fn items_are_created_on_the_note_day_even_for_a_past_day() {
        let (db, _dir) = open();
        let conn = db.connection();
        let sent = send_ok(conn, "2026-10-30", "- 昔の行", 1..=1, ItemKind::Todo);
        assert_eq!(sent.sent[0].item.date, date("2026-10-30"));
        assert_eq!(sent.sent[0].send.date, date("2026-10-30"));
        assert!(
            item_table::list_by_date(conn, date("2026-11-03"))
                .unwrap()
                .is_empty()
        );
        assert!(day_table::get(conn, date("2026-10-30")).unwrap().is_some());
    }

    #[test]
    fn a_stale_base_rev_writes_nothing() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        text_doc::save(conn, TextDoc::Note, d, "- a", 0, &tokyo(), at(T0)).unwrap();
        text_doc::save(conn, TextDoc::Note, d, "- a\n- b", 1, &tokyo(), at(T0)).unwrap();
        let result = send(
            conn,
            &SendLines {
                date: d,
                body: "- a\n- c",
                base_rev: 1,
                lines: 1..=2,
                kind: ItemKind::Todo,
            },
            &tokyo(),
            at(T1),
        );
        match result {
            Err(SendError::Stale { latest }) => {
                assert_eq!(
                    latest,
                    Snapshot {
                        body: "- a\n- b".into(),
                        rev: 2
                    }
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(item_table::list_by_date(conn, d).unwrap().is_empty());
        assert!(note_send_table::list_by_date(conn, d).unwrap().is_empty());
        // 同じ本文なら、版が古くても送れる（画面の保存と同じ）
        let sent = send(
            conn,
            &SendLines {
                date: d,
                body: "- a\n- b",
                base_rev: 1,
                lines: 2..=2,
                kind: ItemKind::Todo,
            },
            &tokyo(),
            at(T1),
        )
        .unwrap();
        assert_eq!(sent.rev, 2);
        assert_eq!(sent.sent.len(), 1);
    }

    #[test]
    fn wrong_line_numbers_write_nothing() {
        let (db, _dir) = open();
        let conn = db.connection();
        let body = "- a\n- b";
        for (lines, expected) in [
            (0..=1, InvalidArgument::LineZero),
            #[allow(clippy::reversed_empty_ranges)]
            (2..=1, InvalidArgument::EmptyRange),
            (
                1..=3,
                InvalidArgument::LineOutOfRange {
                    line: 3,
                    line_count: 2,
                },
            ),
        ] {
            match send_at(conn, "2026-11-03", body, lines, ItemKind::Todo, T1) {
                Err(SendError::InvalidArgument(e)) => assert_eq!(e, expected),
                other => panic!("{other:?}"),
            }
        }
        // 使い切った範囲は、始まりと終わりが同じでも空
        let mut used = 1..=1;
        assert_eq!(used.next(), Some(1));
        match send_at(conn, "2026-11-03", body, used, ItemKind::Todo, T1) {
            Err(SendError::InvalidArgument(e)) => assert_eq!(e, InvalidArgument::EmptyRange),
            other => panic!("{other:?}"),
        }
        assert_eq!(day_table::get(conn, date("2026-11-03")).unwrap(), None);
        assert_eq!(
            text_doc::get(conn, TextDoc::Note, date("2026-11-03"))
                .unwrap()
                .rev,
            0
        );
    }

    #[test]
    fn a_failure_in_the_middle_leaves_nothing_behind() {
        let (db, _dir) = open();
        let conn = db.connection();
        // 2行目の記録を入れるときに失敗させる
        conn.execute_batch(
            "CREATE TEMP TRIGGER fail_second BEFORE INSERT ON note_send \
             WHEN NEW.line_text = '- b' BEGIN SELECT RAISE(ABORT, 'boom'); END",
        )
        .unwrap();
        let result = send_at(conn, "2026-11-03", "- a\n- b", 1..=2, ItemKind::Todo, T1);
        assert!(matches!(result, Err(SendError::Storage(_))), "{result:?}");
        assert!(conn.is_autocommit());
        assert_eq!(
            text_doc::get(conn, TextDoc::Note, date("2026-11-03"))
                .unwrap()
                .rev,
            0
        );
        assert_eq!(day_table::get(conn, date("2026-11-03")).unwrap(), None);
        let items: i64 = conn
            .query_row("SELECT count(*) FROM item", [], |r| r.get(0))
            .unwrap();
        assert_eq!(items, 0);
        assert!(
            note_send_table::list_by_date(conn, date("2026-11-03"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_line_with_a_mark_is_not_sent_again_until_the_mark_is_gone() {
        let (db, _dir) = open();
        let conn = db.connection();
        let body = "- a\n- b";
        let first = send_ok(conn, "2026-11-03", body, 1..=1, ItemKind::Todo);
        let first_id = first.sent[0].item.id;
        // 障害へ送り直しても、印の付いた行は送らない
        let again = send_ok(conn, "2026-11-03", body, 1..=2, ItemKind::Blocker);
        assert_eq!(
            again.skipped,
            [SkippedLine {
                line: 1,
                reason: SkipReason::AlreadySent { item_id: first_id },
            }]
        );
        assert_eq!(again.sent.len(), 1);
        assert_eq!(again.sent[0].line, 2);
        // 送り先を消すと印が外れ、もう一度送れる
        delete(conn, first_id, at(T2)).unwrap();
        assert_eq!(marked_lines(conn, "2026-11-03", body), [2]);
        let third = send_ok(conn, "2026-11-03", body, 1..=1, ItemKind::Todo);
        assert_eq!(third.sent.len(), 1);
        assert_eq!(marked_lines(conn, "2026-11-03", body), [1, 2]);
    }

    // ---- 送信済みの印 ----

    #[test]
    fn a_mark_follows_the_line_when_it_moves_and_is_lost_when_its_text_changes() {
        let (db, _dir) = open();
        let conn = db.connection();
        let body = "- 前\n- 送る行\n- 後";
        let sent = send_ok(conn, "2026-11-03", body, 2..=2, ItemKind::Todo);
        let id = sent.sent[0].item.id;
        let at_line = |b: &str| -> Vec<(usize, Ulid)> {
            marks(conn, date("2026-11-03"), b)
                .unwrap()
                .into_iter()
                .map(|m| (m.line, m.item_id))
                .collect()
        };
        // 上に行を足す・空行を挟む・離れた場所へ動かす
        assert_eq!(at_line("## 9:00\n\n- 前\n\n- 送る行\n- 後"), [(5, id)]);
        assert_eq!(at_line("- 後\n- 前\n- 別\n- 送る行"), [(4, id)]);
        // 末尾の空白の増減は本文の変更に数えない
        assert_eq!(at_line("- 前\n- 送る行  \r\n- 後"), [(2, id)]);
        // 本文を変えると外れる（字下げも本文のうち）
        assert_eq!(at_line("- 前\n- 送る行を直した\n- 後"), []);
        assert_eq!(at_line("- 前\n  - 送る行\n- 後"), []);
        // 本文を戻せば、また付く
        assert_eq!(at_line(body), [(2, id)]);
    }

    #[test]
    fn among_identical_lines_the_mark_goes_to_the_one_with_the_same_neighbors() {
        let (db, _dir) = open();
        let conn = db.connection();
        let body = "## 10:00 A\n- 確認する\n## 11:00 B\n- 確認する";
        send_ok(conn, "2026-11-03", body, 4..=4, ItemKind::Todo);
        assert_eq!(marked_lines(conn, "2026-11-03", body), [4]);
        // 区間の順を入れ替えても、B の行に付いたまま
        let swapped = "## 11:00 B\n- 確認する\n## 10:00 A\n- 確認する";
        assert_eq!(marked_lines(conn, "2026-11-03", swapped), [2]);
        // 前後の行が同じ数だけ一致するなら、上の行
        let both = "x\n- 確認する\n- 確認する\ny";
        assert_eq!(marked_lines(conn, "2026-11-03", both), [2]);
        // 後に行が無いことも前後の行の一致に数える
        let last = "- 確認する\n- 確認する";
        assert_eq!(marked_lines(conn, "2026-11-03", last), [2]);
    }

    #[test]
    fn two_records_of_identical_lines_do_not_share_a_line_while_another_is_free() {
        let (db, _dir) = open();
        let conn = db.connection();
        let body = "p\n- a\nq\np\n- a\nr";
        send_ok(conn, "2026-11-03", body, 2..=2, ItemKind::Todo);
        send_ok(conn, "2026-11-03", body, 5..=5, ItemKind::Todo);
        // 1件目（前 p・後 q）はどちらの行とも1つ一致、2件目（前 p・後 r）は2行目と2つ一致する。
        // 2件目が2行目を取り、1件目は空いている4行目に付く
        let edited = "p\n- a\nr\n- a\nq";
        assert_eq!(marked_lines(conn, "2026-11-03", edited), [2, 4]);
        // どちらの `- a` の行にも印が付いているので、もう送らない
        let again = send_ok(conn, "2026-11-03", edited, 4..=4, ItemKind::Todo);
        assert!(again.sent.is_empty(), "{again:?}");
        // 候補の行より記録が多ければ、残りの記録は同じ行に重ねて付ける
        assert_eq!(marked_lines(conn, "2026-11-03", "- a"), [1, 1]);
    }

    #[test]
    fn marks_are_only_for_the_day_and_report_the_item_kind() {
        let (db, _dir) = open();
        let conn = db.connection();
        let body = "- a\n- b";
        send_ok(conn, "2026-11-03", body, 1..=1, ItemKind::Todo);
        send_ok(conn, "2026-11-03", body, 2..=2, ItemKind::Blocker);
        let kinds: Vec<(usize, ItemKind)> = marks(conn, date("2026-11-03"), body)
            .unwrap()
            .into_iter()
            .map(|m| (m.line, m.kind))
            .collect();
        assert_eq!(kinds, [(1, ItemKind::Todo), (2, ItemKind::Blocker)]);
        assert!(marks(conn, date("2026-11-04"), body).unwrap().is_empty());
    }

    // ---- 送り元の行へ ----

    /// 11.02 のノートの行を TODO へ送り、11.03 へ持ち越す（`review` があれば見直す）。
    /// 返すのは、送った項目と、11.03 の確定の結果。
    fn send_and_carry(
        conn: &Connection,
        body: &str,
        line: usize,
        review: Option<Review>,
    ) -> (Item, carryover::Confirmed) {
        let sent = send_ok(conn, "2026-11-02", body, line..=line, ItemKind::Todo);
        let item = sent.sent[0].item.clone();
        let decision = Decision {
            from: date("2026-11-02"),
            selection: Selection::All,
            reviews: review
                .into_iter()
                .map(|r| (item.id, r))
                .collect::<HashMap<_, _>>(),
        };
        let done =
            carryover::confirm(conn, date("2026-11-03"), &decision, &tokyo(), at(T2)).unwrap();
        (item, done)
    }

    #[test]
    fn the_source_line_of_a_sent_item() {
        let (db, _dir) = open();
        let conn = db.connection();
        let body = "メモ\n- 送る行";
        let sent = send_ok(conn, "2026-11-03", body, 2..=2, ItemKind::Todo);
        let source = source_line(conn, sent.sent[0].item.id).unwrap().unwrap();
        assert_eq!(source.send, sent.sent[0].send);
        assert_eq!(source.line, 2);
        assert_eq!(source.body, body);

        // ノートから来ていない項目と、無い項目
        let other = add(
            conn,
            date("2026-11-03"),
            ItemKind::Todo,
            "手で足した",
            Position::Last,
            &tokyo(),
            at(T1),
        )
        .unwrap();
        assert_eq!(source_line(conn, other.id).unwrap(), None);
        assert_eq!(source_line(conn, Ulid::generate()).unwrap(), None);
    }

    #[test]
    fn the_source_line_follows_the_note_and_is_lost_when_the_line_changes() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        let sent = send_ok(conn, "2026-11-03", "- 送る行", 1..=1, ItemKind::Todo);
        let id = sent.sent[0].item.id;
        text_doc::save(conn, TextDoc::Note, d, "上\n- 送る行", 1, &tokyo(), at(T2)).unwrap();
        assert_eq!(source_line(conn, id).unwrap().unwrap().line, 2);
        text_doc::save(conn, TextDoc::Note, d, "上\n- 直した", 2, &tokyo(), at(T2)).unwrap();
        assert_eq!(source_line(conn, id).unwrap(), None);
    }

    #[test]
    fn the_source_line_is_traced_through_carry_over_and_split() {
        let (db, _dir) = open();
        let conn = db.connection();
        let body = "## 10:00 定例\n- 持ち越す行";
        let (original, done) = send_and_carry(conn, body, 2, None);
        let [carried] = done.created.as_slice() else {
            panic!("{done:?}")
        };
        assert_eq!(carried.carried_from_id, Some(original.id));
        let source = source_line(conn, carried.id).unwrap().unwrap();
        assert_eq!(source.send.item_id, original.id);
        assert_eq!(source.send.date, date("2026-11-02"));
        assert_eq!(source.line, 2);

        // 分割の子も、元の項目の行へさかのぼる
        let (db, _dir) = open();
        let conn = db.connection();
        let (original, done) = send_and_carry(
            conn,
            body,
            2,
            Some(Review::Split(vec!["子1".into(), "子2".into()])),
        );
        assert_eq!(done.created.len(), 2);
        for child in &done.created {
            assert_eq!(child.split_from_id, Some(original.id));
            let source = source_line(conn, child.id).unwrap().unwrap();
            assert_eq!(source.send.item_id, original.id);
            assert_eq!(source.line, 2);
        }
        // 元の項目は取りやめても削除ではないので、印は残る
        assert_eq!(marked_lines(conn, "2026-11-02", body), [2]);
    }

    #[test]
    fn the_source_line_is_kept_when_the_original_item_is_deleted() {
        let (db, _dir) = open();
        let conn = db.connection();
        let body = "## 10:00 定例\n- 持ち越す行\n- 権限待ち";
        let (original, done) = send_and_carry(conn, body, 2, None);
        let carried = &done.created[0];
        // 持ち越した後に、元の日の項目を消して整理する
        delete(conn, original.id, at(T2)).unwrap();
        // 元の行の印は外れるが、持ち越し先からは元の行をたどれる
        assert_eq!(marked_lines(conn, "2026-11-02", body), Vec::<usize>::new());
        let source = source_line(conn, carried.id).unwrap().unwrap();
        assert_eq!(source.send.item_id, original.id);
        assert_eq!(source.line, 2);
        let context = request_context(conn, carried.id).unwrap().unwrap();
        assert_eq!(context.lines, ["- 持ち越す行", "- 権限待ち"]);
        // 本文を変えれば、たどれない
        let d = date("2026-11-02");
        let rev = text_doc::get(conn, TextDoc::Note, d).unwrap().rev;
        text_doc::save(
            conn,
            TextDoc::Note,
            d,
            "## 10:00 定例\n- 直した",
            rev,
            &tokyo(),
            at(T2),
        )
        .unwrap();
        assert_eq!(source_line(conn, carried.id).unwrap(), None);
    }

    #[test]
    fn a_deleted_original_does_not_take_a_line_from_a_live_record() {
        let (db, _dir) = open();
        let conn = db.connection();
        let body = "- 同じ行";
        let (original, done) = send_and_carry(conn, body, 1, None);
        delete(conn, original.id, at(T2)).unwrap();
        // 印が外れたので送り直せる。送り直した項目の印は、削除した元の記録に取られない
        let resent = send_ok(conn, "2026-11-02", body, 1..=1, ItemKind::Todo);
        let again = &resent.sent[0];
        assert_eq!(
            source_line(conn, again.item.id).unwrap().unwrap().send,
            again.send
        );
        // 持ち越し先も、同じ行（重ねて）をたどれる
        assert_eq!(
            source_line(conn, done.created[0].id).unwrap().unwrap().line,
            1
        );
    }

    // ---- 依頼文の書き出しの文脈 ----

    #[test]
    fn the_request_context_is_the_section_and_its_open_blockers() {
        let (db, _dir) = open();
        let conn = db.connection();
        let body =
            "前置き\n## 10:00 定例\n- 調べる\n- 権限待ち\n- 解消済み\n\n## 11:00 別\n- 別の障害";
        let todo = send_ok(conn, "2026-11-03", body, 3..=3, ItemKind::Todo).sent[0]
            .item
            .clone();
        let sent = send_ok(conn, "2026-11-03", body, 4..=5, ItemKind::Blocker);
        let (waiting, resolved) = (sent.sent[0].item.clone(), sent.sent[1].item.clone());
        set_closed(conn, resolved.id, Some(at(T2)), at(T2)).unwrap();
        send_ok(conn, "2026-11-03", body, 8..=8, ItemKind::Blocker);
        // 同じ区間の手で足した障害は載せない
        add(
            conn,
            date("2026-11-03"),
            ItemKind::Blocker,
            "権限待ち",
            Position::Last,
            &tokyo(),
            at(T1),
        )
        .unwrap();

        let context = request_context(conn, todo.id).unwrap().unwrap();
        assert_eq!(context.date, date("2026-11-03"));
        assert_eq!(context.section.heading.as_ref().unwrap().text, "10:00 定例");
        assert_eq!(context.section.lines, 3..7);
        assert_eq!(context.lines, ["- 調べる", "- 権限待ち", "- 解消済み", ""]);
        let blockers: Vec<Ulid> = context.blockers.iter().map(|b| b.id).collect();
        assert_eq!(blockers, [waiting.id]);
    }

    #[test]
    fn the_request_context_without_a_heading_is_the_prelude() {
        let (db, _dir) = open();
        let conn = db.connection();
        let body = "- 調べる\n- 待ち\n## 10:00 定例\n- 後ろ";
        let todo = send_ok(conn, "2026-11-03", body, 1..=1, ItemKind::Todo).sent[0]
            .item
            .clone();
        let context = request_context(conn, todo.id).unwrap().unwrap();
        assert_eq!(context.section.heading, None);
        assert_eq!(context.lines, ["- 調べる", "- 待ち"]);
        assert!(context.blockers.is_empty());

        // ノートから来ていない項目は文脈が無い
        let other = add(
            conn,
            date("2026-11-03"),
            ItemKind::Todo,
            "手で足した",
            Position::Last,
            &tokyo(),
            at(T1),
        )
        .unwrap();
        assert_eq!(request_context(conn, other.id).unwrap(), None);
    }

    #[test]
    fn the_request_context_of_a_carried_todo_uses_the_latest_state_of_carried_blockers() {
        let (db, _dir) = open();
        let conn = db.connection();
        let body = "## 10:00 定例\n- 調べる\n- 権限待ち\n- 予算待ち";
        let todo = send_ok(conn, "2026-11-02", body, 2..=2, ItemKind::Todo).sent[0]
            .item
            .clone();
        let blockers = send_ok(conn, "2026-11-02", body, 3..=4, ItemKind::Blocker);
        let done = carryover::confirm(
            conn,
            date("2026-11-03"),
            &Decision {
                from: date("2026-11-02"),
                selection: Selection::All,
                reviews: HashMap::new(),
            },
            &tokyo(),
            at(T2),
        )
        .unwrap();
        let carried_of = |id: Ulid| {
            done.created
                .iter()
                .find(|i| i.carried_from_id == Some(id))
                .unwrap()
                .clone()
        };
        let carried_todo = carried_of(todo.id);
        let waiting = carried_of(blockers.sent[0].item.id);
        let budget = carried_of(blockers.sent[1].item.id);
        // 持ち越した先で解消した障害は載せない（元の日の障害は未解消のまま）
        set_closed(conn, budget.id, Some(at(T2)), at(T2)).unwrap();

        let context = request_context(conn, carried_todo.id).unwrap().unwrap();
        assert_eq!(context.date, date("2026-11-02"));
        let ids: Vec<Ulid> = context.blockers.iter().map(|b| b.id).collect();
        assert_eq!(ids, [waiting.id]);
        assert_eq!(context.blockers[0].date, date("2026-11-03"));
    }
}
