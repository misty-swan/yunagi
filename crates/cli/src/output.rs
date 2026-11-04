//! テキスト出力（docs/design/05-mcp-cli-spec.md 4.1）。
//!
//! 元にするのは、ローカル API と直接モードの両方が返す同じ JSON の返り値（05 4.2・4.4）。コアの型ではなく
//! JSON から整えるので、どちらの経路でも出力が同じになる。`--json` は返り値をそのまま出す。

use std::fmt::Write as _;

use serde_json::Value;

use crate::error::{self, ErrorObject};

/// 操作 `method` の返り値 `value` を、人が読みやすいテキストにする（末尾は改行）。
pub fn render(method: &str, value: &Value) -> String {
    match method {
        "get_board" => board(value, true),
        "open_board" => board(value, false),
        "get_note" => note(value),
        "search" => search(value),
        "add_todo" => added("TODO", value),
        "add_blocker" => added("blocker", value),
        "edit_todo" | "edit_blocker" => changed(value, "Edited.", "Same text. Nothing changed."),
        "complete_todo" => changed(value, "Completed.", "Already done. Nothing changed."),
        "reopen_todo" => changed(value, "Reopened.", "Already open. Nothing changed."),
        "resolve_blocker" => resolved(value),
        "append_note" | "append_reflection" => changed(value, "Appended.", "Nothing changed."),
        "edit_note" => changed(value, "Edited.", "Nothing changed."),
        // 知らない操作は JSON のまま出す
        _ => format!("{value}\n"),
    }
}

fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

/// 書き込みの成功の文。実際に変えたとき（`updated` が真）は `done`、何も変えなかったときは
/// `unchanged` を、改行をつけて返す（05 3.2・4.1）。
fn changed(value: &Value, done: &str, unchanged: &'static str) -> String {
    let updated = value
        .get("updated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    format!("{}\n", if updated { done } else { unchanged })
}

/// `add_todo`・`add_blocker`：追加した項目の ID が分かるようにする（あとで編集や完了に使う）。
fn added(kind: &str, value: &Value) -> String {
    format!(
        "Added {kind} {} ({}).\n",
        str_of(value, "id"),
        str_of(value, "date")
    )
}

/// `resolve_blocker`（05 3章）：解消した・解消メモを書いた・何も変えていない。`--note` を付けずに解消した
/// 障害に前の解消メモが残っていたら、その本文を添える（利用者に伝えるため）。
fn resolved(value: &Value) -> String {
    let flag = |key| value.get(key).and_then(Value::as_bool).unwrap_or(false);
    let (resolved, note_written) = (flag("resolved"), flag("note_written"));
    let mut out = match (resolved, note_written) {
        (true, true) => "Resolved. Note added.\n".to_owned(),
        (true, false) => "Resolved.\n".to_owned(),
        (false, true) => "Note added.\n".to_owned(),
        (false, false) => "Already resolved. Nothing changed.\n".to_owned(),
    };
    if resolved
        && !note_written
        && let Some(note) = value.get("resolution_note").and_then(Value::as_str)
    {
        let _ = writeln!(out, "Existing resolution note: {note}");
    }
    out
}

fn board(value: &Value, full: bool) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# {}", str_of(value, "date"));
    let _ = writeln!(out, "\n## TODO");
    let todos = value["todos"].as_array().map_or(&[][..], Vec::as_slice);
    if todos.is_empty() {
        let _ = writeln!(out, "(none)");
    }
    for todo in todos {
        let mark = match str_of(todo, "status") {
            "done" => "[x]",
            "dropped" => "[-]",
            _ => "[ ]",
        };
        let _ = writeln!(
            out,
            "{mark} {}  {}{}",
            str_of(todo, "id"),
            str_of(todo, "text"),
            item_notes(todo)
        );
    }
    let _ = writeln!(out, "\n## Blockers");
    let blockers = value["blockers"].as_array().map_or(&[][..], Vec::as_slice);
    if blockers.is_empty() {
        let _ = writeln!(out, "(none)");
    }
    for blocker in blockers {
        let mark = if str_of(blocker, "status") == "resolved" {
            "[x]"
        } else {
            "[ ]"
        };
        let _ = writeln!(
            out,
            "{mark} {}  {}{}",
            str_of(blocker, "id"),
            str_of(blocker, "text"),
            item_notes(blocker)
        );
        if let Some(note) = blocker.get("resolution_note").and_then(Value::as_str) {
            let _ = writeln!(out, "    -> {note}");
        }
    }
    if full {
        let reflection = str_of(value, "reflection");
        let _ = writeln!(out, "\n## Reflection");
        if reflection.is_empty() {
            let _ = writeln!(out, "(none)");
        } else {
            let _ = writeln!(out, "{}", reflection.trim_end_matches('\n'));
        }
        let note = &value["note"];
        let length = note["length"].as_u64().unwrap_or(0);
        let _ = writeln!(out, "\n## Note ({length} characters)");
        let headings = note["headings"].as_array().map_or(&[][..], Vec::as_slice);
        if headings.is_empty() {
            let _ = writeln!(out, "(no headings)");
        }
        for heading in headings {
            let _ = writeln!(out, "- {}", heading.as_str().unwrap_or(""));
        }
    }
    out
}

/// 項目の行の後ろに付ける注記（取りやめた理由・持ち越しの回数と持ち越し先）。
fn item_notes(item: &Value) -> String {
    let mut notes = Vec::new();
    if let Some(reason) = item.get("close_reason").and_then(Value::as_str) {
        notes.push(format!("closed: {reason}"));
    }
    if let Some(count) = item.get("carry_count").and_then(Value::as_u64)
        && count > 0
    {
        notes.push(format!("carried {count}x"));
    }
    if let (Some(id), Some(date)) = (
        item.get("carried_to_id").and_then(Value::as_str),
        item.get("carried_to_date").and_then(Value::as_str),
    ) {
        notes.push(format!("carried over to {date} as {id}"));
    }
    if notes.is_empty() {
        String::new()
    } else {
        format!("  ({})", notes.join(", "))
    }
}

/// `get_note`：全文なら本文、見出し指定なら当たった区間の `text` を順に出す。区間が複数のときは、
/// 見出しの文字と `start_line` が分かる区切りの行（`-- 10:00 定例 (line 12) --`）を前に付ける。
fn note(value: &Value) -> String {
    if let Some(body) = value.get("body").and_then(Value::as_str) {
        return ensure_newline(body);
    }
    let sections = value["sections"].as_array().map_or(&[][..], Vec::as_slice);
    let mut out = String::new();
    for section in sections {
        if sections.len() > 1 {
            let _ = writeln!(
                out,
                "-- {} (line {}) --",
                str_of(section, "heading"),
                section["start_line"].as_u64().unwrap_or(0)
            );
        }
        out.push_str(&ensure_newline(str_of(section, "text")));
    }
    out
}

fn ensure_newline(text: &str) -> String {
    if text.ends_with('\n') || text.is_empty() {
        text.to_owned()
    } else {
        format!("{text}\n")
    }
}

/// `search`：1 件を 1 行で出す。項目は `日付 種類 ID 本文`、ノート・振り返りは一致した行ごとに
/// `日付 種類:行番号 行の本文`。
fn search(value: &Value) -> String {
    let hits = value["hits"].as_array().map_or(&[][..], Vec::as_slice);
    let mut out = String::new();
    for hit in hits {
        let date = str_of(hit, "date");
        let kind = str_of(hit, "kind");
        match kind {
            "todo" | "blocker" => {
                let _ = writeln!(
                    out,
                    "{date} {kind} {}  {}{}",
                    str_of(hit, "id"),
                    str_of(hit, "text"),
                    item_notes(hit)
                );
                if let Some(note) = hit.get("resolution_note").and_then(Value::as_str) {
                    let _ = writeln!(out, "    -> {note}");
                }
            }
            _ => {
                let matches = hit["matches"].as_array().map_or(&[][..], Vec::as_slice);
                if matches.is_empty() {
                    // 行を切り出せなかった（検索語が複数の行にまたがる、など）。全文は note get
                    let _ = writeln!(
                        out,
                        "{date} {kind}  (no single matching line; read it with note get)"
                    );
                }
                for m in matches {
                    let _ = writeln!(
                        out,
                        "{date} {kind}:{}  {}",
                        m["line"].as_u64().unwrap_or(0),
                        str_of(m, "text")
                    );
                }
            }
        }
    }
    if hits.is_empty() {
        out.push_str("No results.\n");
    }
    out
}

/// エラーを標準エラーに出す文にする（`reason` も出す。05 4.1）。
pub fn render_error(error: &ErrorObject) -> String {
    let mut out = format!("yunagi: {}", error::code(error));
    if let Some(reason) = error::reason(error) {
        let _ = write!(out, " ({reason})");
    }
    let _ = write!(out, ": {}", error::message(error));
    if let Some(scope) = error.get("required_scope").and_then(Value::as_str) {
        let _ = write!(out, " [required scope: {scope}]");
    }
    // 次の動きを決める手がかり（`--json` では欄として出ているが、テキストでは `message` に入っていない）
    if let Some(id) = error.get("carried_to_id").and_then(Value::as_str) {
        let date = error
            .get("carried_to_date")
            .and_then(Value::as_str)
            .unwrap_or("");
        let _ = write!(out, " [carried to: {id} ({date})]");
    }
    if let Some(note) = error.get("resolution_note").and_then(Value::as_str) {
        let _ = write!(out, " [existing resolution note: {note}]");
    }
    if error.get("item_kind").is_some()
        && let Some(command) = error.get("use_operation").map(|op| match op.as_str() {
            Some(op) => format!("use {}", cli_command(op)),
            None => "ask the user".to_owned(),
        })
    {
        let _ = write!(out, " [{command}]");
    }
    out.push('\n');
    out
}

/// 操作の名前（MCP のツール名）に当たる CLI のコマンド。
fn cli_command(operation: &str) -> String {
    match operation {
        "edit_todo" => "yunagi todo edit",
        "complete_todo" => "yunagi todo complete",
        "reopen_todo" => "yunagi todo reopen",
        "edit_blocker" => "yunagi blocker edit",
        "resolve_blocker" => "yunagi blocker resolve",
        other => other,
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_board_lists_items_with_ids_and_notes() {
        let board = json!({
            "date": "2026-11-02",
            "todos": [
                {"id": "T1", "text": "survey", "status": "open", "close_reason": null, "carry_count": 2, "carried_to_id": null, "carried_to_date": null},
                {"id": "T2", "text": "done one", "status": "done", "close_reason": null, "carry_count": 0},
                {"id": "T3", "text": "gone", "status": "dropped", "close_reason": "split", "carry_count": 0},
                {"id": "T4", "text": "moved", "status": "open", "close_reason": null, "carry_count": 0, "carried_to_id": "T9", "carried_to_date": "2026-11-03"},
            ],
            "blockers": [
                {"id": "B1", "text": "waiting", "status": "resolved", "carry_count": 0, "resolution_note": "fixed"},
            ],
            "reflection": "good day\n",
            "note": {"headings": ["10:00 sync"], "length": 12},
        });
        let text = render("get_board", &board);
        assert!(text.starts_with("# 2026-11-02\n\n## TODO\n"));
        assert!(text.contains("[ ] T1  survey  (carried 2x)"));
        assert!(text.contains("[x] T2  done one\n"));
        assert!(text.contains("[-] T3  gone  (closed: split)"));
        assert!(text.contains("(carried over to 2026-11-03 as T9)"));
        assert!(text.contains("[x] B1  waiting\n    -> fixed\n"));
        assert!(text.contains("## Reflection\ngood day\n"));
        assert!(text.contains("## Note (12 characters)\n- 10:00 sync\n"));
        // open_board は振り返りとノートを出さない
        let open = render("open_board", &board);
        assert!(!open.contains("Reflection") && !open.contains("Note"));
    }

    #[test]
    fn an_empty_board_says_none() {
        let board = json!({"date": "2026-11-02", "todos": [], "blockers": [], "reflection": "", "note": {"headings": [], "length": 0}});
        let text = render("get_board", &board);
        assert_eq!(text.matches("(none)").count(), 3);
        assert!(text.contains("(no headings)"));
    }

    #[test]
    fn the_note_prints_the_body_or_the_sections() {
        let whole = json!({"date": "d", "rev": 1, "body": "a\nb"});
        assert_eq!(render("get_note", &whole), "a\nb\n");
        let one = json!({"date": "d", "rev": 1, "sections": [{"heading": "10:00 x", "start_line": 3, "text": "## 10:00 x\nbody\n"}]});
        assert_eq!(render("get_note", &one), "## 10:00 x\nbody\n");
        let two = json!({"date": "d", "rev": 1, "sections": [
            {"heading": "10:00 x", "start_line": 3, "text": "## 10:00 x\none"},
            {"heading": "10:00 x", "start_line": 9, "text": "## 10:00 x\ntwo"},
        ]});
        assert_eq!(
            render("get_note", &two),
            "-- 10:00 x (line 3) --\n## 10:00 x\none\n-- 10:00 x (line 9) --\n## 10:00 x\ntwo\n"
        );
    }

    #[test]
    fn search_prints_one_line_per_hit_or_matched_line() {
        let found = json!({"query": "q", "hits": [
            {"kind": "todo", "date": "2026-11-02", "id": "T1", "text": "q one", "carried_to_id": null, "carried_to_date": null},
            {"kind": "note", "date": "2026-11-01", "matches": [{"line": 2, "text": "has q"}]},
        ]});
        assert_eq!(
            render("search", &found),
            "2026-11-02 todo T1  q one\n2026-11-01 note:2  has q\n"
        );
        assert_eq!(
            render("search", &json!({"query": "q", "hits": []})),
            "No results.\n"
        );
    }

    #[test]
    fn write_results_use_the_texts_of_the_spec() {
        let done = |method, v: Value| render(method, &v);
        assert_eq!(
            done(
                "add_todo",
                json!({"id": "T1", "date": "2026-11-04", "text": "x"})
            ),
            "Added TODO T1 (2026-11-04).\n"
        );
        assert_eq!(
            done(
                "add_blocker",
                json!({"id": "B1", "date": "2026-11-04", "text": "x"})
            ),
            "Added blocker B1 (2026-11-04).\n"
        );
        for (method, updated, text) in [
            ("edit_todo", true, "Edited.\n"),
            ("edit_blocker", false, "Same text. Nothing changed.\n"),
            ("complete_todo", true, "Completed.\n"),
            ("complete_todo", false, "Already done. Nothing changed.\n"),
            ("reopen_todo", true, "Reopened.\n"),
            ("reopen_todo", false, "Already open. Nothing changed.\n"),
            ("append_note", true, "Appended.\n"),
            ("append_reflection", false, "Nothing changed.\n"),
            ("edit_note", true, "Edited.\n"),
            ("edit_note", false, "Nothing changed.\n"),
        ] {
            assert_eq!(done(method, json!({"updated": updated})), text, "{method}");
        }
    }

    #[test]
    fn resolving_a_blocker_tells_what_happened_and_the_note_left_behind() {
        let r = |resolved, note_written, note: Value| {
            render(
                "resolve_blocker",
                &json!({"updated": resolved || note_written, "resolved": resolved, "note_written": note_written, "resolution_note": note}),
            )
        };
        assert_eq!(r(true, false, Value::Null), "Resolved.\n");
        assert_eq!(r(true, true, json!("fixed")), "Resolved. Note added.\n");
        assert_eq!(r(false, true, json!("fixed")), "Note added.\n");
        assert_eq!(
            r(false, false, json!("fixed")),
            "Already resolved. Nothing changed.\n"
        );
        // `--note` を付けずに解消したとき、前の解消メモが残っていれば伝える
        assert_eq!(
            r(true, false, json!("old memo")),
            "Resolved.\nExisting resolution note: old memo\n"
        );
    }

    #[test]
    fn errors_show_what_decides_the_next_move() {
        let e = json!({"code": "conflict", "reason": "carried_over", "carried_to_id": "T9", "carried_to_date": "2026-11-05", "message": "m"});
        assert_eq!(
            render_error(&e),
            "yunagi: conflict (carried_over): m [carried to: T9 (2026-11-05)]\n"
        );
        let e = json!({"code": "conflict", "reason": "note_exists", "resolution_note": "old", "message": "m"});
        assert!(render_error(&e).ends_with("[existing resolution note: old]\n"));
        let e = json!({"code": "invalid_argument", "item_kind": "blocker", "use_operation": "edit_blocker", "message": "m"});
        assert_eq!(
            render_error(&e),
            "yunagi: invalid_argument: m [use yunagi blocker edit]\n"
        );
        let e = json!({"code": "invalid_argument", "item_kind": "blocker", "use_operation": null, "message": "m"});
        assert!(render_error(&e).ends_with("[ask the user]\n"));
    }

    #[test]
    fn errors_show_the_reason_and_the_scope() {
        let e = json!({"code": "api_error", "reason": "result_unknown", "message": "lost"});
        assert_eq!(
            render_error(&e),
            "yunagi: api_error (result_unknown): lost\n"
        );
        let e = json!({"code": "permission_denied", "required_scope": "read", "message": "no"});
        assert_eq!(
            render_error(&e),
            "yunagi: permission_denied: no [required scope: read]\n"
        );
    }
}
