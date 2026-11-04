//! ノートと振り返りの本文の読み書き（docs/design/02-data-model.md 3章 note / reflection、
//! MCP・CLI 仕様書 3章 `get_note`・`append_note`・`edit_note`・`append_reflection`、
//! アーキテクチャ設計書 4.1）。
//!
//! - 画面の保存（[`save`]）は、もとにした版の番号を受け取り、いまの版と違えば受け付けずに
//!   最新の本文を返す（保存の行き違い）。人の編集なので変更履歴には記録しない。
//! - エージェント・CLI の書き込み（[`append_note`]・[`append_reflection`]・[`edit_note`]）は、
//!   受け付けた時点で書き込み、change_log に記録する。本文が変わらなかった操作は、版を進めず
//!   記録もしない（データモデル設計書 change_log）。
//!
//! 本文の行・見出し・追記の決まりは [`super::note_body`] にある。
//!
//! エージェントの書き込みは、はじめに権限（スコープ。`append`・`note_edit`）を確かめる
//! （[`super::agent::require_scope`]）。
//!
//! 画面の保存の失敗中は、引数の誤りの後に `storage_unavailable` を返す（[`super::agent::UiState`]。
//! アーキテクチャ設計書 4.1）。
//!
//! `edit_note` は、利用者の入力中の行（守る行）を書き換え・削除・移動しようとしたときと、画面から
//! 知らされた守る行が古い版のものであるときに `conflict`（`reason: user_editing`）にする
//! （[`TextDocError::UserEditing`]。守る行は [`super::agent::UiState::protected_lines`] で受け取る）。

use jiff::Timestamp;
use jiff::tz::TimeZone;
use rusqlite::Connection;

use super::agent::{self, Agent, PermissionDenied, StorageUnavailable};
use super::note_body::{self, Changed, InvalidArgument, LineEdit, LineEditError};
use crate::domain::day;
use crate::domain::logical_date::LogicalDate;
use crate::storage::table::agent_client::Scope;
use crate::storage::table::change_log::{self, Actor, ChangeLog, Target};
use crate::storage::table::text_doc;
use crate::storage::{StorageError, Ulid, in_transaction};

pub use crate::storage::table::text_doc::TextDoc;

/// ある時点の本文と版の番号。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub body: String,
    /// 版の番号。まだ一度も書いていない（行が無い）ときは 0 で、本文は空。
    pub rev: i64,
}

/// 書き込みの結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Written {
    /// 書き込んだ後の版の番号。書かなかったときは、いまの版の番号。
    pub rev: i64,
    /// 本文を実際に変えたか（MCP の成功の応答の `updated`）。`false` のときは版を進めず、
    /// 変更履歴にも記録していない。
    pub updated: bool,
}

/// 読み書きが失敗した理由。
#[derive(Debug)]
pub enum TextDocError {
    /// エージェントに、操作のスコープが許可されていない（`permission_denied`）。何も書いていない。
    PermissionDenied(PermissionDenied),
    /// 引数が誤っている（`invalid_argument`）。何も書いていない。
    InvalidArgument(InvalidArgument),
    /// 画面の保存の書き込みが失敗して再試行している（`storage_unavailable`。アーキテクチャ設計書
    /// 4.1）。何も書いていない。
    StorageUnavailable(StorageUnavailable),
    /// `edit_note` の行が、エージェントの読んだ後に変わっている（`conflict`、`reason: changed`）。
    /// 何も書いていない。
    Changed(Changed),
    /// `edit_note` の行が、利用者の入力中の行（守る行）か、画面が知らせた守る行が古い版のもの
    /// （`conflict`、`reason: user_editing`。アーキテクチャ設計書 4.1）。何も書いていない。
    UserEditing,
    /// 画面の保存の、もとにした版が古い（その間にエージェントが変えた）。何も書いていない。
    /// 画面は `latest` を取り込んでから保存し直す（アーキテクチャ設計書 4.1）。
    Stale { latest: Snapshot },
    /// 読み書きに失敗した。トランザクションは取り消してある。
    Storage(StorageError),
}

impl std::fmt::Display for TextDocError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PermissionDenied(e) => e.fmt(f),
            Self::InvalidArgument(e) => e.fmt(f),
            Self::StorageUnavailable(e) => e.fmt(f),
            Self::Changed(e) => e.fmt(f),
            Self::UserEditing => f.write_str("the user is editing the line"),
            Self::Stale { latest } => write!(
                f,
                "the text was changed in the meantime (now at rev {})",
                latest.rev
            ),
            Self::Storage(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for TextDocError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidArgument(e) => Some(e),
            Self::Storage(e) => Some(e),
            Self::PermissionDenied(_)
            | Self::StorageUnavailable(_)
            | Self::Changed(_)
            | Self::UserEditing
            | Self::Stale { .. } => None,
        }
    }
}

impl From<StorageError> for TextDocError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

impl From<rusqlite::Error> for TextDocError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Storage(e.into())
    }
}

impl From<PermissionDenied> for TextDocError {
    fn from(e: PermissionDenied) -> Self {
        Self::PermissionDenied(e)
    }
}

impl From<StorageUnavailable> for TextDocError {
    fn from(e: StorageUnavailable) -> Self {
        Self::StorageUnavailable(e)
    }
}

impl From<InvalidArgument> for TextDocError {
    fn from(e: InvalidArgument) -> Self {
        Self::InvalidArgument(e)
    }
}

impl From<LineEditError> for TextDocError {
    fn from(e: LineEditError) -> Self {
        match e {
            LineEditError::InvalidArgument(e) => Self::InvalidArgument(e),
            LineEditError::Changed(e) => Self::Changed(e),
        }
    }
}

/// その日の本文と版の番号（`get_note`、画面の表示）。まだ書いていない日は空の本文と版 0。
/// 読むだけなので、day の行も作らない。
pub fn get(conn: &Connection, doc: TextDoc, date: LogicalDate) -> Result<Snapshot, StorageError> {
    Ok(match text_doc::get(conn, doc, date)? {
        Some(row) => Snapshot {
            body: row.body,
            rev: row.rev,
        },
        None => Snapshot {
            body: String::new(),
            rev: 0,
        },
    })
}

/// `get_note` の `--heading` の誤り（`invalid_argument`）。同じ引数でやり直しても成功しない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidHeading {
    /// 前後の空白を除くと空。
    Empty,
    /// 途中に改行を含む（見出しは1行）。
    Multiline,
}

impl std::fmt::Display for InvalidHeading {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Empty => "The heading is empty.",
            Self::Multiline => "The heading must be a single line.",
        })
    }
}

impl std::error::Error for InvalidHeading {}

/// `get_note` が失敗した理由。読み取りなので `storage_unavailable` は無い（MCP・CLI 仕様書 4.3）。
#[derive(Debug)]
pub enum NoteReadError {
    /// `read` スコープが許可されていない（`permission_denied`）。何も読んでいない。
    PermissionDenied(PermissionDenied),
    /// `--heading` が誤っている（`invalid_argument`）。
    InvalidArgument(InvalidHeading),
    /// 指定した見出しがノートに無い（`not_found`）。
    NotFound,
    /// 読み取りに失敗した。
    Storage(StorageError),
}

impl std::fmt::Display for NoteReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PermissionDenied(e) => e.fmt(f),
            Self::InvalidArgument(e) => e.fmt(f),
            Self::NotFound => f.write_str("the note has no such heading"),
            Self::Storage(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for NoteReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidArgument(e) => Some(e),
            Self::Storage(e) => Some(e),
            Self::PermissionDenied(_) | Self::NotFound => None,
        }
    }
}

impl From<PermissionDenied> for NoteReadError {
    fn from(e: PermissionDenied) -> Self {
        Self::PermissionDenied(e)
    }
}

impl From<StorageError> for NoteReadError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

impl From<rusqlite::Error> for NoteReadError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Storage(e.into())
    }
}

/// `get_note --heading` で当たった、1つの見出しの区間。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadingSection {
    /// 見出しの文字（`get_board` の `headings` の1つ。例：`10:00 定例`）。
    pub heading: String,
    /// 見出しの行の、ノート全体での行番号（1始まり）。`text` の1行目がこの行で、`n` 行目は
    /// `start_line + n - 1` 行目。`edit_note` の `--line` はこの番号で数える。
    pub start_line: usize,
    /// 見出しの行から区間の終わり（次の時刻付き見出しの前）までを、LF でつないだもの。
    /// 区間の末尾の空行も含める。
    pub text: String,
}

/// `get_note` の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteRead {
    /// 見出しを指定しない：全文と版（`rev`）。
    Whole(Snapshot),
    /// 見出しを指定した：当たった区間を本文の順に（`heading` と同じ見出しが複数あれば、すべて）。
    Sections {
        rev: i64,
        sections: Vec<HeadingSection>,
    },
}

/// エージェント・CLI が、ノートを読む（`get_note`）。`read` スコープが要る。`heading` が `None` なら
/// 全文、`Some(H)` ならその見出しの区間（照合の決まりはデータモデル設計書 3章 note）。
///
/// 判定の順は `permission_denied`、`invalid_argument`（`H` が空・改行を含む）、`not_found`。
/// 読むだけで、day の行も作らない。
pub fn get_note(
    conn: &Connection,
    client_id: Ulid,
    date: LogicalDate,
    heading: Option<&str>,
) -> Result<NoteRead, NoteReadError> {
    crate::storage::in_read_transaction(conn, || {
        agent::require_scope::<NoteReadError>(conn, client_id, Scope::Read)?;
        let wanted = heading
            .map(|h| {
                let h = h.trim();
                if h.is_empty() {
                    Err(InvalidHeading::Empty)
                } else if h.contains(['\n', '\r']) {
                    Err(InvalidHeading::Multiline)
                } else {
                    Ok(h)
                }
            })
            .transpose()
            .map_err(NoteReadError::InvalidArgument)?;
        let snapshot = get(conn, TextDoc::Note, date)?;
        let Some(wanted) = wanted else {
            return Ok(NoteRead::Whole(snapshot));
        };
        let sections: Vec<HeadingSection> = note_body::sections(&snapshot.body)
            .into_iter()
            .filter_map(|section| {
                let h = section.heading.filter(|h| h.text == wanted)?;
                let text = note_body::lines(&snapshot.body)
                    .skip(h.line - 1)
                    .take(section.lines.end - h.line)
                    .collect::<Vec<_>>()
                    .join("\n");
                Some(HeadingSection {
                    heading: h.text,
                    start_line: h.line,
                    text,
                })
            })
            .collect();
        if sections.is_empty() {
            return Err(NoteReadError::NotFound);
        }
        Ok(NoteRead::Sections {
            rev: snapshot.rev,
            sections,
        })
    })
}

/// 画面からの保存。`base_rev` は、もとにした本文の版の番号（まだ書いていない日は 0）。
///
/// - 本文がいまと同じなら、書かずに `updated: false` といまの版を返す（版を進めない。版が進むと、
///   画面が知らせた守る行が古いものになるため。アーキテクチャ設計書 4.1）。`base_rev` が古くても
///   同じ。まだ書いていない日に空の本文を保存したときも、行を作らない。
/// - そうでなく `base_rev` がいまの版と違えば、書かずに [`TextDocError::Stale`] で最新を返す。
/// - 書くときは、その日の day の行が無ければ端末のタイムゾーン `tz` で作る（[`day::ensure`]）。
///
/// 人の編集なので change_log には記録しない。`now` は `crate::storage::table::now` で作る。
pub fn save(
    conn: &Connection,
    doc: TextDoc,
    date: LogicalDate,
    body: &str,
    base_rev: i64,
    tz: &TimeZone,
    now: Timestamp,
) -> Result<Written, TextDocError> {
    in_transaction(conn, || {
        match save_screen_body(conn, doc, date, body, base_rev, tz, now)? {
            ScreenSave::Saved(written) => Ok(written),
            ScreenSave::Stale(latest) => Err(TextDocError::Stale { latest }),
        }
    })
}

/// [`save_screen_body`] の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScreenSave {
    /// 保存した（本文が同じなら書かずに成功）。
    Saved(Written),
    /// もとにした版が古いので書かなかった。いまの本文と版。
    Stale(Snapshot),
}

/// [`save`] の中身。呼び出し側のトランザクションの中で呼ぶ（行送り
/// [`super::note_send::send`] も、本文の保存と項目の作成を1つのトランザクションにするために使う）。
pub(crate) fn save_screen_body(
    conn: &Connection,
    doc: TextDoc,
    date: LogicalDate,
    body: &str,
    base_rev: i64,
    tz: &TimeZone,
    now: Timestamp,
) -> Result<ScreenSave, StorageError> {
    let current = get(conn, doc, date)?;
    // 同じ本文なら、もとにした版が古くても書かずに成功にする。行き違いにすると、画面の取り込みで
    // 同じ行が両方から足されたものとして二重になるため
    if current.body == body {
        return Ok(ScreenSave::Saved(Written {
            rev: current.rev,
            updated: false,
        }));
    }
    if current.rev != base_rev {
        return Ok(ScreenSave::Stale(current));
    }
    day::ensure(conn, date, tz, now)?;
    let rev = text_doc::save(conn, doc, date, body, now)?;
    Ok(ScreenSave::Saved(Written { rev, updated: true }))
}

/// エージェント・CLI が、ノートの末尾に追記する（`append_note`）。`heading` が `Some` なら、
/// 現在時刻（`now` の、端末のタイムゾーン `tz` での時刻）の見出し `## HH:MM 件名` を入れてから
/// 追記する（件名が空なら `## HH:MM`）。追記の決まりは [`note_body::append`]。
///
/// `agent` は操作したエージェント（CLI は `cli`）。`append` スコープが要る。
/// change_log に `note.append` として記録する。
pub fn append_note(
    conn: &Connection,
    date: LogicalDate,
    text: &str,
    heading: Option<&str>,
    agent: &Agent<'_>,
    tz: &TimeZone,
    now: Timestamp,
) -> Result<Written, TextDocError> {
    let new_body = |body: &str| {
        // 件名の誤りも引数の誤りなので、権限を確かめた後に確かめる
        let heading = heading
            .map(|title| note_body::time_heading_line(now.to_zoned(tz.clone()).time(), title))
            .transpose()?;
        Ok(note_body::append(body, text, heading.as_deref())?)
    };
    write_by_agent(
        conn,
        date,
        AgentOperation::NoteAppend,
        agent,
        tz,
        now,
        None,
        new_body,
    )
}

/// エージェント・CLI が、振り返りの末尾に追記する（`append_reflection`）。追記の決まりは
/// [`note_body::append`]。`append` スコープが要る。change_log に `reflection.append` として記録する。
///
/// 振り返りの既存の文章を書き換える操作は無い（MCP・CLI 仕様書 2章）。
pub fn append_reflection(
    conn: &Connection,
    date: LogicalDate,
    text: &str,
    agent: &Agent<'_>,
    tz: &TimeZone,
    now: Timestamp,
) -> Result<Written, TextDocError> {
    let new_body = |body: &str| Ok(note_body::append(body, text, None)?);
    write_by_agent(
        conn,
        date,
        AgentOperation::ReflectionAppend,
        agent,
        tz,
        now,
        None,
        new_body,
    )
}

/// エージェント・CLI が、ノートの1行を書き換え・削除・移動する（`edit_note`）。
///
/// 行の本文が `expected_text` と一致しなければ [`TextDocError::Changed`]、引数の誤りは
/// [`TextDocError::InvalidArgument`]（確かめる順は [`note_body::apply_line_edit`]）。
/// 画面が守る行を知らせている間（[`super::agent::UiState::protected_lines`]）は、知らせた版が
/// いまの版と違うか、`edit.line`（書き換え・削除・移動する行）が守る行なら
/// [`TextDocError::UserEditing`]。`changed` に当たるときはそちらを返し（`--expect` を先に確かめる。
/// MCP・CLI 仕様書 3章）、本文が変わらない書き換えでも `user_editing` を返す（項目の本文と同じく、
/// 入力中の行はまだ保存されておらず、利用者の見ている行と違いうるため）。ほかの行を動かして
/// 守る行の番号がずれるだけの操作は受け付ける（画面は差分として取り込む）。
/// まだ書いていない日のノートは、空の1行の本文として扱う。本文が変わらない書き換えは
/// `updated: false` で、記録しない。`note_edit` スコープが要る。change_log には `note.edit` として
/// 記録する。
pub fn edit_note(
    conn: &Connection,
    date: LogicalDate,
    edit: &LineEdit,
    agent: &Agent<'_>,
    tz: &TimeZone,
    now: Timestamp,
) -> Result<Written, TextDocError> {
    let new_body = |body: &str| Ok(note_body::apply_line_edit(body, edit)?);
    write_by_agent(
        conn,
        date,
        AgentOperation::NoteEdit,
        agent,
        tz,
        now,
        Some(edit.line),
        new_body,
    )
}

/// 画面が知らせた守る行に照らして、`edit_note` が行 `line` を変えてよいか（アーキテクチャ設計書 4.1）。
/// `rev` はいまの版。守る行が無ければ（知らせが無いか空なら）変えてよい。
fn check_protected(
    agent: &Agent<'_>,
    doc: TextDoc,
    date: LogicalDate,
    rev: i64,
    line: usize,
) -> Result<(), TextDocError> {
    match agent.ui.protected_lines(doc, date) {
        Some(p) if !p.lines.is_empty() && (p.rev != rev || p.lines.contains(&line)) => {
            Err(TextDocError::UserEditing)
        }
        _ => Ok(()),
    }
}

/// エージェント・CLI による本文の書き込みの種類。change_log の `operation` と対象の表を決める。
#[derive(Debug, Clone, Copy)]
enum AgentOperation {
    NoteAppend,
    ReflectionAppend,
    NoteEdit,
}

impl AgentOperation {
    /// change_log の `operation`（MCP・CLI 仕様書 3章の命名規則）。
    const fn as_str(self) -> &'static str {
        match self {
            Self::NoteAppend => "note.append",
            Self::ReflectionAppend => "reflection.append",
            Self::NoteEdit => "note.edit",
        }
    }

    /// 操作に要るスコープ（MCP・CLI 仕様書 2章）。
    const fn scope(self) -> Scope {
        match self {
            Self::NoteAppend | Self::ReflectionAppend => Scope::Append,
            Self::NoteEdit => Scope::NoteEdit,
        }
    }

    const fn doc(self) -> TextDoc {
        match self {
            Self::NoteAppend | Self::NoteEdit => TextDoc::Note,
            Self::ReflectionAppend => TextDoc::Reflection,
        }
    }
}

/// エージェントの書き込みの共通の手順。権限・引数・保存の状態を確かめてから、いまの本文から `new_body` で
/// 新しい本文を作り、いまと違えば、本文と change_log の記録を1つのトランザクションで書く。
/// `edited_line` は `edit_note` の変える行（守る行の判定に使う。[`check_protected`]）。
#[allow(clippy::too_many_arguments)]
fn write_by_agent(
    conn: &Connection,
    date: LogicalDate,
    operation: AgentOperation,
    agent: &Agent<'_>,
    tz: &TimeZone,
    now: Timestamp,
    edited_line: Option<usize>,
    new_body: impl FnOnce(&str) -> Result<String, TextDocError>,
) -> Result<Written, TextDocError> {
    let doc = operation.doc();
    in_transaction(conn, || {
        // 権限の確かめも、書き込みと同じトランザクションで行う（項目の操作と同じ）
        agent::require_scope::<TextDocError>(conn, agent.client_id, operation.scope())?;
        let current = get(conn, doc, date)?;
        // 判定の順：引数の誤り、保存の失敗中、行の食い違い（changed）、入力中の行（user_editing）
        // （MCP・CLI 仕様書 3章・3.2・4.3）
        let body = new_body(&current.body);
        if let Err(e @ TextDocError::InvalidArgument(_)) = body {
            return Err(e);
        }
        agent::require_save_working(agent)?;
        let body = body?;
        if let Some(line) = edited_line {
            check_protected(agent, doc, date, current.rev, line)?;
        }
        if body == current.body {
            return Ok(Written {
                rev: current.rev,
                updated: false,
            });
        }
        day::ensure(conn, date, tz, now)?;
        let rev = text_doc::save(conn, doc, date, &body, now)?;
        let target = match doc {
            TextDoc::Note => Target::Note(date),
            TextDoc::Reflection => Target::Reflection(date),
        };
        change_log::insert(
            conn,
            &ChangeLog {
                id: Ulid::generate(),
                actor: Actor::Agent,
                agent_client_id: Some(agent.client_id),
                operation: operation.as_str().to_owned(),
                target,
                // まだ書いていなかった日も、空の本文として記録する（`get` と同じ。取り消しで
                // 書き戻す本文が、行の有無にかかわらず1通りに決まるように）
                before: Some(json_string(&current.body)),
                after: Some(json_string(&body)),
                created_at: now,
                reverted_at: None,
                revert_of_id: None,
                seen_at: None,
            },
        )?;
        Ok(Written { rev, updated: true })
    })
}

/// 本文を change_log の `before` / `after` に入れる形（JSON の文字列）にする。
fn json_string(body: &str) -> String {
    serde_json::to_string(body).expect("a string always serializes to JSON")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::test_support::{FakeUi, grant};
    use crate::domain::note_body::LineOp;
    use crate::storage::Database;
    use crate::storage::table::agent_client::{CLI_ID, UNKNOWN_ID};
    use crate::storage::table::test_support::{at, date, fail_change_log_inserts};
    use crate::storage::table::{day as day_table, text_doc as text_doc_table};

    fn cli() -> Agent<'static> {
        Agent::without_ui(CLI_ID)
    }

    /// データベースを開き、`cli` と `unknown` に書き込みのスコープを許可する。
    fn open() -> (Database, tempfile::TempDir) {
        let (db, dir) = crate::storage::table::test_support::open();
        for client in [CLI_ID, UNKNOWN_ID] {
            grant(db.connection(), client, &[Scope::Append, Scope::NoteEdit]);
        }
        (db, dir)
    }

    fn tokyo() -> TimeZone {
        TimeZone::get("Asia/Tokyo").unwrap()
    }

    /// 東京の 2026-11-03 10:05。
    const NOW: &str = "2026-11-03T01:05:00Z";

    fn logs_of(conn: &Connection, target: Target) -> Vec<ChangeLog> {
        change_log::list_by_target(conn, target).unwrap()
    }

    #[test]
    fn get_returns_an_empty_body_at_rev_0_without_creating_rows() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        for doc in [TextDoc::Note, TextDoc::Reflection] {
            assert_eq!(
                get(conn, doc, d).unwrap(),
                Snapshot {
                    body: String::new(),
                    rev: 0
                }
            );
        }
        assert_eq!(day_table::get(conn, d).unwrap(), None);
    }

    #[test]
    fn save_creates_the_day_and_advances_the_rev() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        for doc in [TextDoc::Note, TextDoc::Reflection] {
            let w = save(conn, doc, d, "a", 0, &tokyo(), at(NOW)).unwrap();
            assert_eq!(
                w,
                Written {
                    rev: 1,
                    updated: true
                }
            );
            let w = save(conn, doc, d, "ab", 1, &tokyo(), at(NOW)).unwrap();
            assert_eq!(
                w,
                Written {
                    rev: 2,
                    updated: true
                }
            );
            assert_eq!(
                get(conn, doc, d).unwrap(),
                Snapshot {
                    body: "ab".into(),
                    rev: 2
                }
            );
        }
        assert_eq!(day_table::get(conn, d).unwrap().unwrap().tz, "Asia/Tokyo");
        // 人の保存は記録しない
        assert!(logs_of(conn, Target::Note(d)).is_empty());
        assert!(logs_of(conn, Target::Reflection(d)).is_empty());
    }

    #[test]
    fn save_with_an_old_rev_returns_the_latest_without_writing() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        save(conn, TextDoc::Note, d, "a", 0, &tokyo(), at(NOW)).unwrap();
        append_note(conn, d, "agent", None, &cli(), &tokyo(), at(NOW)).unwrap();
        for base_rev in [0, 1, 3] {
            match save(
                conn,
                TextDoc::Note,
                d,
                "a\nuser",
                base_rev,
                &tokyo(),
                at(NOW),
            ) {
                Err(TextDocError::Stale { latest }) => assert_eq!(
                    latest,
                    Snapshot {
                        body: "a\nagent".into(),
                        rev: 2
                    }
                ),
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(get(conn, TextDoc::Note, d).unwrap().body, "a\nagent");
        // 最新の版をもとにすれば保存できる
        let w = save(
            conn,
            TextDoc::Note,
            d,
            "a\nagent\nuser",
            2,
            &tokyo(),
            at(NOW),
        )
        .unwrap();
        assert_eq!(
            w,
            Written {
                rev: 3,
                updated: true
            }
        );
    }

    #[test]
    fn saving_the_same_body_does_not_advance_the_rev() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        // まだ書いていない日に空の本文を保存しても、行を作らない
        let w = save(conn, TextDoc::Note, d, "", 0, &tokyo(), at(NOW)).unwrap();
        assert_eq!(
            w,
            Written {
                rev: 0,
                updated: false
            }
        );
        assert_eq!(text_doc_table::get(conn, TextDoc::Note, d).unwrap(), None);
        assert_eq!(day_table::get(conn, d).unwrap(), None);

        save(conn, TextDoc::Note, d, "a", 0, &tokyo(), at(NOW)).unwrap();
        let w = save(conn, TextDoc::Note, d, "a", 1, &tokyo(), at(NOW)).unwrap();
        assert_eq!(
            w,
            Written {
                rev: 1,
                updated: false
            }
        );

        // もとにした版が古くても、同じ本文なら行き違いにせず、いまの版を返す
        append_note(conn, d, "x", None, &cli(), &tokyo(), at(NOW)).unwrap();
        let w = save(conn, TextDoc::Note, d, "a\nx", 1, &tokyo(), at(NOW)).unwrap();
        assert_eq!(
            w,
            Written {
                rev: 2,
                updated: false
            }
        );
    }

    #[test]
    fn append_note_writes_and_records_the_change() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        let w = append_note(conn, d, "first", None, &cli(), &tokyo(), at(NOW)).unwrap();
        assert_eq!(
            w,
            Written {
                rev: 1,
                updated: true
            }
        );
        let w = append_note(conn, d, "second", None, &cli(), &tokyo(), at(NOW)).unwrap();
        assert_eq!(
            w,
            Written {
                rev: 2,
                updated: true
            }
        );
        assert_eq!(get(conn, TextDoc::Note, d).unwrap().body, "first\nsecond");
        assert_eq!(day_table::get(conn, d).unwrap().unwrap().tz, "Asia/Tokyo");

        let logs = logs_of(conn, Target::Note(d));
        assert_eq!(logs.len(), 2);
        let first = &logs[0];
        assert_eq!(first.actor, Actor::Agent);
        assert_eq!(first.agent_client_id, Some(CLI_ID));
        assert_eq!(first.operation, "note.append");
        // まだ書いていなかった日は、空の本文が変更前
        assert_eq!(first.before.as_deref(), Some(r#""""#));
        assert_eq!(first.after.as_deref(), Some(r#""first""#));
        assert_eq!(first.created_at, at(NOW));
        assert_eq!(logs[1].before.as_deref(), Some(r#""first""#));
        assert_eq!(logs[1].after.as_deref(), Some(r#""first\nsecond""#));
    }

    #[test]
    fn append_note_inserts_a_heading_at_the_local_time() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        save(conn, TextDoc::Note, d, "memo", 0, &tokyo(), at(NOW)).unwrap();
        append_note(conn, d, "- 議題", Some("定例"), &cli(), &tokyo(), at(NOW)).unwrap();
        append_note(conn, d, "x", Some(""), &cli(), &tokyo(), at(NOW)).unwrap();
        // 見出しの時刻は端末のタイムゾーンでの時刻
        let utc = TimeZone::UTC;
        append_note(conn, d, "y", Some("utc"), &cli(), &utc, at(NOW)).unwrap();
        let body = get(conn, TextDoc::Note, d).unwrap().body;
        assert_eq!(
            body,
            "memo\n\n## 10:05 定例\n- 議題\n\n## 10:05\nx\n\n## 01:05 utc\ny"
        );
        let texts: Vec<_> = note_body::headings(&body)
            .into_iter()
            .map(|h| h.text)
            .collect();
        assert_eq!(texts, ["10:05 定例", "10:05", "01:05 utc"]);
    }

    #[test]
    fn append_reflection_records_reflection_append() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        let agent = Agent::without_ui(UNKNOWN_ID);
        append_reflection(conn, d, "良かった\n", &agent, &tokyo(), at(NOW)).unwrap();
        append_reflection(conn, d, "明日", &agent, &tokyo(), at(NOW)).unwrap();
        assert_eq!(
            get(conn, TextDoc::Reflection, d).unwrap(),
            Snapshot {
                body: "良かった\n明日".into(),
                rev: 2
            }
        );
        assert_eq!(get(conn, TextDoc::Note, d).unwrap().rev, 0);
        let logs = logs_of(conn, Target::Reflection(d));
        assert_eq!(logs.len(), 2);
        assert!(
            logs.iter().all(
                |l| l.operation == "reflection.append" && l.agent_client_id == Some(UNKNOWN_ID)
            )
        );
        assert!(logs_of(conn, Target::Note(d)).is_empty());
    }

    #[test]
    fn invalid_appends_write_nothing() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        assert!(matches!(
            append_note(conn, d, " \n", None, &cli(), &tokyo(), at(NOW)),
            Err(TextDocError::InvalidArgument(InvalidArgument::EmptyText))
        ));
        assert!(matches!(
            append_note(conn, d, "x", Some("a\nb"), &cli(), &tokyo(), at(NOW)),
            Err(TextDocError::InvalidArgument(
                InvalidArgument::MultilineTitle
            ))
        ));
        assert!(matches!(
            append_reflection(conn, d, "", &cli(), &tokyo(), at(NOW)),
            Err(TextDocError::InvalidArgument(InvalidArgument::EmptyText))
        ));
        assert_eq!(day_table::get(conn, d).unwrap(), None);
        assert!(logs_of(conn, Target::Note(d)).is_empty());
    }

    fn edit(line: usize, expected: &str, op: LineOp) -> LineEdit {
        LineEdit {
            line,
            expected_text: expected.into(),
            op,
        }
    }

    #[test]
    fn edit_note_rewrites_a_line_and_records_the_change() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        save(conn, TextDoc::Note, d, "a\nb\nc", 0, &tokyo(), at(NOW)).unwrap();
        let later = at("2026-11-03T02:00:00Z");
        let w = edit_note(
            conn,
            d,
            &edit(2, "b", LineOp::Replace("B".into())),
            &cli(),
            &tokyo(),
            later,
        )
        .unwrap();
        assert_eq!(
            w,
            Written {
                rev: 2,
                updated: true
            }
        );
        edit_note(
            conn,
            d,
            &edit(1, "a", LineOp::MoveTo(3)),
            &cli(),
            &tokyo(),
            later,
        )
        .unwrap();
        edit_note(
            conn,
            d,
            &edit(1, "B", LineOp::Delete),
            &cli(),
            &tokyo(),
            later,
        )
        .unwrap();
        assert_eq!(
            get(conn, TextDoc::Note, d).unwrap(),
            Snapshot {
                body: "c\na".into(),
                rev: 4
            }
        );
        let logs = logs_of(conn, Target::Note(d));
        assert_eq!(logs.len(), 3);
        assert!(logs.iter().all(|l| l.operation == "note.edit"));
        assert_eq!(logs[0].before.as_deref(), Some(r#""a\nb\nc""#));
        assert_eq!(logs[0].after.as_deref(), Some(r#""a\nB\nc""#));
        assert_eq!(logs[0].created_at, later);
        assert_eq!(
            text_doc_table::get(conn, TextDoc::Note, d)
                .unwrap()
                .unwrap()
                .updated_at,
            later
        );
    }

    #[test]
    fn edit_note_conflicts_when_the_line_changed() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        save(conn, TextDoc::Note, d, "a\nb", 0, &tokyo(), at(NOW)).unwrap();
        let result = edit_note(
            conn,
            d,
            &edit(1, "b", LineOp::Delete),
            &cli(),
            &tokyo(),
            at(NOW),
        );
        match result {
            Err(TextDocError::Changed(Changed::LineMismatch { line: 1, actual })) => {
                assert_eq!(actual, "a");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            edit_note(
                conn,
                d,
                &edit(3, "", LineOp::Delete),
                &cli(),
                &tokyo(),
                at(NOW)
            ),
            Err(TextDocError::Changed(Changed::LineOutOfRange { .. }))
        ));
        assert!(matches!(
            edit_note(
                conn,
                d,
                &edit(0, "a", LineOp::Delete),
                &cli(),
                &tokyo(),
                at(NOW)
            ),
            Err(TextDocError::InvalidArgument(InvalidArgument::LineZero))
        ));
        assert_eq!(
            get(conn, TextDoc::Note, d).unwrap(),
            Snapshot {
                body: "a\nb".into(),
                rev: 1
            }
        );
        assert!(logs_of(conn, Target::Note(d)).is_empty());
    }

    #[test]
    fn edits_that_change_nothing_are_not_recorded() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        save(conn, TextDoc::Note, d, "a\nb", 0, &tokyo(), at(NOW)).unwrap();
        for e in [
            edit(1, "a", LineOp::Replace("a".into())),
            edit(2, "b", LineOp::MoveTo(2)),
        ] {
            let w = edit_note(conn, d, &e, &cli(), &tokyo(), at(NOW)).unwrap();
            assert_eq!(
                w,
                Written {
                    rev: 1,
                    updated: false
                }
            );
        }
        assert!(logs_of(conn, Target::Note(d)).is_empty());
    }

    #[test]
    fn edit_note_on_an_unwritten_day_sees_one_empty_line() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        let w = edit_note(
            conn,
            d,
            &edit(1, "", LineOp::Replace("x".into())),
            &cli(),
            &tokyo(),
            at(NOW),
        )
        .unwrap();
        assert_eq!(
            w,
            Written {
                rev: 1,
                updated: true
            }
        );
        assert_eq!(get(conn, TextDoc::Note, d).unwrap().body, "x");
        assert_eq!(
            logs_of(conn, Target::Note(d))[0].before.as_deref(),
            Some(r#""""#)
        );
    }

    #[test]
    fn a_failed_write_leaves_nothing_behind() {
        // change_log に入れられないと、本文と day の行も書かれない
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        fail_change_log_inserts(conn);
        assert!(matches!(
            append_note(conn, d, "x", None, &cli(), &tokyo(), at(NOW)),
            Err(TextDocError::Storage(_))
        ));
        assert_eq!(get(conn, TextDoc::Note, d).unwrap().rev, 0);
        assert_eq!(day_table::get(conn, d).unwrap(), None);
        // 接続はトランザクションの外に戻っている
        assert!(conn.is_autocommit());
    }

    #[test]
    fn writes_need_the_scope_before_anything_else() {
        let (db, _dir) = crate::storage::table::test_support::open();
        let conn = db.connection();
        let d = date("2026-11-03");
        let denied = |r: Result<Written, TextDocError>, scope: Scope| {
            assert!(
                matches!(r, Err(TextDocError::PermissionDenied(PermissionDenied { required_scope })) if required_scope == scope)
            );
        };
        // 引数の誤り（空の本文・改行を含む件名・行番号 0）より先に確かめる
        denied(
            append_note(conn, d, "", Some("a\nb"), &cli(), &tokyo(), at(NOW)),
            Scope::Append,
        );
        denied(
            append_reflection(conn, d, "x", &cli(), &tokyo(), at(NOW)),
            Scope::Append,
        );
        grant(conn, CLI_ID, &[Scope::Append]);
        denied(
            edit_note(
                conn,
                d,
                &edit(0, "", LineOp::Delete),
                &cli(),
                &tokyo(),
                at(NOW),
            ),
            Scope::NoteEdit,
        );
        // 登録されていないエージェントには何も許可しない
        denied(
            append_note(
                conn,
                d,
                "x",
                None,
                &Agent::without_ui(Ulid::from_u128(99)),
                &tokyo(),
                at(NOW),
            ),
            Scope::Append,
        );
        assert_eq!(day_table::get(conn, d).unwrap(), None);
        append_reflection(conn, d, "x", &cli(), &tokyo(), at(NOW)).unwrap();
    }

    #[test]
    fn writes_are_refused_while_the_screen_save_is_failing() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        save(conn, TextDoc::Note, d, "a", 0, &tokyo(), at(NOW)).unwrap();
        let ui = FakeUi {
            save_failing: true,
            ..FakeUi::default()
        };
        let agent = Agent {
            client_id: CLI_ID,
            ui: &ui,
        };
        // 引数の誤りは先に返す
        assert!(matches!(
            append_note(conn, d, " ", None, &agent, &tokyo(), at(NOW)),
            Err(TextDocError::InvalidArgument(_))
        ));
        assert!(matches!(
            edit_note(
                conn,
                d,
                &edit(0, "a", LineOp::Delete),
                &agent,
                &tokyo(),
                at(NOW)
            ),
            Err(TextDocError::InvalidArgument(_))
        ));
        // 行の食い違いや、何も変えない書き換えより先に返す
        for e in [
            edit(1, "x", LineOp::Delete),
            edit(1, "a", LineOp::Replace("a".into())),
        ] {
            assert!(matches!(
                edit_note(conn, d, &e, &agent, &tokyo(), at(NOW)),
                Err(TextDocError::StorageUnavailable(_))
            ));
        }
        assert!(matches!(
            append_reflection(conn, d, "x", &agent, &tokyo(), at(NOW)),
            Err(TextDocError::StorageUnavailable(_))
        ));
        assert_eq!(get(conn, TextDoc::Note, d).unwrap().rev, 1);
    }

    /// 守る行（アーキテクチャ設計書 4.1）：書き換え・削除・移動は `user_editing`、ほかの行は受け付ける。
    /// 知らせた版が古ければ、どの行も `user_editing`。`changed` が先。
    #[test]
    fn edit_note_refuses_protected_lines() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = date("2026-11-03");
        save(conn, TextDoc::Note, d, "a\nb\nc\nd", 0, &tokyo(), at(NOW)).unwrap();
        let ui = FakeUi::default();
        let agent = Agent {
            client_id: CLI_ID,
            ui: &ui,
        };
        let run = |e: LineEdit| edit_note(conn, d, &e, &agent, &tokyo(), at(NOW));
        let user_editing = |r: Result<Written, TextDocError>| {
            assert!(matches!(r, Err(TextDocError::UserEditing)), "{r:?}");
        };
        ui.protect(TextDoc::Note, d, 1, &[2]);
        user_editing(run(edit(2, "b", LineOp::Replace("B".into()))));
        user_editing(run(edit(2, "b", LineOp::Delete)));
        user_editing(run(edit(2, "b", LineOp::MoveTo(4))));
        // 何も変えない書き換えでも、入力中の行なら user_editing
        user_editing(run(edit(2, "b", LineOp::Replace("b".into()))));
        // --expect の食い違いが先
        assert!(matches!(
            run(edit(2, "x", LineOp::Delete)),
            Err(TextDocError::Changed(_))
        ));
        // 振り返りの守る行は、ノートの判定に使わない
        ui.protect(TextDoc::Reflection, d, 1, &[1, 3, 4]);
        // ほかの行は受け付ける（守る行の番号がずれる移動も）
        let w = run(edit(4, "d", LineOp::MoveTo(1))).unwrap();
        assert_eq!(w.rev, 2);
        assert_eq!(get(conn, TextDoc::Note, d).unwrap().body, "d\na\nb\nc");
        // 画面の知らせ直しが届くまで（版が古い間）は、どの行も user_editing
        user_editing(run(edit(1, "d", LineOp::Delete)));
        ui.protect(TextDoc::Note, d, 2, &[3]);
        user_editing(run(edit(3, "b", LineOp::Delete)));
        run(edit(1, "d", LineOp::Delete)).unwrap();
        // 守る行が無い知らせ（空）なら、版が違っても通常の判定
        ui.protect(TextDoc::Note, d, 1, &[]);
        run(edit(2, "b", LineOp::Replace("B".into()))).unwrap();
        // 追記は守る行に関係なく受け付ける
        ui.protect(TextDoc::Note, d, 1, &[1]);
        append_note(conn, d, "e", None, &agent, &tokyo(), at(NOW)).unwrap();
        assert_eq!(get(conn, TextDoc::Note, d).unwrap().body, "a\nB\nc\ne");
        assert_eq!(logs_of(conn, Target::Note(d)).len(), 4);
    }

    #[test]
    fn writes_join_the_callers_transaction() {
        let (db, _dir) = open();
        let d = date("2026-11-03");
        {
            let tx = db.connection().unchecked_transaction().unwrap();
            append_note(&tx, d, "x", None, &cli(), &tokyo(), at(NOW)).unwrap();
            // 確定せずに手放すと取り消される
        }
        let conn = db.connection();
        assert_eq!(get(conn, TextDoc::Note, d).unwrap().rev, 0);
        assert!(logs_of(conn, Target::Note(d)).is_empty());
    }

    const D: &str = "2026-11-03";

    fn note_of(conn: &Connection, body: &str) {
        save(conn, TextDoc::Note, date(D), body, 0, &tokyo(), at(NOW)).unwrap();
    }

    fn sections(r: Result<NoteRead, NoteReadError>) -> Vec<(String, usize, String)> {
        match r.unwrap() {
            NoteRead::Sections { sections, .. } => sections
                .into_iter()
                .map(|s| (s.heading, s.start_line, s.text))
                .collect(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn get_note_needs_the_read_scope() {
        let (db, _dir) = open();
        let conn = db.connection();
        note_of(conn, "## 10:00 定例\n議事");
        let denied = |r: Result<NoteRead, NoteReadError>| {
            assert!(
                matches!(
                    r,
                    Err(NoteReadError::PermissionDenied(PermissionDenied {
                        required_scope: Scope::Read
                    }))
                ),
                "{r:?}"
            );
        };
        // 登録されていないエージェント
        let nobody = Ulid::from_u128(99);
        denied(get_note(conn, nobody, date(D), None));
        denied(get_note(conn, nobody, date(D), Some("10:00 定例")));
        // スコープを空にしたクライアントは、権限の誤りが引数の誤りや not_found より先に返る
        assert!(crate::storage::table::agent_client::update_scopes(conn, CLI_ID, &[]).unwrap());
        denied(get_note(conn, CLI_ID, date(D), None));
        denied(get_note(conn, CLI_ID, date(D), Some("")));
        denied(get_note(conn, CLI_ID, date(D), Some("no such")));
        // 書き込みのスコープだけでは読めない
        assert!(
            crate::storage::table::agent_client::update_scopes(conn, CLI_ID, &[Scope::Append])
                .unwrap()
        );
        denied(get_note(conn, CLI_ID, date(D), None));
        grant(conn, CLI_ID, &[]);
        assert!(get_note(conn, CLI_ID, date(D), None).is_ok());
    }

    #[test]
    fn get_note_without_a_heading_returns_the_whole_note_without_writing() {
        let (db, _dir) = open();
        let conn = db.connection();
        assert_eq!(
            get_note(conn, CLI_ID, date(D), None).unwrap(),
            NoteRead::Whole(Snapshot {
                body: String::new(),
                rev: 0
            })
        );
        assert!(day_table::get(conn, date(D)).unwrap().is_none());
        note_of(conn, "前置き\n## 10:00 定例\n議事");
        assert_eq!(
            get_note(conn, CLI_ID, date(D), None).unwrap(),
            NoteRead::Whole(Snapshot {
                body: "前置き\n## 10:00 定例\n議事".into(),
                rev: 1
            })
        );
    }

    #[test]
    fn get_note_heading_matches_the_whole_heading_text() {
        let (db, _dir) = open();
        let conn = db.connection();
        note_of(
            conn,
            "前置き\n## 9:05 朝会\nA\n\n## 09:05 朝会\nB\n## 10:00\nC\n## 14:00\u{3000}設計 レビュー \nD\n## 良かったこと\nE",
        );
        let get = |h: &str| get_note(conn, CLI_ID, date(D), Some(h));
        // 見出しの行から次の時刻付き見出しの前まで（末尾の空行も含む）。始まりの行は全体での行番号
        assert_eq!(
            sections(get("9:05 朝会")),
            [("9:05 朝会".into(), 2, "## 9:05 朝会\nA\n".into())]
        );
        // 時刻の書き方は書いたまま：09:05 と 9:05 は別の見出し
        assert_eq!(
            sections(get("09:05 朝会")),
            [("09:05 朝会".into(), 5, "## 09:05 朝会\nB".into())]
        );
        // 件名の無い見出し
        assert_eq!(
            sections(get("10:00")),
            [("10:00".into(), 7, "## 10:00\nC".into())]
        );
        // 全角空白の件名・途中の空白は、書いたまま完全一致。指定の前後の空白は除く
        assert_eq!(
            sections(get("  14:00\u{3000}設計 レビュー\n")),
            [(
                "14:00\u{3000}設計 レビュー".into(),
                9,
                // 時刻の無い見出しは区間を区切らないので、その区間に含まれる
                "## 14:00\u{3000}設計 レビュー \nD\n## 良かったこと\nE".into()
            )]
        );
        // 時刻だけ・件名だけ・途中までの一致は当たらない
        for h in [
            "9:05",
            "朝会",
            "9:05 朝",
            "09:05  朝会",
            "## 10:00",
            "良かったこと",
        ] {
            // 良かったこと は時刻付き見出しではないので、見出しでは選べない
            assert!(matches!(get(h), Err(NoteReadError::NotFound)), "{h}");
        }
    }

    #[test]
    fn get_note_heading_returns_every_section_with_the_same_heading_in_order() {
        let (db, _dir) = open();
        let conn = db.connection();
        note_of(conn, "## 10:00 定例\nA\n## 11:00 別件\nB\n## 10:00 定例\nC");
        let r = get_note(conn, CLI_ID, date(D), Some("10:00 定例")).unwrap();
        let NoteRead::Sections { rev, sections } = r else {
            panic!()
        };
        assert_eq!(rev, 1);
        let got: Vec<_> = sections.iter().map(|s| (s.start_line, &*s.text)).collect();
        assert_eq!(got, [(1, "## 10:00 定例\nA"), (5, "## 10:00 定例\nC")]);
    }

    #[test]
    fn get_note_line_numbers_are_the_ones_edit_note_takes() {
        let (db, _dir) = open();
        let conn = db.connection();
        note_of(conn, "前置き\n## 10:00 定例\nA\n## 11:00 別件\nB\nC");
        let r = get_note(conn, CLI_ID, date(D), Some("11:00 別件")).unwrap();
        let NoteRead::Sections { sections, .. } = r else {
            panic!()
        };
        let s = &sections[0];
        // text の 3 行目（"C"）は、全体の start_line + 2 行目
        let line = s.start_line + 2;
        assert_eq!(s.text.split('\n').nth(2), Some("C"));
        grant(conn, CLI_ID, &[Scope::NoteEdit]);
        edit_note(
            conn,
            date(D),
            &edit(line, "C", LineOp::Replace("C!".into())),
            &cli(),
            &tokyo(),
            at(NOW),
        )
        .unwrap();
        assert_eq!(
            get(conn, TextDoc::Note, date(D)).unwrap().body,
            "前置き\n## 10:00 定例\nA\n## 11:00 別件\nB\nC!"
        );
    }

    #[test]
    fn get_note_heading_errors() {
        let (db, _dir) = open();
        let conn = db.connection();
        // 空・空白だけ・途中の改行は invalid_argument（ノートの有無に関係なく）
        for (h, expected) in [
            ("", InvalidHeading::Empty),
            (" \u{3000}\n", InvalidHeading::Empty),
            ("10:00\n定例", InvalidHeading::Multiline),
            ("10:00\r定例", InvalidHeading::Multiline),
        ] {
            assert!(
                matches!(
                    get_note(conn, CLI_ID, date(D), Some(h)),
                    Err(NoteReadError::InvalidArgument(e)) if e == expected
                ),
                "{h:?}"
            );
        }
        // まだ書いていない日は見出しが無いので not_found。day の行も作らない
        assert!(matches!(
            get_note(conn, CLI_ID, date(D), Some("10:00 定例")),
            Err(NoteReadError::NotFound)
        ));
        assert!(day_table::get(conn, date(D)).unwrap().is_none());
        // 見出しの無い前置きは見出しでは選べない
        note_of(conn, "前置き");
        assert!(matches!(
            get_note(conn, CLI_ID, date(D), Some("前置き")),
            Err(NoteReadError::NotFound)
        ));
    }
}
