//! Markdown 書き出し（要件定義書 F-11・4章、アーキテクチャ設計書 6.4）。
//!
//! - [`day_markdown`]・[`markdown_between`]：1日・期間のボード（TODO・障害・振り返り・ノート）を、
//!   日ごとの Markdown の文字列にする。
//! - [`write_files`]：日ごとの文字列を、呼び出し側が選んだフォルダに `YYYY-MM-DD.md` として書く。
//!
//! 書き出すだけで、読み戻し（取り込み）はしない。データベースには書かない。

use std::collections::{BTreeMap, HashMap};
use std::fmt::{self, Write as _};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::{error, fs};

use rusqlite::Connection;

use super::item::{CarriedTo, CloseReason, Item, ItemKind, ItemStatus};
use super::logical_date::LogicalDate;
use crate::storage::table::item as item_table;
use crate::storage::table::text_doc::{self, TextDoc};
use crate::storage::{StorageError, Ulid, in_read_transaction};

/// 1日分の書き出し。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayMarkdown {
    pub date: LogicalDate,
    /// その日の Markdown（形はアーキテクチャ設計書 6.4）。改行は LF で、改行で終わる。
    pub markdown: String,
}

/// その日の Markdown。書き出すものが無い日（項目が無く、ノートと振り返りが空白だけ）は `None`。
pub fn day_markdown(conn: &Connection, date: LogicalDate) -> Result<Option<String>, StorageError> {
    Ok(markdown_between(conn, date, date)?
        .pop()
        .map(|day| day.markdown))
}

/// `from` から `to` まで（両端を含む）の日ごとの Markdown を、日付の順に返す。`from` が `to` より
/// 後なら、入れ替えた期間とする。書き出すものが無い日は入れない。
///
/// 期間のすべての日を、同じ時点の内容で読む（途中でほかの接続が書いても食い違わない）。
pub fn markdown_between(
    conn: &Connection,
    from: LogicalDate,
    to: LogicalDate,
) -> Result<Vec<DayMarkdown>, StorageError> {
    let (from, to) = if from <= to { (from, to) } else { (to, from) };
    let days = in_read_transaction(conn, || read_days(conn, from, to))?;
    Ok(days
        .into_iter()
        .filter_map(|(date, day)| {
            day.render(date)
                .map(|markdown| DayMarkdown { date, markdown })
        })
        .collect())
}

/// 1日分の読んだ内容。
#[derive(Default)]
struct Day {
    /// その日の項目（並び順）と、それぞれの持ち越し元・持ち越し先の日付。
    items: Vec<(Item, Option<LogicalDate>, Option<CarriedTo>)>,
    note: String,
    reflection: String,
}

fn read_days(
    conn: &Connection,
    from: LogicalDate,
    to: LogicalDate,
) -> Result<BTreeMap<LogicalDate, Day>, StorageError> {
    let items = item_table::list_between(conn, from, to)?;
    let ids: Vec<Ulid> = items.iter().map(|i| i.id).collect();
    let carried_to = item_table::carried_to_many(conn, &ids)?;
    // 持ち越し元は、削除されていてもその日付を示す（持ち越してきたことは変わらないため）
    let source_ids: Vec<Ulid> = items.iter().filter_map(|i| i.carried_from_id).collect();
    let sources: HashMap<Ulid, LogicalDate> = item_table::get_many(conn, &source_ids)?
        .into_iter()
        .map(|(id, item)| (id, item.date))
        .collect();

    let mut days: BTreeMap<LogicalDate, Day> = BTreeMap::new();
    for item in items {
        let from = item
            .carried_from_id
            .and_then(|id| sources.get(&id).copied());
        let to = carried_to.get(&item.id).copied();
        days.entry(item.date)
            .or_default()
            .items
            .push((item, from, to));
    }
    // 空白だけの本文は、書いていないものとして扱う（その日を書き出すかの判定にも入れない）
    for doc in [TextDoc::Note, TextDoc::Reflection] {
        for row in text_doc::list_between(conn, doc, from, to)? {
            if row.body.trim().is_empty() {
                continue;
            }
            let day = days.entry(row.date).or_default();
            match doc {
                TextDoc::Note => day.note = row.body,
                TextDoc::Reflection => day.reflection = row.body,
            }
        }
    }
    Ok(days)
}

impl Day {
    /// Markdown にする。書き出すものが無ければ `None`。
    fn render(&self, date: LogicalDate) -> Option<String> {
        if self.items.is_empty() && self.note.is_empty() && self.reflection.is_empty() {
            return None;
        }
        let mut out = format!("# {date}\n");
        for (kind, title) in [(ItemKind::Todo, "TODO"), (ItemKind::Blocker, "Blockers")] {
            let mut items = self
                .items
                .iter()
                .filter(|(i, ..)| i.kind == kind)
                .peekable();
            if items.peek().is_none() {
                continue;
            }
            write!(out, "\n## {title}\n\n").unwrap();
            for (item, from, to) in items {
                push_item(&mut out, item, *from, *to);
            }
        }
        for (title, body) in [("Reflection", &self.reflection), ("Notes", &self.note)] {
            if !body.is_empty() {
                write!(out, "\n## {title}\n\n{}\n", trim_body(body)).unwrap();
            }
        }
        Some(out)
    }
}

/// 項目の1行（と、解消した障害の解消メモの行）。
fn push_item(out: &mut String, item: &Item, from: Option<LogicalDate>, to: Option<CarriedTo>) {
    let text = one_line(&item.text);
    match item.status {
        ItemStatus::Open => write!(out, "- [ ] {text}"),
        ItemStatus::Done | ItemStatus::Resolved => write!(out, "- [x] {text}"),
        ItemStatus::Dropped => write!(out, "- ~~{text}~~"),
    }
    .unwrap();
    let mut notes = Vec::new();
    if let Some(from) = from {
        notes.push(format!("carried over from {from}"));
    }
    if item.status == ItemStatus::Dropped {
        notes.push(
            match item.close_reason {
                Some(CloseReason::Split) => "split",
                Some(CloseReason::ReturnedExternal) => "returned to external tool",
                Some(CloseReason::Dropped) | None => "dropped",
            }
            .to_owned(),
        );
    }
    if let Some(to) = to {
        notes.push(format!("carried over to {}", to.date));
    }
    if !notes.is_empty() {
        write!(out, " ({})", notes.join("; ")).unwrap();
    }
    out.push('\n');
    // 解消メモは、解消した障害のものだけを出す（未解消に戻した障害に残るものは、画面にも出ない。
    // 画面設計書 3章）
    if item.status == ItemStatus::Resolved
        && let Some(note) = item.resolution_note.as_deref().map(str::trim)
        && !note.is_empty()
    {
        writeln!(out, "  - Resolution: {}", one_line(note)).unwrap();
    }
}

/// 項目の本文・解消メモを1行にする（コアは改行を含む本文を保存しないが、壊れたデータでも、
/// 次の行の Markdown を崩さないため）。
fn one_line(s: &str) -> String {
    s.replace(['\r', '\n'], " ")
}

/// ノート・振り返りの本文の、先頭の空白だけの行と、末尾の空白と改行を除く（ほかはそのまま）。
fn trim_body(body: &str) -> &str {
    let body = body.trim_end();
    // 空白だけの本文はここに来ない。最初に空白以外の文字がある行の先頭から
    let first = body
        .find(|c: char| !c.is_whitespace())
        .unwrap_or(body.len());
    let start = body[..first].rfind('\n').map_or(0, |i| i + 1);
    &body[start..]
}

/// その日の書き出しのファイル名（`YYYY-MM-DD.md`）。
pub fn file_name(date: LogicalDate) -> String {
    format!("{date}.md")
}

/// 書き出しのファイルを書けなかった理由。
#[derive(Debug)]
pub enum ExportError {
    /// 書き出し先が絶対パスでない。何も書いていない。
    RelativeDir { dir: PathBuf },
    /// ファイル `path` を書けなかった。それより前の日のファイルは書き終えている。
    Write { path: PathBuf, source: io::Error },
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RelativeDir { dir } => write!(
                f,
                "the export folder must be an absolute path: {}",
                dir.display()
            ),
            Self::Write { path, .. } => write!(f, "failed to write {}", path.display()),
        }
    }
}

impl error::Error for ExportError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::RelativeDir { .. } => None,
            Self::Write { source, .. } => Some(source),
        }
    }
}

/// `days` を、フォルダ `dir` に日ごとのファイル（[`file_name`]）として、日付の順に書き、書いたファイルの
/// パスを返す。同じ名前のファイルは置き換える。フォルダは作らない（呼び出し側が選んだ、あるフォルダに書く）。
///
/// 1つずつ、同じフォルダの一時ファイルに書いてディスクに反映してから名前を変える（同期のツールや
/// エディタが、書きかけのファイルを読まないように）。書けなかったら、その一時ファイルを消して止める
/// （それより前に書いたファイルは残す）。
pub fn write_files(dir: &Path, days: &[DayMarkdown]) -> Result<Vec<PathBuf>, ExportError> {
    if !dir.is_absolute() {
        return Err(ExportError::RelativeDir {
            dir: dir.to_path_buf(),
        });
    }
    let mut written = Vec::with_capacity(days.len());
    for day in days {
        let name = file_name(day.date);
        let path = dir.join(&name);
        write_replacing(dir, &name, &path, day.markdown.as_bytes()).map_err(|source| {
            ExportError::Write {
                path: path.clone(),
                source,
            }
        })?;
        written.push(path);
    }
    Ok(written)
}

/// `bytes` を一時ファイルに書いて反映し、`path` に名前を変える。失敗したら一時ファイルを消す。
fn write_replacing(dir: &Path, name: &str, path: &Path, bytes: &[u8]) -> io::Result<()> {
    // 一時ファイルの名前は、ほかの書き出しと重ならないよう ULID を付け、隠しファイルにする
    let tmp = dir.join(format!(".{name}.{}.tmp", Ulid::generate()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use jiff::tz::TimeZone;

    use super::*;
    use crate::domain::carryover::{self, Decision, Selection};
    use crate::domain::day;
    use crate::domain::item::{self, Position};
    use crate::storage::table::item::ItemChanges;
    use crate::storage::table::test_support::{at, date, open};

    fn tokyo() -> TimeZone {
        TimeZone::get("Asia/Tokyo").unwrap()
    }

    fn now() -> Timestamp {
        at("2026-11-03T01:00:00Z")
    }

    fn put(conn: &Connection, d: &str, kind: ItemKind, text: &str) -> Item {
        item::add(conn, date(d), kind, text, Position::Last, &tokyo(), now()).unwrap()
    }

    fn change(conn: &Connection, item: &Item, changes: ItemChanges) {
        item_table::update_fields(conn, item.id, &changes, now()).unwrap();
    }

    fn status(conn: &Connection, item: &Item, status: ItemStatus, reason: Option<CloseReason>) {
        change(
            conn,
            item,
            ItemChanges {
                status: Some(status),
                close_reason: Some(reason),
                ..ItemChanges::default()
            },
        );
    }

    fn write(conn: &Connection, doc: TextDoc, d: &str, body: &str) {
        day::ensure(conn, date(d), &tokyo(), now()).unwrap();
        text_doc::save(conn, doc, date(d), body, now()).unwrap();
    }

    fn carry_all(conn: &Connection, from: &str, today: &str) {
        let decision = Decision {
            from: date(from),
            selection: Selection::All,
            reviews: Default::default(),
        };
        carryover::confirm(conn, date(today), &decision, &tokyo(), now()).unwrap();
    }

    #[test]
    fn a_day_shows_every_part_of_the_board() {
        let (db, _dir) = open();
        let conn = db.connection();
        let d = "2026-11-02";
        put(conn, d, ItemKind::Todo, "未完了");
        let done = put(conn, d, ItemKind::Todo, "完了");
        status(conn, &done, ItemStatus::Done, None);
        let dropped = put(conn, d, ItemKind::Todo, "やめた");
        status(
            conn,
            &dropped,
            ItemStatus::Dropped,
            Some(CloseReason::Dropped),
        );
        let split = put(conn, d, ItemKind::Todo, "分けた");
        status(conn, &split, ItemStatus::Dropped, Some(CloseReason::Split));
        let returned = put(conn, d, ItemKind::Todo, "戻した");
        status(
            conn,
            &returned,
            ItemStatus::Dropped,
            Some(CloseReason::ReturnedExternal),
        );
        let deleted = put(conn, d, ItemKind::Todo, "消した");
        item::delete(conn, deleted.id, now()).unwrap();
        put(conn, d, ItemKind::Blocker, "未解消");
        let resolved = put(conn, d, ItemKind::Blocker, "解消");
        change(
            conn,
            &resolved,
            ItemChanges {
                status: Some(ItemStatus::Resolved),
                resolution_note: Some(Some(" 権限をもらった ".into())),
                ..ItemChanges::default()
            },
        );
        // 未解消に戻した障害に残る解消メモは出さない
        let reopened = put(conn, d, ItemKind::Blocker, "戻した障害");
        change(
            conn,
            &reopened,
            ItemChanges {
                resolution_note: Some(Some("古いメモ".into())),
                ..ItemChanges::default()
            },
        );
        write(conn, TextDoc::Reflection, d, "\n  \n良かった\n\n");
        write(conn, TextDoc::Note, d, "## 10:00 定例\n  - 字下げ  \n");

        assert_eq!(
            day_markdown(conn, date(d)).unwrap().unwrap(),
            "# 2026-11-02\n\
             \n\
             ## TODO\n\
             \n\
             - [ ] 未完了\n\
             - [x] 完了\n\
             - ~~やめた~~ (dropped)\n\
             - ~~分けた~~ (split)\n\
             - ~~戻した~~ (returned to external tool)\n\
             \n\
             ## Blockers\n\
             \n\
             - [ ] 未解消\n\
             - [x] 解消\n\
             \x20 - Resolution: 権限をもらった\n\
             - [ ] 戻した障害\n\
             \n\
             ## Reflection\n\
             \n\
             良かった\n\
             \n\
             ## Notes\n\
             \n\
             ## 10:00 定例\n\
             \x20 - 字下げ\n"
        );
    }

    #[test]
    fn empty_parts_and_empty_days_are_left_out() {
        let (db, _dir) = open();
        let conn = db.connection();
        // 項目もノートも振り返りも無い日、空白だけのノートの日、削除した項目だけの日は書き出さない
        write(conn, TextDoc::Note, "2026-11-02", " \n　\t");
        let gone = put(conn, "2026-11-03", ItemKind::Todo, "消した");
        item::delete(conn, gone.id, now()).unwrap();
        for d in ["2026-11-01", "2026-11-02", "2026-11-03"] {
            assert_eq!(day_markdown(conn, date(d)).unwrap(), None, "{d}");
        }
        // 障害だけの日は、TODO の節を出さない
        put(conn, "2026-11-04", ItemKind::Blocker, "待ち");
        assert_eq!(
            day_markdown(conn, date("2026-11-04")).unwrap().unwrap(),
            "# 2026-11-04\n\n## Blockers\n\n- [ ] 待ち\n"
        );
        // ノートだけの日
        write(conn, TextDoc::Note, "2026-11-05", "メモ");
        assert_eq!(
            day_markdown(conn, date("2026-11-05")).unwrap().unwrap(),
            "# 2026-11-05\n\n## Notes\n\nメモ\n"
        );
    }

    #[test]
    fn carried_items_show_where_they_came_from_and_went() {
        let (db, _dir) = open();
        let conn = db.connection();
        put(conn, "2026-11-02", ItemKind::Todo, "続ける");
        put(conn, "2026-11-02", ItemKind::Blocker, "待ち");
        carry_all(conn, "2026-11-02", "2026-11-03");
        carry_all(conn, "2026-11-03", "2026-11-04");
        let days = markdown_between(conn, date("2026-11-02"), date("2026-11-04")).unwrap();
        let text: Vec<_> = days.iter().map(|d| d.markdown.as_str()).collect();
        assert_eq!(
            text,
            [
                "# 2026-11-02\n\n## TODO\n\n- [ ] 続ける (carried over to 2026-11-03)\n\n\
                 ## Blockers\n\n- [ ] 待ち (carried over to 2026-11-03)\n",
                "# 2026-11-03\n\n## TODO\n\n\
                 - [ ] 続ける (carried over from 2026-11-02; carried over to 2026-11-04)\n\n\
                 ## Blockers\n\n\
                 - [ ] 待ち (carried over from 2026-11-02; carried over to 2026-11-04)\n",
                "# 2026-11-04\n\n## TODO\n\n- [ ] 続ける (carried over from 2026-11-03)\n\n\
                 ## Blockers\n\n- [ ] 待ち (carried over from 2026-11-03)\n",
            ]
        );
        // 間の日の持ち越し先を消すと、元の日は消した先を飛ばした先へ持ち越し済みのまま（データモデル
        // 設計書 3章 item）。持ち越し先の側は、消した持ち越し元の日付を示し続ける
        let middle = item_table::list_by_date(conn, date("2026-11-03")).unwrap();
        for i in &middle {
            item::delete(conn, i.id, now()).unwrap();
        }
        assert_eq!(
            day_markdown(conn, date("2026-11-02")).unwrap().unwrap(),
            "# 2026-11-02\n\n## TODO\n\n- [ ] 続ける (carried over to 2026-11-04)\n\n\
             ## Blockers\n\n- [ ] 待ち (carried over to 2026-11-04)\n"
        );
        assert!(
            day_markdown(conn, date("2026-11-04"))
                .unwrap()
                .unwrap()
                .contains("- [ ] 続ける (carried over from 2026-11-03)\n")
        );
    }

    #[test]
    fn a_range_lists_days_in_order_and_accepts_reversed_ends() {
        let (db, _dir) = open();
        let conn = db.connection();
        put(conn, "2026-11-05", ItemKind::Todo, "範囲の外");
        put(conn, "2026-11-04", ItemKind::Todo, "四日");
        write(conn, TextDoc::Reflection, "2026-11-02", "振り返りだけ");
        put(conn, "2026-10-31", ItemKind::Todo, "範囲の外");
        let dates = |from, to| -> Vec<String> {
            markdown_between(conn, date(from), date(to))
                .unwrap()
                .into_iter()
                .map(|d| d.date.to_string())
                .collect()
        };
        assert_eq!(
            dates("2026-11-01", "2026-11-04"),
            ["2026-11-02", "2026-11-04"]
        );
        assert_eq!(
            dates("2026-11-04", "2026-11-01"),
            ["2026-11-02", "2026-11-04"]
        );
        assert!(dates("2026-11-06", "2026-12-31").is_empty());
        assert_eq!(
            markdown_between(conn, date("2026-11-04"), date("2026-11-04")).unwrap(),
            [DayMarkdown {
                date: date("2026-11-04"),
                markdown: day_markdown(conn, date("2026-11-04")).unwrap().unwrap(),
            }]
        );
    }

    #[test]
    fn broken_multiline_text_stays_on_one_line() {
        let (db, _dir) = open();
        let conn = db.connection();
        let todo = put(conn, "2026-11-02", ItemKind::Todo, "x");
        // コアは改行を含む本文を保存しないので、表に直接書く
        change(
            conn,
            &todo,
            ItemChanges {
                text: Some("一\n二\r三".into()),
                ..ItemChanges::default()
            },
        );
        assert_eq!(
            day_markdown(conn, date("2026-11-02")).unwrap().unwrap(),
            "# 2026-11-02\n\n## TODO\n\n- [ ] 一 二 三\n"
        );
    }

    #[test]
    fn trim_body_keeps_the_indent_of_the_first_line() {
        assert_eq!(trim_body("\n \n  コード\n本文 \n\n"), "  コード\n本文");
        assert_eq!(trim_body("本文"), "本文");
        assert_eq!(trim_body("\r\n本文\r\n"), "本文");
    }

    fn day(d: &str, markdown: &str) -> DayMarkdown {
        DayMarkdown {
            date: date(d),
            markdown: markdown.into(),
        }
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn writes_one_file_per_day_and_replaces_old_ones() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("2026-11-02.md"), "古い書き出し").unwrap();
        fs::write(dir.path().join("memo.txt"), "利用者のファイル").unwrap();
        let paths = write_files(
            dir.path(),
            &[
                day("2026-11-02", "# 2026-11-02\n"),
                day("2026-11-03", "# 三日\n"),
            ],
        )
        .unwrap();
        assert_eq!(
            paths,
            [
                dir.path().join("2026-11-02.md"),
                dir.path().join("2026-11-03.md")
            ]
        );
        assert_eq!(fs::read_to_string(&paths[0]).unwrap(), "# 2026-11-02\n");
        assert_eq!(fs::read_to_string(&paths[1]).unwrap(), "# 三日\n");
        // 一時ファイルは残らず、ほかのファイルには触れない
        assert_eq!(
            leftovers(dir.path()),
            ["2026-11-02.md", "2026-11-03.md", "memo.txt"]
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("memo.txt")).unwrap(),
            "利用者のファイル"
        );
        assert_eq!(write_files(dir.path(), &[]).unwrap(), Vec::<PathBuf>::new());
    }

    #[test]
    fn write_failures_name_the_file_and_keep_earlier_days() {
        let dir = tempfile::tempdir().unwrap();
        // 2日目の名前のフォルダがあると、置き換えられない
        fs::create_dir(dir.path().join("2026-11-03.md")).unwrap();
        fs::write(dir.path().join("2026-11-03.md").join("x"), "").unwrap();
        let err = write_files(
            dir.path(),
            &[
                day("2026-11-02", "二日\n"),
                day("2026-11-03", "三日\n"),
                day("2026-11-04", "四日\n"),
            ],
        )
        .unwrap_err();
        let ExportError::Write { path, .. } = &err else {
            panic!("{err:?}");
        };
        assert_eq!(path, &dir.path().join("2026-11-03.md"));
        assert!(error::Error::source(&err).is_some());
        assert_eq!(leftovers(dir.path()), ["2026-11-02.md", "2026-11-03.md"]);

        // 無いフォルダには書かない（作らない）
        let missing = dir.path().join("無い");
        assert!(matches!(
            write_files(&missing, &[day("2026-11-02", "二日\n")]),
            Err(ExportError::Write { .. })
        ));
        assert!(!missing.exists());
    }

    #[test]
    fn relative_folders_are_refused() {
        let err = write_files(Path::new("exports"), &[day("2026-11-02", "二日\n")]).unwrap_err();
        assert!(matches!(err, ExportError::RelativeDir { .. }));
        assert!(!Path::new("exports").exists());
    }
}
