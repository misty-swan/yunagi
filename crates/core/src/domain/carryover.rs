//! 持ち越し（データモデル設計書 3章 day・item「持ち越し」、画面設計書 5章 S-03）。
//!
//! 前回の日の未完了の TODO と未解消の障害を、今日へ持ち越す。持ち越しは前回の日の項目を動かさず、
//! 今日に新しい項目を作って `carried_from_id` で元を指す。元の項目の状態は変えない（見直しの
//! 「やめる」「分割する」「外部ツールへ戻す」を除く）。
//!
//! - [`previous_day`]：前回の日を求める。
//! - [`candidates`]：持ち越しの画面に出す項目（前回の日の、まだ持ち越し済みでない未完了・未解消の項目）。
//! - [`status`]：持ち越しの画面を出すか（後回し・1回だけの再表示を含む）。
//! - [`postpone`]・[`mark_reshown`]：後回しと再表示を記録する。
//! - [`confirm`]：持ち越しを確定する。
//!
//! どれも画面の操作で、人の操作なので change_log には記録しない。項目の欄の決まり（本文・並び順・
//! 持ち越し済みの項目への操作）は [`super::item`] にある。

use std::collections::{HashMap, HashSet};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use rusqlite::Connection;

use super::item::{self, CloseReason, InvalidArgument, Item, ItemKind, ItemStatus, carried_to};
use crate::domain::logical_date::LogicalDate;
use crate::domain::{day, sort_key};
use crate::storage::table::day::{self as day_table, CarryoverMark};
use crate::storage::table::item::{self as item_table, ItemChanges};
use crate::storage::table::setting;
use crate::storage::table::text_doc::{self, TextDoc};
use crate::storage::{StorageError, Ulid, in_transaction};

/// 設定のキー：持ち越しの見直しの回数（データモデル設計書 3章 setting）。
pub const REVIEW_THRESHOLD_KEY: &str = "carryover_review_threshold";

/// 持ち越しの見直しの回数の既定値。
pub const DEFAULT_REVIEW_THRESHOLD: i64 = 3;

/// 今日 `today` の「前回の日」：今日より前で、削除されていない項目か、空白以外の文字を含むノートが
/// ある直近の日（データモデル設計書 3章 item）。振り返りだけの日は含めない。無ければ `None`。
pub fn previous_day(
    conn: &Connection,
    today: LogicalDate,
) -> Result<Option<LogicalDate>, StorageError> {
    let with_items = item_table::latest_date_before(conn, today)?;
    // 項目のある日より後のノートだけを見ればよい
    let with_note = text_doc::latest_nonblank_date(conn, TextDoc::Note, today, with_items)?;
    Ok(with_note.or(with_items))
}

/// 持ち越しの見直しの回数（設定 `carryover_review_threshold`）。1 以上の整数でない値（未設定を含む）は
/// 既定値の 3 として扱う。
pub fn review_threshold(conn: &Connection) -> Result<i64, StorageError> {
    let value = setting::get(conn, REVIEW_THRESHOLD_KEY)?;
    Ok(value
        .and_then(|v| serde_json::from_str::<i64>(&v).ok())
        .filter(|&n| n >= 1)
        .unwrap_or(DEFAULT_REVIEW_THRESHOLD))
}

/// 持ち越しの画面に出す1件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// 前回の日の項目（未完了の TODO か、未解消の障害）。
    pub item: Item,
    /// 見直しの選択肢（分割する／やめる）を添えるか：持ち越し回数が見直しの回数以上の TODO。
    /// 「外部ツールへ戻す」は、さらに外部連携が有効で、外部課題と紐付いた（`external_ref_id` がある）
    /// TODO にだけ添える（外部連携は MVP の後）。
    pub review: bool,
}

/// 持ち越しの画面の中身。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidates {
    /// 前回の日。
    pub from: LogicalDate,
    /// 前回の日の並び順。
    pub items: Vec<Candidate>,
}

/// 持ち越しの画面に出すもの。前回の日が無ければ `None`。前回の日に出す項目が無ければ、
/// `items` は空。
pub fn candidates(
    conn: &Connection,
    today: LogicalDate,
) -> Result<Option<Candidates>, StorageError> {
    let Some(from) = previous_day(conn, today)? else {
        return Ok(None);
    };
    let threshold = review_threshold(conn)?;
    let items = open_items(conn, from)?
        .into_iter()
        .map(|item| Candidate {
            review: item.kind == ItemKind::Todo && item.carry_count >= threshold,
            item,
        })
        .collect();
    Ok(Some(Candidates { from, items }))
}

/// その日の、未完了の TODO と未解消の障害のうち、まだ持ち越し済みでないもの（並び順）。
fn open_items(conn: &Connection, date: LogicalDate) -> Result<Vec<Item>, StorageError> {
    let mut items = Vec::new();
    for item in item_table::list_by_date(conn, date)? {
        if is_pending(conn, &item)? {
            items.push(item);
        }
    }
    Ok(items)
}

/// その日に持ち越しの対象が1件でもあるか（見つかったら止める）。
fn has_pending(conn: &Connection, date: LogicalDate) -> Result<bool, StorageError> {
    for item in item_table::list_by_date(conn, date)? {
        if is_pending(conn, &item)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// 持ち越しの対象か：未完了・未解消で、まだ持ち越し済みでない。
fn is_pending(conn: &Connection, item: &Item) -> Result<bool, StorageError> {
    Ok(item.status == ItemStatus::Open && carried_to(conn, item.id)?.is_none())
}

/// 持ち越しの画面を、いつ出すか（画面設計書 5章）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prompt {
    /// まだ後回しにしていない：表示したときに出す。
    Show,
    /// 後回しにしたが、まだ再表示していない：次にアプリを前面に出したとき1回だけ出し、
    /// [`mark_reshown`] で記録する。
    ReshowOnFocus,
    /// 後回しにして、再表示も済んだ：自動では出さない。パレットの「持ち越しを開く」で開ける。
    OnRequest,
}

/// 持ち越しが残っているときの状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pending {
    /// 前回の日。
    pub from: LogicalDate,
    pub prompt: Prompt,
}

/// 今日の持ち越しの状態。持ち越しを終えた（`carryover_done_at` がある）か、前回の日に持ち越す項目が
/// 無ければ `None`（画面を出さず、パレットの「持ち越しを開く」も出さない）。
///
/// 状態は day の行に記録するので、アプリを再起動しても保たれる（データモデル設計書 3章 day）。
pub fn status(conn: &Connection, today: LogicalDate) -> Result<Option<Pending>, StorageError> {
    let row = day_table::get(conn, today)?;
    if row.as_ref().is_some_and(|d| d.carryover_done_at.is_some()) {
        return Ok(None);
    }
    let Some(from) = previous_day(conn, today)? else {
        return Ok(None);
    };
    // 1件あれば足りる（前面に出すたびに呼ばれるので、全件をたどらない）
    if !has_pending(conn, from)? {
        return Ok(None);
    }
    let prompt = match row {
        Some(d) if d.carryover_postponed_at.is_some() => {
            if d.carryover_reshown_at.is_some() {
                Prompt::OnRequest
            } else {
                Prompt::ReshowOnFocus
            }
        }
        _ => Prompt::Show,
    };
    Ok(Some(Pending { from, prompt }))
}

/// 持ち越しを後回しにした（Esc）ことを記録する。その日の day の行が無ければ作る。持ち越しを
/// 終えていれば何も書かない（古い画面からの後回し）。
/// すでに後回しにしていれば、最初の時刻のまま変えない（再表示やパレットから開いて、もう一度
/// 後回しにしても、自動の再表示は増えない）。
pub fn postpone(
    conn: &Connection,
    today: LogicalDate,
    tz: &TimeZone,
    now: Timestamp,
) -> Result<(), StorageError> {
    in_transaction(conn, || {
        day::ensure(conn, today, tz, now)?;
        if is_done(conn, today)? {
            return Ok(());
        }
        mark_once(conn, today, CarryoverMark::Postponed, now).map(|_| ())
    })
}

/// 後回しにした持ち越しを、前面に出したときに自動で再表示したことを記録する。記録したら `true`。
/// 後回しにしていないか、すでに再表示を記録したか、持ち越しを終えていれば、何も書かずに `false`
/// （自動の再表示は1回だけ）。
pub fn mark_reshown(
    conn: &Connection,
    today: LogicalDate,
    now: Timestamp,
) -> Result<bool, StorageError> {
    in_transaction(conn, || {
        // 後回しにしたときに day の行を作っているので、行が無ければ後回しにしていない
        match day_table::get(conn, today)? {
            Some(d)
                if d.carryover_done_at.is_none()
                    && d.carryover_postponed_at.is_some()
                    && d.carryover_reshown_at.is_none() =>
            {
                day_table::set_carryover(conn, today, CarryoverMark::Reshown, Some(now))
            }
            _ => Ok(false),
        }
    })
}

/// 今日の持ち越しを終えているか。
fn is_done(conn: &Connection, today: LogicalDate) -> Result<bool, StorageError> {
    Ok(day_table::get(conn, today)?.is_some_and(|d| d.carryover_done_at.is_some()))
}

/// 持ち越しの時刻 `mark` が未設定なら `now` を書く。書いたら `true`。day の行は呼び出し側が先に作る。
fn mark_once(
    conn: &Connection,
    today: LogicalDate,
    mark: CarryoverMark,
    now: Timestamp,
) -> Result<bool, StorageError> {
    let Some(row) = day_table::get(conn, today)? else {
        return Ok(false);
    };
    let current = match mark {
        CarryoverMark::Done => row.carryover_done_at,
        CarryoverMark::Postponed => row.carryover_postponed_at,
        CarryoverMark::Reshown => row.carryover_reshown_at,
    };
    if current.is_some() {
        return Ok(false);
    }
    day_table::set_carryover(conn, today, mark, Some(now))
}

/// 持ち越しの画面のボタン。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// 「すべて持ち越す」：確定の時点で持ち越しの対象のもの（画面を開いた後に加わったものも含む）を
    /// すべて持ち越す。
    All,
    /// 「選んだものを持ち越す」：チェックの付いた項目だけを持ち越す。
    Selected(HashSet<Ulid>),
    /// 「持ち越さない」。
    None,
}

/// 見直しの選択（持ち越し回数が見直しの回数以上の TODO に添える選択肢）。どのボタンで確定しても
/// 反映する（画面設計書 5章）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Review {
    /// やめる：持ち越さず、元の項目を `dropped`（`close_reason = dropped`）にする。
    Drop,
    /// 外部ツールへ戻す：持ち越さず、元の項目を `dropped`（`close_reason = returned_external`）にする。
    /// 外部の課題はそのまま。外部課題と紐付いた TODO にだけ選べる。
    ReturnExternal,
    /// 分割する：子の TODO（本文の並び）を今日に作り、元の項目を `dropped`（`close_reason = split`）に
    /// する。前後の空白を除いて空の子は作らない。子が1件も無ければ分割せず、見直しを選ばなかった
    /// ものとして扱う。
    Split(Vec<String>),
}

/// 持ち越しの確定の中身。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// 画面に出していた前回の日（[`Candidates::from`]）。
    pub from: LogicalDate,
    pub selection: Selection,
    /// 見直しを選んだ TODO と、その選択。
    pub reviews: HashMap<Ulid, Review>,
}

/// [`confirm`] の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirmed {
    /// 今日に作った項目（持ち越した項目と分割の子）。今日の並び順。
    pub created: Vec<Item>,
    /// 見直しで取りやめた、前回の日の項目（操作の後）。
    pub closed: Vec<Item>,
}

/// 持ち越しの確定が失敗した理由。どれも、何も書いていない（トランザクションは取り消してある）。
#[derive(Debug)]
pub enum CarryoverError {
    /// 画面に出していた日が、今の前回の日と違う（その間に、より新しい日に項目かノートができたなど）。
    /// 画面は持ち越しの一覧を読み直す。`previous_day` は今の前回の日。
    PreviousDayChanged { previous_day: Option<LogicalDate> },
    /// 今日の持ち越しはすでに終えている（`carryover_done_at` がある）。古い画面からの二度目の確定などで、
    /// 先の確定を覆さないよう断る。
    AlreadyDone,
    /// 見直しの選択が誤っている：障害への見直し（[`InvalidArgument::WrongKind`]）、外部課題と
    /// 紐付いていない TODO の「外部ツールへ戻す」（[`InvalidArgument::NotLinkedExternal`]）、
    /// 途中に改行のある子の本文（[`InvalidArgument::MultilineText`]）。
    InvalidArgument(InvalidArgument),
    /// 読み書きに失敗した。
    Storage(StorageError),
}

impl std::fmt::Display for CarryoverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PreviousDayChanged { .. } => {
                f.write_str("the previous day changed while the carry-over was shown")
            }
            Self::AlreadyDone => f.write_str("the carry-over for the day is already done"),
            Self::InvalidArgument(e) => e.fmt(f),
            Self::Storage(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for CarryoverError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidArgument(e) => Some(e),
            Self::Storage(e) => Some(e),
            Self::PreviousDayChanged { .. } | Self::AlreadyDone => None,
        }
    }
}

impl From<StorageError> for CarryoverError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

impl From<rusqlite::Error> for CarryoverError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Storage(e.into())
    }
}

impl From<InvalidArgument> for CarryoverError {
    fn from(e: InvalidArgument) -> Self {
        Self::InvalidArgument(e)
    }
}

/// 今日に作る項目のもと。
enum NewEntry<'a> {
    /// 持ち越す。
    Carry(&'a Item),
    /// 分割の子。
    Child { from: &'a Item, text: String },
}

/// 持ち越しを確定する（画面設計書 5章）。その日の day の行が無ければ作り、`carryover_done_at` を
/// 記録する。すでに終えていれば、何もせずに [`CarryoverError::AlreadyDone`]。
///
/// - 確定の時点で持ち越しの対象のもの（前回の日の、まだ持ち越し済みでない未完了・未解消の項目）だけを
///   扱う。画面を開いた後に完了・解消した項目や、持ち越し済みになった項目は、選んでいても、見直しを
///   選んでいても何もしない。本文などは確定の時点の値を写す。
/// - 持ち越す項目は、今日に新しく作る：`carried_from_id` は元の項目、`carry_count` は元の値＋1、
///   状態は未完了・未解消、`origin = human`。本文・種類・外部課題との紐付けは元の項目から写し、
///   解消メモは写さない。元の項目は変えない。
/// - 見直しを選んだ TODO は持ち越さず、元の項目を取りやめた TODO にする（[`Review`]）。分割の子は
///   未完了の TODO で、`split_from_id` が元の項目、`carry_count` は 0、`origin = human`。
/// - 作った項目（持ち越した項目と分割の子）は、今日の並びの先頭に、前回の日の順のまま入れる。
///   分割の子は元の項目の位置に、書いた順に入れる。
pub fn confirm(
    conn: &Connection,
    today: LogicalDate,
    decision: &Decision,
    tz: &TimeZone,
    now: Timestamp,
) -> Result<Confirmed, CarryoverError> {
    in_transaction(conn, || {
        if is_done(conn, today)? {
            return Err(CarryoverError::AlreadyDone);
        }
        let from = previous_day(conn, today)?;
        if from != Some(decision.from) {
            return Err(CarryoverError::PreviousDayChanged { previous_day: from });
        }
        let sources = open_items(conn, decision.from)?;

        // 今日に作るもの（前回の日の順）と、取りやめるもの
        let mut entries = Vec::new();
        let mut closing = Vec::new();
        for source in &sources {
            let review = decision
                .reviews
                .get(&source.id)
                .map(|r| check_review(source, r))
                .transpose()?
                .flatten();
            match review {
                Some(Checked::Close(reason)) => closing.push((source, reason)),
                Some(Checked::Split(children)) => {
                    closing.push((source, CloseReason::Split));
                    entries.extend(
                        children
                            .into_iter()
                            .map(|text| NewEntry::Child { from: source, text }),
                    );
                }
                None => {
                    let carry = match &decision.selection {
                        Selection::All => true,
                        Selection::Selected(ids) => ids.contains(&source.id),
                        Selection::None => false,
                    };
                    if carry {
                        entries.push(NewEntry::Carry(source));
                    }
                }
            }
        }

        day::ensure(conn, today, tz, now)?;
        let created = insert_at_start(conn, today, &entries, now)?;
        let mut closed = Vec::new();
        for (source, reason) in closing {
            let changes = ItemChanges {
                status: Some(ItemStatus::Dropped),
                close_reason: Some(Some(reason)),
                ..ItemChanges::default()
            };
            let updated = item_table::update_fields(conn, source.id, &changes, now)?;
            // 同じトランザクション（BEGIN IMMEDIATE）で、削除されていない行を読んだばかり
            debug_assert!(updated, "the source was read in the same transaction");
            closed.push(changes.applied_to(source, now));
        }
        mark_once(conn, today, CarryoverMark::Done, now)?;
        Ok(Confirmed { created, closed })
    })
}

/// 確かめた見直しの選択。
enum Checked {
    /// 取りやめる（やめる・外部ツールへ戻す）。
    Close(CloseReason),
    /// 分割する（整えた子の本文。1件以上）。
    Split(Vec<String>),
}

/// 見直しの選択を確かめる。子が1件も無い分割は `None`（見直しを選ばなかったものとして扱う）。
fn check_review(source: &Item, review: &Review) -> Result<Option<Checked>, InvalidArgument> {
    // 子の無い分割は、項目の種類によらず、見直しを選ばなかったものとして扱う
    if let Review::Split(texts) = review
        && texts.iter().all(|t| t.trim().is_empty())
    {
        return Ok(None);
    }
    if source.kind != ItemKind::Todo {
        // 障害には取りやめの状態が無い（データモデル設計書 3章 item）
        return Err(InvalidArgument::WrongKind {
            item_kind: source.kind,
            use_operation: None,
        });
    }
    Ok(match review {
        Review::Drop => Some(Checked::Close(CloseReason::Dropped)),
        Review::ReturnExternal => {
            if source.external_ref_id.is_none() {
                return Err(InvalidArgument::NotLinkedExternal);
            }
            Some(Checked::Close(CloseReason::ReturnedExternal))
        }
        Review::Split(texts) => {
            let mut children = Vec::new();
            for text in texts {
                match item::one_line_text(text) {
                    Ok(text) => children.push(text),
                    Err(InvalidArgument::EmptyText) => {}
                    Err(e) => return Err(e),
                }
            }
            (!children.is_empty()).then_some(Checked::Split(children))
        }
    })
}

/// `entries` の項目を、今日の並びの先頭に、この順で作る。
fn insert_at_start(
    conn: &Connection,
    today: LogicalDate,
    entries: &[NewEntry<'_>],
    now: Timestamp,
) -> Result<Vec<Item>, StorageError> {
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    let existing = item_table::list_by_date(conn, today)?;
    let keys = keys_before(existing.first().map(|i| i.sort_key.as_str()), entries.len());
    let keys = match keys {
        Some(keys) => keys,
        // 先頭のキーが形に当てはまらない：その日のキーを付け直して、先頭を空ける
        None => item::renumber(conn, &existing, 0, entries.len())?,
    };
    let mut created = Vec::with_capacity(entries.len());
    for (entry, sort_key) in entries.iter().zip(keys) {
        let item = match entry {
            NewEntry::Carry(source) => Item {
                id: Ulid::generate(),
                date: today,
                kind: source.kind,
                text: source.text.trim().to_owned(),
                status: ItemStatus::Open,
                close_reason: None,
                sort_key,
                carry_count: source.carry_count + 1,
                carried_from_id: Some(source.id),
                split_from_id: None,
                resolution_note: None,
                // 持ち越しは人の操作で、change_log にも記録しないので、エージェントが作った項目には
                // しない（作ったものは元の項目に残る）
                origin: item::Origin::Human,
                agent_client_id: None,
                external_ref_id: source.external_ref_id,
                completed_at: None,
                created_at: now,
                updated_at: now,
                deleted_at: None,
            },
            NewEntry::Child { from, text } => Item {
                id: Ulid::generate(),
                date: today,
                kind: ItemKind::Todo,
                text: text.clone(),
                status: ItemStatus::Open,
                close_reason: None,
                sort_key,
                carry_count: 0,
                carried_from_id: None,
                split_from_id: Some(from.id),
                resolution_note: None,
                origin: item::Origin::Human,
                agent_client_id: None,
                external_ref_id: None,
                completed_at: None,
                created_at: now,
                updated_at: now,
                deleted_at: None,
            },
        };
        item_table::insert(conn, &item)?;
        created.push(item);
    }
    Ok(created)
}

/// `first`（`None` なら空の日）より前に並ぶ `n` 個のキー（先頭から順に）。`first` が形に当てはまらず
/// 作れなければ `None`。
fn keys_before(first: Option<&str>, n: usize) -> Option<Vec<String>> {
    match first {
        None => Some(sort_key::spread(n)),
        Some(first) => sort_key::n_between(None, Some(first), n),
    }
}

#[cfg(test)]
mod tests {
    //! テスト計画書 4.4 のうち、コアで確かめられるもの（画面の表示は 2. 画面で確かめる）。

    use super::*;
    use crate::domain::agent::Agent;
    use crate::domain::agent::test_support::grant;
    use crate::domain::item::{
        Conflict, ItemError, Origin, Position, RestoreRefused, add, add_todo, complete_todo,
        delete, edit_blocker, edit_todo, reopen_todo, restore, save_resolution_note, save_text,
        set_closed,
    };
    use crate::domain::logical_date::DayBoundary;
    use crate::storage::Database;
    use crate::storage::table::agent_client::{CLI_ID, Scope};
    use crate::storage::table::test_support::{at, date};

    const T0: &str = "2026-11-03T01:00:00Z";
    const T1: &str = "2026-11-03T02:00:00Z";
    const T2: &str = "2026-11-03T03:00:00Z";

    fn tokyo() -> TimeZone {
        TimeZone::get("Asia/Tokyo").unwrap()
    }

    fn open() -> (Database, tempfile::TempDir) {
        let (db, dir) = crate::storage::table::test_support::open();
        grant(db.connection(), CLI_ID, &[Scope::Todo, Scope::Blocker]);
        (db, dir)
    }

    fn cli() -> Agent<'static> {
        Agent::without_ui(CLI_ID)
    }

    /// `d` の末尾に項目を足す（画面の操作）。
    fn put(conn: &Connection, d: &str, kind: ItemKind, text: &str) -> Item {
        add(conn, date(d), kind, text, Position::Last, &tokyo(), at(T0)).unwrap()
    }

    fn todo(conn: &Connection, d: &str, text: &str) -> Item {
        put(conn, d, ItemKind::Todo, text)
    }

    fn blocker(conn: &Connection, d: &str, text: &str) -> Item {
        put(conn, d, ItemKind::Blocker, text)
    }

    fn get(conn: &Connection, id: Ulid) -> Item {
        item_table::get(conn, id).unwrap().unwrap()
    }

    fn texts(conn: &Connection, d: &str) -> Vec<String> {
        item_table::list_by_date(conn, date(d))
            .unwrap()
            .into_iter()
            .map(|i| i.text)
            .collect()
    }

    fn candidate_texts(conn: &Connection, today: &str) -> Vec<String> {
        candidates(conn, date(today))
            .unwrap()
            .map(|c| c.items.into_iter().map(|c| c.item.text).collect())
            .unwrap_or_default()
    }

    fn decision(from: &str, selection: Selection) -> Decision {
        Decision {
            from: date(from),
            selection,
            reviews: HashMap::new(),
        }
    }

    fn confirm_at(conn: &Connection, today: &str, d: &Decision) -> Confirmed {
        confirm(conn, date(today), d, &tokyo(), at(T1)).unwrap()
    }

    /// `from` から `today` へ、すべて持ち越す。
    fn carry_all(conn: &Connection, from: &str, today: &str) -> Confirmed {
        confirm_at(conn, today, &decision(from, Selection::All))
    }

    fn write_note(conn: &Connection, d: &str, body: &str) {
        day::ensure(conn, date(d), &tokyo(), at(T0)).unwrap();
        text_doc::save(conn, TextDoc::Note, date(d), body, at(T0)).unwrap();
    }

    // ---- 前回の日 ----

    #[test]
    fn previous_day_is_the_latest_day_with_items_or_a_note() {
        let (db, _dir) = open();
        let conn = db.connection();
        let today = date("2026-11-03");
        assert_eq!(previous_day(conn, today).unwrap(), None);

        todo(conn, "2026-10-27", "古い日");
        todo(conn, "2026-10-28", "前回の日");
        assert_eq!(previous_day(conn, today).unwrap(), Some(date("2026-10-28")));

        // 削除した項目だけの日、空白だけのノートの日、振り返りだけの日、day の行だけの日は含めない
        let deleted = todo(conn, "2026-10-30", "削除した");
        delete(conn, deleted.id, at(T1)).unwrap();
        write_note(conn, "2026-10-31", " \n　");
        day::ensure(conn, date("2026-11-01"), &tokyo(), at(T0)).unwrap();
        text_doc::save(
            conn,
            TextDoc::Reflection,
            date("2026-11-01"),
            "振り返り",
            at(T0),
        )
        .unwrap();
        day::ensure(conn, date("2026-11-02"), &tokyo(), at(T0)).unwrap();
        assert_eq!(previous_day(conn, today).unwrap(), Some(date("2026-10-28")));

        // ノートだけの日は含める。今日と先の日は含めない
        write_note(conn, "2026-10-29", "メモ");
        todo(conn, "2026-11-03", "今日");
        todo(conn, "2026-11-04", "先の日");
        assert_eq!(previous_day(conn, today).unwrap(), Some(date("2026-10-29")));
    }

    #[test]
    fn days_apart_offers_only_the_previous_day() {
        // 数日空けて起動しても、前回の日の項目だけが出て、それより前の日の項目は出ない
        let (db, _dir) = open();
        let conn = db.connection();
        todo(conn, "2026-10-26", "もっと前");
        todo(conn, "2026-10-28", "前回の日の TODO");
        blocker(conn, "2026-10-28", "前回の日の障害");
        let c = candidates(conn, date("2026-11-03")).unwrap().unwrap();
        assert_eq!(c.from, date("2026-10-28"));
        assert_eq!(
            candidate_texts(conn, "2026-11-03"),
            ["前回の日の TODO", "前回の日の障害"]
        );

        // ノートだけの日が前回の日なら、それより前の日の項目は出ない
        write_note(conn, "2026-11-01", "メモ");
        let c = candidates(conn, date("2026-11-03")).unwrap().unwrap();
        assert_eq!(c.from, date("2026-11-01"));
        assert!(c.items.is_empty());
        assert_eq!(status(conn, date("2026-11-03")).unwrap(), None);
    }

    #[test]
    fn the_previous_day_follows_the_day_boundary() {
        // 切り替わり時刻（4:00）の前後で今日が変わり、前回の日も変わる
        let (db, _dir) = open();
        let conn = db.connection();
        todo(conn, "2026-11-02", "11/2 の TODO");
        let b = DayBoundary::DEFAULT;
        let before = LogicalDate::at(at("2026-11-02T18:59:00Z"), &tokyo(), b); // 11/3 3:59
        let after = LogicalDate::at(at("2026-11-02T19:01:00Z"), &tokyo(), b); // 11/3 4:01
        assert_eq!(before, date("2026-11-02"));
        assert_eq!(after, date("2026-11-03"));
        assert_eq!(status(conn, before).unwrap(), None);
        assert_eq!(
            status(conn, after).unwrap(),
            Some(Pending {
                from: date("2026-11-02"),
                prompt: Prompt::Show
            })
        );
    }

    // ---- 候補と見直し ----

    #[test]
    fn candidates_are_open_items_not_yet_carried_over() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = "2026-11-02";
        todo(conn, d, "未完了");
        let done = todo(conn, d, "完了");
        set_closed(conn, done.id, Some(at(T0)), at(T0)).unwrap();
        blocker(conn, d, "未解消");
        let resolved = blocker(conn, d, "解消");
        set_closed(conn, resolved.id, Some(at(T0)), at(T0)).unwrap();
        let carried = todo(conn, d, "持ち越し済み");
        confirm_at(
            conn,
            "2026-11-03",
            &decision(d, Selection::Selected(HashSet::from([carried.id]))),
        );
        // 今日に持ち越した項目ができたので、前回の日の一覧は open_items で直接見る
        assert_eq!(texts(conn, "2026-11-03"), ["持ち越し済み"]);
        let items: Vec<String> = open_items(conn, date(d))
            .unwrap()
            .into_iter()
            .map(|i| i.text)
            .collect();
        assert_eq!(items, ["未完了", "未解消"]);
    }

    #[test]
    fn review_is_offered_to_todos_carried_at_least_the_threshold() {
        let (db, _dir) = open();
        let conn = db.connection();
        // 3回持ち越した TODO と障害、2回の TODO を作る
        todo(conn, "2026-10-30", "TODO");
        blocker(conn, "2026-10-30", "障害");
        todo(conn, "2026-10-31", "2回");
        for (from, to) in [
            ("2026-10-30", "2026-10-31"),
            ("2026-10-31", "2026-11-01"),
            ("2026-11-01", "2026-11-02"),
        ] {
            carry_all(conn, from, to);
        }
        let reviews = |conn: &Connection| -> Vec<(String, i64, bool)> {
            candidates(conn, date("2026-11-03"))
                .unwrap()
                .unwrap()
                .items
                .into_iter()
                .map(|c| (c.item.text, c.item.carry_count, c.review))
                .collect()
        };
        assert_eq!(
            reviews(conn),
            [
                ("TODO".into(), 3, true),
                ("障害".into(), 3, false),
                ("2回".into(), 2, false),
            ]
        );
        // 設定で変えられる。1 以上の整数でない値は既定の 3
        setting::set(conn, REVIEW_THRESHOLD_KEY, "2").unwrap();
        assert_eq!(reviews(conn)[2], ("2回".into(), 2, true));
        assert!(!reviews(conn)[1].2);
        for bad in ["0", "-1", "\"2\"", "2.5", "null"] {
            setting::set(conn, REVIEW_THRESHOLD_KEY, bad).unwrap();
            assert_eq!(review_threshold(conn).unwrap(), 3, "{bad}");
        }
    }

    // ---- 確定 ----

    #[test]
    fn carry_all_creates_copies_at_the_top_in_the_previous_order() {
        let (db, _dir) = open();
        let conn = db.connection();
        let a = todo(conn, "2026-11-02", "A");
        let b = blocker(conn, "2026-11-02", "B");
        let c = add_todo(conn, &cli(), date("2026-11-02"), "C", &tokyo(), at(T0)).unwrap();
        // 解消メモが残っている（未解消に戻した）障害
        set_closed(conn, b.id, Some(at(T0)), at(T0)).unwrap();
        save_resolution_note(conn, b.id, "メモ", at(T0)).unwrap();
        set_closed(conn, b.id, None, at(T0)).unwrap();
        // 今日にすでに項目がある（エージェントが先に足したなど）
        todo(conn, "2026-11-03", "今日の TODO");
        blocker(conn, "2026-11-03", "今日の障害");
        let before: Vec<Item> = [a.id, b.id, c.id].map(|id| get(conn, id)).into();

        let done = carry_all(conn, "2026-11-02", "2026-11-03");
        assert_eq!(
            texts(conn, "2026-11-03"),
            ["A", "B", "C", "今日の TODO", "今日の障害"]
        );
        assert!(done.closed.is_empty());
        assert_eq!(done.created.len(), 3);
        for (copy, source) in done.created.iter().zip(&before) {
            assert_eq!(get(conn, copy.id), *copy);
            assert_eq!(copy.date, date("2026-11-03"));
            assert_eq!(copy.kind, source.kind);
            assert_eq!(copy.text, source.text);
            assert_eq!(copy.status, ItemStatus::Open);
            assert_eq!(copy.carried_from_id, Some(source.id));
            assert_eq!(copy.carry_count, source.carry_count + 1);
            assert_eq!(copy.split_from_id, None);
            assert_eq!(copy.resolution_note, None);
            assert_eq!(copy.origin, Origin::Human);
            assert_eq!(copy.agent_client_id, None);
            assert_eq!(copy.completed_at, None);
            assert!(crate::domain::sort_key::is_valid(&copy.sort_key));
            // 元の項目は変えない（過去日のボードは当時のまま）
            assert_eq!(get(conn, source.id), *source);
            assert_eq!(carried_to(conn, source.id).unwrap().unwrap().id, copy.id);
        }
        assert_eq!(get(conn, b.id).resolution_note.as_deref(), Some("メモ"));
        assert_eq!(c.origin, Origin::Agent);

        // 持ち越しを終えたので、もう出さない
        let day = day_table::get(conn, date("2026-11-03")).unwrap().unwrap();
        assert_eq!(day.carryover_done_at, Some(at(T1)));
        assert_eq!(status(conn, date("2026-11-03")).unwrap(), None);
    }

    #[test]
    fn selected_and_none_carry_only_what_was_chosen() {
        let (db, _dir) = open();
        let conn = db.connection();
        let a = todo(conn, "2026-11-02", "A");
        let b = todo(conn, "2026-11-02", "B");
        let c = blocker(conn, "2026-11-02", "C");
        // 前回の日の項目でない ID は無視する
        let other = todo(conn, "2026-11-01", "他の日");
        let chosen = HashSet::from([a.id, c.id, other.id]);
        let done = confirm_at(
            conn,
            "2026-11-03",
            &decision("2026-11-02", Selection::Selected(chosen)),
        );
        assert_eq!(texts(conn, "2026-11-03"), ["A", "C"]);
        assert_eq!(done.created.len(), 2);
        assert_eq!(carried_to(conn, b.id).unwrap(), None);
        assert_eq!(carried_to(conn, other.id).unwrap(), None);

        // 持ち越さない：何も作らず、持ち越しを終える。day の行も作る
        let (db, _dir) = open();
        let conn = db.connection();
        todo(conn, "2026-11-02", "A");
        let done = confirm_at(conn, "2026-11-03", &decision("2026-11-02", Selection::None));
        assert!(done.created.is_empty());
        assert!(texts(conn, "2026-11-03").is_empty());
        assert_eq!(status(conn, date("2026-11-03")).unwrap(), None);
        assert!(
            day_table::get(conn, date("2026-11-03"))
                .unwrap()
                .unwrap()
                .carryover_done_at
                .is_some()
        );
    }

    #[test]
    fn drop_and_split_close_the_source() {
        let (db, _dir) = open();
        let conn = db.connection();
        let keep = todo(conn, "2026-11-02", "持ち越す");
        let dropped = todo(conn, "2026-11-02", "やめる");
        let split = todo(conn, "2026-11-02", "分割する");
        let last = todo(conn, "2026-11-02", "最後");
        let d = Decision {
            from: date("2026-11-02"),
            // 選んでいなくても、見直しの選択は反映する
            selection: Selection::Selected(HashSet::from([keep.id, last.id, dropped.id])),
            reviews: HashMap::from([
                (dropped.id, Review::Drop),
                (
                    split.id,
                    Review::Split(vec![" 子1 ".into(), "".into(), "子2".into()]),
                ),
            ]),
        };
        let done = confirm_at(conn, "2026-11-03", &d);
        // 分割の子は元の項目の位置に入る
        assert_eq!(
            texts(conn, "2026-11-03"),
            ["持ち越す", "子1", "子2", "最後"]
        );
        let children: Vec<&Item> = done
            .created
            .iter()
            .filter(|i| i.split_from_id.is_some())
            .collect();
        assert_eq!(children.len(), 2);
        for child in children {
            assert_eq!(child.split_from_id, Some(split.id));
            assert_eq!(child.carry_count, 0);
            assert_eq!(child.carried_from_id, None);
            assert_eq!(child.kind, ItemKind::Todo);
            assert_eq!(child.status, ItemStatus::Open);
            assert_eq!(child.origin, Origin::Human);
        }
        let dropped_after = get(conn, dropped.id);
        assert_eq!(dropped_after.status, ItemStatus::Dropped);
        assert_eq!(dropped_after.close_reason, Some(CloseReason::Dropped));
        let split_after = get(conn, split.id);
        assert_eq!(split_after.status, ItemStatus::Dropped);
        assert_eq!(split_after.close_reason, Some(CloseReason::Split));
        assert_eq!(done.closed, [dropped_after.clone(), split_after]);
        // 取りやめた TODO は持ち越さず、本文も状態も変えられない
        assert_eq!(carried_to(conn, dropped.id).unwrap(), None);
        assert!(matches!(
            save_text(conn, dropped.id, "x", at(T2)),
            Err(ItemError::InvalidArgument(InvalidArgument::Dropped { .. }))
        ));
        assert!(matches!(
            set_closed(conn, dropped.id, Some(at(T2)), at(T2)),
            Err(ItemError::InvalidArgument(InvalidArgument::Dropped { .. }))
        ));
        assert!(matches!(
            complete_todo(conn, &cli(), dropped.id, at(T2)),
            Err(ItemError::InvalidArgument(InvalidArgument::Dropped { .. }))
        ));
        assert_eq!(get(conn, dropped.id), dropped_after);
        // 削除はできる
        delete(conn, dropped.id, at(T2)).unwrap();
    }

    #[test]
    fn a_split_without_children_is_not_a_split() {
        let (db, _dir) = open();
        let conn = db.connection();
        let t = todo(conn, "2026-11-02", "分割しない");
        // 障害への子の無い分割も、誤りにせず見直しを選ばなかったものとして扱う
        let b = blocker(conn, "2026-11-02", "障害");
        let d = Decision {
            from: date("2026-11-02"),
            selection: Selection::All,
            reviews: HashMap::from([
                (t.id, Review::Split(vec![" ".into(), "".into()])),
                (b.id, Review::Split(vec!["".into()])),
            ]),
        };
        let done = confirm_at(conn, "2026-11-03", &d);
        assert!(done.closed.is_empty());
        assert_eq!(get(conn, t.id).status, ItemStatus::Open);
        assert_eq!(done.created[0].carried_from_id, Some(t.id));
        assert_eq!(done.created[1].carried_from_id, Some(b.id));
    }

    #[test]
    fn a_second_confirm_is_refused() {
        let (db, _dir) = open();
        let conn = db.connection();
        let t = todo(conn, "2026-11-02", "A");
        confirm_at(conn, "2026-11-03", &decision("2026-11-02", Selection::None));
        // 古い画面から「すべて持ち越す」が届いても、先の「持ち越さない」を覆さない
        let mut d = decision("2026-11-02", Selection::All);
        d.reviews.insert(t.id, Review::Drop);
        assert!(matches!(
            confirm(conn, date("2026-11-03"), &d, &tokyo(), at(T2)),
            Err(CarryoverError::AlreadyDone)
        ));
        assert!(texts(conn, "2026-11-03").is_empty());
        assert_eq!(get(conn, t.id).status, ItemStatus::Open);
        assert_eq!(
            day_table::get(conn, date("2026-11-03"))
                .unwrap()
                .unwrap()
                .carryover_done_at,
            Some(at(T1))
        );
    }

    #[test]
    fn many_carried_items_get_short_keys_before_existing_ones() {
        let (db, _dir) = open();
        let conn = db.connection();
        for i in 0..40 {
            todo(conn, "2026-11-02", &format!("{i}"));
        }
        let today = todo(conn, "2026-11-03", "今日");
        let done = carry_all(conn, "2026-11-02", "2026-11-03");
        assert_eq!(done.created.len(), 40);
        let items = item_table::list_by_date(conn, date("2026-11-03")).unwrap();
        assert_eq!(items.last().unwrap().id, today.id);
        assert!(items.windows(2).all(|w| w[0].sort_key < w[1].sort_key));
        assert!(items.iter().all(|i| i.sort_key.len() <= 8), "{items:?}");
        // 今日の項目のキーは変えない
        assert_eq!(get(conn, today.id), today);
    }

    #[test]
    fn invalid_reviews_change_nothing() {
        let (db, _dir) = open();
        let conn = db.connection();
        let t = todo(conn, "2026-11-02", "TODO");
        let b = blocker(conn, "2026-11-02", "障害");
        for (id, review, expected) in [
            (
                b.id,
                Review::Drop,
                InvalidArgument::WrongKind {
                    item_kind: ItemKind::Blocker,
                    use_operation: None,
                },
            ),
            (
                t.id,
                Review::ReturnExternal,
                InvalidArgument::NotLinkedExternal,
            ),
            (
                t.id,
                Review::Split(vec!["a\nb".into()]),
                InvalidArgument::MultilineText,
            ),
        ] {
            let d = Decision {
                from: date("2026-11-02"),
                selection: Selection::All,
                reviews: HashMap::from([(id, review)]),
            };
            match confirm(conn, date("2026-11-03"), &d, &tokyo(), at(T1)) {
                Err(CarryoverError::InvalidArgument(e)) => assert_eq!(e, expected),
                other => panic!("{other:?}"),
            }
        }
        assert!(texts(conn, "2026-11-03").is_empty());
        assert_eq!(day_table::get(conn, date("2026-11-03")).unwrap(), None);
        assert_eq!(get(conn, t.id).status, ItemStatus::Open);
    }

    #[test]
    fn return_external_closes_a_linked_todo() {
        let (db, _dir) = open();
        let conn = db.connection();
        let t = todo(conn, "2026-11-02", "外部の課題");
        conn.execute(
            "UPDATE item SET external_ref_id = ?1 WHERE id = ?2",
            rusqlite::params![Ulid::generate(), t.id],
        )
        .unwrap();
        let linked = todo(conn, "2026-11-02", "持ち越す外部の課題");
        let ext = Ulid::generate();
        conn.execute(
            "UPDATE item SET external_ref_id = ?1 WHERE id = ?2",
            rusqlite::params![ext, linked.id],
        )
        .unwrap();
        let d = Decision {
            from: date("2026-11-02"),
            selection: Selection::All,
            reviews: HashMap::from([(t.id, Review::ReturnExternal)]),
        };
        let done = confirm_at(conn, "2026-11-03", &d);
        let after = get(conn, t.id);
        assert_eq!(after.status, ItemStatus::Dropped);
        assert_eq!(after.close_reason, Some(CloseReason::ReturnedExternal));
        // 持ち越した項目は外部課題との紐付けを保つ
        assert_eq!(done.created[0].external_ref_id, Some(ext));
    }

    #[test]
    fn confirm_uses_the_state_at_the_time_of_confirming() {
        // 持ち越しの画面を表示している間のエージェントの変更（テスト計画書 4.4 の最後の項目）
        let (db, _dir) = open();
        let conn = db.connection();
        let p = "2026-11-02";
        let completed = todo(conn, p, "完了にされる");
        let split_then_completed = todo(conn, p, "分割を選んだが完了にされる");
        let reopened = todo(conn, p, "未完了に戻される");
        set_closed(conn, reopened.id, Some(at(T0)), at(T0)).unwrap();
        let edited = todo(conn, p, "古い本文");
        let edited_blocker = blocker(conn, p, "古い障害");
        // 画面を開いた時点の一覧
        assert_eq!(
            candidate_texts(conn, "2026-11-03"),
            [
                "完了にされる",
                "分割を選んだが完了にされる",
                "古い本文",
                "古い障害"
            ]
        );

        complete_todo(conn, &cli(), completed.id, at(T0)).unwrap();
        complete_todo(conn, &cli(), split_then_completed.id, at(T0)).unwrap();
        reopen_todo(conn, &cli(), reopened.id, at(T0)).unwrap();
        edit_todo(conn, &cli(), edited.id, "新しい本文", at(T0)).unwrap();
        edit_blocker(conn, &cli(), edited_blocker.id, "新しい障害", at(T0)).unwrap();
        let added = add_todo(conn, &cli(), date(p), "足された", &tokyo(), at(T0)).unwrap();
        assert_eq!(
            candidate_texts(conn, "2026-11-03"),
            ["未完了に戻される", "新しい本文", "新しい障害", "足された"]
        );

        // 「選んだものを持ち越す」：加わった項目は、チェックを付けたときだけ
        let mut d = Decision {
            from: date(p),
            selection: Selection::Selected(HashSet::from([
                completed.id,
                edited.id,
                edited_blocker.id,
            ])),
            reviews: HashMap::from([(split_then_completed.id, Review::Split(vec!["子".into()]))]),
        };
        {
            let tx = conn.unchecked_transaction().unwrap();
            let done = confirm(&tx, date("2026-11-03"), &d, &tokyo(), at(T1)).unwrap();
            assert_eq!(
                done.created
                    .iter()
                    .map(|i| i.text.as_str())
                    .collect::<Vec<_>>(),
                ["新しい本文", "新しい障害"]
            );
            // 確定せずに手放す（「すべて持ち越す」を同じ状態から確かめるため）
        }

        // 「すべて持ち越す」：完了にされた項目は持ち越さず、分割もしない。加わった項目も持ち越す
        d.selection = Selection::All;
        let done = confirm_at(conn, "2026-11-03", &d);
        assert_eq!(
            texts(conn, "2026-11-03"),
            ["未完了に戻される", "新しい本文", "新しい障害", "足された"]
        );
        assert!(done.closed.is_empty());
        assert_eq!(get(conn, split_then_completed.id).status, ItemStatus::Done);
        assert_eq!(
            carried_to(conn, added.id).unwrap().unwrap().text,
            "足された"
        );
        assert_eq!(carried_to(conn, completed.id).unwrap(), None);
    }

    #[test]
    fn confirm_refuses_when_the_previous_day_changed() {
        let (db, _dir) = open();
        let conn = db.connection();
        todo(conn, "2026-10-30", "古い");
        let d = decision("2026-10-30", Selection::All);
        // 画面を出している間に、より新しい日に項目ができた
        add_todo(conn, &cli(), date("2026-11-01"), "新しい", &tokyo(), at(T0)).unwrap();
        match confirm(conn, date("2026-11-03"), &d, &tokyo(), at(T1)) {
            Err(CarryoverError::PreviousDayChanged { previous_day }) => {
                assert_eq!(previous_day, Some(date("2026-11-01")));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(day_table::get(conn, date("2026-11-03")).unwrap(), None);
    }

    // ---- 表示の判定と後回し ----

    #[test]
    fn postponing_reshows_once_and_then_only_on_request() {
        let (db, _dir) = open();
        let conn = db.connection();
        let today = date("2026-11-03");
        assert_eq!(status(conn, today).unwrap(), None);
        todo(conn, "2026-11-02", "A");
        let pending = |prompt| {
            Some(Pending {
                from: date("2026-11-02"),
                prompt,
            })
        };
        assert_eq!(status(conn, today).unwrap(), pending(Prompt::Show));
        // 後回しにする前の再表示は記録しない
        assert!(!mark_reshown(conn, today, at(T0)).unwrap());
        assert_eq!(day_table::get(conn, today).unwrap(), None);

        postpone(conn, today, &tokyo(), at(T0)).unwrap();
        assert_eq!(status(conn, today).unwrap(), pending(Prompt::ReshowOnFocus));
        assert!(mark_reshown(conn, today, at(T1)).unwrap());
        assert_eq!(status(conn, today).unwrap(), pending(Prompt::OnRequest));
        // 再表示は1回だけ。もう一度後回しにしても、最初の時刻のまま
        assert!(!mark_reshown(conn, today, at(T2)).unwrap());
        postpone(conn, today, &tokyo(), at(T2)).unwrap();
        let row = day_table::get(conn, today).unwrap().unwrap();
        assert_eq!(row.carryover_postponed_at, Some(at(T0)));
        assert_eq!(row.carryover_reshown_at, Some(at(T1)));
        assert_eq!(status(conn, today).unwrap(), pending(Prompt::OnRequest));

        // 持ち越しを終えると出さない。古い画面からの後回し・再表示も記録しない
        carry_all(conn, "2026-11-02", "2026-11-03");
        assert_eq!(status(conn, today).unwrap(), None);
        let (db2, _dir2) = open();
        let c2 = db2.connection();
        todo(c2, "2026-11-02", "A");
        confirm_at(c2, "2026-11-03", &decision("2026-11-02", Selection::None));
        postpone(c2, today, &tokyo(), at(T2)).unwrap();
        assert!(!mark_reshown(c2, today, at(T2)).unwrap());
        let row = day_table::get(c2, today).unwrap().unwrap();
        assert_eq!(row.carryover_postponed_at, None);
        assert_eq!(row.carryover_reshown_at, None);
        // 次の日は、今日を前回の日として改めて出す
        assert_eq!(
            status(conn, date("2026-11-04")).unwrap(),
            Some(Pending {
                from: today,
                prompt: Prompt::Show
            })
        );
    }

    // ---- 持ち越し済みの項目 ----

    #[test]
    fn carried_sources_are_locked_until_the_copy_is_deleted() {
        let (db, _dir) = open();
        let conn = db.connection();
        let source = todo(conn, "2026-11-02", "元");
        let copy = carry_all(conn, "2026-11-02", "2026-11-03")
            .created
            .remove(0);
        let carried = Conflict::CarriedOver {
            carried_to_id: copy.id,
            carried_to_date: date("2026-11-03"),
        };
        assert!(
            matches!(save_text(conn, source.id, "x", at(T2)), Err(ItemError::Conflict(c)) if c == carried)
        );
        assert!(
            matches!(set_closed(conn, source.id, Some(at(T2)), at(T2)), Err(ItemError::Conflict(c)) if c == carried)
        );
        assert!(
            matches!(complete_todo(conn, &cli(), source.id, at(T2)), Err(ItemError::Conflict(c)) if c == carried)
        );
        assert_eq!(get(conn, source.id), source);

        // 持ち越し先を削除すると、元の項目を操作できる
        let deleted = delete(conn, copy.id, at(T2)).unwrap();
        assert_eq!(deleted.source.as_ref().unwrap().id, source.id);
        assert!(
            save_text(conn, source.id, "元を直す", at(T2))
                .unwrap()
                .updated
        );
        // 元の項目を変えたので、削除を戻せない
        assert_eq!(
            restore(conn, &deleted, at(T2)).unwrap_err().to_string(),
            "the item it was carried over from was changed after the delete"
        );
        assert!(get(conn, copy.id).deleted_at.is_some());
    }

    #[test]
    fn restoring_a_deleted_copy_works_when_the_source_is_unchanged() {
        let (db, _dir) = open();
        let conn = db.connection();
        let source = todo(conn, "2026-11-02", "元");
        let copy = carry_all(conn, "2026-11-02", "2026-11-03")
            .created
            .remove(0);
        let deleted = delete(conn, copy.id, at(T2)).unwrap();
        // 並べ替えは変更に数えない
        crate::domain::item::move_item(conn, source.id, Position::First, at(T2)).unwrap();
        let restored = restore(conn, &deleted, at(T2)).unwrap();
        assert_eq!(restored.deleted_at, None);
        assert_eq!(carried_to(conn, source.id).unwrap().unwrap().id, copy.id);
    }

    #[test]
    fn a_chain_stays_carried_over_when_the_middle_is_deleted() {
        // A→B→C と持ち越して B を削除しても、A は C へ持ち越し済みのまま
        let (db, _dir) = open();
        let conn = db.connection();
        let a = todo(conn, "2026-11-01", "A");
        let b = carry_all(conn, "2026-11-01", "2026-11-02")
            .created
            .remove(0);
        let c = carry_all(conn, "2026-11-02", "2026-11-03")
            .created
            .remove(0);
        let deleted_b = delete(conn, b.id, at(T2)).unwrap();
        assert_eq!(carried_to(conn, a.id).unwrap().unwrap().id, c.id);
        assert!(matches!(
            save_text(conn, a.id, "x", at(T2)),
            Err(ItemError::Conflict(Conflict::CarriedOver { carried_to_id, .. })) if carried_to_id == c.id
        ));
        // B は戻せる（A はほかへ持ち越されていない）
        restore(conn, &deleted_b, at(T2)).unwrap();
        assert_eq!(carried_to(conn, a.id).unwrap().unwrap().id, b.id);
    }

    #[test]
    fn a_source_offered_again_cannot_have_its_old_copy_restored() {
        // 持ち越し先を削除してその日に何も無くなると、次の日に元の項目がもう一度出る。持ち越した後は、
        // 前の持ち越し先の削除を戻せない
        let (db, _dir) = open();
        let conn = db.connection();
        let source = todo(conn, "2026-11-02", "元");
        let copy = carry_all(conn, "2026-11-02", "2026-11-03")
            .created
            .remove(0);
        let deleted = delete(conn, copy.id, at(T2)).unwrap();
        assert_eq!(candidate_texts(conn, "2026-11-04"), ["元"]);
        let again = carry_all(conn, "2026-11-02", "2026-11-04")
            .created
            .remove(0);
        assert_eq!(again.carry_count, 1);
        match restore(conn, &deleted, at(T2)) {
            Err(ItemError::RestoreRefused(RestoreRefused::SourceCarriedOver {
                carried_to_id,
                carried_to_date,
            })) => {
                assert_eq!(carried_to_id, again.id);
                assert_eq!(carried_to_date, date("2026-11-04"));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(carried_to(conn, source.id).unwrap().unwrap().id, again.id);
    }
}
