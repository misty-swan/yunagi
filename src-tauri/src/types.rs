//! 画面（TypeScript）と受け渡す型（アーキテクチャ設計書 2章「Tauri コマンド」）。
//!
//! コアの型をそのまま渡さず、ここで画面向けの形（名前は camelCase、ID・日付は文字列、時刻は
//! UNIX ミリ秒の数）にする。コアの型は CLI・MCP とも共有するので、画面の都合を持ち込まないため。
//! TypeScript の定義は ts-rs でここから作り、`src/lib/bindings.ts` に置く（`crate::bindings`）。
//! 作るのはテストだけなので、ts-rs の derive はテストのビルドにだけ付ける（配るアプリに含めない）。

use std::fmt;
use std::str::FromStr;

use jiff::Timestamp;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
#[cfg(test)]
use ts_rs::TS;
use yunagi_core::domain::agent::ItemField as CoreItemField;
use yunagi_core::domain::board;
use yunagi_core::domain::item::{self as core_item, CloseReason as CoreCloseReason};
use yunagi_core::domain::logical_date::LogicalDate;
use yunagi_core::domain::note_body;
use yunagi_core::domain::text_doc::{self, TextDoc as CoreTextDoc};
use yunagi_core::storage::Ulid;

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
}

impl From<board::ScreenBoard> for Board {
    fn from(sb: board::ScreenBoard) -> Self {
        let board::ScreenBoard {
            board: b,
            mut agent_names,
            unseen,
        } = sb;
        let mut items = |list: Vec<board::BoardItem>| {
            list.into_iter()
                .map(|bi| BoardItem {
                    agent_name: agent_names.remove(&bi.item.id),
                    unseen: unseen.contains(&bi.item.id),
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

/// 起動時にバックアップから戻した結果（`restore_at_startup`）。開いた状態は `startup` で届く。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StartupRestored {
    /// 退避した、それまでのデータベース（`backups/damaged-<秒>.db`）。無かったときは `None`。
    pub set_aside: Option<String>,
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

/// `backup-failed` イベント：定期バックアップを作れなかった。`detail` はログ用の文（英語）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BackupFailed {
    pub detail: String,
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
    /// 選んだバックアップ・保存先を使えないので、何も変えなかった（起動時の
    /// `restore_at_startup`・`choose_data_dir`）。
    Rejected { reason: RejectReason },
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
    /// ノートの行の指定の誤り（エージェントの操作だけで起きる）。
    NoteLine,
    /// 守る行の番号が 0（1始まり）。
    InvalidLine,
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
