//! その日のボードと件数の読み取り（データモデル設計書 3章 item「件数の数え方」、MCP・CLI 仕様書
//! 4.2 `get_board`、画面設計書 3章「凪メーター」「サイドバー」・9章 S-08）。
//!
//! - [`counts`]・[`counts_between`]・[`week_counts`]：件数（TODO 数・完了数・残り・解消数・持ち越し数）を、
//!   日ごと・週ごとに求める。凪メーター・サイドバー・ふりかえりで使う。
//! - [`get_board`]・[`open_board`]：その日のボード（MCP の `get_board`・`open_board`。画面の表示にも使う）。
//! - [`recent_days`]：サイドバーの Recent に並べる日。
//!
//! どれも読むだけで、day の行も作らない。

use std::collections::BTreeMap;
use std::ops::{Add, AddAssign};

use rusqlite::Connection;

use super::day;
use super::item::{CarriedTo, Item, ItemKind, ItemStatus};
use super::logical_date::LogicalDate;
use super::note_body::{self, TimeHeading};
use crate::storage::table::item as item_table;
use crate::storage::table::text_doc::{self, TextDoc};
use crate::storage::{StorageError, in_read_transaction};

// ---- 件数 ----

/// 件数（データモデル設計書 3章 item「件数の数え方」）。削除されていない項目だけを数える。
///
/// 残り（LEFT）は、凪メーター（「TODO 6 · Blockers 3」）と各領域の見出し（「6 left」「3 open」）で
/// TODO と障害に分けて出すので、分けて持つ。合計は [`Counts::left`]。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// TODO 数：TODO のうち、取りやめた（`dropped`）でないもの。
    pub todos: u64,
    /// 完了数（Done）：完了した TODO。
    pub done: u64,
    /// 残り（LEFT）のうち、未完了の TODO。
    pub left_todos: u64,
    /// 残り（LEFT）のうち、未解消の障害。
    pub left_blockers: u64,
    /// 解消数（Resolved）：解消した障害。
    pub resolved: u64,
    /// 持ち越し数（Carried）：前の日から持ち越してきた項目（`carried_from_id` がある。種類と状態は問わない）。
    pub carried: u64,
}

impl Counts {
    /// 項目 `items`（削除されていないもの）を数える。
    pub fn of<'a>(items: impl IntoIterator<Item = &'a Item>) -> Self {
        let mut c = Self::default();
        for item in items {
            c.count(item);
        }
        c
    }

    fn count(&mut self, item: &Item) {
        debug_assert!(item.deleted_at.is_none(), "deleted items are not counted");
        match (item.kind, item.status) {
            (ItemKind::Todo, ItemStatus::Open) => {
                self.todos += 1;
                self.left_todos += 1;
            }
            (ItemKind::Todo, ItemStatus::Done) => {
                self.todos += 1;
                self.done += 1;
            }
            (ItemKind::Blocker, ItemStatus::Open) => self.left_blockers += 1,
            (ItemKind::Blocker, ItemStatus::Resolved) => self.resolved += 1,
            // 取りやめた TODO は TODO 数に数えない。種類と状態の組が合わない行（障害の done など）は、
            // コアが作らない壊れたデータなので、どの件数にも入れない
            _ => {}
        }
        if item.carried_from_id.is_some() {
            self.carried += 1;
        }
    }

    /// 残り（LEFT）：未完了の TODO と未解消の障害の合計。
    pub fn left(&self) -> u64 {
        self.left_todos + self.left_blockers
    }
}

impl AddAssign for Counts {
    fn add_assign(&mut self, other: Self) {
        self.todos += other.todos;
        self.done += other.done;
        self.left_todos += other.left_todos;
        self.left_blockers += other.left_blockers;
        self.resolved += other.resolved;
        self.carried += other.carried;
    }
}

impl Add for Counts {
    type Output = Self;

    fn add(mut self, other: Self) -> Self {
        self += other;
        self
    }
}

impl std::iter::Sum for Counts {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::default(), Add::add)
    }
}

/// その日の件数。項目の無い日はすべて 0。
pub fn counts(conn: &Connection, date: LogicalDate) -> Result<Counts, StorageError> {
    Ok(Counts::of(&item_table::list_by_date(conn, date)?))
}

/// `from` から `to` まで（両端を含む）の日ごとの件数。項目のある日だけを入れる（無い日はすべて 0 として
/// 扱う。`get` が `None` なら [`Counts::default`]）。`from` が `to` より後なら空。
pub fn counts_between(
    conn: &Connection,
    from: LogicalDate,
    to: LogicalDate,
) -> Result<BTreeMap<LogicalDate, Counts>, StorageError> {
    let mut by_day: BTreeMap<LogicalDate, Counts> = BTreeMap::new();
    for item in item_table::list_between(conn, from, to)? {
        by_day.entry(item.date).or_default().count(&item);
    }
    Ok(by_day)
}

/// 週の件数（ふりかえり S-08 の This week と週のカード）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeekCounts {
    /// 週の日（[`week_days`]）と、その日の件数（項目の無い日はすべて 0）。古い順。
    pub days: Vec<(LogicalDate, Counts)>,
    /// 週の合計。日ごとの件数を足したもの（持ち越しを続けた項目は、日ごとに数える）。
    pub total: Counts,
}

/// `date` を含む週の日：月曜から金曜まで（画面設計書 9章。土日の日付では、その前の月曜から金曜）。
/// 古い順。
pub fn week_days(date: LogicalDate) -> [LogicalDate; 5] {
    // 月曜は 0、日曜は 6。0001-01-01 は月曜、9999-12-31 は金曜なので、週の5日はいつも暦の中にある
    let from_monday = i32::from(date.as_date().weekday().to_monday_zero_offset());
    std::array::from_fn(|i| {
        // i は 0〜4
        date.add_days(i as i32 - from_monday)
            .expect("Monday to Friday of a week is in the calendar")
    })
}

/// `date` を含む週（[`week_days`]）の日ごとの件数と合計。
pub fn week_counts(conn: &Connection, date: LogicalDate) -> Result<WeekCounts, StorageError> {
    let days = week_days(date);
    let by_day = counts_between(conn, days[0], days[4])?;
    let days: Vec<_> = days
        .into_iter()
        .map(|d| (d, by_day.get(&d).copied().unwrap_or_default()))
        .collect();
    let total = days.iter().map(|(_, c)| *c).sum();
    Ok(WeekCounts { days, total })
}

// ---- ボード ----

/// ボードの項目の1件：項目と、その持ち越し先（MCP・CLI の `carried_to_id`・`carried_to_date`。
/// 持ち越し済みでなければ `None`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardItem {
    pub item: Item,
    pub carried_to: Option<CarriedTo>,
}

/// ボードのノート：時刻付き見出しと長さだけ（本文は `get_note` で読む。MCP・CLI 仕様書 4.2）。
/// まだ書いていない日は [`Default`]（見出しが無く、長さ 0）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoteOutline {
    /// 時刻付き見出し（上から順）。`get_board` の `headings` は各見出しの [`TimeHeading::text`]。
    pub headings: Vec<TimeHeading>,
    /// 本文の長さ：文字数（Unicode のコードポイント）。まだ書いていない日は 0。
    pub length: usize,
}

/// その日のボード（MCP の `get_board`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Board {
    pub date: LogicalDate,
    /// TODO（並び順）。取りやめた TODO も入れる（`status` が `dropped`、理由は `close_reason`）。
    pub todos: Vec<BoardItem>,
    /// 障害（並び順）。
    pub blockers: Vec<BoardItem>,
    /// 振り返りの本文。まだ書いていない日は空。
    pub reflection: String,
    pub note: NoteOutline,
}

/// その日の未完了の TODO と未解消の障害だけのボード（MCP の `open_board`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenBoard {
    pub date: LogicalDate,
    /// 未完了の TODO（並び順）。持ち越し済みの項目も入れる（`carried_to` で見分ける）。
    pub todos: Vec<BoardItem>,
    /// 未解消の障害（並び順）。持ち越し済みの項目も入れる。
    pub blockers: Vec<BoardItem>,
}

/// その日のボード。項目の無い日は、項目が空で、振り返りが空、ノートは見出しが無く長さ 0。
///
/// 項目・持ち越し先・振り返り・ノートは、同じ時点の内容を読む（途中でほかの接続が書いても食い違わない）。
pub fn get_board(conn: &Connection, date: LogicalDate) -> Result<Board, StorageError> {
    in_read_transaction(conn, || read_board(conn, date))
}

fn read_board(conn: &Connection, date: LogicalDate) -> Result<Board, StorageError> {
    let (todos, blockers) = board_items(conn, date, |_| true)?;
    let reflection = text_doc::get(conn, TextDoc::Reflection, date)?
        .map(|row| row.body)
        .unwrap_or_default();
    let note = text_doc::get(conn, TextDoc::Note, date)?
        .map(|row| NoteOutline {
            headings: note_body::headings(&row.body),
            length: row.body.chars().count(),
        })
        .unwrap_or_default();
    Ok(Board {
        date,
        todos,
        blockers,
        reflection,
        note,
    })
}

/// その日の未完了の TODO と未解消の障害（`status = open`）。過去の日の項目は持ち越しても `status` を
/// 変えないので、持ち越し済みの項目も入る（件数の「残り」と同じく、その日の終わりの状態を表す）。
/// 項目と持ち越し先は、同じ時点の内容を読む。
pub fn open_board(conn: &Connection, date: LogicalDate) -> Result<OpenBoard, StorageError> {
    let (todos, blockers) = in_read_transaction(conn, || {
        board_items(conn, date, |i| i.status == ItemStatus::Open)
    })?;
    Ok(OpenBoard {
        date,
        todos,
        blockers,
    })
}

/// その日の項目のうち `keep` に当たるものを、持ち越し先を付けて、TODO と障害に分けて返す（並び順）。
fn board_items(
    conn: &Connection,
    date: LogicalDate,
    keep: impl Fn(&Item) -> bool,
) -> Result<(Vec<BoardItem>, Vec<BoardItem>), StorageError> {
    let items: Vec<Item> = item_table::list_by_date(conn, date)?
        .into_iter()
        .filter(|i| keep(i))
        .collect();
    let ids: Vec<_> = items.iter().map(|i| i.id).collect();
    let carried = item_table::carried_to_many(conn, &ids)?;
    let mut todos = Vec::new();
    let mut blockers = Vec::new();
    for item in items {
        let carried_to = carried.get(&item.id).copied();
        let list = match item.kind {
            ItemKind::Todo => &mut todos,
            ItemKind::Blocker => &mut blockers,
        };
        list.push(BoardItem { item, carried_to });
    }
    Ok((todos, blockers))
}

// ---- Recent ----

/// サイドバーの Recent に並べる日の数（今日を含む。画面設計書 3章）。
pub const RECENT_DAYS: usize = 7;

/// サイドバーの Recent に並べる日：今日 `today` と、今日より前で、削除されていない項目か、空白以外の
/// 文字を含むノートがある日（前回の日と同じ決まり。データモデル設計書 3章 item。
/// [`day::days_with_items_or_note`]）を、新しい順に、
/// 今日を含めて `limit` 日まで。今日は、項目もノートも無くてもいつも先頭に入る（`limit` が 0 なら空）。
/// 今日より後の日（先の日付に足した項目など）は入れない。振り返りだけの日も入れない。
pub fn recent_days(
    conn: &Connection,
    today: LogicalDate,
    limit: usize,
) -> Result<Vec<LogicalDate>, StorageError> {
    let Some(before) = limit.checked_sub(1) else {
        return Ok(Vec::new());
    };
    let mut days = day::days_with_items_or_note(conn, today, before)?;
    days.insert(0, today);
    Ok(days)
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use jiff::tz::TimeZone;

    use super::*;
    use crate::domain::carryover::{self, Decision, Selection};
    use crate::domain::day;
    use crate::domain::item::{self, CloseReason, Position, delete};
    use crate::storage::Database;
    use crate::storage::table::item::ItemChanges;
    use crate::storage::table::test_support::{at, date, open};

    const T0: &str = "2026-11-03T01:00:00Z";

    fn tokyo() -> TimeZone {
        TimeZone::get("Asia/Tokyo").unwrap()
    }

    fn now() -> Timestamp {
        at(T0)
    }

    fn put(conn: &Connection, d: &str, kind: ItemKind, text: &str) -> Item {
        item::add(conn, date(d), kind, text, Position::Last, &tokyo(), now()).unwrap()
    }

    fn todo(conn: &Connection, d: &str, text: &str) -> Item {
        put(conn, d, ItemKind::Todo, text)
    }

    fn blocker(conn: &Connection, d: &str, text: &str) -> Item {
        put(conn, d, ItemKind::Blocker, text)
    }

    fn set_status(conn: &Connection, item: &Item, status: ItemStatus) {
        let close_reason = (status == ItemStatus::Dropped).then_some(CloseReason::Dropped);
        item_table::update_fields(
            conn,
            item.id,
            &ItemChanges {
                status: Some(status),
                close_reason: Some(close_reason),
                ..ItemChanges::default()
            },
            now(),
        )
        .unwrap();
    }

    fn write(conn: &Connection, kind: TextDoc, d: &str, body: &str) {
        day::ensure(conn, date(d), &tokyo(), now()).unwrap();
        text_doc::save(conn, kind, date(d), body, now()).unwrap();
    }

    fn carry_all(conn: &Connection, from: &str, today: &str) {
        let decision = Decision {
            from: date(from),
            selection: Selection::All,
            reviews: Default::default(),
        };
        carryover::confirm(conn, date(today), &decision, &tokyo(), now()).unwrap();
    }

    /// 2026-11-02 に、どの件数にも当たる項目を作る：TODO（未完了2・完了1・取りやめ1・削除1）、
    /// 障害（未解消1・解消1）。
    fn fill_day(db: &Database) {
        let conn = db.connection();
        let d = "2026-11-02";
        todo(conn, d, "未完了1");
        todo(conn, d, "未完了2");
        let done = todo(conn, d, "完了");
        set_status(conn, &done, ItemStatus::Done);
        let dropped = todo(conn, d, "やめた");
        set_status(conn, &dropped, ItemStatus::Dropped);
        let deleted = todo(conn, d, "消した");
        delete(conn, deleted.id, now()).unwrap();
        blocker(conn, d, "未解消");
        let resolved = blocker(conn, d, "解消");
        set_status(conn, &resolved, ItemStatus::Resolved);
    }

    // ---- 件数 ----

    #[test]
    fn counts_follow_the_definitions() {
        let (db, _dir) = open();
        let conn = db.connection();
        fill_day(&db);
        let c = counts(conn, date("2026-11-02")).unwrap();
        assert_eq!(
            c,
            Counts {
                todos: 3,
                done: 1,
                left_todos: 2,
                left_blockers: 1,
                resolved: 1,
                carried: 0,
            }
        );
        assert_eq!(c.left(), 3);
        // 項目の無い日はすべて 0
        assert_eq!(counts(conn, date("2026-11-01")).unwrap(), Counts::default());
    }

    #[test]
    fn carried_items_count_on_both_days() {
        // 持ち越し数は持ち越してきた日に数え、前の日の残りには、持ち越した元の項目も入る
        // （その日の終わりの状態）
        let (db, _dir) = open();
        let conn = db.connection();
        fill_day(&db);
        carry_all(conn, "2026-11-02", "2026-11-03");
        let before = counts(conn, date("2026-11-02")).unwrap();
        assert_eq!((before.left(), before.carried), (3, 0));
        let today = counts(conn, date("2026-11-03")).unwrap();
        assert_eq!(
            today,
            Counts {
                todos: 2,
                left_todos: 2,
                left_blockers: 1,
                carried: 3,
                ..Counts::default()
            }
        );
        // 持ち越してきた項目は、完了しても持ち越し数に入る
        let copy = &item_table::list_by_date(conn, date("2026-11-03")).unwrap()[0];
        set_status(conn, copy, ItemStatus::Done);
        let today = counts(conn, date("2026-11-03")).unwrap();
        assert_eq!((today.done, today.left_todos, today.carried), (1, 1, 3));
    }

    #[test]
    fn counts_between_returns_only_days_with_items() {
        let (db, _dir) = open();
        let conn = db.connection();
        fill_day(&db);
        todo(conn, "2026-11-04", "先の日");
        todo(conn, "2026-11-06", "範囲の外");
        // ノートだけの日と、削除した項目だけの日は入らない
        write(conn, TextDoc::Note, "2026-11-03", "メモ");
        let gone = todo(conn, "2026-11-05", "消した");
        delete(conn, gone.id, now()).unwrap();

        let by_day = counts_between(conn, date("2026-11-01"), date("2026-11-05")).unwrap();
        assert_eq!(
            by_day.keys().copied().collect::<Vec<_>>(),
            [date("2026-11-02"), date("2026-11-04")]
        );
        assert_eq!(
            by_day[&date("2026-11-02")],
            counts(conn, date("2026-11-02")).unwrap()
        );
        assert_eq!(by_day[&date("2026-11-04")].left_todos, 1);
        assert!(
            counts_between(conn, date("2026-11-05"), date("2026-11-01"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn weeks_run_from_monday_to_friday() {
        // 2026-11-02 は月曜
        let mon_to_fri: Vec<_> = (2..=6).map(|d| date(&format!("2026-11-0{d}"))).collect();
        for d in 2..=8 {
            assert_eq!(
                week_days(date(&format!("2026-11-0{d}"))).to_vec(),
                mon_to_fri,
                "{d}"
            );
        }
        assert_eq!(week_days(date("2026-11-09"))[0], date("2026-11-09"));
        // 年をまたぐ週
        assert_eq!(
            week_days(date("2027-01-01")),
            [
                "2026-12-28",
                "2026-12-29",
                "2026-12-30",
                "2026-12-31",
                "2027-01-01"
            ]
            .map(date)
        );
        // 暦の端：0001-01-01 は月曜。9999-12-31 は金曜
        assert_eq!(week_days(LogicalDate::MIN)[0], LogicalDate::MIN);
        assert_eq!(week_days(LogicalDate::MAX).last(), Some(&LogicalDate::MAX));
        assert_eq!(week_days(LogicalDate::MAX).len(), 5);
    }

    #[test]
    fn week_counts_add_up_the_days() {
        let (db, _dir) = open();
        let conn = db.connection();
        fill_day(&db);
        carry_all(conn, "2026-11-02", "2026-11-03");
        // 土曜の項目は週（月〜金）に入らない
        todo(conn, "2026-11-07", "土曜");

        let week = week_counts(conn, date("2026-11-04")).unwrap();
        let mon = counts(conn, date("2026-11-02")).unwrap();
        let tue = counts(conn, date("2026-11-03")).unwrap();
        assert_eq!(
            week.days,
            [
                (date("2026-11-02"), mon),
                (date("2026-11-03"), tue),
                (date("2026-11-04"), Counts::default()),
                (date("2026-11-05"), Counts::default()),
                (date("2026-11-06"), Counts::default()),
            ]
        );
        assert_eq!(week.total, mon + tue);
        assert_eq!(
            (week.total.done, week.total.carried, week.total.resolved),
            (1, 3, 1)
        );
    }

    // ---- ボード ----

    fn texts(items: &[BoardItem]) -> Vec<&str> {
        items.iter().map(|i| i.item.text.as_str()).collect()
    }

    #[test]
    fn board_lists_items_in_order_with_where_they_were_carried() {
        let (db, _dir) = open();
        let conn = db.connection();
        fill_day(&db);
        carry_all(conn, "2026-11-02", "2026-11-03");

        let board = get_board(conn, date("2026-11-02")).unwrap();
        assert_eq!(board.date, date("2026-11-02"));
        // 取りやめた TODO も入り、削除した項目は入らない
        assert_eq!(
            texts(&board.todos),
            ["未完了1", "未完了2", "完了", "やめた"]
        );
        assert_eq!(texts(&board.blockers), ["未解消", "解消"]);
        let copies = item_table::list_by_date(conn, date("2026-11-03")).unwrap();
        let carried: Vec<_> = board
            .todos
            .iter()
            .chain(&board.blockers)
            .map(|i| {
                i.carried_to
                    .map(|c| (c.date, copies.iter().position(|x| x.id == c.id)))
            })
            .collect();
        let to = |n| Some((date("2026-11-03"), Some(n)));
        assert_eq!(carried, [to(0), to(1), None, None, to(2), None]);

        // 持ち越してきた日の項目は、持ち越し済みでない
        let today = get_board(conn, date("2026-11-03")).unwrap();
        assert!(today.todos.iter().all(|i| i.carried_to.is_none()));
        assert_eq!(today.todos[0].item.carry_count, 1);
    }

    #[test]
    fn open_board_keeps_only_open_items_including_carried_ones() {
        let (db, _dir) = open();
        let conn = db.connection();
        fill_day(&db);
        carry_all(conn, "2026-11-02", "2026-11-03");

        let open = open_board(conn, date("2026-11-02")).unwrap();
        assert_eq!(texts(&open.todos), ["未完了1", "未完了2"]);
        assert_eq!(texts(&open.blockers), ["未解消"]);
        assert!(
            open.todos
                .iter()
                .chain(&open.blockers)
                .all(|i| i.carried_to.is_some())
        );
        let board = get_board(conn, date("2026-11-02")).unwrap();
        let open_in_board: Vec<_> = board
            .todos
            .iter()
            .filter(|i| i.item.status == ItemStatus::Open)
            .cloned()
            .collect();
        assert_eq!(open.todos, open_in_board);
    }

    #[test]
    fn board_has_the_reflection_and_only_the_outline_of_the_note() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = "2026-11-03";
        let note = "朝のメモ\n## 10:00 定例\n議事\n## 良かったこと\n## 14:00  設計レビュー \n";
        write(conn, TextDoc::Note, d, note);
        write(
            conn,
            TextDoc::Reflection,
            d,
            "## 良かったこと\n早く終わった",
        );

        let board = get_board(conn, date(d)).unwrap();
        assert!(board.todos.is_empty() && board.blockers.is_empty());
        assert_eq!(board.reflection, "## 良かったこと\n早く終わった");
        let headings: Vec<_> = board
            .note
            .headings
            .iter()
            .map(|h| h.text.as_str())
            .collect();
        assert_eq!(headings, ["10:00 定例", "14:00  設計レビュー"]);
        // 長さは文字数（バイト数ではない）
        assert_eq!(board.note.length, note.chars().count());
        assert_ne!(board.note.length, note.len());
    }

    #[test]
    fn board_of_an_unwritten_day_is_empty_and_writes_nothing() {
        let (db, _dir) = open();
        let conn = db.connection();
        let board = get_board(conn, date("2026-11-03")).unwrap();
        assert_eq!(
            board,
            Board {
                date: date("2026-11-03"),
                todos: Vec::new(),
                blockers: Vec::new(),
                reflection: String::new(),
                note: NoteOutline::default(),
            }
        );
        let open = open_board(conn, date("2026-11-03")).unwrap();
        assert!(open.todos.is_empty() && open.blockers.is_empty());
        let days: i64 = conn
            .query_row("SELECT count(*) FROM day", [], |r| r.get(0))
            .unwrap();
        assert_eq!(days, 0);
    }

    // ---- Recent ----

    #[test]
    fn recent_days_are_today_and_the_latest_days_with_items_or_a_note() {
        let (db, _dir) = open();
        let conn = db.connection();
        todo(conn, "2026-10-20", "古い");
        todo(conn, "2026-10-25", "項目");
        write(conn, TextDoc::Note, "2026-10-27", "ノート");
        todo(conn, "2026-10-27", "項目とノートの日");
        write(conn, TextDoc::Note, "2026-10-28", "ノートだけ");
        // 入らない日：空白だけのノート、振り返りだけ、削除した項目だけ、今日より後
        write(conn, TextDoc::Note, "2026-10-29", " \n　");
        write(conn, TextDoc::Reflection, "2026-10-30", "振り返りだけ");
        let gone = todo(conn, "2026-10-31", "消した");
        delete(conn, gone.id, now()).unwrap();
        todo(conn, "2026-11-04", "先の日");

        let today = date("2026-11-03");
        assert_eq!(
            recent_days(conn, today, RECENT_DAYS).unwrap(),
            [
                "2026-11-03",
                "2026-10-28",
                "2026-10-27",
                "2026-10-25",
                "2026-10-20"
            ]
            .map(date)
        );
        // 上限は今日を含めて数える
        assert_eq!(
            recent_days(conn, today, 3).unwrap(),
            ["2026-11-03", "2026-10-28", "2026-10-27"].map(date)
        );
        assert_eq!(recent_days(conn, today, 1).unwrap(), [today]);
        assert!(recent_days(conn, today, 0).unwrap().is_empty());
        // 今日に項目があっても、今日は1回だけ
        todo(conn, "2026-11-03", "今日");
        assert_eq!(
            recent_days(conn, today, 2).unwrap(),
            ["2026-11-03", "2026-10-28"].map(date)
        );
    }

    #[test]
    fn recent_days_stop_at_the_limit_of_days_with_items() {
        // 項目のある日が上限より多くても、上限の日数だけを返す
        let (db, _dir) = open();
        let conn = db.connection();
        for d in 1..=20 {
            todo(conn, &format!("2026-10-{d:02}"), "項目");
        }
        let days = recent_days(conn, date("2026-11-03"), RECENT_DAYS).unwrap();
        assert_eq!(days.len(), RECENT_DAYS);
        assert_eq!(days[1], date("2026-10-20"));
        assert_eq!(days[6], date("2026-10-15"));
    }
}
