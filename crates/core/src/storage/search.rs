//! 全日付の TODO・障害・ノート・振り返りの横断検索（docs/design/02-data-model.md 4章）。
//!
//! 索引（FTS5 の `search_index`）はマイグレーション 2 で作り、item・note・reflection の
//! 更新のたびにトリガーでそろえる（`migrations/0002_search_index.sql`）。ここは検索だけを行う。
//!
//! # 一致の決まり
//!
//! - 検索語は前後の空白を除き、全体を1つの文字列として、本文のどこかに含まれていれば一致とする
//!   （部分一致。空白で語に分けない）。前後の空白を除いて空なら、何も返さない。
//! - 3文字以上の検索語は索引（trigram）で探す。trigram は3文字未満の語に索引を使えないため、
//!   2文字以下の検索語は元の表（item.text、note.body、reflection.body）への `LIKE` で探す。
//!   文字数は Unicode の文字（コードポイント）で数える。
//! - ASCII の英字の大文字・小文字は区別しない。ASCII 以外（全角英字・`Ä` など）の大文字・小文字は、
//!   3文字以上では区別せず（trigram の既定）、2文字以下では区別する（SQLite の `LIKE` は
//!   ASCII しか大文字・小文字をそろえないため）。
//! - 論理削除した項目は出さない。
//! - 並びは日付の新しい順。同じ日の中は TODO・障害・ノート・振り返りの順、その中は id の順。

use rusqlite::{Connection, Row, params};

use super::StorageError;
use super::table::text_enum;
use crate::domain::logical_date::LogicalDate;

text_enum! {
    /// 検索結果の種類（MCP・CLI の `--kind`。MCP・CLI 仕様書 3章）。
    pub enum SearchKind {
        Todo => "todo",
        Blocker => "blocker",
        Note => "note",
        Reflection => "reflection",
    }
}

/// 検索の条件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchQuery<'a> {
    /// 検索語。
    pub text: &'a str,
    /// この日以降（この日を含む）。`None` なら下限なし。
    pub from: Option<LogicalDate>,
    /// この日以前（この日を含む）。`None` なら上限なし。
    pub to: Option<LogicalDate>,
    /// 絞り込む種類。空ならすべての種類。
    pub kinds: &'a [SearchKind],
    /// 返す件数の上限。
    pub limit: usize,
}

/// 検索結果の1件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub kind: SearchKind,
    /// 元の行の id。TODO・障害は item.id、ノート・振り返りはその日の日付（`YYYY-MM-DD`）。
    pub ref_id: String,
    pub date: LogicalDate,
    /// 本文の全体（項目は1行の本文、ノート・振り返りは本文すべて）。
    /// 一致した部分の切り出しは使う側で行う。
    pub body: String,
}

/// 3文字以上の検索語は索引で探す。
const MIN_INDEXED_CHARS: usize = 3;

/// 検索する。
pub fn search(conn: &Connection, query: &SearchQuery<'_>) -> Result<Vec<SearchHit>, StorageError> {
    let text = query.text.trim();
    if text.is_empty() || query.limit == 0 {
        return Ok(Vec::new());
    }
    // 種類は JSON の配列で渡し、SQL の中で json_each で開く（文を種類の数によらず1つにするため）
    let kinds = (!query.kinds.is_empty()).then(|| {
        let list: Vec<String> = query
            .kinds
            .iter()
            .map(|k| format!("\"{}\"", k.as_str()))
            .collect();
        format!("[{}]", list.join(","))
    });
    let limit = i64::try_from(query.limit).unwrap_or(i64::MAX);

    let (sql, pattern) = if text.chars().count() >= MIN_INDEXED_CHARS {
        (INDEXED_SQL, fts_phrase(text))
    } else {
        (LIKE_SQL, like_pattern(text))
    };
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(
        params![pattern, query.from, query.to, kinds, limit],
        from_row,
    )?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// 絞り込みと並び（両方の文で共通。`?2` 期間の始め、`?3` 終わり、`?4` 種類、`?5` 件数）。
macro_rules! filter_and_order {
    () => {
        " WHERE (?2 IS NULL OR date >= ?2) AND (?3 IS NULL OR date <= ?3) \
          AND (?4 IS NULL OR kind IN (SELECT value FROM json_each(?4))) \
          ORDER BY date DESC, \
          CASE kind WHEN 'todo' THEN 0 WHEN 'blocker' THEN 1 WHEN 'note' THEN 2 ELSE 3 END, \
          ref_id \
          LIMIT ?5"
    };
}

/// 索引で探す文。`?1` は FTS5 の句（[`fts_phrase`]）。
const INDEXED_SQL: &str = concat!(
    "SELECT kind, ref_id, date, body FROM (",
    "SELECT kind, ref_id, date, body FROM search_index WHERE search_index MATCH ?1",
    ")",
    filter_and_order!(),
);

/// 元の表への部分一致で探す文。`?1` は LIKE の型（[`like_pattern`]）。
const LIKE_SQL: &str = concat!(
    "SELECT kind, ref_id, date, body FROM (",
    "SELECT kind, id AS ref_id, date, text AS body FROM item ",
    "WHERE deleted_at IS NULL AND text LIKE ?1 ESCAPE '\\' ",
    "UNION ALL SELECT 'note', date, date, body FROM note WHERE body LIKE ?1 ESCAPE '\\' ",
    "UNION ALL SELECT 'reflection', date, date, body FROM reflection WHERE body LIKE ?1 ESCAPE '\\'",
    ")",
    filter_and_order!(),
);

fn from_row(row: &Row<'_>) -> rusqlite::Result<SearchHit> {
    Ok(SearchHit {
        kind: row.get(0)?,
        ref_id: row.get(1)?,
        date: row.get(2)?,
        body: row.get(3)?,
    })
}

/// 検索語を、FTS5 の1つの句（`"…"`）にする。句の中の `"` は2つ重ねる。
/// trigram では、句は本文の中の連続した文字列に一致する（部分一致）。
fn fts_phrase(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "\"\""))
}

/// 検索語を、どこかに含むことを表す LIKE の型にする。`%`・`_`・`\` は `\` で打ち消す。
fn like_pattern(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('%');
    for c in text.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use jiff::Timestamp;

    use super::*;
    use crate::storage::table::item::{self, Item, ItemKind, ItemStatus, Origin};
    use crate::storage::table::test_support::{at, date, ensure_day, open};
    use crate::storage::table::text_doc::{self, TextDoc};
    use crate::storage::{Database, DbLock, Ulid, migration};

    const T0: &str = "2026-11-03T00:00:00Z";

    fn new_item(d: LogicalDate, kind: ItemKind, text: &str) -> Item {
        Item {
            id: Ulid::generate(),
            date: d,
            kind,
            text: text.into(),
            status: ItemStatus::Open,
            close_reason: None,
            sort_key: "a0".into(),
            carry_count: 0,
            carried_from_id: None,
            split_from_id: None,
            resolution_note: None,
            origin: Origin::Human,
            agent_client_id: None,
            external_ref_id: None,
            completed_at: None,
            created_at: at(T0),
            updated_at: at(T0),
            deleted_at: None,
        }
    }

    fn add(db: &Database, d: &str, kind: ItemKind, text: &str) -> Item {
        let d = ensure_day(db, d);
        let item = new_item(d, kind, text);
        item::insert(db.connection(), &item).unwrap();
        item
    }

    fn save(db: &Database, doc: TextDoc, d: &str, body: &str) {
        let d = ensure_day(db, d);
        text_doc::save(db.connection(), doc, d, body, at(T0)).unwrap();
    }

    fn query(text: &str) -> SearchQuery<'_> {
        SearchQuery {
            text,
            from: None,
            to: None,
            kinds: &[],
            limit: 100,
        }
    }

    /// 結果を「日付 種類 本文」の文字列にする。
    fn find(db: &Database, q: &SearchQuery<'_>) -> Vec<String> {
        search(db.connection(), q)
            .unwrap()
            .into_iter()
            .map(|h| format!("{} {} {}", h.date, h.kind, h.body))
            .collect()
    }

    /// 索引が元の表と食い違っていないか（FTS5 の検査）と、行番号の表と索引の件数がそろっているか。
    fn assert_index_consistent(db: &Database) {
        let conn = db.connection();
        conn.execute(
            "INSERT INTO search_index (search_index) VALUES ('integrity-check')",
            [],
        )
        .unwrap();
        let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        let expected = count("SELECT count(*) FROM item WHERE deleted_at IS NULL")
            + count("SELECT count(*) FROM note")
            + count("SELECT count(*) FROM reflection");
        assert_eq!(count("SELECT count(*) FROM search_index"), expected);
        assert_eq!(count("SELECT count(*) FROM search_index_ref"), expected);
        assert_eq!(
            count(
                "SELECT count(*) FROM search_index AS s JOIN search_index_ref AS r \
                 ON r.id = s.rowid AND r.ref_id = s.ref_id"
            ),
            expected
        );
    }

    #[test]
    fn search_index_uses_the_trigram_tokenizer() {
        let (db, _dir) = open();
        let sql: String = db
            .connection()
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE name = 'search_index'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(sql.contains("USING fts5"), "{sql}");
        assert!(sql.contains("tokenize = 'trigram'"), "{sql}");
    }

    #[test]
    fn finds_japanese_text_of_three_or_more_characters_through_the_index() {
        let (db, _dir) = open();
        let todo = add(&db, "2026-11-02", ItemKind::Todo, "API のレート制限を調査");
        add(
            &db,
            "2026-11-03",
            ItemKind::Blocker,
            "ステージング環境の権限待ち",
        );
        save(
            &db,
            TextDoc::Note,
            "2026-11-03",
            "## 10:00 定例\n外部APIのレート制限が厳しい",
        );
        save(
            &db,
            TextDoc::Reflection,
            "2026-11-01",
            "レート制限の調査は明日",
        );

        assert_eq!(
            find(&db, &query("レート制限")),
            [
                "2026-11-03 note ## 10:00 定例\n外部APIのレート制限が厳しい",
                "2026-11-02 todo API のレート制限を調査",
                "2026-11-01 reflection レート制限の調査は明日",
            ]
        );
        let hits = search(db.connection(), &query("制限を調査")).unwrap();
        assert_eq!(
            hits,
            [SearchHit {
                kind: SearchKind::Todo,
                ref_id: todo.id.to_string(),
                date: date("2026-11-02"),
                body: "API のレート制限を調査".into(),
            }]
        );
        // ノート・振り返りの ref_id は日付
        let hits = search(db.connection(), &query("ステージング")).unwrap();
        assert_eq!(hits[0].kind, SearchKind::Blocker);
        let hits = search(db.connection(), &query("定例\n外部")).unwrap();
        assert_eq!(hits[0].ref_id, "2026-11-03");
        assert!(find(&db, &query("レート調査")).is_empty());
    }

    #[test]
    fn finds_text_of_one_or_two_characters_in_the_original_tables() {
        // trigram の索引は2文字以下の語に一致しないので、見つかれば元の表を探している
        let (db, _dir) = open();
        let deleted = add(&db, "2026-11-01", ItemKind::Todo, "権限を消した");
        item::soft_delete(db.connection(), deleted.id, at(T0)).unwrap();
        add(&db, "2026-11-02", ItemKind::Blocker, "権限待ち");
        save(&db, TextDoc::Note, "2026-11-03", "ステージングの権限");
        save(&db, TextDoc::Reflection, "2026-11-03", "明日は権限を確認");
        let in_index: i64 = db
            .connection()
            .query_row(
                "SELECT count(*) FROM search_index WHERE search_index MATCH '\"権限\"'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(in_index, 0);

        let expected = [
            "2026-11-03 note ステージングの権限",
            "2026-11-03 reflection 明日は権限を確認",
            "2026-11-02 blocker 権限待ち",
        ];
        assert_eq!(find(&db, &query("権限")), expected);
        assert_eq!(find(&db, &query("待")), ["2026-11-02 blocker 権限待ち"]);
        // 前後の空白は除く
        assert_eq!(find(&db, &query(" 権限\u{3000}")), expected);
    }

    #[test]
    fn index_and_like_agree_on_case_and_special_characters() {
        let (db, _dir) = open();
        add(
            &db,
            "2026-11-01",
            ItemKind::Todo,
            "Fix \"quoted\" AND NEAR(x) 100%_done\\ok",
        );
        add(&db, "2026-11-02", ItemKind::Todo, "100 done");
        for q in [
            "fix",
            "FIX \"QUOTED\"",
            "\"quoted\"",
            "and near(",
            "AND",
            "%_d",
            "%",
            "_",
            "\\",
            "e\\o",
            "x)",
            "*",
        ] {
            let hits = find(&db, &query(q));
            let expected: &[&str] = if q == "*" {
                &[]
            } else {
                &["2026-11-01 todo Fix \"quoted\" AND NEAR(x) 100%_done\\ok"]
            };
            assert_eq!(hits, expected, "{q:?}");
        }
    }

    #[test]
    fn non_ascii_case_is_folded_only_for_three_or_more_characters() {
        // モジュールの説明の「一致の決まり」のとおり。変わったら説明と設計書も直す
        let (db, _dir) = open();
        add(&db, "2026-11-01", ItemKind::Todo, "ａｂｃ");
        assert_eq!(find(&db, &query("ＡＢＣ")).len(), 1);
        assert_eq!(find(&db, &query("ａｂ")).len(), 1);
        assert!(find(&db, &query("ＡＢ")).is_empty());
    }

    #[test]
    fn empty_query_or_zero_limit_returns_nothing() {
        let (db, _dir) = open();
        add(&db, "2026-11-01", ItemKind::Todo, "  何か  ");
        assert!(find(&db, &query("")).is_empty());
        assert!(find(&db, &query(" \u{3000}\n")).is_empty());
        assert!(
            find(
                &db,
                &SearchQuery {
                    limit: 0,
                    ..query("何か")
                }
            )
            .is_empty()
        );
    }

    #[test]
    fn follows_item_updates_soft_deletes_restores_and_deletes() {
        let (db, _dir) = open();
        let conn = db.connection();
        let mut todo = add(&db, "2026-11-03", ItemKind::Todo, "障害対応の手順書");
        assert_eq!(find(&db, &query("手順書")).len(), 1);

        // 状態だけの変更では入れ直さない（結果は変わらない）
        todo.status = ItemStatus::Done;
        todo.completed_at = Some(at(T0));
        assert!(item::update(conn, &todo).unwrap());
        assert_eq!(find(&db, &query("手順書")).len(), 1);

        todo.text = "障害対応の振り返り会".into();
        item::update(conn, &todo).unwrap();
        assert!(find(&db, &query("手順書")).is_empty());
        assert_eq!(
            find(&db, &query("振り返り会")),
            ["2026-11-03 todo 障害対応の振り返り会"]
        );
        assert_index_consistent(&db);

        let later = Timestamp::from_second(at(T0).as_second() + 60).unwrap();
        assert!(item::soft_delete(conn, todo.id, later).unwrap());
        assert!(find(&db, &query("振り返り会")).is_empty());
        assert!(find(&db, &query("会")).is_empty());
        assert_index_consistent(&db);

        assert!(item::restore(conn, todo.id, later).unwrap());
        assert_eq!(find(&db, &query("振り返り会")).len(), 1);
        assert_index_consistent(&db);

        conn.execute("DELETE FROM item WHERE id = ?1", [todo.id])
            .unwrap();
        assert!(find(&db, &query("振り返り会")).is_empty());
        assert_index_consistent(&db);
    }

    #[test]
    fn follows_note_and_reflection_updates_and_deletes() {
        let (db, _dir) = open();
        for doc in [TextDoc::Note, TextDoc::Reflection] {
            save(&db, doc, "2026-11-03", "一回目の本文");
            save(&db, doc, "2026-11-03", "二回目の本文");
        }
        assert!(find(&db, &query("一回目")).is_empty());
        assert_eq!(
            find(&db, &query("二回目")),
            [
                "2026-11-03 note 二回目の本文",
                "2026-11-03 reflection 二回目の本文"
            ]
        );
        assert_index_consistent(&db);
        db.connection()
            .execute_batch("DELETE FROM note; DELETE FROM reflection;")
            .unwrap();
        assert!(find(&db, &query("二回目")).is_empty());
        assert_index_consistent(&db);
    }

    #[test]
    fn filters_by_period_and_kind_and_limits_the_count() {
        let (db, _dir) = open();
        for d in ["2026-10-31", "2026-11-01", "2026-11-02", "2026-11-03"] {
            add(&db, d, ItemKind::Todo, &format!("{d} の打ち合わせ"));
            add(&db, d, ItemKind::Blocker, &format!("{d} 打ち合わせ待ち"));
            save(&db, TextDoc::Note, d, "打ち合わせのメモ");
            save(&db, TextDoc::Reflection, d, "打ち合わせが長い");
        }
        for q in ["打ち合わせ", "打ち"] {
            let period = SearchQuery {
                from: Some(date("2026-11-01")),
                to: Some(date("2026-11-02")),
                kinds: &[SearchKind::Todo, SearchKind::Reflection],
                ..query(q)
            };
            assert_eq!(
                find(&db, &period),
                [
                    "2026-11-02 todo 2026-11-02 の打ち合わせ",
                    "2026-11-02 reflection 打ち合わせが長い",
                    "2026-11-01 todo 2026-11-01 の打ち合わせ",
                    "2026-11-01 reflection 打ち合わせが長い",
                ],
                "{q}"
            );
            let only_from = SearchQuery {
                from: Some(date("2026-11-03")),
                kinds: &[SearchKind::Blocker],
                ..query(q)
            };
            assert_eq!(
                find(&db, &only_from),
                ["2026-11-03 blocker 2026-11-03 打ち合わせ待ち"]
            );
            let only_to = SearchQuery {
                to: Some(date("2026-10-31")),
                kinds: &[SearchKind::Note],
                ..query(q)
            };
            assert_eq!(find(&db, &only_to), ["2026-10-31 note 打ち合わせのメモ"]);
            assert_eq!(find(&db, &query(q)).len(), 16);
            let limited = SearchQuery {
                limit: 3,
                ..query(q)
            };
            assert_eq!(
                find(&db, &limited),
                [
                    "2026-11-03 todo 2026-11-03 の打ち合わせ",
                    "2026-11-03 blocker 2026-11-03 打ち合わせ待ち",
                    "2026-11-03 note 打ち合わせのメモ",
                ]
            );
        }
    }

    #[test]
    fn migration_2_indexes_rows_written_before_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("yunagi.db");
        let open_with = |migrations| {
            let lock = DbLock::acquire(dir.path(), Duration::ZERO).unwrap();
            Database::open_with_migrations(lock, &path, migrations).unwrap()
        };
        {
            let db = open_with(&migration::MIGRATIONS[..1]);
            add(&db, "2026-11-02", ItemKind::Todo, "番号1で書いた項目");
            let deleted = add(&db, "2026-11-02", ItemKind::Todo, "番号1で消した項目");
            item::soft_delete(db.connection(), deleted.id, at(T0)).unwrap();
            save(&db, TextDoc::Note, "2026-11-02", "番号1で書いたノート");
            save(
                &db,
                TextDoc::Reflection,
                "2026-11-01",
                "番号1で書いた振り返り",
            );
            db.close().unwrap();
        }
        let db = open_with(migration::MIGRATIONS);
        assert_eq!(
            find(&db, &query("番号1で")),
            [
                "2026-11-02 todo 番号1で書いた項目",
                "2026-11-02 note 番号1で書いたノート",
                "2026-11-01 reflection 番号1で書いた振り返り",
            ]
        );
        assert_index_consistent(&db);
        // 取り込んだ後の更新も索引に反映される
        save(&db, TextDoc::Note, "2026-11-02", "書き直した");
        assert!(find(&db, &query("番号1で書いたノート")).is_empty());
        assert_index_consistent(&db);
    }

    /// 5年分（1,800日）の模擬データでの検索の時間を測る（テスト計画書 5章。基準は p95 で1秒以内）。
    ///
    /// 量はデータモデル設計書 5章の見積もり（1日あたり TODO・障害 20件、ノート 10KB、振り返り 2KB）。
    /// データを作るのに時間がかかるので、普段のテストでは飛ばす。測るときは次のように走らせる：
    ///
    /// ```sh
    /// cargo test --release -p yunagi-core search::tests::five_years -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "重い。測るときに --ignored で走らせる"]
    fn five_years_of_data_is_searched_within_one_second() {
        const DAYS: i64 = 1_800;
        const ITEMS_PER_DAY: usize = 20;
        const NOTE_BYTES: usize = 10 * 1024;
        const REFLECTION_BYTES: usize = 2 * 1024;
        const RUNS: usize = 20;
        const WORDS: &[&str] = &[
            "API",
            "レート制限",
            "調査",
            "ステージング",
            "環境",
            "権限",
            "待ち",
            "定例",
            "設計",
            "レビュー",
            "障害",
            "対応",
            "デプロイ",
            "ロールバック",
            "監視",
            "アラート",
            "ログ",
            "データベース",
            "マイグレーション",
            "索引",
            "検索",
            "性能",
            "改善",
            "テスト",
            "CI",
            "パイプライン",
            "認証",
            "トークン",
            "期限",
            "確認",
            "依頼",
            "返信",
            "仕様",
            "議事録",
            "見積もり",
            "リリース",
            "手順書",
            "共有",
            "Slack",
            "メール",
            "顧客",
            "問い合わせ",
            "再現",
            "修正",
            "原因",
            "切り分け",
            "バックアップ",
            "復旧",
            "負荷",
            "キャッシュ",
            "の",
            "を",
            "に",
            "が",
            "は",
            "で",
            "と",
            "から",
            "まで",
            "した",
            "する",
            "予定",
            "済み",
            "中",
            "未着手",
            "明日",
            "午後",
            "午前",
            "来週",
            "今日",
        ];

        /// 決まった並びの疑似乱数（xorshift）。走らせるたびに同じデータになる。
        struct Rng(u64);
        impl Rng {
            fn next(&mut self) -> u64 {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 7;
                self.0 ^= self.0 << 17;
                self.0
            }
            fn word(&mut self) -> &'static str {
                WORDS[(self.next() % WORDS.len() as u64) as usize]
            }
            fn text(&mut self, min_bytes: usize) -> String {
                let mut s = String::new();
                while s.len() < min_bytes {
                    s.push_str(self.word());
                    if self.next().is_multiple_of(8) {
                        s.push(if self.next().is_multiple_of(3) {
                            '\n'
                        } else {
                            '。'
                        });
                    }
                }
                s
            }
        }

        let (db, dir) = open();
        let started = std::time::Instant::now();
        {
            let mut rng = Rng(0x2026_1103);
            let conn = db.connection();
            conn.execute_batch("BEGIN").unwrap();
            let first = date("2021-11-08");
            for n in 0..DAYS {
                let d: LogicalDate = first
                    .to_string()
                    .parse::<jiff::civil::Date>()
                    .unwrap()
                    .checked_add(jiff::Span::new().days(n))
                    .unwrap()
                    .to_string()
                    .parse()
                    .unwrap();
                crate::storage::table::day::ensure(conn, d, "Asia/Tokyo", at(T0)).unwrap();
                for i in 0..ITEMS_PER_DAY {
                    let kind = if i % 4 == 0 {
                        ItemKind::Blocker
                    } else {
                        ItemKind::Todo
                    };
                    let mut it = new_item(d, kind, &rng.text(40));
                    it.sort_key = format!("a{i:02}");
                    item::insert(conn, &it).unwrap();
                }
                text_doc::save(conn, TextDoc::Note, d, &rng.text(NOTE_BYTES), at(T0)).unwrap();
                text_doc::save(
                    conn,
                    TextDoc::Reflection,
                    d,
                    &rng.text(REFLECTION_BYTES),
                    at(T0),
                )
                .unwrap();
            }
            conn.execute_batch("COMMIT").unwrap();
        }
        db.connection()
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        let mb = |bytes: i64| bytes as f64 / 1_048_576.0;
        let size = std::fs::metadata(dir.path().join("yunagi.db"))
            .unwrap()
            .len();
        let bytes_of = |pattern: &str| -> i64 {
            db.connection()
                .query_row(
                    "SELECT coalesce(sum(pgsize), 0) FROM dbstat WHERE name LIKE ?1",
                    [pattern],
                    |r| r.get(0),
                )
                .unwrap()
        };
        println!(
            "generated {DAYS} days in {:.1?}: yunagi.db {:.1} MB \
             (search index {:.1} MB, of which copied text {:.1} MB)",
            started.elapsed(),
            mb(size as i64),
            mb(bytes_of("search_index%")),
            mb(bytes_of("search_index_content")),
        );

        let all = [
            SearchKind::Todo,
            SearchKind::Blocker,
            SearchKind::Note,
            SearchKind::Reflection,
        ];
        let cases: Vec<(&str, SearchQuery<'_>)> = vec![
            ("rare, 3+ chars", query("ロールバック手順書")),
            ("common, 3+ chars", query("レート制限")),
            ("very common, 3+ chars", query("確認の")),
            (
                "common, 3+ chars, limit 1000",
                SearchQuery {
                    limit: 1_000,
                    ..query("レート制限")
                },
            ),
            ("not found, 3+ chars", query("存在しない語")),
            ("2 chars", query("権限")),
            ("1 char", query("の")),
            ("2 chars, not found", query("鯨鮫")),
            (
                "2 chars, limit 1000",
                SearchQuery {
                    limit: 1_000,
                    ..query("権限")
                },
            ),
            (
                "3+ chars, 1 year, todo+blocker",
                SearchQuery {
                    from: Some(date("2025-01-01")),
                    to: Some(date("2025-12-31")),
                    kinds: &all[..2],
                    ..query("デプロイ")
                },
            ),
            (
                "2 chars, 1 year, note",
                SearchQuery {
                    from: Some(date("2025-01-01")),
                    to: Some(date("2025-12-31")),
                    kinds: &all[2..3],
                    ..query("負荷")
                },
            ),
        ];
        let mut worst = Duration::ZERO;
        for (label, q) in &cases {
            let mut times: Vec<Duration> = (0..RUNS)
                .map(|_| {
                    let t = std::time::Instant::now();
                    let hits = search(db.connection(), q).unwrap();
                    std::hint::black_box(hits);
                    t.elapsed()
                })
                .collect();
            times.sort();
            let p95 = times[(RUNS * 95).div_ceil(100) - 1];
            let hits = search(db.connection(), q).unwrap().len();
            println!(
                "{label:<34} hits={hits:>5}  median={:>8.1?}  p95={:>8.1?}  max={:>8.1?}",
                times[RUNS / 2],
                p95,
                times[RUNS - 1]
            );
            worst = worst.max(p95);
        }
        assert!(
            worst < Duration::from_secs(1),
            "p95 {worst:?} is over 1 second"
        );
    }
}
