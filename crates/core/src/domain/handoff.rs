//! 依頼文の書き出し（A-06。MCP・CLI 仕様書 5章）：選んだ TODO 1件を、エージェント向けの Markdown に
//! する。
//!
//! - [`export`]：エージェント・CLI の入り口（`export_handoff`）。呼び出し元のクライアントを受け取り、
//!   はじめに `read` を確かめる（MCP・CLI 仕様書 2章）。
//! - [`build`]：画面の入り口（Mod+Shift+A）。画面の操作はエージェントの操作ではないので、権限を
//!   確かめない。
//! - [`render`]：Markdown にする部分（データベースに触れない）。
//!
//! 読み取りだけで、change_log にも記録しない。

use rusqlite::Connection;

use super::agent::{self, PermissionDenied};
use super::logical_date::LogicalDate;
use super::note_send::{self, RequestContext};
use crate::storage::table::item::{self as item_table, ItemKind};
use crate::storage::{StorageError, Ulid, in_read_transaction};

/// 書き出した依頼文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handoff {
    /// 対象の TODO の id。
    pub id: Ulid,
    /// 依頼文（Markdown。末尾は改行1つ）。
    pub markdown: String,
}

/// 書き出せなかった理由。何も読み書きしていない（読み取りだけで、書くものも無い）。
#[derive(Debug)]
pub enum HandoffError {
    /// `read` スコープが許可されていない（`permission_denied`）。
    PermissionDenied(PermissionDenied),
    /// ID の項目が無いか、削除されている（`not_found`）。
    NotFound,
    /// ID が障害を指す（`invalid_argument`）。書き出せるのは TODO だけ。
    NotTodo,
    /// 読み取りに失敗した。
    Storage(StorageError),
}

impl std::fmt::Display for HandoffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PermissionDenied(e) => e.fmt(f),
            Self::NotFound => f.write_str("the item does not exist"),
            Self::NotTodo => f.write_str("the item is a blocker, not a TODO"),
            Self::Storage(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for HandoffError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(e) => Some(e),
            _ => None,
        }
    }
}

impl From<PermissionDenied> for HandoffError {
    fn from(e: PermissionDenied) -> Self {
        Self::PermissionDenied(e)
    }
}

impl From<StorageError> for HandoffError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

impl From<rusqlite::Error> for HandoffError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Storage(e.into())
    }
}

/// TODO `item_id` の依頼文を書き出す。`client_id` に `read` が要る（権限はコアが確かめる）。`today` は
/// 末尾の日付（呼び出し側が設定とタイムゾーンで求めた論理日付の今日）。
pub fn export(
    conn: &Connection,
    client_id: Ulid,
    item_id: Ulid,
    today: LogicalDate,
) -> Result<Handoff, HandoffError> {
    agent::read_as(conn, client_id, || read(conn, item_id, today))
}

/// 画面の入り口。[`export`] と同じ依頼文を、権限を確かめずに書き出す。
pub fn build(
    conn: &Connection,
    item_id: Ulid,
    today: LogicalDate,
) -> Result<Handoff, HandoffError> {
    in_read_transaction(conn, || read(conn, item_id, today))
}

fn read(conn: &Connection, item_id: Ulid, today: LogicalDate) -> Result<Handoff, HandoffError> {
    let item = item_table::get(conn, item_id)?
        .filter(|i| i.deleted_at.is_none())
        .ok_or(HandoffError::NotFound)?;
    if item.kind != ItemKind::Todo {
        return Err(HandoffError::NotTodo);
    }
    let context = note_send::request_context(conn, item_id)?;
    Ok(Handoff {
        id: item_id,
        markdown: render(&item.text, context.as_ref(), today),
    })
}

/// 依頼文の Markdown（MCP・CLI 仕様書 5章）。見出しと注記は英語に固定する（画面の言語の設定に
/// かかわらず、貼り付けた先で同じ形にするため）。中身の無い節は出さない。
///
/// - `## Task`：TODO の本文。
/// - `## Context (from today's notes)`：送り元の区間の行（[`RequestContext::context_texts`]）。区間に見出しが
///   あれば、各行の前に `<見出しの文字>: ` を付ける。送り元のノートの日が `today` でなければ、見出しは
///   `(from notes on YYYY-MM-DD)`。
/// - `## Related blockers`：区間の行から送った未解消の障害の本文。
/// - 末尾：`(Exported from Yunagi on YYYY-MM-DD)`。
///
/// ノートから送られていない TODO（`context` が `None`）は、Task だけを書き出す。
pub fn render(task: &str, context: Option<&RequestContext>, today: LogicalDate) -> String {
    let mut out = format!("## Task\n{task}\n");
    if let Some(context) = context {
        let texts = context.context_texts();
        if !texts.is_empty() {
            let from = if context.date == today {
                "today's notes".to_owned()
            } else {
                format!("notes on {}", context.date)
            };
            out.push_str(&format!("\n## Context (from {from})\n"));
            let prefix = context
                .section
                .heading
                .as_ref()
                .map_or_else(String::new, |h| format!("{}: ", h.text));
            for text in texts {
                out.push_str(&format!("- {prefix}{text}\n"));
            }
        }
        if !context.blockers.is_empty() {
            out.push_str("\n## Related blockers\n");
            for blocker in &context.blockers {
                out.push_str(&format!("- {}\n", blocker.text));
            }
        }
    }
    out.push_str(&format!("\n(Exported from Yunagi on {today})\n"));
    out
}

#[cfg(test)]
mod tests {
    use std::ops::RangeInclusive;

    use jiff::tz::TimeZone;

    use super::*;
    use crate::domain::note_send::{SendLines, send};
    use crate::domain::text_doc::{self, TextDoc};
    use crate::domain::{carryover, item};
    use crate::storage::table::agent_client::{self, Scope};
    use crate::storage::table::test_support::{at, date, open};

    const T1: &str = "2026-11-03T02:00:00Z";

    fn tokyo() -> TimeZone {
        TimeZone::get("Asia/Tokyo").unwrap()
    }

    fn send_lines(
        conn: &Connection,
        d: &str,
        body: &str,
        lines: RangeInclusive<usize>,
        kind: ItemKind,
    ) -> Vec<Ulid> {
        let base_rev = text_doc::get(conn, TextDoc::Note, date(d)).unwrap().rev;
        let sent = send(
            conn,
            &SendLines {
                date: date(d),
                body,
                base_rev,
                lines,
                kind,
            },
            &tokyo(),
            at(T1),
        )
        .unwrap();
        sent.sent.iter().map(|s| s.item.id).collect()
    }

    #[test]
    fn a_todo_from_a_note_gets_its_section_and_open_blockers() {
        let (db, _dir) = open();
        let conn = db.connection();
        let body = "前置き\n## 10:00 定例\n- API のレート制限を調査\n- 外部APIの429が増えている。上限は1分あたり600件らしい\n  - リトライは指数バックオフで\n\n- ステージング環境の権限待ち\n## 11:00 別件\n- 関係ない";
        let todo = send_lines(conn, "2026-11-03", body, 3..=3, ItemKind::Todo)[0];
        send_lines(conn, "2026-11-03", body, 7..=7, ItemKind::Blocker);
        let h = build(conn, todo, date("2026-11-03")).unwrap();
        assert_eq!(h.id, todo);
        assert_eq!(
            h.markdown,
            "## Task\nAPI のレート制限を調査\n\n\
             ## Context (from today's notes)\n\
             - 10:00 定例: 外部APIの429が増えている。上限は1分あたり600件らしい\n\
             - 10:00 定例: リトライは指数バックオフで\n\n\
             ## Related blockers\n- ステージング環境の権限待ち\n\n\
             (Exported from Yunagi on 2026-11-03)\n"
        );
    }

    #[test]
    fn a_todo_that_did_not_come_from_a_note_has_only_the_task() {
        let (db, _dir) = open();
        let conn = db.connection();
        let cli = agent_client::CLI_ID;
        crate::domain::agent::test_support::grant(conn, cli, &[Scope::Todo]);
        let added = item::add_todo(
            conn,
            &crate::domain::agent::Agent::without_ui(cli),
            date("2026-11-03"),
            "調べる",
            &tokyo(),
            at(T1),
        )
        .unwrap();
        let h = export(conn, cli, added.id, date("2026-11-04")).unwrap();
        assert_eq!(
            h.markdown,
            "## Task\n調べる\n\n(Exported from Yunagi on 2026-11-04)\n"
        );
    }

    #[test]
    fn the_prelude_has_no_heading_prefix_and_an_old_note_is_dated() {
        let (db, _dir) = open();
        let conn = db.connection();
        let body = "メモ\n- 調べる\n\n```\n- ログ  \n\n```\n---\n## 10:00 後";
        let todo = send_lines(conn, "2026-11-02", body, 2..=2, ItemKind::Todo)[0];
        let h = build(conn, todo, date("2026-11-03")).unwrap();
        // 送った行は載せない。空行・区切り線・囲みの印は載せず、囲みの中は印のように見えてもそのまま
        assert_eq!(
            h.markdown,
            "## Task\n調べる\n\n## Context (from notes on 2026-11-02)\n- メモ\n- - ログ\n\n(Exported from Yunagi on 2026-11-03)\n"
        );
    }

    #[test]
    fn a_section_with_nothing_to_show_has_no_context() {
        let (db, _dir) = open();
        let conn = db.connection();
        let body = "## 10:00 定例\n- 調べる\n";
        let todo = send_lines(conn, "2026-11-03", body, 2..=2, ItemKind::Todo)[0];
        let h = build(conn, todo, date("2026-11-03")).unwrap();
        assert_eq!(
            h.markdown,
            "## Task\n調べる\n\n(Exported from Yunagi on 2026-11-03)\n"
        );
    }

    #[test]
    fn a_carried_todo_uses_the_original_note() {
        let (db, _dir) = open();
        let conn = db.connection();
        let body = "## 10:00 定例\n- 調べる\n- 背景の話";
        let todo = send_lines(conn, "2026-11-02", body, 2..=2, ItemKind::Todo)[0];
        let done = carryover::confirm(
            conn,
            date("2026-11-03"),
            &carryover::Decision {
                from: date("2026-11-02"),
                selection: carryover::Selection::All,
                reviews: std::collections::HashMap::new(),
            },
            &tokyo(),
            at(T1),
        )
        .unwrap();
        let carried = done
            .created
            .iter()
            .find(|i| i.carried_from_id == Some(todo))
            .unwrap();
        let h = build(conn, carried.id, date("2026-11-03")).unwrap();
        assert!(
            h.markdown
                .contains("(from notes on 2026-11-02)\n- 10:00 定例: 背景の話\n")
        );
    }

    #[test]
    fn errors_in_the_order_permission_not_found_blocker() {
        let (db, _dir) = open();
        let conn = db.connection();
        let body = "- 調べる\n- 待ち";
        let todo = send_lines(conn, "2026-11-03", body, 1..=1, ItemKind::Todo)[0];
        let blocker = send_lines(conn, "2026-11-03", body, 2..=2, ItemKind::Blocker)[0];
        let today = date("2026-11-03");
        // 登録されていないクライアントには、何も許可されない（read も）
        let unknown = Ulid::generate();
        let err = export(conn, unknown, todo, today).unwrap_err();
        assert!(matches!(
            err,
            HandoffError::PermissionDenied(p) if p.required_scope == Scope::Read
        ));
        // 権限が先：存在しない ID でも permission_denied
        let err = export(conn, unknown, Ulid::generate(), today).unwrap_err();
        assert!(matches!(err, HandoffError::PermissionDenied(_)));
        let cli = agent_client::CLI_ID;
        assert!(matches!(
            export(conn, cli, Ulid::generate(), today).unwrap_err(),
            HandoffError::NotFound
        ));
        assert!(matches!(
            export(conn, cli, blocker, today).unwrap_err(),
            HandoffError::NotTodo
        ));
        // 削除した TODO は、無いものとして扱う
        item::delete(conn, todo, at(T1)).unwrap();
        assert!(matches!(
            export(conn, cli, todo, today).unwrap_err(),
            HandoffError::NotFound
        ));
    }

    #[test]
    fn a_closed_or_carried_todo_is_still_exported() {
        let (db, _dir) = open();
        let conn = db.connection();
        let todo = send_lines(conn, "2026-11-03", "- 調べる", 1..=1, ItemKind::Todo)[0];
        item::set_closed(conn, todo, Some(at(T1)), at(T1)).unwrap();
        let h = build(conn, todo, date("2026-11-03")).unwrap();
        assert!(h.markdown.starts_with("## Task\n調べる\n"));
    }
}
