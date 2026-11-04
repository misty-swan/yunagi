//! エージェント・CLI の全文検索（MCP・CLI 仕様書 3章 `search`、4.2）。
//!
//! 検索そのもの（一致の決まり、並び、持ち越し先）は [`crate::storage::search`] が行う。ここは、呼び出し元の
//! クライアントを受け取り、はじめに `read` スコープを確かめてから検索する入り口と、ノート・振り返りの
//! 一致した行の切り出し（本文すべてを返すと大きいため）を持つ。画面の検索パレットは、エージェントの
//! 操作ではないので権限を確かめない（`storage::search::search` を直接使う）。

use rusqlite::Connection;

use super::agent::{self, ReadError};
use crate::storage::Ulid;
use crate::storage::search::{self as store, SearchHit, SearchKind, SearchQuery};

/// 件数の指定が無いときの上限。
pub const DEFAULT_LIMIT: usize = 50;
/// 件数の上限の最大。これを超える指定は、この値にする。
pub const MAX_LIMIT: usize = 200;
/// ノート・振り返りの 1 件につき返す、一致した行の数の上限。
pub const MAX_MATCHED_LINES: usize = 5;

/// 一致した行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchedLine {
    /// 本文の中の行の番号（1 始まり。`edit_note` の `--line` と同じ数え方）。
    pub line: usize,
    /// 行の本文（保存してあるとおり。CR は行の文字として残る）。
    pub text: String,
}

/// 検索結果の 1 件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub hit: SearchHit,
    /// ノート・振り返りの、一致した行（上から [`MAX_MATCHED_LINES`] まで）。項目では空。検索語が複数の行に
    /// またがるときも空。
    pub lines: Vec<MatchedLine>,
}

/// 検索する。`client_id` に `read` が要る（MCP・CLI 仕様書 2章。権限はコアが確かめる）。`query.limit` は
/// [`MAX_LIMIT`] までにする。
pub fn search(
    conn: &Connection,
    client_id: Ulid,
    query: &SearchQuery<'_>,
) -> Result<Vec<Found>, ReadError> {
    let query = SearchQuery {
        limit: query.limit.min(MAX_LIMIT),
        ..*query
    };
    agent::read_as(conn, client_id, || {
        let hits = store::search(conn, &query)?;
        let needle = query.text.trim();
        Ok(hits
            .into_iter()
            .map(|hit| {
                let lines = match hit.kind {
                    SearchKind::Note | SearchKind::Reflection => matched_lines(&hit.body, needle),
                    SearchKind::Todo | SearchKind::Blocker => Vec::new(),
                };
                Found { hit, lines }
            })
            .collect())
    })
}

/// `body` の行のうち `needle` を含むもの（ASCII の英字は大文字・小文字を区別しない。検索の決まりと
/// そろえる）。
fn matched_lines(body: &str, needle: &str) -> Vec<MatchedLine> {
    let needle = needle.to_ascii_lowercase();
    super::note_body::lines(body)
        .enumerate()
        .filter(|(_, line)| line.to_ascii_lowercase().contains(&needle))
        .take(MAX_MATCHED_LINES)
        .map(|(i, line)| MatchedLine {
            line: i + 1,
            text: line.to_owned(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use jiff::tz::TimeZone;

    use super::*;
    use crate::domain::agent::Agent;
    use crate::domain::{item, text_doc};
    use crate::storage::table::agent_client::{self, Scope};
    use crate::storage::table::test_support::{at, date, open};

    fn query<'a>(text: &'a str) -> SearchQuery<'a> {
        SearchQuery {
            text,
            from: None,
            to: None,
            kinds: &[],
            limit: DEFAULT_LIMIT,
        }
    }

    #[test]
    fn matched_lines_are_numbered_from_one_and_ignore_ascii_case() {
        let lines = matched_lines("first\nAPI limit\nthird\napi again", "api");
        assert_eq!(
            lines,
            vec![
                MatchedLine {
                    line: 2,
                    text: "API limit".into()
                },
                MatchedLine {
                    line: 4,
                    text: "api again".into()
                },
            ]
        );
    }

    #[test]
    fn matched_lines_stop_at_the_limit_and_a_multiline_needle_matches_none() {
        let body = "x\n".repeat(20);
        assert_eq!(matched_lines(&body, "x").len(), MAX_MATCHED_LINES);
        assert!(matched_lines("a\nb", "a\nb").is_empty());
    }

    #[test]
    fn searching_needs_the_read_scope_first() {
        let (db, _dir) = open();
        let conn = db.connection();
        let denied = Ulid::generate();
        let err = search(conn, denied, &query("abc")).unwrap_err();
        assert!(matches!(
            err,
            ReadError::PermissionDenied(p) if p.required_scope == Scope::Read
        ));
        // read を持つ cli は検索できる（何も無ければ空）
        let found = search(conn, agent_client::CLI_ID, &query("abc")).unwrap();
        assert!(found.is_empty());
    }

    #[test]
    fn items_and_note_lines_are_found_and_the_limit_is_capped() {
        let (db, _dir) = open();
        let conn = db.connection();
        let now = at("2026-11-02T01:00:00Z");
        let tz = TimeZone::UTC;
        let cli = Agent::without_ui(agent_client::CLI_ID);
        crate::domain::agent::test_support::grant(
            conn,
            agent_client::CLI_ID,
            &[Scope::Todo, Scope::Append],
        );
        let d = date("2026-11-02");
        item::add_todo(conn, &cli, d, "rate limit survey", &tz, now).unwrap();
        text_doc::append_note(
            conn,
            d,
            "intro\nthe rate LIMIT is 600",
            None,
            &cli,
            &tz,
            now,
        )
        .unwrap();
        let found = search(
            conn,
            agent_client::CLI_ID,
            &SearchQuery {
                limit: usize::MAX,
                ..query("rate limit")
            },
        )
        .unwrap();
        let kinds: Vec<_> = found.iter().map(|f| f.hit.kind).collect();
        assert_eq!(kinds, [SearchKind::Todo, SearchKind::Note]);
        assert!(found[0].lines.is_empty());
        assert_eq!(found[1].lines.len(), 1);
        assert_eq!(found[1].lines[0].line, 2);
    }
}
