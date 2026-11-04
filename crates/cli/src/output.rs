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
        // 依頼文はそのまま出す（貼り付けやパイプで使うため。05 5章）。欄が無い応答は JSON のまま出す
        "export_handoff" => match value.get("markdown").and_then(Value::as_str) {
            Some(markdown) => ensure_newline(markdown),
            None => format!("{value}\n"),
        },
        "add_todo" => added("TODO", value),
        "add_blocker" => added("blocker", value),
        "edit_todo" | "edit_blocker" => changed(value, "Edited.", "Same text. Nothing changed."),
        "complete_todo" => changed(value, "Completed.", "Already done. Nothing changed."),
        "reopen_todo" => changed(value, "Reopened.", "Already open. Nothing changed."),
        "resolve_blocker" => resolved(value),
        "append_note" | "append_reflection" => changed(value, "Appended.", "Nothing changed."),
        "edit_note" => changed(value, "Edited.", "Nothing changed."),
        "list_changes" => changes(value),
        // 知らない操作は JSON のまま出す
        _ => format!("{value}\n"),
    }
}

fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

/// 書き込みの成功の文。実際に変えたとき（`updated` が真）は `done`、何も変えなかったときは
/// `unchanged` を、改行をつけて返す（05 3.2・4.1）。
/// 応答に `updated` が無いときは、書き込みが行われたかを偽って伝えないよう、JSON のまま出す。
fn changed(value: &Value, done: &str, unchanged: &'static str) -> String {
    match value.get("updated").and_then(Value::as_bool) {
        Some(true) => format!("{done}\n"),
        Some(false) => format!("{unchanged}\n"),
        None => format!("{value}\n"),
    }
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
    let flag = |key| value.get(key).and_then(Value::as_bool);
    // 欄が無い応答は、何も変えていないと偽らず、JSON のまま出す（`changed` と同じ）
    let (Some(resolved), Some(note_written)) = (flag("resolved"), flag("note_written")) else {
        return format!("{value}\n");
    };
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

/// 保存した時刻（UTC）を、端末のタイムゾーンの `YYYY-MM-DD HH:MM` にする。読めなければそのまま。
pub fn local_time(utc: &str) -> String {
    match utc.parse::<jiff::Timestamp>() {
        Ok(t) => t
            .to_zoned(jiff::tz::TimeZone::try_system().unwrap_or(jiff::tz::TimeZone::UTC))
            .strftime("%Y-%m-%d %H:%M")
            .to_string(),
        Err(_) => utc.to_owned(),
    }
}

/// 値を1行に収めて短くする（欄の前後の値の表示）。
fn brief(value: &Value) -> String {
    let text = match value {
        Value::String(s) => format!("{s:?}"),
        other => other.to_string(),
    };
    if text.chars().count() > 60 {
        let cut: String = text.chars().take(57).collect();
        format!("{cut}...")
    } else {
        text
    }
}

/// `agent permissions` の一覧（`only` があればそのクライアントだけ）。見つからなければ `None`。
pub fn agents(value: &Value, only: Option<&str>) -> Option<String> {
    let list = value["agents"].as_array().map_or(&[][..], Vec::as_slice);
    let rows: Vec<&Value> = list
        .iter()
        .filter(|a| only.is_none_or(|name| str_of(a, "name") == name))
        .collect();
    if rows.is_empty() {
        return None;
    }
    let scopes = |a: &Value| {
        let names: Vec<&str> = a["scopes"]
            .as_array()
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .filter_map(Value::as_str)
            .collect();
        if names.is_empty() {
            "(none)".to_owned()
        } else {
            names.join(", ")
        }
    };
    let width = rows
        .iter()
        .map(|a| str_of(a, "name").chars().count())
        .max()
        .unwrap_or(0)
        .max(6);
    let scope_width = rows
        .iter()
        .map(|a| scopes(a).chars().count())
        .max()
        .unwrap_or(0)
        .max(6);
    let mut out = format!(
        "{:<width$}  {:<scope_width$}  LAST SEEN\n",
        "CLIENT", "SCOPES"
    );
    for a in rows {
        let seen = a
            .get("last_seen_at")
            .and_then(Value::as_str)
            .map_or("-".to_owned(), local_time);
        let _ = writeln!(
            out,
            "{:<width$}  {:<scope_width$}  {seen}",
            str_of(a, "name"),
            scopes(a)
        );
    }
    Some(out)
}

/// `agent history`：1 件を 1 行で、変えた内容を続く行で出す。まだ確認していないエージェントの変更には
/// `*` を付ける（05 4.1）。
fn changes(value: &Value) -> String {
    let list = value["changes"].as_array().map_or(&[][..], Vec::as_slice);
    if list.is_empty() {
        return "No changes.\n".to_owned();
    }
    let mut out = String::new();
    for c in list {
        let unseen = str_of(c, "actor") == "agent" && c.get("seen_at").is_some_and(Value::is_null);
        let who = c.get("client").and_then(Value::as_str).unwrap_or("you");
        let reverted = c
            .get("reverted_at")
            .and_then(Value::as_str)
            .map_or(String::new(), |_| "  [reverted]".to_owned());
        let of = c
            .get("revert_of")
            .and_then(Value::as_str)
            .map_or(String::new(), |id| format!("  (reverts {id})"));
        let _ = writeln!(
            out,
            "{} {}  {}  {who}  {}  {}{reverted}{of}",
            if unseen { '*' } else { ' ' },
            str_of(c, "id"),
            local_time(str_of(c, "created_at")),
            str_of(c, "operation"),
            target(&c["target"])
        );
        let summary = &c["summary"];
        match str_of(summary, "type") {
            "added" => {
                let _ = writeln!(out, "      added: {}", brief(&summary["text"]));
            }
            "fields" => {
                for f in summary["fields"].as_array().map_or(&[][..], Vec::as_slice) {
                    let _ = writeln!(
                        out,
                        "      {}: {} -> {}",
                        str_of(f, "name"),
                        brief(&f["before"]),
                        brief(&f["after"])
                    );
                }
            }
            "text" => {
                let _ = writeln!(
                    out,
                    "      text: {} -> {} characters",
                    summary["before_chars"].as_u64().unwrap_or(0),
                    summary["after_chars"].as_u64().unwrap_or(0)
                );
            }
            _ => {}
        }
    }
    if let Some(next) = value.get("next_before").and_then(Value::as_str) {
        let _ = writeln!(out, "(older changes: yunagi agent history --before {next})");
    }
    out
}

/// 履歴の対象の表示：項目は `日付 種類 "本文"`、ノート・振り返りは `種類 日付`。
fn target(t: &Value) -> String {
    match str_of(t, "type") {
        "item" => {
            if t.get("text").is_none() {
                return format!("item {}", str_of(t, "id"));
            }
            let deleted = if t["deleted"].as_bool() == Some(true) {
                " (deleted)"
            } else {
                ""
            };
            format!(
                "{} {} {}{deleted}",
                str_of(t, "date"),
                str_of(t, "kind"),
                brief(&t["text"])
            )
        }
        kind => format!("{kind} {}", str_of(t, "date")),
    }
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

/// 操作の名前（MCP のツール名 `<動詞>_<名詞>`）に当たる CLI のコマンド `yunagi <名詞> <動詞>`
/// （05 3章の命名規則。`complete_todo` なら `yunagi todo complete`）。
fn cli_command(operation: &str) -> String {
    match operation.split_once('_') {
        Some((verb, noun)) => format!("yunagi {noun} {verb}"),
        None => operation.to_owned(),
    }
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
    fn export_handoff_prints_the_markdown_as_it_is() {
        let value = json!({"id": "T1", "markdown": "## Task\nx\n\n(Exported from Yunagi on 2026-11-03)\n"});
        assert_eq!(
            render("export_handoff", &value),
            "## Task\nx\n\n(Exported from Yunagi on 2026-11-03)\n"
        );
        // 欄が無い応答は、JSON のまま出す
        assert_eq!(
            render("export_handoff", &json!({"id": "T1"})),
            "{\"id\":\"T1\"}\n"
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
        // `updated` の無い応答は、書き込みが行われたかを偽らず、JSON のまま出す
        assert_eq!(done("append_note", json!({"rev": 3})), "{\"rev\":3}\n");
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
        // 欄が無い応答は、JSON のまま出す
        assert_eq!(
            render("resolve_blocker", &json!({"rev": 3})),
            "{\"rev\":3}\n"
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
    fn agents_are_listed_in_a_table_and_filtered_by_name() {
        let list = json!({"agents": [
            {"name": "cli", "scopes": ["read"], "last_seen_at": null},
            {"name": "claude-code", "scopes": [], "last_seen_at": null},
        ]});
        let all = agents(&list, None).unwrap();
        let rows: Vec<&str> = all.lines().collect();
        assert_eq!(rows[0], "CLIENT       SCOPES  LAST SEEN");
        assert_eq!(rows[1], "cli          read    -");
        assert_eq!(rows[2], "claude-code  (none)  -");
        assert_eq!(agents(&list, Some("cli")).unwrap().lines().count(), 2);
        assert!(agents(&list, Some("nobody")).is_none());
    }

    #[test]
    fn the_history_marks_unseen_and_reverted_changes() {
        let value = json!({"changes": [
            {"id": "C2", "created_at": "bad", "actor": "human", "client": null, "operation": "agent.revert",
             "target": {"type": "item", "id": "T1", "date": "2026-11-03", "kind": "todo", "text": "x", "deleted": false},
             "summary": {"type": "fields", "fields": [{"name": "status", "before": "done", "after": "open"}]},
             "seen_at": null, "reverted_at": null, "revert_of": "C1"},
            {"id": "C1", "created_at": "bad", "actor": "agent", "client": "claude-code", "operation": "todo.complete",
             "target": {"type": "item", "id": "T1", "date": "2026-11-03", "kind": "todo", "text": "x", "deleted": false},
             "summary": {"type": "fields", "fields": [{"name": "status", "before": "open", "after": "done"}]},
             "seen_at": null, "reverted_at": "2026-11-03T01:00:00.000Z", "revert_of": null},
            {"id": "N1", "created_at": "bad", "actor": "agent", "client": "cli", "operation": "note.append",
             "target": {"type": "note", "date": "2026-11-03"},
             "summary": {"type": "text", "before_chars": 0, "after_chars": 9},
             "seen_at": "2026-11-03T01:00:00.000Z", "reverted_at": null, "revert_of": null},
        ], "next_before": "N1"});
        let text = render("list_changes", &value);
        let lines: Vec<&str> = text.lines().collect();
        // 人の取り消しの記録は未確認に数えない（`*` なし）。クライアントが無ければ you
        assert_eq!(
            lines[0],
            "  C2  bad  you  agent.revert  2026-11-03 todo \"x\"  (reverts C1)"
        );
        assert_eq!(lines[1], "      status: \"done\" -> \"open\"");
        assert_eq!(
            lines[2],
            "* C1  bad  claude-code  todo.complete  2026-11-03 todo \"x\"  [reverted]"
        );
        assert_eq!(lines[4], "  N1  bad  cli  note.append  note 2026-11-03");
        assert_eq!(lines[5], "      text: 0 -> 9 characters");
        assert_eq!(
            lines[6],
            "(older changes: yunagi agent history --before N1)"
        );
        assert_eq!(
            render("list_changes", &json!({"changes": []})),
            "No changes.\n"
        );
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
