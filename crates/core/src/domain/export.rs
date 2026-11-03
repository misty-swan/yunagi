//! Markdown 書き出し（要件定義書 F-11・4章、アーキテクチャ設計書 6.4）。
//!
//! - [`read_between`]・[`Boards::render`]：1日・期間のボード（TODO・障害・振り返り・ノート）を読み、
//!   日ごとの Markdown の文字列にする（[`markdown_between`]・[`day_markdown`] は両方を続けて行う）。
//!   データベースを使うのは読む間だけで、文字列にするのは読んだ後（接続を手放してから）でよい。
//! - [`write_files`]：日ごとの文字列を、呼び出し側が選んだフォルダに `YYYY-MM-DD.md` として書く。
//!
//! 書き出すだけで、読み戻し（取り込み）はしない。データベースには書かない。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::{self, Write as _};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{error, fs, thread};

use rusqlite::Connection;

use super::board::{self, BoardItem};
use super::item::{CloseReason, ItemKind, ItemStatus};
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

/// `from` から `to` まで（両端を含む）の日ごとの Markdown（[`read_between`] して [`Boards::render`]）。
pub fn markdown_between(
    conn: &Connection,
    from: LogicalDate,
    to: LogicalDate,
) -> Result<Vec<DayMarkdown>, StorageError> {
    Ok(read_between(conn, from, to)?.render())
}

/// 書き出すために読んだ、期間の日ごとのボード（[`read_between`]）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Boards {
    /// 書き出すものがある日だけ。
    days: BTreeMap<LogicalDate, Day>,
}

/// 1日分の読んだ内容。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Day {
    /// その日の項目（並び順。持ち越し先付き）と、それぞれの持ち越し元の項目の日付。
    items: Vec<(BoardItem, Option<LogicalDate>)>,
    /// ノート・振り返りの本文。空白だけの本文は空にする。
    note: String,
    reflection: String,
}

/// `from` から `to` まで（両端を含む）の日のうち、書き出すものがある日（削除されていない項目があるか、
/// ノートか振り返りが空白以外の文字を含む日）のボードを読む。`from` が `to` より後なら、入れ替えた
/// 期間とする。
///
/// 期間のすべての日を、同じ時点の内容で読む（途中でほかの接続が書いても食い違わない）。
pub fn read_between(
    conn: &Connection,
    from: LogicalDate,
    to: LogicalDate,
) -> Result<Boards, StorageError> {
    let (from, to) = if from <= to { (from, to) } else { (to, from) };
    in_read_transaction(conn, || {
        let items = board::with_carried_to(conn, item_table::list_between(conn, from, to)?)?;
        // 持ち越し元は、削除されていてもその日付を示す（持ち越してきたことは変わらないため）
        let source_ids: Vec<Ulid> = items
            .iter()
            .filter_map(|i| i.item.carried_from_id)
            .collect();
        let sources: HashMap<Ulid, LogicalDate> = item_table::get_many(conn, &source_ids)?
            .into_iter()
            .map(|(id, item)| (id, item.date))
            .collect();

        let mut days: BTreeMap<LogicalDate, Day> = BTreeMap::new();
        for item in items {
            let from = item
                .item
                .carried_from_id
                .and_then(|id| sources.get(&id).copied());
            days.entry(item.item.date)
                .or_default()
                .items
                .push((item, from));
        }
        for doc in [TextDoc::Note, TextDoc::Reflection] {
            for row in text_doc::nonblank_between(conn, doc, from, to)? {
                let day = days.entry(row.date).or_default();
                match doc {
                    TextDoc::Note => day.note = row.body,
                    TextDoc::Reflection => day.reflection = row.body,
                }
            }
        }
        Ok(Boards { days })
    })
}

impl Boards {
    /// 日ごとの Markdown を、日付の順に返す。
    pub fn render(&self) -> Vec<DayMarkdown> {
        self.days
            .iter()
            .map(|(&date, day)| DayMarkdown {
                date,
                markdown: day.render(date),
            })
            .collect()
    }

    /// 日ごとの Markdown を、日付の順に1日ずつ作る（[`Boards::render`] と同じもの）。作った日の読んだ
    /// 内容はすぐ手放すので、[`write_files`] に渡すと、期間全体の文字列を一度に持たずに済む。
    pub fn into_days(self) -> impl Iterator<Item = DayMarkdown> {
        self.days.into_iter().map(|(date, day)| DayMarkdown {
            date,
            markdown: day.render(date),
        })
    }
}

impl Day {
    fn render(&self, date: LogicalDate) -> String {
        let mut out = format!("# {date}\n");
        for (kind, title) in [(ItemKind::Todo, "TODO"), (ItemKind::Blocker, "Blockers")] {
            let mut items = self
                .items
                .iter()
                .filter(|(i, _)| i.item.kind == kind)
                .peekable();
            if items.peek().is_none() {
                continue;
            }
            write!(out, "\n## {title}\n\n").unwrap();
            for (item, from) in items {
                push_item(&mut out, item, *from);
            }
        }
        for (title, body) in [("Reflection", &self.reflection), ("Notes", &self.note)] {
            if !body.is_empty() {
                write!(out, "\n## {title}\n\n{}\n", trim_body(&to_lf(body))).unwrap();
            }
        }
        out
    }
}

/// 項目の1行（と、解消した障害の解消メモの行）。
fn push_item(out: &mut String, board_item: &BoardItem, from: Option<LogicalDate>) {
    let item = &board_item.item;
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
    if let Some(to) = board_item.carried_to {
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

/// 改行（CRLF・CR）を LF にそろえる（書き出すファイルの改行を LF だけにするため。コアは CR を行の
/// 文字として保存する。データモデル設計書 3章 note / reflection）。
fn to_lf(body: &str) -> String {
    body.replace("\r\n", "\n").replace('\r', "\n")
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
    /// ファイル `path` を書けなかった。`replaced` は、それまでに置き換え終えたファイルの数（一時
    /// ファイルを書く段階で失敗したときは 0。[`write_files`]）。
    Write {
        path: PathBuf,
        replaced: usize,
        source: io::Error,
    },
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RelativeDir { dir } => write!(
                f,
                "the export folder must be an absolute path: {}",
                dir.display()
            ),
            Self::Write { path, replaced, .. } => {
                if *replaced > 0 {
                    write!(f, "{replaced} files were exported, then ")?;
                }
                write!(f, "failed to write {}", path.display())
            }
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

/// 一時ファイルの名前（`.yunagi-export-<ULID>.tmp`）の前と後ろ。日付を入れないのは、シンボリック
/// リンクのリンク先（名前が日付でない）の横に作るものも、同じ形で見つけて片付けるため。
const TEMP_PREFIX: &str = ".yunagi-export-";
const TEMP_SUFFIX: &str = ".tmp";

/// 中断で残った一時ファイルとみなすまでの時間（[`write_files`]）。書いている途中のほかの書き出しの
/// 一時ファイルを消さないよう、十分に長くする。
const STALE_TEMP_AGE: Duration = Duration::from_secs(60 * 60);

/// 名前の変更をやり直す回数と、1回目の待ち時間（2回目以降は、回数に比例して長くする）。Windows では、
/// 同期のツールなどが置き換える先のファイルを開いていると、少しの間だけ名前を変えられないため。
#[cfg(windows)]
const RENAME_RETRIES: (u32, Duration) = (5, Duration::from_millis(100));
#[cfg(not(windows))]
const RENAME_RETRIES: (u32, Duration) = (0, Duration::ZERO);

/// 1日分の、置き換える前の一時ファイル。
struct Pending {
    /// 呼び出し側に返すパス（`dir` の中の `YYYY-MM-DD.md`）。
    path: PathBuf,
    /// 実際に置き換えるファイル（実体のパス。`path` がシンボリックリンクなら、リンク先）。
    target: PathBuf,
    tmp: PathBuf,
}

/// `days` を、フォルダ `dir` に日ごとのファイル（[`file_name`]）として書き、書いたファイルのパス
/// （`dir` の中のもの）を `days` の順に返す。フォルダは作らない（呼び出し側が選んだ、あるフォルダに書く）。
/// `days` は1日ずつ取り出して一時ファイルに書き、書いた後は持たない（[`Boards::into_days`] と合わせると、
/// 文字列にした期間全体を一度に持たずに済む）。
///
/// - 同じ名前のファイルは置き換える。置き換えたファイルの権限（Unix の権限の数字）は保ち、シンボリック
///   リンクはリンク先を置き換える（リンクのまま残す）。ほかのファイルには触れない。
/// - まずすべての日を、置き換える先と同じフォルダの一時ファイル（`.yunagi-export-<ULID>.tmp`）に書いて
///   ディスクに反映し、それから順に名前を変える（同期のツールやエディタが、書きかけのファイルを
///   読まないように）。一時ファイルを書けないとき、置き換える先がファイルでないとき、2つの日が同じ
///   ファイルを指すとき（同じリンク先へのリンク）、Windows で置き換える先が読み取り専用のときは、書いた
///   一時ファイルを消し、何も置き換えずに失敗とする。名前の変更に失敗したら、残りの一時ファイルを消して
///   止める（それより前の日は置き換え済みで、その数をエラーに入れる）。
/// - 一時ファイルを置く各フォルダで、書き始める前に、中断で残った一時ファイル（1時間以上前に作ったもの）
///   を消す（消せなくても続ける）。
pub fn write_files(
    dir: &Path,
    days: impl IntoIterator<Item = DayMarkdown>,
) -> Result<Vec<PathBuf>, ExportError> {
    if !dir.is_absolute() {
        return Err(ExportError::RelativeDir {
            dir: dir.to_path_buf(),
        });
    }
    let mut pending: Vec<Pending> = Vec::new();
    let mut cleaned: HashSet<PathBuf> = HashSet::new();
    for day in days {
        let path = dir.join(file_name(day.date));
        let result = write_temp(&path, day.markdown.as_bytes(), |folder| {
            if cleaned.insert(folder.to_path_buf()) {
                remove_stale_temp_files(folder, SystemTime::now());
            }
        })
        .and_then(|p| {
            if pending.iter().any(|q| q.target == p.target) {
                let _ = fs::remove_file(&p.tmp);
                Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "two export files point to the same file",
                ))
            } else {
                Ok(p)
            }
        });
        match result {
            Ok(p) => pending.push(p),
            Err(source) => {
                remove_temps(&pending);
                return Err(ExportError::Write {
                    path,
                    replaced: 0,
                    source,
                });
            }
        }
    }

    for (replaced, p) in pending.iter().enumerate() {
        if let Err(source) = retry(RENAME_RETRIES, is_transient, || {
            fs::rename(&p.tmp, &p.target)
        }) {
            remove_temps(&pending[replaced..]);
            sync_dirs(&pending[..replaced]);
            return Err(ExportError::Write {
                path: p.path.clone(),
                replaced,
                source,
            });
        }
    }
    sync_dirs(&pending);
    Ok(pending.into_iter().map(|p| p.path).collect())
}

/// `path` を置き換える一時ファイルを、置き換える先と同じフォルダに書いて反映する。そのフォルダは、
/// 一時ファイルを作る前に `before_create` に渡す。置き換える先にファイルがあれば、その権限を
/// 一時ファイルに付ける（Unix では作るときから付け、ほかの利用者が読める間を作らない）。失敗したら、
/// 作りかけの一時ファイルを消す。
fn write_temp(path: &Path, bytes: &[u8], before_create: impl FnOnce(&Path)) -> io::Result<Pending> {
    // 実体のパスで比べられるよう、フォルダとリンクを辿る。リンク先が無いリンクは、リンク先を
    // 求められないので失敗にする（リンクを黙ってファイルに置き換えないように）
    let is_link = fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink());
    let target = match (is_link, path.parent(), path.file_name()) {
        (true, ..) => fs::canonicalize(path)?,
        (false, Some(parent), Some(name)) => fs::canonicalize(parent)?.join(name),
        _ => return Err(io::Error::other("the export file has no parent folder")),
    };
    let Some(folder) = target.parent() else {
        return Err(io::Error::other("the export file has no parent folder"));
    };
    let existing = fs::metadata(&target).ok();
    if let Some(m) = &existing {
        if !m.is_file() {
            // フォルダなどは置き換えられない
            return Err(io::Error::new(
                io::ErrorKind::IsADirectory,
                "the export file name is used by something that is not a file",
            ));
        }
        // Windows は読み取り専用のファイルを名前の変更で置き換えられない（一時ファイルに写すと、その
        // 一時ファイルも消せなくなる）
        if cfg!(windows) && m.permissions().readonly() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the export file is read-only",
            ));
        }
    }
    before_create(folder);
    let tmp = folder.join(format!("{TEMP_PREFIX}{}{TEMP_SUFFIX}", Ulid::generate()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        if let Some(m) = &existing {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            options.mode(m.permissions().mode() & 0o777);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        // 作るときの権限は umask で狭まるので、同じ数字に付け直す
        #[cfg(unix)]
        if let Some(m) = &existing {
            fs::set_permissions(&tmp, m.permissions())?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => Ok(Pending {
            path: path.to_path_buf(),
            target,
            tmp,
        }),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

fn remove_temps(pending: &[Pending]) {
    for p in pending {
        let _ = fs::remove_file(&p.tmp);
    }
}

/// 名前を変えたフォルダを反映する（名前の変更が、電源が落ちても残るように）。Unix だけで行う
/// （Windows はフォルダを開いて反映できない）。反映できなくても失敗にしない（ファイルの中身は反映済み）。
fn sync_dirs(pending: &[Pending]) {
    #[cfg(unix)]
    {
        let mut dirs: Vec<&Path> = pending.iter().filter_map(|p| p.target.parent()).collect();
        dirs.sort_unstable();
        dirs.dedup();
        for dir in dirs {
            if let Ok(d) = fs::File::open(dir) {
                let _ = d.sync_all();
            }
        }
    }
    #[cfg(not(unix))]
    let _ = pending;
}

/// 名前の変更の失敗のうち、待てば通りうるもの（Windows で、ほかのプログラムが置き換える先を開いている：
/// アクセスの拒否・共有の違反・ロックの違反）。
fn is_transient(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::PermissionDenied || matches!(e.raw_os_error(), Some(32 | 33))
}

/// `f` を、`transient` に当たる失敗なら `retries` 回までやり直す（`n` 回目のやり直しの前に
/// `delay × n` 待つ）。
fn retry<T>(
    (retries, delay): (u32, Duration),
    transient: impl Fn(&io::Error) -> bool,
    mut f: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    let mut attempt = 0;
    loop {
        match f() {
            Err(e) if attempt < retries && transient(&e) => {
                attempt += 1;
                thread::sleep(delay * attempt);
            }
            result => return result,
        }
    }
}

/// フォルダ `folder` の、中断で残った一時ファイル（`.yunagi-export-<ULID>.tmp` の形で、ULID の時刻が
/// `now` より [`STALE_TEMP_AGE`] 以上前のもの）を消す。読めない・消せないものは、そのままにする。
fn remove_stale_temp_files(folder: &Path, now: SystemTime) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    let now_ms = now
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    let stale_ms = u64::try_from(STALE_TEMP_AGE.as_millis()).unwrap_or(u64::MAX);
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(created_ms) = name.to_str().and_then(temp_file_time) else {
            continue;
        };
        if now_ms.saturating_sub(created_ms) >= stale_ms
            && entry.file_type().is_ok_and(|t| t.is_file())
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// 書き出しの一時ファイルの名前（`.yunagi-export-<ULID>.tmp`）なら、作った時刻（UNIX 時刻のミリ秒）。
fn temp_file_time(name: &str) -> Option<u64> {
    let ulid = name.strip_prefix(TEMP_PREFIX)?.strip_suffix(TEMP_SUFFIX)?;
    Some(ulid.parse::<Ulid>().ok()?.timestamp_ms())
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use jiff::tz::TimeZone;

    use super::*;
    use crate::domain::carryover::{self, Decision, Selection};
    use crate::domain::day;
    use crate::domain::item::{self, Item, Position};
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
    fn line_endings_become_lf() {
        let (db, _dir) = open();
        let conn = db.connection();
        write(
            conn,
            TextDoc::Note,
            "2026-11-02",
            "## 10:00 定例\r\n一\r二\r\n",
        );
        assert_eq!(
            day_markdown(conn, date("2026-11-02")).unwrap().unwrap(),
            "# 2026-11-02\n\n## Notes\n\n## 10:00 定例\n一\n二\n"
        );
    }

    #[test]
    fn reading_and_rendering_can_be_done_separately() {
        let (db, _dir) = open();
        let conn = db.connection();
        put(conn, "2026-11-02", ItemKind::Todo, "二日");
        write(conn, TextDoc::Note, "2026-11-03", "三日");
        let boards = read_between(conn, date("2026-11-01"), date("2026-11-03")).unwrap();
        // 読んだ後に変えても、読んだ時点の内容で書き出す
        put(conn, "2026-11-02", ItemKind::Todo, "後から");
        let days = boards.render();
        assert_eq!(
            days.iter().map(|d| d.date.to_string()).collect::<Vec<_>>(),
            ["2026-11-02", "2026-11-03"]
        );
        assert!(!days[0].markdown.contains("後から"));
        assert_eq!(
            read_between(conn, date("2026-12-01"), date("2026-12-31"))
                .unwrap()
                .render(),
            []
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
            [
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
        assert_eq!(
            write_files(dir.path(), Vec::new()).unwrap(),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn nothing_is_replaced_when_a_file_cannot_be_written() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("2026-11-02.md"), "古い書き出し").unwrap();
        // 2日目の名前のフォルダがあると、置き換えられない
        fs::create_dir(dir.path().join("2026-11-03.md")).unwrap();
        let err = write_files(
            dir.path(),
            [
                day("2026-11-02", "二日\n"),
                day("2026-11-03", "三日\n"),
                day("2026-11-04", "四日\n"),
            ],
        )
        .unwrap_err();
        let ExportError::Write { path, replaced, .. } = &err else {
            panic!("{err:?}");
        };
        assert_eq!(path, &dir.path().join("2026-11-03.md"));
        assert_eq!(*replaced, 0);
        assert!(error::Error::source(&err).is_some());
        // 前の日も置き換えず、一時ファイルも残さない
        assert_eq!(leftovers(dir.path()), ["2026-11-02.md", "2026-11-03.md"]);
        assert_eq!(
            fs::read_to_string(dir.path().join("2026-11-02.md")).unwrap(),
            "古い書き出し"
        );

        // 無いフォルダには書かない（作らない）
        let missing = dir.path().join("無い");
        assert!(matches!(
            write_files(&missing, [day("2026-11-02", "二日\n")]),
            Err(ExportError::Write { replaced: 0, .. })
        ));
        assert!(!missing.exists());
    }

    #[test]
    fn the_error_tells_how_many_files_were_replaced() {
        let err = |replaced| {
            ExportError::Write {
                path: PathBuf::from("/x/2026-11-03.md"),
                replaced,
                source: io::Error::other("busy"),
            }
            .to_string()
        };
        assert_eq!(err(0), "failed to write /x/2026-11-03.md");
        assert_eq!(
            err(2),
            "2 files were exported, then failed to write /x/2026-11-03.md"
        );
    }

    #[test]
    fn retry_runs_again_only_after_failures() {
        let no_wait = Duration::ZERO;
        let mut calls = 0;
        let ok = retry(
            (3, no_wait),
            |_| true,
            || {
                calls += 1;
                if calls < 3 {
                    Err(io::Error::other("busy"))
                } else {
                    Ok(calls)
                }
            },
        );
        assert_eq!(ok.unwrap(), 3);
        let mut calls = 0;
        let failed: io::Result<()> = retry(
            (2, no_wait),
            |_| true,
            || {
                calls += 1;
                Err(io::Error::other("busy"))
            },
        );
        assert!(failed.is_err());
        assert_eq!(calls, 3);
        let mut calls = 0;
        let failed: io::Result<()> = retry(
            (0, no_wait),
            |_| true,
            || {
                calls += 1;
                Err(io::Error::other("busy"))
            },
        );
        assert!(failed.is_err());
        assert_eq!(calls, 1);
        // 待っても通らない失敗はやり直さない
        let mut calls = 0;
        let failed: io::Result<()> = retry((3, no_wait), is_transient, || {
            calls += 1;
            Err(io::Error::from(io::ErrorKind::NotFound))
        });
        assert!(failed.is_err());
        assert_eq!(calls, 1);
        assert!(is_transient(&io::Error::from(
            io::ErrorKind::PermissionDenied
        )));
        assert!(is_transient(&io::Error::from_raw_os_error(32)));
        assert!(!is_transient(&io::Error::from(io::ErrorKind::IsADirectory)));
    }

    #[test]
    fn stale_temp_files_are_removed_and_fresh_ones_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let now = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let ulid_at = |secs_ago: u64| {
            let ms = 1_800_000_000_000 - secs_ago * 1000;
            Ulid::from_u128(u128::from(ms) << 80)
        };
        let old = format!(".yunagi-export-{}.tmp", ulid_at(2 * 60 * 60));
        let fresh = format!(".yunagi-export-{}.tmp", ulid_at(60));
        let others = [
            "2026-11-02.md".to_owned(),
            // 形の違うもの（利用者のファイル）
            format!(".memo-{}.tmp", ulid_at(2 * 60 * 60)),
            ".yunagi-export-x.tmp".to_owned(),
        ];
        for name in [&old, &fresh].into_iter().chain(&others) {
            fs::write(dir.path().join(name), "").unwrap();
        }
        remove_stale_temp_files(dir.path(), now);
        let mut expected: Vec<String> = others.to_vec();
        expected.push(fresh.clone());
        expected.sort();
        assert_eq!(leftovers(dir.path()), expected);
        assert_eq!(temp_file_time(&old), Some(1_800_000_000_000 - 7_200_000));
        assert_eq!(temp_file_time("2026-11-02.md"), None);
        assert_eq!(temp_file_time(".yunagi-export-x.tmp"), None);
    }

    #[cfg(unix)]
    #[test]
    fn replacing_keeps_permissions_and_symlinks() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        let plain = dir.path().join("2026-11-02.md");
        fs::write(&plain, "古い").unwrap();
        fs::set_permissions(&plain, fs::Permissions::from_mode(0o600)).unwrap();
        let linked = vault.path().join("三日.md");
        fs::write(&linked, "古い").unwrap();
        symlink(&linked, dir.path().join("2026-11-03.md")).unwrap();

        write_files(
            dir.path(),
            [day("2026-11-02", "二日\n"), day("2026-11-03", "三日\n")],
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&plain).unwrap(), "二日\n");
        assert_eq!(
            fs::metadata(&plain).unwrap().permissions().mode() & 0o777,
            0o600
        );
        // リンクはリンクのまま、リンク先が新しくなる
        let link = dir.path().join("2026-11-03.md");
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&linked).unwrap(), "三日\n");
        assert_eq!(leftovers(vault.path()), ["三日.md"]);

        // リンク先のフォルダに中断で残った一時ファイルも片付ける
        let stale = vault
            .path()
            .join(format!(".yunagi-export-{}.tmp", Ulid::from_u128(1 << 80)));
        fs::write(&stale, "").unwrap();
        write_files(dir.path(), [day("2026-11-03", "三日\n")]).unwrap();
        assert!(!stale.exists());

        // 2つの日が同じファイルを指すときは、何も置き換えない
        symlink(&linked, dir.path().join("2026-11-04.md")).unwrap();
        let err = write_files(
            dir.path(),
            [
                day("2026-11-03", "新しい三日\n"),
                day("2026-11-04", "四日\n"),
            ],
        )
        .unwrap_err();
        let ExportError::Write { path, replaced, .. } = &err else {
            panic!("{err:?}");
        };
        assert_eq!((path, *replaced), (&dir.path().join("2026-11-04.md"), 0));
        assert_eq!(fs::read_to_string(&linked).unwrap(), "三日\n");
        assert_eq!(leftovers(vault.path()), ["三日.md"]);
        // リンクではない名前と、それを指すリンクも同じ
        symlink(&plain, dir.path().join("2026-11-05.md")).unwrap();
        assert!(matches!(
            write_files(
                dir.path(),
                [day("2026-11-02", "二\n"), day("2026-11-05", "五\n")]
            ),
            Err(ExportError::Write { replaced: 0, .. })
        ));
        assert_eq!(fs::read_to_string(&plain).unwrap(), "二日\n");
        fs::remove_file(dir.path().join("2026-11-04.md")).unwrap();
        fs::remove_file(dir.path().join("2026-11-05.md")).unwrap();

        // リンク先の無いリンクは、ファイルに置き換えずに失敗にする
        fs::remove_file(&linked).unwrap();
        assert!(matches!(
            write_files(dir.path(), [day("2026-11-03", "三日\n")]),
            Err(ExportError::Write { replaced: 0, .. })
        ));
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn relative_folders_are_refused() {
        let err = write_files(Path::new("exports"), [day("2026-11-02", "二日\n")]).unwrap_err();
        assert!(matches!(err, ExportError::RelativeDir { .. }));
        assert!(!Path::new("exports").exists());
    }

    /// 5年分（1,800日）の模擬データを書き出すときに、データベースを使う時間（[`read_between`]）と、
    /// 文字列にする時間を測る（アーキテクチャ設計書 6.4。期間に上限を設けないため、画面の自動保存を
    /// 待たせる長さを確かめる）。量はデータモデル設計書 5章の見積もり（1日あたり TODO・障害 20件、
    /// ノート 10KB、振り返り 2KB）で、項目の4分の1は前の日から持ち越したもの。データを作るのに時間が
    /// かかるので、普段のテストでは飛ばす。測るときは次のように走らせる：
    ///
    /// ```sh
    /// cargo test --release -p yunagi-core export::tests::five_years -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "重い。測るときに --ignored で走らせる"]
    fn five_years_of_data_are_read_quickly() {
        use crate::storage::table::day as day_table;
        use crate::storage::table::item::Origin;
        const DAYS: usize = 1_800;
        const ITEMS_PER_DAY: usize = 20;
        let (db, _dir) = open();
        let conn = db.connection();
        let first = date("2021-11-08");
        let note =
            "外部APIの429が増えている。上限は1分あたり600件らしい。\n".repeat(10 * 1024 / 80);
        let reflection = "良かったこと：設計の見直しが進んだ。\n".repeat(2 * 1024 / 50);
        conn.execute_batch("BEGIN").unwrap();
        let mut previous: Vec<Ulid> = Vec::new();
        let mut d = first;
        for _ in 0..DAYS {
            day_table::ensure(conn, d, "Asia/Tokyo", now()).unwrap();
            let mut today = Vec::new();
            for i in 0..ITEMS_PER_DAY {
                let id = Ulid::generate();
                let it = Item {
                    id,
                    date: d,
                    kind: if i % 4 == 0 {
                        ItemKind::Blocker
                    } else {
                        ItemKind::Todo
                    },
                    text: format!("API のレート制限を調査 {i}"),
                    status: ItemStatus::Open,
                    close_reason: None,
                    sort_key: format!("a{i:02}"),
                    carry_count: 0,
                    carried_from_id: previous.get(i).filter(|_| i % 4 == 1).copied(),
                    split_from_id: None,
                    resolution_note: None,
                    origin: Origin::Human,
                    agent_client_id: None,
                    external_ref_id: None,
                    completed_at: None,
                    created_at: now(),
                    updated_at: now(),
                    deleted_at: None,
                };
                item_table::insert(conn, &it).unwrap();
                today.push(id);
            }
            previous = today;
            text_doc::save(conn, TextDoc::Note, d, &note, now()).unwrap();
            text_doc::save(conn, TextDoc::Reflection, d, &reflection, now()).unwrap();
            d = d.add_days(1).unwrap();
        }
        conn.execute_batch("COMMIT").unwrap();
        let last = d.add_days(-1).unwrap();

        let started = std::time::Instant::now();
        let boards = read_between(conn, first, last).unwrap();
        let read = started.elapsed();
        let started = std::time::Instant::now();
        let days = boards.render();
        let render = started.elapsed();
        let bytes: usize = days.iter().map(|d| d.markdown.len()).sum();
        println!(
            "read {read:?}, render {render:?}, {} days, {bytes} bytes",
            days.len()
        );
        assert_eq!(days.len(), DAYS);
        assert!(read < Duration::from_secs(5), "{read:?}");
    }
}
