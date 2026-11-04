//! ふりかえり（S-08 Archive）に出す内容の読み取り（画面設計書 9章）。
//!
//! 週の件数（This week と週のカード。[`board::week_counts`]）、記録のある直近の日（Daily notes）、
//! 選んだ日のプレビュー（数字・振り返りの全文・ノートの時刻付き見出しと各見出しの最初の行）を、
//! 同じ時点の内容として読む。読むだけで、day の行も作らない。
//!
//! 振り返りの GOOD / STUCK の1行（見出しの文字から探す）は、見出しの文字が画面の言語のテンプレート
//! （`src/lib/i18n.ts`）と同じ所で決まるので、本文のまま返して画面が取り出す。

use rusqlite::Connection;

use super::board::{self, Counts, WeekCounts};
use super::day;
use super::logical_date::LogicalDate;
use super::note_body;
use crate::storage::table::text_doc::{self, TextDoc};
use crate::storage::{StorageError, in_read_transaction};

/// Daily notes に並べる日の数の上限（画面設計書 9章）。
pub const DAILY_DAYS: usize = 8;

/// プレビューのノートで、時刻付き見出しごとに出す行の数（空白だけの行は数えない）。
pub const NOTE_PREVIEW_LINES: usize = 2;

/// Daily notes の1日。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyEntry {
    pub date: LogicalDate,
    pub counts: Counts,
    /// 振り返りの本文（まだ書いていない日は空）。
    pub reflection: String,
}

/// プレビューのノートの1区間：時刻付き見出しと、その下の最初の行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotePreview {
    pub hour: u8,
    pub minute: u8,
    /// 件名（件名の無い見出しでは空）。
    pub title: String,
    /// 見出しの次の行から、空白だけの行を除いて [`NOTE_PREVIEW_LINES`] 行まで（行末の空白は除く）。
    pub lines: Vec<String>,
}

/// 選んだ日のプレビュー。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayPreview {
    pub counts: Counts,
    /// 振り返りの全文（まだ書いていない日は空）。
    pub reflection: String,
    /// ノートの時刻付き見出し（上から順）。最初の見出しより前の前置きは入れない。
    pub note: Vec<NotePreview>,
}

/// ふりかえり画面の内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archive {
    /// 選んだ日（今日より後は今日にする。指定が無ければ [`default_day`]）。
    pub date: LogicalDate,
    /// 今日の論理日付。
    pub today: LogicalDate,
    /// 選んだ日を含む週（月〜金）の件数。
    pub week: WeekCounts,
    /// 記録のある直近の日（[`records_upto`]。今日を含む）。新しい順に [`DAILY_DAYS`] 日まで。
    pub daily: Vec<DailyEntry>,
    pub preview: DayPreview,
}

/// `upto`（その日を含む）までで、記録のある直近の日を、新しい順に `limit` 日まで返す。記録のある日は、
/// 削除されていない項目がある日、空白以外の文字を含むノートがある日、空白以外の文字を含む振り返りが
/// ある日（2026-11-04 決定。サイドバーの Recent は振り返りだけの日を入れないが、見返す画面では
/// 振り返りだけを書いた日も見返したいため）。
pub fn records_upto(
    conn: &Connection,
    upto: LogicalDate,
    limit: usize,
) -> Result<Vec<LogicalDate>, StorageError> {
    in_read_transaction(conn, || {
        // 下の読み取りは「その日より前」なので、翌日を境にする。暦の最後の日なら境にできる日が無いので、
        // その日より前を読み、その日だけを別に確かめる（実際には使わない日付）
        let Some(before) = upto.add_days(1) else {
            let mut days = records_before(conn, upto, limit)?;
            let mut on_last = board::counts(conn, upto)? != Counts::default();
            for doc in [TextDoc::Note, TextDoc::Reflection] {
                on_last |= !body_of(conn, doc, upto)?.trim().is_empty();
            }
            if on_last && limit > 0 {
                days.insert(0, upto);
                days.truncate(limit);
            }
            return Ok(days);
        };
        records_before(conn, before, limit)
    })
}

/// `before` より前の、記録のある日（[`records_upto`] の記録）を新しい順に `limit` 日まで。
fn records_before(
    conn: &Connection,
    before: LogicalDate,
    limit: usize,
) -> Result<Vec<LogicalDate>, StorageError> {
    let mut days = day::days_with_items_or_note(conn, before, limit)?;
    days.extend(text_doc::nonblank_dates(
        conn,
        TextDoc::Reflection,
        before,
        None,
        limit,
    )?);
    days.sort_unstable_by(|a, b| b.cmp(a));
    days.dedup();
    days.truncate(limit);
    Ok(days)
}

/// ふりかえりを開いたとき最初に選ぶ日：今日より前で、記録のある最も新しい日。無ければ今日。
pub fn default_day(conn: &Connection, today: LogicalDate) -> Result<LogicalDate, StorageError> {
    let Some(yesterday) = today.add_days(-1) else {
        return Ok(today);
    };
    Ok(records_upto(conn, yesterday, 1)?
        .first()
        .copied()
        .unwrap_or(today))
}

fn body_of(conn: &Connection, doc: TextDoc, date: LogicalDate) -> Result<String, StorageError> {
    Ok(text_doc::get(conn, doc, date)?
        .map(|row| row.body)
        .unwrap_or_default())
}

/// 選んだ日 `date`（`None` なら [`default_day`]。今日より後は今日にする）のふりかえり画面の内容。
/// `today` は今日の論理日付。どれも同じ時点の内容を読む。
pub fn get_archive(
    conn: &Connection,
    date: Option<LogicalDate>,
    today: LogicalDate,
) -> Result<Archive, StorageError> {
    in_read_transaction(conn, || {
        let date = match date {
            Some(d) => d.min(today),
            None => default_day(conn, today)?,
        };
        let week = board::week_counts(conn, date)?;
        let days = records_upto(conn, today, DAILY_DAYS)?;
        // 件数は、Daily notes の日と選んだ日をまとめて数える
        let oldest = days.last().copied().map_or(date, |d| d.min(date));
        let by_day = board::counts_between(conn, oldest, today.max(date))?;
        let counts_of = |d: LogicalDate| by_day.get(&d).copied().unwrap_or_default();
        let daily = days
            .into_iter()
            .map(|d| {
                Ok(DailyEntry {
                    date: d,
                    counts: counts_of(d),
                    reflection: body_of(conn, TextDoc::Reflection, d)?,
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        let note_body = body_of(conn, TextDoc::Note, date)?;
        let note = note_body::sections(&note_body)
            .into_iter()
            .filter_map(|section| {
                let heading = section.heading.clone()?;
                let lines = section
                    .lines_in(&note_body)
                    .filter(|line| !line.trim().is_empty())
                    .take(NOTE_PREVIEW_LINES)
                    .map(|line| line.trim_end().to_owned())
                    .collect();
                Some(NotePreview {
                    hour: heading.hour,
                    minute: heading.minute,
                    title: heading.title,
                    lines,
                })
            })
            .collect();
        let preview = DayPreview {
            counts: counts_of(date),
            reflection: body_of(conn, TextDoc::Reflection, date)?,
            note,
        };
        Ok(Archive {
            date,
            today,
            week,
            daily,
            preview,
        })
    })
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use jiff::tz::TimeZone;

    use super::*;
    use crate::domain::item::ItemKind;
    use crate::domain::item::{self, Position};
    use crate::storage::table::test_support::{at, date, open};

    fn tokyo() -> TimeZone {
        TimeZone::get("Asia/Tokyo").unwrap()
    }

    fn now() -> Timestamp {
        at("2026-11-03T01:00:00Z")
    }

    fn todo(conn: &Connection, d: &str, text: &str) {
        item::add(
            conn,
            date(d),
            ItemKind::Todo,
            text,
            Position::Last,
            &tokyo(),
            now(),
        )
        .unwrap();
    }

    fn write(conn: &Connection, doc: TextDoc, d: &str, body: &str) {
        day::ensure(conn, date(d), &tokyo(), now()).unwrap();
        text_doc::save(conn, doc, date(d), body, now()).unwrap();
    }

    #[test]
    fn records_include_reflection_only_days_up_to_the_given_day() {
        let (db, _dir) = open();
        let conn = db.connection();
        todo(conn, "2026-11-02", "項目の日");
        write(conn, TextDoc::Note, "2026-10-30", "ノートの日");
        write(
            conn,
            TextDoc::Reflection,
            "2026-10-29",
            "## 良かったこと\n- 振り返りだけの日",
        );
        // 空白だけの本文と、上限より先の日、記録の無い日は入らない
        write(conn, TextDoc::Note, "2026-10-28", " \n\t\n");
        write(conn, TextDoc::Reflection, "2026-11-05", "先の日");
        let upto = date("2026-11-03");
        assert_eq!(
            records_upto(conn, upto, 8).unwrap(),
            [date("2026-11-02"), date("2026-10-30"), date("2026-10-29")]
        );
        assert_eq!(records_upto(conn, upto, 2).unwrap().len(), 2);
        // 指定した日を含む
        assert_eq!(
            records_upto(conn, date("2026-11-02"), 1).unwrap(),
            [date("2026-11-02")]
        );
        // 今日が最初に選ぶ日になるのは、記録のある前の日が無いときだけ
        assert_eq!(default_day(conn, upto).unwrap(), date("2026-11-02"));
        assert_eq!(
            default_day(conn, date("2026-10-01")).unwrap(),
            date("2026-10-01")
        );
    }

    #[test]
    fn records_work_on_the_last_day_of_the_calendar() {
        let (db, _dir) = open();
        let conn = db.connection();
        todo(conn, "9999-12-30", "前の日");
        let last = date("9999-12-31");
        assert_eq!(records_upto(conn, last, 8).unwrap(), [date("9999-12-30")]);
        write(conn, TextDoc::Reflection, "9999-12-31", "最後の日");
        assert_eq!(
            records_upto(conn, last, 8).unwrap(),
            [last, date("9999-12-30")]
        );
    }

    #[test]
    fn archive_reads_the_week_the_daily_days_and_the_preview() {
        let (db, _dir) = open();
        let conn = db.connection();
        // 2026-11-02 は月曜、11-07 は土曜
        todo(conn, "2026-11-02", "月曜");
        todo(conn, "2026-11-03", "火曜");
        todo(conn, "2026-11-07", "土曜");
        let note =
            "前置き\n## 10:00 定例\n\n議事1\n議事2\n議事3\n## 9:30\n## 14:00　設計 \n- 案  \n";
        write(conn, TextDoc::Note, "2026-11-03", note);
        write(
            conn,
            TextDoc::Reflection,
            "2026-11-03",
            "## 良かったこと\n- 早く終わった",
        );

        let today = date("2026-11-04");
        let a = get_archive(conn, Some(date("2026-11-03")), today).unwrap();
        assert_eq!((a.date, a.today), (date("2026-11-03"), today));
        assert_eq!(a.week.days.len(), 5);
        assert_eq!(a.week.days[0].0, date("2026-11-02"));
        // 土曜の項目は週の合計に入らない
        assert_eq!(a.week.total.todos, 2);
        assert_eq!(
            a.daily.iter().map(|d| d.date).collect::<Vec<_>>(),
            // 今日（11-04）より後の土曜は入らない
            [date("2026-11-03"), date("2026-11-02")]
        );
        assert_eq!(a.daily[0].reflection, "## 良かったこと\n- 早く終わった");
        assert_eq!(a.daily[0].counts.todos, 1);
        assert_eq!(a.preview.counts.todos, 1);
        assert_eq!(a.preview.reflection, "## 良かったこと\n- 早く終わった");
        let sections: Vec<_> = a
            .preview
            .note
            .iter()
            .map(|n| (n.hour, n.minute, n.title.as_str(), n.lines.clone()))
            .collect();
        assert_eq!(
            sections,
            [
                (10, 0, "定例", vec!["議事1".to_owned(), "議事2".to_owned()]),
                (9, 30, "", vec![]),
                (14, 0, "設計", vec!["- 案".to_owned()]),
            ]
        );
    }

    #[test]
    fn archive_clamps_future_days_and_picks_the_previous_day_by_default() {
        let (db, _dir) = open();
        let conn = db.connection();
        todo(conn, "2026-11-02", "前の日");
        let today = date("2026-11-04");
        assert_eq!(
            get_archive(conn, None, today).unwrap().date,
            date("2026-11-02")
        );
        assert_eq!(
            get_archive(conn, Some(date("2026-12-01")), today)
                .unwrap()
                .date,
            today
        );
        // 記録の無い日も選べる（数字は 0、振り返りとノートは空）
        let a = get_archive(conn, Some(date("2026-11-03")), today).unwrap();
        assert_eq!(a.preview.counts, Counts::default());
        assert!(a.preview.reflection.is_empty() && a.preview.note.is_empty());
        // 何も書いていない日を読んでも day の行は作らない
        let days: i64 = conn
            .query_row(
                "SELECT count(*) FROM day WHERE date = '2026-11-03'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(days, 0);
    }
}
