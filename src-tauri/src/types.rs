//! 画面（TypeScript）と受け渡す型（アーキテクチャ設計書 2章「Tauri コマンド」）。
//!
//! コアの型をそのまま渡さず、ここで画面向けの形（名前は camelCase、ID・日付は文字列、時刻は
//! UNIX ミリ秒の数）にする。コアの型は CLI・MCP とも共有するので、画面の都合を持ち込まないため。
//! TypeScript の定義は ts-rs でここから作り、`src/lib/bindings.ts` に置く（`crate::bindings`）。
//! 作るのはテストだけなので、ts-rs の derive はテストのビルドにだけ付ける（配るアプリに含めない）。

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use jiff::Timestamp;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
#[cfg(test)]
use ts_rs::TS;
use yunagi_core::domain::agent::ItemField as CoreItemField;
use yunagi_core::domain::archive;
use yunagi_core::domain::board;
use yunagi_core::domain::carryover;
use yunagi_core::domain::item::{self as core_item, CloseReason as CoreCloseReason};
use yunagi_core::domain::logical_date::{DayBoundary, LogicalDate};
use yunagi_core::domain::note_body;
use yunagi_core::domain::note_send;
use yunagi_core::domain::settings;
use yunagi_core::domain::text_doc::{self, TextDoc as CoreTextDoc};
use yunagi_core::storage::{Ulid, cloud_sync, search};

// ---- 値 ----

/// 文字列で受け渡す値（Display で書き、FromStr で読む）の Serialize・Deserialize。
macro_rules! string_value {
    ($name:ident, $inner:ty, $expected:literal) => {
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                <$inner>::from_str(&s).map(Self).map_err(|_| {
                    serde::de::Error::invalid_value(serde::de::Unexpected::Str(&s), &$expected)
                })
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

/// 項目などの ID（ULID の 26 文字）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(TS))]
pub struct Id(#[cfg_attr(test, ts(type = "string"))] pub Ulid);
string_value!(Id, Ulid, "a ULID");

/// 論理日付（`YYYY-MM-DD`）。TypeScript では、組み込みの `Date` と分けて `LogicalDate` と呼ぶ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(TS))]
#[cfg_attr(test, ts(rename = "LogicalDate"))]
pub struct Date(#[cfg_attr(test, ts(type = "string"))] pub LogicalDate);
string_value!(Date, LogicalDate, "a date (YYYY-MM-DD)");

/// 時刻：UNIX 時刻のミリ秒（JavaScript の `Date.now()` と同じ）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(TS))]
pub struct Millis(#[cfg_attr(test, ts(type = "number"))] pub Timestamp);

impl Serialize for Millis {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_i64(self.0.as_millisecond())
    }
}

impl<'de> Deserialize<'de> for Millis {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let ms = i64::deserialize(d)?;
        Timestamp::from_millisecond(ms).map(Self).map_err(|_| {
            serde::de::Error::invalid_value(
                serde::de::Unexpected::Signed(ms),
                &"a time in milliseconds",
            )
        })
    }
}

/// コアの列挙型と、同じ値を持つ画面向けの列挙型を、相互に変換する。
macro_rules! mirror_enum {
    ($(#[$meta:meta])* $name:ident <=> $core:ty { $($variant:ident),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[cfg_attr(test, derive(TS))]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($variant),+
        }

        impl From<$core> for $name {
            fn from(v: $core) -> Self {
                match v {
                    $(<$core>::$variant => Self::$variant),+
                }
            }
        }

        impl From<$name> for $core {
            fn from(v: $name) -> Self {
                match v {
                    $($name::$variant => Self::$variant),+
                }
            }
        }
    };
}

mirror_enum! {
    /// 項目の種類。
    ItemKind <=> core_item::ItemKind { Todo, Blocker }
}

mirror_enum! {
    /// 項目の状態。TODO は `open` / `done` / `dropped`、障害は `open` / `resolved`。
    ItemStatus <=> core_item::ItemStatus { Open, Done, Dropped, Resolved }
}

mirror_enum! {
    /// 取りやめた（`dropped`）理由。
    CloseReason <=> CoreCloseReason { Dropped, Split, ReturnedExternal }
}

mirror_enum! {
    /// 項目を作ったもの。
    Origin <=> core_item::Origin { Human, Agent, Note, External }
}

mirror_enum! {
    /// 項目の欄のうち、利用者が入力するもの（アーキテクチャ設計書 4.1「項目の入力中」）。
    ItemField <=> CoreItemField { Text, ResolutionNote }
}

mirror_enum! {
    /// ノートか振り返りか。
    TextDoc <=> CoreTextDoc { Note, Reflection }
}

// ---- 項目 ----

/// TODO か障害の1件（データモデル設計書 3章 item）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: Id,
    pub date: Date,
    pub kind: ItemKind,
    pub text: String,
    pub status: ItemStatus,
    pub close_reason: Option<CloseReason>,
    /// 並び順のキー。文字列の大小で並ぶ。
    pub sort_key: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub carry_count: i64,
    pub carried_from_id: Option<Id>,
    pub split_from_id: Option<Id>,
    pub resolution_note: Option<String>,
    pub origin: Origin,
    pub agent_client_id: Option<Id>,
    pub external_ref_id: Option<Id>,
    /// 完了時刻（障害では解消時刻）。
    pub completed_at: Option<Millis>,
    pub created_at: Millis,
    pub updated_at: Millis,
    pub deleted_at: Option<Millis>,
}

impl From<core_item::Item> for Item {
    fn from(i: core_item::Item) -> Self {
        Self {
            id: Id(i.id),
            date: Date(i.date),
            kind: i.kind.into(),
            text: i.text,
            status: i.status.into(),
            close_reason: i.close_reason.map(Into::into),
            sort_key: i.sort_key,
            carry_count: i.carry_count,
            carried_from_id: i.carried_from_id.map(Id),
            split_from_id: i.split_from_id.map(Id),
            resolution_note: i.resolution_note,
            origin: i.origin.into(),
            agent_client_id: i.agent_client_id.map(Id),
            external_ref_id: i.external_ref_id.map(Id),
            completed_at: i.completed_at.map(Millis),
            created_at: Millis(i.created_at),
            updated_at: Millis(i.updated_at),
            deleted_at: i.deleted_at.map(Millis),
        }
    }
}

/// 項目を入れる・動かす位置（同じ日の並びの中）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(tag = "at", rename_all = "snake_case")]
pub enum Position {
    /// その日の先頭。
    First,
    /// その日の末尾。
    Last,
    /// 項目 `id` のすぐ前。
    Before { id: Id },
    /// 項目 `id` のすぐ後。
    After { id: Id },
}

impl From<Position> for core_item::Position {
    fn from(p: Position) -> Self {
        match p {
            Position::First => Self::First,
            Position::Last => Self::Last,
            Position::Before { id } => Self::Before(id.0),
            Position::After { id } => Self::After(id.0),
        }
    }
}

/// 画面からの項目の保存：利用者が変えた欄だけ（アーキテクチャ設計書 4.1「項目の保存」）。
/// 無い（`undefined`・`null`）欄には触れない。
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ItemFields {
    /// 本文。
    #[cfg_attr(test, ts(optional))]
    pub text: Option<String>,
    /// 障害の解消メモ（空なら消す）。
    #[cfg_attr(test, ts(optional))]
    pub resolution_note: Option<String>,
    /// 状態（完了・解消の切り替え）。
    #[cfg_attr(test, ts(optional))]
    pub status: Option<StatusChange>,
}

/// 項目の状態の保存（アーキテクチャ設計書 4.1「利用者とエージェントが項目を同じ状態にした行き違い」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StatusChange {
    /// 変える前の値：利用者がこの保存で最初に状態を変えたときに、画面に出ていた状態。
    pub before: ItemStatus,
    /// 数なら完了・解消にしてその時刻を完了時刻に、`null` なら未完了・未解消に戻す（完了時刻は
    /// 画面が決める。操作仕様書 2章）。
    pub closed_at: Option<Millis>,
    /// 利用者がこの保存で最初に状態を変えた操作の時刻。
    pub changed_at: Millis,
}

impl From<ItemFields> for core_item::ItemSave {
    fn from(f: ItemFields) -> Self {
        Self {
            text: f.text,
            resolution_note: f.resolution_note,
            status: f.status.map(|s| core_item::StatusSave {
                before: s.before.into(),
                closed_at: s.closed_at.map(|m| m.0),
                changed_at: s.changed_at.0,
            }),
        }
    }
}

/// 項目の書き込みの結果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ItemOutcome {
    /// 実際に何かを変えたか。`false` なら何も書いていない。
    pub updated: bool,
    /// 操作の後の項目。
    pub item: Item,
}

impl From<core_item::Outcome> for ItemOutcome {
    fn from(o: core_item::Outcome) -> Self {
        Self {
            updated: o.updated,
            item: o.item.into(),
        }
    }
}

/// 項目を2つに分けた結果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SplitResult {
    /// 操作の後の元の項目。
    pub original: Item,
    /// 新しく作った項目（元の項目のすぐ後）。
    pub created: Item,
}

/// 削除した項目。削除の取り消し（Mod+Z）のために画面が持ち、そのまま `restore_item` に渡す。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Deleted {
    /// 削除した項目の ID。
    pub id: Id,
    /// 持ち越しで作った項目なら、削除の時点の持ち越し元の欄（変わっていたら戻せない）。
    pub source: Option<SourceState>,
}

/// 持ち越し先を削除した時点の、持ち越し元の項目の欄（`restore_item` が変わっていないかを確かめる）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SourceState {
    pub id: Id,
    pub status: ItemStatus,
    pub text: String,
    pub resolution_note: Option<String>,
    pub completed_at: Option<Millis>,
}

impl From<core_item::Deleted> for Deleted {
    fn from(d: core_item::Deleted) -> Self {
        Self {
            id: Id(d.item.id),
            source: d.source.map(|s| SourceState {
                id: Id(s.id),
                status: s.status.into(),
                text: s.text,
                resolution_note: s.resolution_note,
                completed_at: s.completed_at.map(Millis),
            }),
        }
    }
}

impl SourceState {
    /// コアの [`core_item::SourceState`]（[`core_item::restore`] に渡す）。
    pub fn into_core(self) -> core_item::SourceState {
        core_item::SourceState {
            id: self.id.0,
            status: self.status.into(),
            text: self.text,
            resolution_note: self.resolution_note,
            completed_at: self.completed_at.map(|m| m.0),
        }
    }
}

// ---- ボード ----

/// 持ち越し先：持ち越して作った項目と、その日付。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CarriedTo {
    pub id: Id,
    pub date: Date,
}

/// ボードの項目の1件：項目と、持ち越し先（持ち越し済みでなければ `null`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BoardItem {
    pub item: Item,
    pub carried_to: Option<CarriedTo>,
    /// エージェントが作った項目なら、作ったエージェントの識別子（`claude-code` など。agent_client の
    /// `name`）。画面は本文の後ろに名前を添える（画面設計書 3章「各領域の表示」）。
    pub agent_name: Option<String>,
    /// エージェントによる未確認の変更がある（行頭の菱形）。
    pub unseen: bool,
    /// ノートから送った項目なら、送り元の行（持ち越し・分割の元へさかのぼる）。画面は行末の
    /// 「← NOTE」からその行へ移る（画面設計書 3章「各領域の表示」）。
    pub note_source: Option<NoteSource>,
}

/// 項目の送り元の行（データモデル設計書 3章 note_send）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NoteSource {
    /// 送り元のノートの日。
    pub date: Date,
    /// 保存済みの本文での、送り元の行（1始まり）。
    pub line: usize,
    /// 送った記録の送り先の項目（持ち越し・分割で作った項目では、元の項目）。画面は、エディタの
    /// 本文の送信済みの印（`note_marks`）からこの項目の行を探し、無ければ `line` を使う。
    pub item_id: Id,
}

impl From<note_send::SourceRef> for NoteSource {
    fn from(s: note_send::SourceRef) -> Self {
        Self {
            date: Date(s.date),
            line: s.line,
            item_id: Id(s.item_id),
        }
    }
}

/// ノートの時刻付き見出し（`## 10:00 定例`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TimeHeading {
    /// 見出しの行の番号（1始まり）。
    pub line: usize,
    pub hour: u8,
    pub minute: u8,
    /// 件名（無ければ空）。
    pub title: String,
    /// `## ` の後ろの文字（例：`10:00 定例`）。
    pub text: String,
}

impl From<note_body::TimeHeading> for TimeHeading {
    fn from(h: note_body::TimeHeading) -> Self {
        Self {
            line: h.line,
            hour: h.hour,
            minute: h.minute,
            title: h.title,
            text: h.text,
        }
    }
}

/// ボードのノート：見出しと長さだけ（本文は `get_text_doc` で読む）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NoteOutline {
    pub headings: Vec<TimeHeading>,
    /// 本文の文字数。まだ書いていない日は 0。
    pub length: usize,
}

/// その日のボード。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Board {
    pub date: Date,
    /// TODO（並び順）。取りやめた TODO も入る。
    pub todos: Vec<BoardItem>,
    /// 障害（並び順）。
    pub blockers: Vec<BoardItem>,
    /// 振り返りの本文。まだ書いていない日は空。
    pub reflection: String,
    pub note: NoteOutline,
    /// 振り返りの版（`get_text_doc` の `rev`。まだ書いていない日は 0）。画面は、これが進んだときだけ
    /// 振り返りの本文を読み直して取り込む。
    #[cfg_attr(test, ts(type = "number"))]
    pub reflection_rev: i64,
    /// ノートの版（同上）。
    #[cfg_attr(test, ts(type = "number"))]
    pub note_rev: i64,
}

impl From<board::ScreenBoard> for Board {
    fn from(sb: board::ScreenBoard) -> Self {
        let board::ScreenBoard {
            board: b,
            mut agent_names,
            unseen,
            revs,
            note_sources,
        } = sb;
        let mut items = |list: Vec<board::BoardItem>| {
            list.into_iter()
                .map(|bi| BoardItem {
                    agent_name: agent_names.remove(&bi.item.id),
                    unseen: unseen.contains(&bi.item.id),
                    note_source: note_sources.get(&bi.item.id).copied().map(Into::into),
                    item: bi.item.into(),
                    carried_to: bi.carried_to.map(|c| CarriedTo {
                        id: Id(c.id),
                        date: Date(c.date),
                    }),
                })
                .collect()
        };
        Self {
            date: Date(b.date),
            todos: items(b.todos),
            blockers: items(b.blockers),
            reflection: b.reflection,
            note: NoteOutline {
                headings: b.note.headings.into_iter().map(Into::into).collect(),
                length: b.note.length,
            },
            reflection_rev: revs.reflection,
            note_rev: revs.note,
        }
    }
}

/// 件数（データモデル設計書 3章 item「件数の数え方」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    /// TODO 数（取りやめた TODO を除く）。
    #[cfg_attr(test, ts(type = "number"))]
    pub todos: u64,
    /// 完了した TODO。
    #[cfg_attr(test, ts(type = "number"))]
    pub done: u64,
    /// 残りのうち、未完了の TODO。
    #[cfg_attr(test, ts(type = "number"))]
    pub left_todos: u64,
    /// 残りのうち、未解消の障害。
    #[cfg_attr(test, ts(type = "number"))]
    pub left_blockers: u64,
    /// 解消した障害。
    #[cfg_attr(test, ts(type = "number"))]
    pub resolved: u64,
    /// 前の日から持ち越してきた項目。
    #[cfg_attr(test, ts(type = "number"))]
    pub carried: u64,
}

impl From<board::Counts> for Counts {
    fn from(c: board::Counts) -> Self {
        Self {
            todos: c.todos,
            done: c.done,
            left_todos: c.left_todos,
            left_blockers: c.left_blockers,
            resolved: c.resolved,
            carried: c.carried,
        }
    }
}

/// サイドバーの Recent の1日。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RecentDay {
    pub date: Date,
    pub counts: Counts,
}

/// 画面の骨組み（サイドバー・上部のバー・凪メーター）に出す件数（画面設計書 3章「画面の骨組み」）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    /// 表示している日の件数（凪メーター）。
    pub counts: Counts,
    /// サイドバーの Recent（先頭はいつも今日の論理日付。新しい順に最大7日）。
    pub recent: Vec<RecentDay>,
    /// エージェントによる未確認の変更の数。
    #[cfg_attr(test, ts(type = "number"))]
    pub unseen_changes: u64,
}

impl Overview {
    pub fn new(o: board::Overview) -> Self {
        Self {
            counts: o.counts.into(),
            recent: o
                .recent
                .into_iter()
                .map(|(date, counts)| RecentDay {
                    date: Date(date),
                    counts: counts.into(),
                })
                .collect(),
            unseen_changes: o.unseen_changes,
        }
    }
}

// ---- ふりかえり（S-08） ----

/// 週のカードの1日：日付とその日の件数。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DayCounts {
    pub date: Date,
    pub counts: Counts,
}

/// Daily notes の1日：日付、件数、振り返りの本文（GOOD / STUCK の1行は画面が取り出す）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DailyEntry {
    pub date: Date,
    pub counts: Counts,
    pub reflection: String,
}

/// プレビューのノートの1区間：時刻付き見出しと、その下の最初の行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NotePreview {
    #[cfg_attr(test, ts(type = "number"))]
    pub hour: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub minute: u8,
    pub title: String,
    pub lines: Vec<String>,
}

/// 選んだ日のプレビュー。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DayPreview {
    pub counts: Counts,
    pub reflection: String,
    pub note: Vec<NotePreview>,
}

/// ふりかえり画面の内容（画面設計書 9章。コアの `domain::archive`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Archive {
    /// 選んだ日。
    pub date: Date,
    /// 今日の論理日付。
    pub today: Date,
    /// 選んだ日を含む週（月〜金）の日ごとの件数。
    pub week: Vec<DayCounts>,
    /// 週の合計。
    pub week_total: Counts,
    /// 記録のある直近の日（新しい順）。
    pub daily: Vec<DailyEntry>,
    pub preview: DayPreview,
}

impl From<archive::Archive> for Archive {
    fn from(a: archive::Archive) -> Self {
        Self {
            date: Date(a.date),
            today: Date(a.today),
            week: a
                .week
                .days
                .into_iter()
                .map(|(date, counts)| DayCounts {
                    date: Date(date),
                    counts: counts.into(),
                })
                .collect(),
            week_total: a.week.total.into(),
            daily: a
                .daily
                .into_iter()
                .map(|d| DailyEntry {
                    date: Date(d.date),
                    counts: d.counts.into(),
                    reflection: d.reflection,
                })
                .collect(),
            preview: DayPreview {
                counts: a.preview.counts.into(),
                reflection: a.preview.reflection,
                note: a
                    .preview
                    .note
                    .into_iter()
                    .map(|n| NotePreview {
                        hour: n.hour,
                        minute: n.minute,
                        title: n.title,
                        lines: n.lines,
                    })
                    .collect(),
            },
        }
    }
}

// ---- ノート・振り返り ----

/// ある時点の本文と版の番号（まだ書いていない日は空の本文と版 0）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub body: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub rev: i64,
}

impl From<text_doc::Snapshot> for Snapshot {
    fn from(s: text_doc::Snapshot) -> Self {
        Self {
            body: s.body,
            rev: s.rev,
        }
    }
}

/// 画面からのノート・振り返りの保存の結果（アーキテクチャ設計書 4.1「保存の行き違いを防ぐ」）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum TextSave {
    /// 保存した（本文が同じなら書かずに `updated: false`）。`rev` は保存の後の版。
    Saved {
        #[cfg_attr(test, ts(type = "number"))]
        rev: i64,
        updated: bool,
    },
    /// もとにした版が古いので書かなかった。いまの本文と版。画面は取り込んでから保存し直す。
    Stale {
        body: String,
        #[cfg_attr(test, ts(type = "number"))]
        rev: i64,
    },
}

// ---- 行送り ----

/// 送信済みの印を付けるノートの行（`note_marks`。同じ行に2つ付くこともある）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NoteMark {
    /// 行の番号（1始まり。渡した本文で数える）。
    pub line: usize,
    /// 送り先の種類（「→ TODO」「→ BLOCKER」）。
    pub kind: ItemKind,
    /// 送り先の項目。
    pub item_id: Id,
}

impl From<note_send::Mark> for NoteMark {
    fn from(m: note_send::Mark) -> Self {
        Self {
            line: m.line,
            kind: m.kind.into(),
            item_id: Id(m.item_id),
        }
    }
}

/// 送らなかった行の理由（データモデル設計書 3章 note_send「送らない行」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// 見出しの行。
    Heading,
    /// リストの印などを除くと本文が無い（空の行・区切り線・コードの囲みの行など）。
    NoText,
    /// コードの囲みの中の行。
    InCode,
    /// すでに送信済みの印が付いている。
    AlreadySent,
}

impl From<&note_send::SkipReason> for SkipReason {
    fn from(r: &note_send::SkipReason) -> Self {
        match r {
            note_send::SkipReason::Heading => Self::Heading,
            note_send::SkipReason::NoText => Self::NoText,
            note_send::SkipReason::InCode => Self::InCode,
            note_send::SkipReason::AlreadySent { .. } => Self::AlreadySent,
        }
    }
}

/// 送った1行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SentLine {
    pub line: usize,
    /// 作った項目。
    pub item: Item,
}

/// 送らなかった1行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SkippedLine {
    pub line: usize,
    pub reason: SkipReason,
}

/// 行送り（`send_note_lines`）の結果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum SendResult {
    /// 本文を保存し、送れる行を送った（1行も送らなくても本文は保存する）。`rev` は保存の後の版。
    Sent {
        #[cfg_attr(test, ts(type = "number"))]
        rev: i64,
        /// 送った行（上から順）。
        sent: Vec<SentLine>,
        /// 送らなかった行（上から順）。
        skipped: Vec<SkippedLine>,
    },
    /// もとにした版が古いので、何も書かなかった。いまの本文と版。画面は取り込んでから送り直す。
    Stale {
        body: String,
        #[cfg_attr(test, ts(type = "number"))]
        rev: i64,
    },
}

impl From<note_send::Sent> for SendResult {
    fn from(s: note_send::Sent) -> Self {
        Self::Sent {
            rev: s.rev,
            sent: s
                .sent
                .into_iter()
                .map(|l| SentLine {
                    line: l.line,
                    item: l.item.into(),
                })
                .collect(),
            skipped: s
                .skipped
                .iter()
                .map(|l| SkippedLine {
                    line: l.line,
                    reason: (&l.reason).into(),
                })
                .collect(),
        }
    }
}

// ---- 持ち越し ----

mirror_enum! {
    /// 持ち越しの画面を、いつ出すか（画面設計書 5章）。
    /// - `show`：まだ後回しにしていない。表示したときに出す。
    /// - `reshow_on_focus`：後回しにしたが、まだ再表示していない。次にアプリを前面に出したとき
    ///   1回だけ出し、`mark_carryover_reshown` で記録する。
    /// - `on_request`：後回しにして、再表示も済んだ。自動では出さず、パレットの「持ち越しを開く」で開く。
    CarryoverPrompt <=> carryover::Prompt { Show, ReshowOnFocus, OnRequest }
}

/// 今日の持ち越しが残っているときの状態（`carryover_status`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CarryoverStatus {
    /// 前回の日。
    pub from: Date,
    pub prompt: CarryoverPrompt,
}

impl From<carryover::Pending> for CarryoverStatus {
    fn from(p: carryover::Pending) -> Self {
        Self {
            from: Date(p.from),
            prompt: p.prompt.into(),
        }
    }
}

/// 持ち越しの画面に出す1件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CarryoverCandidate {
    /// 前回の日の項目（未完了の TODO か、未解消の障害）。
    pub item: Item,
    /// 見直しの選択肢（分割する／やめる）を添えるか：持ち越し回数が見直しの回数（設定）以上の TODO。
    pub review: bool,
}

/// 持ち越しの画面の中身（`carryover_candidates`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CarryoverList {
    /// 前回の日。確定（`confirm_carryover`）にそのまま渡す。
    pub from: Date,
    /// 前回の日の並び順。
    pub items: Vec<CarryoverCandidate>,
    /// 見直しの回数（設定）。持ち越し回数がこれ以上の印を `--warn` にする（障害にも。デザインガイド 6章）。
    #[cfg_attr(test, ts(type = "number"))]
    pub review_threshold: i64,
}

impl From<carryover::Candidates> for CarryoverList {
    fn from(c: carryover::Candidates) -> Self {
        Self {
            from: Date(c.from),
            items: c
                .items
                .into_iter()
                .map(|c| CarryoverCandidate {
                    item: c.item.into(),
                    review: c.review,
                })
                .collect(),
            review_threshold: c.review_threshold,
        }
    }
}

/// 持ち越しの画面のボタン。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(tag = "select", rename_all = "snake_case")]
pub enum CarryoverSelection {
    /// 「すべて持ち越す」：確定の時点で持ち越しの対象のものをすべて。
    All,
    /// 「選んだものを持ち越す」：チェックの付いた項目 `ids` だけ。
    Selected { ids: Vec<Id> },
    /// 「持ち越さない」。
    None,
}

/// 見直しの選択。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(tag = "review", rename_all = "snake_case")]
pub enum CarryoverReview {
    /// やめる。
    Drop,
    /// 外部ツールへ戻す（外部課題と紐付いた TODO だけ）。
    ReturnExternal,
    /// 分割する：子の TODO の本文（書いた順）。空の子は作らず、子が無ければ見直しを選ばなかったものとする。
    Split { children: Vec<String> },
}

/// 見直しを選んだ TODO と、その選択。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CarryoverReviewChoice {
    pub id: Id,
    pub review: CarryoverReview,
}

/// 持ち越しの確定の中身（`confirm_carryover`）。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CarryoverDecision {
    /// 画面に出していた前回の日（`CarryoverList` の `from`）。
    pub from: Date,
    pub selection: CarryoverSelection,
    /// 見直しを選んだ TODO（どのボタンで確定しても反映する）。同じ項目が2回あれば後のものを使う。
    pub reviews: Vec<CarryoverReviewChoice>,
}

impl From<CarryoverDecision> for carryover::Decision {
    fn from(d: CarryoverDecision) -> Self {
        Self {
            from: d.from.0,
            selection: match d.selection {
                CarryoverSelection::All => carryover::Selection::All,
                CarryoverSelection::Selected { ids } => {
                    carryover::Selection::Selected(ids.into_iter().map(|id| id.0).collect())
                }
                CarryoverSelection::None => carryover::Selection::None,
            },
            reviews: d
                .reviews
                .into_iter()
                .map(|c| {
                    let review = match c.review {
                        CarryoverReview::Drop => carryover::Review::Drop,
                        CarryoverReview::ReturnExternal => carryover::Review::ReturnExternal,
                        CarryoverReview::Split { children } => carryover::Review::Split(children),
                    };
                    (c.id.0, review)
                })
                .collect(),
        }
    }
}

/// 持ち越しを確定した結果。確定は Mod+Z で取り消せないので、画面は取り消しの履歴に持たない
/// （操作仕様書 2章「取り消し・やり直し」）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CarryoverConfirmed {
    /// 今日に作った項目（持ち越した項目と分割の子）。今日の並び順。
    pub created: Vec<Item>,
    /// 見直しで取りやめた、前回の日の項目（操作の後）。
    pub closed: Vec<Item>,
}

impl From<carryover::Confirmed> for CarryoverConfirmed {
    fn from(c: carryover::Confirmed) -> Self {
        Self {
            created: c.created.into_iter().map(Into::into).collect(),
            closed: c.closed.into_iter().map(Into::into).collect(),
        }
    }
}

// ---- 検索 ----

mirror_enum! {
    /// 検索結果の種類（画面設計書 6章の英字ラベル TODO / BLOCKER / NOTE / REFLECTION）。
    SearchKind <=> search::SearchKind { Todo, Blocker, Note, Reflection }
}

/// 検索結果の1件（データモデル設計書 4.1）。一致した部分の切り出しは画面が行う。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub kind: SearchKind,
    /// 元の行：TODO・障害は項目の id、ノート・振り返りはその日の日付（`YYYY-MM-DD`）。
    pub ref_id: String,
    pub date: Date,
    /// 本文の全体（項目は1行の本文、ノート・振り返りは本文すべて）。
    pub body: String,
    /// 解消済みの障害の解消メモ（無ければ `null`）。一致したのが本文か解消メモかは画面が見分ける。
    pub resolution_note: Option<String>,
    /// 項目の持ち越し先。持ち越し済みでない項目と、ノート・振り返りは `null`。
    pub carried_to: Option<CarriedTo>,
}

impl From<search::SearchHit> for SearchHit {
    fn from(h: search::SearchHit) -> Self {
        Self {
            kind: h.kind.into(),
            ref_id: h.ref_id,
            date: Date(h.date),
            body: h.body,
            resolution_note: h.resolution_note,
            carried_to: h.carried_to.map(|c| CarriedTo {
                id: Id(c.id),
                date: Date(c.date),
            }),
        }
    }
}

// ---- 画面の入力中の状態 ----

/// 画面が知らせる、入力中の状態（アーキテクチャ設計書 4.1）。変わるたびに全体を知らせ直す。
///
/// 知らせはそれぞれ別のスレッドで処理されるので、続けて送ると届く順が入れ替わりうる。そこで
/// 画面の読み込みごとの `session` と、その中で増える `seq` を添え、コアは同じ `session` の古い
/// 知らせ（`seq` が最後に受け付けたもの以下）を捨てる。`src/lib/core.ts` が付ける。
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct UiReport {
    /// 画面の読み込みごとに変わる文字列（読み込み直すと `seq` が 0 に戻るため）。
    pub session: String,
    /// 同じ `session` の中で、送るたびに増える番号。
    pub seq: u32,
    /// 入力中の項目の欄（フォーカスがあり最後の入力から 10 秒以内か IME の変換中、
    /// またはまだ保存していない入力の書き込みを待っている欄）。
    pub editing_items: Vec<EditingItem>,
    /// ノート・振り返りの守る行。守る行の無いエディタは入れない（入れても無いものとして扱う）。
    pub editors: Vec<EditorLines>,
}

/// 入力中の項目の欄。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EditingItem {
    pub item_id: Id,
    pub field: ItemField,
}

/// ノート・振り返りの守る行。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EditorLines {
    pub doc: TextDoc,
    pub date: Date,
    /// 行番号を数えた本文（最後に保存した本文）の版。
    #[cfg_attr(test, ts(type = "number"))]
    pub rev: i64,
    /// 守る行の番号（1始まり）。
    pub lines: Vec<usize>,
}

// ---- 起動とイベント ----

/// データベースを開く処理の状態（起動直後に画面が `startup_status` で問い合わせ、変わると
/// `startup` イベントでも届く）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Startup {
    /// 開いている途中（ロックを待っているときは最大 5 秒）。
    Opening,
    /// 開いた。コマンドを使える。
    Ready,
    /// 起動時にバックアップから戻して開いた（`restore_at_startup`）。コマンドは使えるが、画面は
    /// 退避した場所を示し、利用者が確かめたら `acknowledge_restore` を呼ぶ（その後は `ready`）。
    #[serde(rename_all = "camelCase")]
    Restored {
        /// 退避した、それまでのデータベース（`backups/damaged-<秒>.db`）。無かったときは `None`。
        set_aside: Option<String>,
    },
    /// 保存先にデータベースが無く、初めて使うときでもない（アーキテクチャ設計書 6.3）。
    /// 画面は探した場所を示して案内する。新しく作るのは `create_database`。
    #[serde(rename_all = "camelCase")]
    NotFound {
        /// 探した `yunagi.db` のパス。
        path: String,
        /// 保存先。
        data_dir: String,
        /// 保存先を変えているときの、それを記録した `config.json`。
        configured_in: Option<String>,
        /// 保存先のフォルダそのものがあるか。
        data_dir_exists: bool,
    },
    /// 開けなかった。`detail` はログ用の文（英語。画面にそのまま出さない。問い合わせのときに
    /// 添えてもらうため、画面は「詳しい情報」として写せるように示す）。
    #[serde(rename_all = "camelCase")]
    Failed {
        reason: StartupFailure,
        detail: String,
        /// 開こうとした `yunagi.db` のパス（保存先が分かったとき）。
        database: Option<String>,
        /// マイグレーション前のバックアップ（`migration_failed`・`migration_restore_failed` のとき。
        /// 適用前のデータがある）。
        backup: Option<String>,
        /// `config.json` のパス（`config` のとき）。
        config: Option<String>,
    },
}

/// `backups/` にある1つのバックアップ（`list_backups`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BackupEntry {
    /// ファイルのパス。戻すときに `restore_at_startup` に渡す。
    pub path: String,
    pub kind: BackupKind,
    /// 作った時刻（秒まで）。
    pub created_at: Millis,
}

/// バックアップの種類（アーキテクチャ設計書 6.2）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BackupKind {
    /// 1日1回の定期バックアップ。`date` は作った日の論理日付。
    Daily { date: Date },
    /// マイグレーションの適用前のもの（スキーマの番号 `from` から `to` へ）。
    PreMigration { from: u32, to: u32 },
    /// 復元の前に、それまでのデータを退避したもの。
    PreRestore,
}

impl From<yunagi_core::storage::backup::Backup> for BackupEntry {
    fn from(b: yunagi_core::storage::backup::Backup) -> Self {
        use yunagi_core::storage::backup::BackupKind as K;
        Self {
            path: b.path.display().to_string(),
            kind: match b.kind {
                K::Daily { date } => BackupKind::Daily { date: Date(date) },
                K::PreMigration { from, to } => BackupKind::PreMigration { from, to },
                K::PreRestore => BackupKind::PreRestore,
            },
            created_at: Millis(b.created_at),
        }
    }
}

/// データベースを開けなかった理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum StartupFailure {
    /// 既定ディレクトリを求められない（環境変数が無い）か、作れない。
    DefaultDir,
    /// `yunagi.lock` を 5 秒待っても取れない（CLI・MCP の操作中など）。
    LockBusy,
    /// `yunagi.lock` を開けない・ロックできない。
    Lock,
    /// `config.json` が壊れている・読めない。
    Config,
    /// 新しく作ろうとした場所に、すでにデータベースがある。
    AlreadyExists,
    /// マイグレーションの適用に失敗し、適用前の内容に戻した（データは無事）。旧版を入れ直せば
    /// そのまま開ける（ビルド・配布手順書 5章）。
    MigrationFailed,
    /// マイグレーションの適用に失敗し、適用前の内容に戻すことにも失敗した。適用前のデータは
    /// `backup` に残っている。
    MigrationRestoreFailed,
    /// データベースが、このアプリより新しい版で作られている（スキーマの番号を知らない）。
    /// ファイルには触れていない。
    UnsupportedSchema,
    /// データベースが壊れている（マイグレーション前のバックアップの整合性の確認が通らない、
    /// SQLite のデータベースとして読めない）。ファイルには触れていない。バックアップから戻せる
    /// （`restore_at_startup`）。
    Corrupted,
    /// そのほかの理由で、開けなかった（WAL にできない、読み書きの失敗など）。
    Storage,
}

/// `changed` イベント：コアでデータが変わった。画面は表示中のデータを読み込み直す
/// （アーキテクチャ設計書 4章 6）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Changed {
    pub by: ChangeSource,
}

/// 変えたもの。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum ChangeSource {
    /// 画面からの保存（自分の保存の後にも届く。行き違いの判定で記録が変わることがあるため）。
    Screen,
    /// エージェント・CLI（ローカル API 経由）。
    Agent,
}

/// `today-changed` イベント：今日の論理日付が変わった（切り替わり時刻を越えた）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TodayChanged {
    pub today: Date,
}

/// `show-requested` イベント：窓を前面に出した（`yunagi show`、2 つ目の起動の引き渡し）。画面は、
/// `note` が真でノートモードに切り替えられる状態（データベースを開いていて、持ち越しの画面などを
/// 出していない）なら、ノートモードにする（操作仕様書 1章「OS 全体の操作」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ShowRequested {
    pub note: bool,
}

/// `backup-failed` イベント：定期バックアップを作れなかった。`detail` はログ用の文（英語）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BackupFailed {
    pub detail: String,
}

// ---- 設定 ----

/// テーマ（画面設計書 7章「表示」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    /// OS に合わせる（既定）。
    System,
    Light,
    Dark,
}

/// ノートの書体（画面設計書 7章「表示」、デザインガイド 4章）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum NoteFont {
    /// 本文と同じ（既定）。
    Body,
    /// 等幅。
    Mono,
}

/// 整理モードのレイアウト（画面設計書 3章「レイアウト」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
pub enum Layout {
    /// 2段（既定）。
    A,
    /// 3列。
    B,
    /// 縦積み。
    C,
}

/// 画面の言語。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum Locale {
    Ja,
    En,
}

macro_rules! mirror_settings_enum {
    ($name:ident <=> $core:ty { $($variant:ident),+ $(,)? }) => {
        impl From<$core> for $name {
            fn from(v: $core) -> Self {
                match v {
                    $(<$core>::$variant => Self::$variant),+
                }
            }
        }

        impl From<$name> for $core {
            fn from(v: $name) -> Self {
                match v {
                    $($name::$variant => Self::$variant),+
                }
            }
        }
    };
}

mirror_settings_enum!(Theme <=> settings::Theme { System, Light, Dark });
mirror_settings_enum!(NoteFont <=> settings::NoteFont { Body, Mono });
mirror_settings_enum!(Layout <=> settings::Layout { A, B, C });
mirror_settings_enum!(Locale <=> settings::Locale { Ja, En });

/// 画面に出す設定（既定値を当てた後のもの。データモデル設計書 3章 setting）。時刻は `HH:MM`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// 日付の切り替わり時刻（既定 `04:00`）。
    pub day_boundary: String,
    /// 凪メーターの始業（既定 `09:00`）。
    pub meter_start: String,
    /// 凪メーターの終業（既定 `19:00`）。始業より後。
    pub meter_end: String,
    /// 画面の言語。設定が無ければ `null`（OS の言語に合わせる）。
    pub locale: Option<Locale>,
    /// 更新の自動確認（既定 `true`）。
    pub update_check: bool,
    pub theme: Theme,
    /// フォントサイズ（px。13〜19、既定 15）。
    pub font_size: u8,
    pub note_font: NoteFont,
    pub layout: Layout,
    /// 持ち越し見直しの回数（1 以上、既定 3）。
    pub carryover_review_threshold: u32,
    /// サイドバーの開閉（広いウィンドウの整理モードでの状態。既定は開く）。
    pub sidebar_open: bool,
    /// ショートカットの割当：操作の名前から割当の文字列へ。初期割当から変えたものだけ
    /// （データモデル設計書 3章 setting）。
    pub shortcuts: BTreeMap<String, String>,
    /// Vim モード（既定 `false`）。
    pub vim_mode: bool,
    /// Vim モードの挿入モードで、直前の入力状態に戻す（既定 `true`）。
    pub vim_ime_restore: bool,
}

impl From<settings::Settings> for Settings {
    fn from(s: settings::Settings) -> Self {
        Self {
            day_boundary: s.day_boundary.to_string(),
            meter_start: s.meter_hours.start.to_string(),
            meter_end: s.meter_hours.end.to_string(),
            locale: s.locale.map(Locale::from),
            update_check: s.update_check,
            theme: s.theme.into(),
            font_size: s.font_size,
            note_font: s.note_font.into(),
            layout: s.layout.into(),
            carryover_review_threshold: s.carryover_review_threshold,
            sidebar_open: s.sidebar_open,
            shortcuts: s.shortcuts,
            vim_mode: s.vim_mode,
            vim_ime_restore: s.vim_ime_restore,
        }
    }
}

impl SettingsPatch {
    /// コアの形にする。時刻が `HH:MM` でなければ `invalid_argument`（`time_of_day`）。
    /// 凪メーターの始業・終業は、片方だけのときもう片方に `current` の値を使う。
    pub fn into_core(
        self,
        current: &settings::Settings,
    ) -> Result<settings::SettingsPatch, CommandError> {
        let time = |s: Option<String>| -> Result<_, CommandError> {
            s.map(|s| {
                s.parse::<DayBoundary>()
                    .map_err(|_| CommandError::InvalidArgument {
                        reason: InvalidReason::TimeOfDay,
                    })
            })
            .transpose()
        };
        let (start, end) = (time(self.meter_start)?, time(self.meter_end)?);
        let meter_hours = (start.is_some() || end.is_some()).then(|| {
            (
                start.unwrap_or(current.meter_hours.start),
                end.unwrap_or(current.meter_hours.end),
            )
        });
        Ok(settings::SettingsPatch {
            day_boundary: time(self.day_boundary)?,
            meter_hours,
            locale: self.locale.map(Into::into),
            update_check: self.update_check,
            theme: self.theme.map(Into::into),
            font_size: self.font_size,
            note_font: self.note_font.map(Into::into),
            layout: self.layout.map(Into::into),
            carryover_review_threshold: self.carryover_review_threshold,
            sidebar_open: self.sidebar_open,
            shortcuts: self.shortcuts,
            vim_mode: self.vim_mode,
            vim_ime_restore: self.vim_ime_restore,
        })
    }
}

/// 設定の保存：変える欄だけ（無い欄には触れない）。`meterStart`・`meterEnd` は、片方だけでも、
/// もう片方は今の値を使う。
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    #[cfg_attr(test, ts(optional))]
    pub day_boundary: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub meter_start: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub meter_end: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub locale: Option<Locale>,
    #[cfg_attr(test, ts(optional))]
    pub update_check: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub theme: Option<Theme>,
    #[cfg_attr(test, ts(optional))]
    pub font_size: Option<u8>,
    #[cfg_attr(test, ts(optional))]
    pub note_font: Option<NoteFont>,
    #[cfg_attr(test, ts(optional))]
    pub layout: Option<Layout>,
    #[cfg_attr(test, ts(optional))]
    pub carryover_review_threshold: Option<u32>,
    #[cfg_attr(test, ts(optional))]
    pub sidebar_open: Option<bool>,
    /// 割当の全体（欄ごと置き換える。初期割当と同じものは入れない）。
    #[cfg_attr(test, ts(optional))]
    pub shortcuts: Option<BTreeMap<String, String>>,
    #[cfg_attr(test, ts(optional))]
    pub vim_mode: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub vim_ime_restore: Option<bool>,
}

/// クラウド同期のサービス（保存先の警告に使う。画面設計書 7章）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum CloudService {
    ICloudDrive,
    Dropbox,
    OneDrive,
    GoogleDrive,
}

impl From<cloud_sync::CloudService> for CloudService {
    fn from(s: cloud_sync::CloudService) -> Self {
        use cloud_sync::CloudService as C;
        match s {
            C::ICloudDrive => Self::ICloudDrive,
            C::Dropbox => Self::Dropbox,
            C::OneDrive => Self::OneDrive,
            C::GoogleDrive => Self::GoogleDrive,
        }
    }
}

/// 保存先の変更の結果（`change_data_dir`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DataDirChanged {
    /// 移した後に消せなかった、元の保存先のファイル・フォルダ（データは新しい保存先に移っている。
    /// 消すかどうかは利用者に任せる）。
    pub leftovers: Vec<String>,
}

// ---- 日本語入力（IME）の切替 ----

/// IME の切替に使う仕組み（操作仕様書 4.4。`src-tauri/src/ime.rs`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum ImeMethod {
    /// Linux：fcitx5（`fcitx5-remote`）。
    Fcitx5,
    /// Linux：IBus（`ibus engine`）。
    Ibus,
    /// macOS：入力ソース API。
    MacosInputSource,
    /// Windows：IMM のオープン状態。
    WindowsImm,
}

/// この環境で IME の切替ができるか（`ime_status`）。できないとき（Linux で fcitx5 も IBus も動いていない
/// など）、設定画面が制約を表示する（画面設計書 7章「操作」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ImeStatus {
    pub available: bool,
    pub method: Option<ImeMethod>,
}

/// OS 全体のショートカット（Yunagi を前面に呼び出す）の、いまの登録の状態（`set_global_shortcut`）。
/// 設定画面が、登録できなかった理由の表示に使う（操作仕様書 1章「割当の変更」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum GlobalShortcutState {
    /// 登録していない（割当が使えない状態のとき）。
    Off,
    /// OS に登録した。
    Registered,
    /// ほかのアプリが同じキーを使っていて、登録できなかった。
    Taken,
    /// そのほかの理由で OS に登録できなかった（macOS は、ほかのアプリが使っている場合もここに入る）。
    Failed,
    /// この環境では OS 全体のショートカットの部品が使えない（部品を起動できなかった）。
    Unavailable,
}

/// OS 全体のショートカットの登録の結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct GlobalShortcutStatus {
    pub state: GlobalShortcutState,
    /// Linux の Wayland のセッションか。Wayland では、登録できたように見えても効かないことがあり
    /// （登録の成否を OS から知る手段が無い）、設定画面は常に案内を出す（アーキテクチャ設計書 7章）。
    pub wayland: bool,
}

// ---- エラー ----

/// コマンドの失敗。`kind` で見分ける。画面に出す文は画面が `kind` から決める
/// （Display の文をそのまま使わない。README「エラーの表示」）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommandError {
    /// データベースをまだ開いていない（開いている途中・見つからない・開けなかった）。
    /// 画面は `startup_status` を見る。
    NotReady,
    /// 今の状態ではできない操作（データベースが見つからないとき以外の `create_database` など）。
    InvalidState,
    /// 項目が無いか、削除されている。
    NotFound,
    /// 引数が誤っている。何も書いていない。
    InvalidArgument { reason: InvalidReason },
    /// 受け付けられない状態にある。何も書いていない。
    Conflict(Conflict),
    /// 削除した持ち越し先を戻せない。何も書いていない。
    RestoreRefused(RestoreRefused),
    /// 保存の書き込みに失敗した（ディスクがいっぱい・保存先が外れたなど）。`detail` はログ用の文。
    /// これを返してから次に保存が成功するまで、コアはエージェントの書き込みを受け付けない
    /// （アーキテクチャ設計書 4.1「保存に失敗している間」）。
    Storage { detail: String },
    /// 選んだバックアップ・保存先を使えないので、何も変えなかった（`restore_at_startup`・
    /// `choose_data_dir`・設定画面の `change_data_dir`・`use_data_dir`・`restore_backup`）。
    Rejected { reason: RejectReason },
    /// Markdown の書き出しで、ファイルを書けなかった（書き出し先のフォルダが無い・書き込めないなど）。
    /// `replaced` は、それまでに置き換え終えたファイルの数（0 なら何も変えていない）。`detail` は
    /// ログ用の文。データベースの読み取りの失敗は `storage`（画面設計書 7章・アーキテクチャ設計書 6.4）。
    ExportFailed { replaced: usize, detail: String },
    /// バックアップから戻すのに失敗し、退避した前のデータベースを元の場所に戻すこともできなかった
    /// （`restore_at_startup`）。前のデータベースは `set_aside`（とその横のファイル）に残っている。
    /// `detail` はログ用の文。
    #[serde(rename_all = "camelCase")]
    PutBackFailed { set_aside: String, detail: String },
    /// アプリの不具合（コマンドの処理が途中で止まった）。`detail` はログ用の文。
    Internal { detail: String },
}

/// 選んだバックアップ・保存先を使えない理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    /// 保存先のパスの形が誤っている（絶対パスでない、`..`・`.` を含む、UTF-8 で表せない）。
    InvalidPath,
    /// 選んだ保存先が無いか、フォルダでない。
    NotADirectory,
    /// 選んだ保存先に、中身のある `yunagi.db` が無い。
    NoDatabase,
    /// 読めない（SQLite のデータベースでない、ファイルが無い、権限が無いなど）。
    Unreadable,
    /// 整合性の確認（`integrity_check`）が通らない。
    Corrupted,
    /// Yunagi のデータベースではない（スキーマの番号が 0）。
    NotYunagi,
    /// このアプリより新しい版で作られている。
    NewerSchema,
    /// 選んだ保存先が、今の保存先と同じ（保存先の変更）。
    SameDirectory,
    /// 選んだ保存先が、今の保存先の `backups/` の中にある（保存先の変更）。
    InsideBackups,
    /// 選んだ保存先に、すでにデータベースの跡か、空でない `backups/` がある（保存先の変更。
    /// 上書きしない）。
    TargetHasData,
}

/// 引数の誤りの種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum InvalidReason {
    /// 操作と違う種類の項目（TODO に解消メモなど）。
    WrongKind,
    /// 取りやめた TODO は、状態と本文を変えられない。
    Dropped,
    /// 本文が空。
    EmptyText,
    /// 本文に改行がある。
    MultilineText,
    /// 解消メモが空。
    EmptyNote,
    /// 解消メモに改行がある。
    MultilineNote,
    /// 位置の基準が、同じ日のほかの項目でない。
    InvalidPosition,
    /// 外部課題と紐付いていない TODO。
    NotLinkedExternal,
    /// ノートの行の指定の誤り（エージェントの操作と、行送りの行の番号の誤り）。
    NoteLine,
    /// 守る行の番号が 0（1始まり）。
    InvalidLine,
    /// 設定：凪メーターの始業が終業より前でない。
    MeterHours,
    /// 設定：フォントサイズが 13〜19 でない。
    FontSize,
    /// 設定：持ち越し見直しの回数が 1 以上でない。
    ReviewThreshold,
    /// 設定：ショートカットの割当の形が誤っている。
    Shortcuts,
    /// 設定：`HH:MM` の形でない時刻。
    TimeOfDay,
}

/// 受け付けられない状態の理由。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Conflict {
    /// 項目は持ち越し済み。持ち越し先とその日付。
    #[serde(rename_all = "camelCase")]
    CarriedOver {
        carried_to_id: Id,
        carried_to_date: Date,
    },
    /// 利用者がその欄を入力中（エージェントの操作だけで起きる）。
    UserEditing,
    /// 障害に、違う解消メモがすでにある（エージェントの操作だけで起きる）。
    #[serde(rename_all = "camelCase")]
    NoteExists { resolution_note: String },
    /// 持ち越しの画面に出していた前回の日が、今の前回の日と違う（その間に、より新しい日に項目か
    /// ノートができた）。`previousDay` は今の前回の日。画面は一覧を読み直す。
    #[serde(rename_all = "camelCase")]
    PreviousDayChanged { previous_day: Option<Date> },
    /// 今日の持ち越しはすでに終えている（古い画面からの二度目の確定など）。
    CarryoverDone,
}

/// 削除を戻せない理由。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum RestoreRefused {
    /// 削除した後に、持ち越し元の項目が変わった。
    SourceChanged,
    /// 削除した後に、持ち越し元の項目がほかの項目へ持ち越された。
    #[serde(rename_all = "camelCase")]
    SourceCarriedOver {
        carried_to_id: Id,
        carried_to_date: Date,
    },
}

impl From<core_item::InvalidArgument> for InvalidReason {
    fn from(e: core_item::InvalidArgument) -> Self {
        use core_item::InvalidArgument as A;
        match e {
            A::WrongKind { .. } => Self::WrongKind,
            A::Dropped { .. } => Self::Dropped,
            A::EmptyText => Self::EmptyText,
            A::MultilineText => Self::MultilineText,
            A::EmptyNote => Self::EmptyNote,
            A::MultilineNote => Self::MultilineNote,
            A::InvalidPosition => Self::InvalidPosition,
            A::NotLinkedExternal => Self::NotLinkedExternal,
        }
    }
}

impl From<core_item::ItemError> for CommandError {
    fn from(e: core_item::ItemError) -> Self {
        use core_item::ItemError as E;
        match e {
            E::NotFound => Self::NotFound,
            E::InvalidArgument(a) => Self::InvalidArgument { reason: a.into() },
            E::Conflict(c) => Self::Conflict(match c {
                core_item::Conflict::CarriedOver {
                    carried_to_id,
                    carried_to_date,
                } => Conflict::CarriedOver {
                    carried_to_id: Id(carried_to_id),
                    carried_to_date: Date(carried_to_date),
                },
                core_item::Conflict::UserEditing => Conflict::UserEditing,
                core_item::Conflict::NoteExists { resolution_note } => {
                    Conflict::NoteExists { resolution_note }
                }
            }),
            E::RestoreRefused(r) => Self::RestoreRefused(match r {
                core_item::RestoreRefused::SourceChanged => RestoreRefused::SourceChanged,
                core_item::RestoreRefused::SourceCarriedOver {
                    carried_to_id,
                    carried_to_date,
                } => RestoreRefused::SourceCarriedOver {
                    carried_to_id: Id(carried_to_id),
                    carried_to_date: Date(carried_to_date),
                },
            }),
            // 画面の操作では起きない（エージェントの操作だけ）。画面の操作は Agent を渡さない
            E::PermissionDenied(_) | E::StorageUnavailable(_) => Self::Internal {
                detail: crate::app_core::chain(&e),
            },
            E::Storage(_) => Self::storage(&e),
        }
    }
}

impl From<text_doc::TextDocError> for CommandError {
    fn from(e: text_doc::TextDocError) -> Self {
        use text_doc::TextDocError as E;
        match e {
            E::InvalidArgument(_) => Self::InvalidArgument {
                reason: InvalidReason::NoteLine,
            },
            E::Storage(_) => Self::storage(&e),
            // 保存の行き違い（Stale）は、コマンドでは成功（TextSave::Stale）として返す。
            // ほかはエージェントの操作だけで起きる
            E::Stale { .. }
            | E::PermissionDenied(_)
            | E::StorageUnavailable(_)
            | E::Changed(_)
            | E::UserEditing => Self::Internal {
                detail: crate::app_core::chain(&e),
            },
        }
    }
}

impl From<note_send::SendError> for CommandError {
    fn from(e: note_send::SendError) -> Self {
        use note_send::SendError as E;
        match e {
            E::InvalidArgument(_) => Self::InvalidArgument {
                reason: InvalidReason::NoteLine,
            },
            E::Storage(_) => Self::storage(&e),
            // 行き違いは、コマンドでは成功（SendResult::Stale）として返す
            E::Stale { .. } => Self::Internal {
                detail: crate::app_core::chain(&e),
            },
        }
    }
}

impl From<carryover::CarryoverError> for CommandError {
    fn from(e: carryover::CarryoverError) -> Self {
        use carryover::CarryoverError as E;
        match e {
            E::PreviousDayChanged { previous_day } => {
                Self::Conflict(Conflict::PreviousDayChanged {
                    previous_day: previous_day.map(Date),
                })
            }
            E::AlreadyDone => Self::Conflict(Conflict::CarryoverDone),
            E::InvalidArgument(a) => Self::InvalidArgument { reason: a.into() },
            E::Storage(_) => Self::storage(&e),
        }
    }
}

impl From<settings::SettingsError> for CommandError {
    fn from(e: settings::SettingsError) -> Self {
        use settings::InvalidSetting as I;
        match e {
            settings::SettingsError::Invalid(field) => Self::InvalidArgument {
                reason: match field {
                    I::MeterHours => InvalidReason::MeterHours,
                    I::FontSize => InvalidReason::FontSize,
                    I::ReviewThreshold => InvalidReason::ReviewThreshold,
                    I::Shortcuts => InvalidReason::Shortcuts,
                },
            },
            settings::SettingsError::Storage(e) => Self::storage(&e),
        }
    }
}

impl From<yunagi_core::domain::export::ExportError> for CommandError {
    fn from(e: yunagi_core::domain::export::ExportError) -> Self {
        use yunagi_core::domain::export::ExportError as E;
        let replaced = match &e {
            E::RelativeDir { .. } => 0,
            E::Write { replaced, .. } => *replaced,
        };
        Self::ExportFailed {
            replaced,
            detail: crate::app_core::chain(&e),
        }
    }
}

impl From<yunagi_core::storage::StorageError> for CommandError {
    fn from(e: yunagi_core::storage::StorageError) -> Self {
        Self::storage(&e)
    }
}

impl CommandError {
    /// 読み書きの失敗。`detail` は原因までつないだ文（ログ用）。
    pub fn storage(e: &(dyn std::error::Error + 'static)) -> Self {
        Self::Storage {
            detail: crate::app_core::chain(e),
        }
    }
}
