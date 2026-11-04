//! MCP のツールの定義（`tools/list` の中身。docs/design/05-mcp-cli-spec.md 3章・4.4）。
//!
//! ツール名と引数の名前は、ローカル API の操作（`yunagi_api::ops::Call::parse`）と同じ。引数の読み取りと
//! 判定はそちらとコアが行い、ここは名前・説明・引数の形（JSON スキーマ）を手で書いて持つだけにする（引数の
//! 決まりを 2 か所に書き写さないため、スキーマから引数を作り直したり検査したりはしない）。スキーマの
//! 名前が `Call::parse` と食い違わないことは、このファイルの試験で確かめる。
//!
//! 公開するのは 05 3章の表のうち、A-07（導入）と A-08（権限・履歴）を除いたもの（エージェントが自分の
//! 権限を変えられないようにするため。05 1章）。`export_handoff`（A-06）は、コアの入口を作るとき（YUN-70）に
//! 足す。

use std::sync::OnceLock;

use serde_json::{Map, Value, json};

/// ツール 1 つの定義。
struct Tool {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    /// 引数の名前と形。
    properties: Vec<(&'static str, Value)>,
    required: &'static [&'static str],
    /// 読み取りだけで何も変えない（`annotations.readOnlyHint`）。
    read_only: bool,
}

fn string(description: &str) -> Value {
    json!({"type": "string", "description": description})
}

fn date() -> Value {
    string("Day as YYYY-MM-DD, \"today\" or \"yesterday\". Default: today (the logical date).")
}

fn id(what: &str) -> Value {
    string(&format!(
        "ID of the {what}, from get_board or open_board (or search)."
    ))
}

fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "get_board",
            title: "Get the board of a day",
            description: "Read a day's board: TODOs, blockers, the reflection, and the headings and length of the note. Needs the read scope. Read this first to get item IDs. Items that were carried over have carried_to_id and carried_to_date; send changes to carried_to_id instead. Read the note text with get_note.",
            properties: vec![("date", date())],
            required: &[],
            read_only: true,
        },
        Tool {
            name: "open_board",
            title: "Get the open items of a day",
            description: "Read only the open TODOs and unresolved blockers of a day (no reflection or note). Needs the read scope.",
            properties: vec![("date", date())],
            required: &[],
            read_only: true,
        },
        Tool {
            name: "get_note",
            title: "Get the note of a day",
            description: "Read the note of a day: the whole text with its revision, or only the sections under a time heading. Needs the read scope. Lines are numbered from 1 over the whole note (the same numbers edit_note uses); each section has start_line.",
            properties: vec![
                ("date", date()),
                (
                    "heading",
                    string(
                        "A heading exactly as listed in get_board's note.headings (for example \"10:00 sync\"). Returns every section with that heading. Omit it to get the whole note.",
                    ),
                ),
            ],
            required: &[],
            read_only: true,
        },
        Tool {
            name: "search",
            title: "Search the work notes",
            description: "Search TODOs, blockers, notes and reflections for text (partial match on the whole query), newest day first. Needs the read scope. Note and reflection hits list up to 5 matching lines each; read the rest with get_note.",
            properties: vec![
                ("query", string("Text to search for.")),
                (
                    "from",
                    string("First day to include (same forms as date). Default: no limit."),
                ),
                (
                    "to",
                    string("Last day to include (same forms as date). Default: no limit."),
                ),
                (
                    "kinds",
                    json!({
                        "type": "array",
                        "items": {"type": "string", "enum": ["todo", "blocker", "note", "reflection"]},
                        "description": "Kinds to search. Default: all."
                    }),
                ),
                (
                    "limit",
                    json!({"type": "integer", "minimum": 1, "description": "Maximum number of hits. Default 50, at most 200."}),
                ),
            ],
            required: &["query"],
            read_only: true,
        },
        Tool {
            name: "add_todo",
            title: "Add a TODO",
            description: "Add a TODO to the end of a day. Needs the todo scope. The text is one line. Returns the new item's id.",
            properties: vec![
                ("text", string("The TODO text (one line).")),
                ("date", date()),
            ],
            required: &["text"],
            read_only: false,
        },
        Tool {
            name: "edit_todo",
            title: "Edit the text of a TODO",
            description: "Change the text of a TODO. Needs the todo scope. Sending the same text again changes nothing (updated: false). A blocker's ID is rejected (use edit_blocker). Fails with conflict (user_editing) while the user is editing the item; retry later.",
            properties: vec![
                ("id", id("TODO")),
                ("text", string("The new text (one line).")),
            ],
            required: &["id", "text"],
            read_only: false,
        },
        Tool {
            name: "complete_todo",
            title: "Complete a TODO",
            description: "Mark a TODO done. Needs the todo scope. Safe to send again: an already done TODO is left as it is (updated: false). A blocker's ID is rejected (use resolve_blocker).",
            properties: vec![("id", id("TODO"))],
            required: &["id"],
            read_only: false,
        },
        Tool {
            name: "reopen_todo",
            title: "Reopen a TODO",
            description: "Mark a done TODO open again. Needs the todo scope. Safe to send again (updated: false when it is already open). TODOs the user dropped cannot be reopened.",
            properties: vec![("id", id("TODO"))],
            required: &["id"],
            read_only: false,
        },
        Tool {
            name: "add_blocker",
            title: "Add a blocker",
            description: "Add a blocker to the end of a day. Needs the blocker scope. The text is one line. Returns the new item's id.",
            properties: vec![
                ("text", string("The blocker text (one line).")),
                ("date", date()),
            ],
            required: &["text"],
            read_only: false,
        },
        Tool {
            name: "edit_blocker",
            title: "Edit the text of a blocker",
            description: "Change the text of a blocker. Needs the blocker scope. Sending the same text again changes nothing (updated: false). A TODO's ID is rejected (use edit_todo).",
            properties: vec![
                ("id", id("blocker")),
                ("text", string("The new text (one line).")),
            ],
            required: &["id", "text"],
            read_only: false,
        },
        Tool {
            name: "resolve_blocker",
            title: "Resolve a blocker",
            description: "Mark a blocker resolved, optionally with a resolution note. Needs the blocker scope. Safe to send again. If the blocker already has a different resolution note it fails with conflict (note_exists): send the request without a note, or tell the user.",
            properties: vec![
                ("id", id("blocker")),
                (
                    "note",
                    string("Resolution note (one line). Omit it to resolve without a note."),
                ),
            ],
            required: &["id"],
            read_only: false,
        },
        Tool {
            name: "append_reflection",
            title: "Append to the reflection",
            description: "Append text to the end of a day's reflection. Needs the append scope. Existing reflection text cannot be changed. Do not send it again unless you checked it was not applied: appending twice duplicates the text.",
            properties: vec![
                ("text", string("Text to append (may have several lines).")),
                ("date", date()),
            ],
            required: &["text"],
            read_only: false,
        },
        Tool {
            name: "append_note",
            title: "Append to the note",
            description: "Append text to the end of a day's note. Needs the append scope. Do not send it again unless you checked it was not applied: appending twice duplicates the text.",
            properties: vec![
                ("text", string("Text to append (may have several lines).")),
                ("date", date()),
                (
                    "heading",
                    string(
                        "If given, first insert a heading \"## HH:MM <heading>\" with the current time (\"\" gives just the time). Omit it for no heading.",
                    ),
                ),
            ],
            required: &["text"],
            read_only: false,
        },
        Tool {
            name: "edit_note",
            title: "Change a line of the note",
            description: "Replace, delete or move one line of a day's note. Needs the note_edit scope. Give exactly one of replace, delete (true) and move_to. expected_text must be the line's current text (read it with get_note); if it differs the call fails with conflict (changed) and you must read the note again. Fails with conflict (user_editing) for a line the user is editing.",
            properties: vec![
                ("date", date()),
                (
                    "line",
                    json!({"type": "integer", "minimum": 1, "description": "Line number in the whole note, from 1."}),
                ),
                (
                    "expected_text",
                    string("The current text of that line, exactly as get_note returns it."),
                ),
                ("replace", string("New text for the line.")),
                (
                    "delete",
                    json!({"type": "boolean", "description": "Send true to delete the line."}),
                ),
                (
                    "move_to",
                    json!({"type": "integer", "minimum": 1, "description": "Line number to move the line to, from 1."}),
                ),
            ],
            required: &["line", "expected_text"],
            read_only: false,
        },
    ]
}

/// `tools/list` の `tools`。最初に 1 回だけ組み立てる。
pub fn list() -> &'static [Value] {
    static LIST: OnceLock<Vec<Value>> = OnceLock::new();
    LIST.get_or_init(build)
}

fn build() -> Vec<Value> {
    tools()
        .into_iter()
        .map(|t| {
            let properties: Map<String, Value> = t
                .properties
                .into_iter()
                .map(|(name, schema)| (name.to_owned(), schema))
                .collect();
            let mut schema = json!({
                "type": "object",
                "properties": properties,
                "additionalProperties": false,
            });
            if !t.required.is_empty() {
                schema["required"] = json!(t.required);
            }
            json!({
                "name": t.name,
                "title": t.title,
                "description": t.description,
                "inputSchema": schema,
                "annotations": {"readOnlyHint": t.read_only},
            })
        })
        .collect()
}

/// 公開しているツールの名前か。後から `ops` に操作が増えても、ここに足すまでは MCP に出さない。
pub fn exists(name: &str) -> bool {
    list().iter().any(|t| t["name"] == name)
}

#[cfg(test)]
mod tests {
    use yunagi_api::ops::{Call, Failure};

    use super::*;

    #[test]
    fn the_tools_are_the_expected_fourteen() {
        let names: Vec<String> = list()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            names,
            [
                "get_board",
                "open_board",
                "get_note",
                "search",
                "add_todo",
                "edit_todo",
                "complete_todo",
                "reopen_todo",
                "add_blocker",
                "edit_blocker",
                "resolve_blocker",
                "append_reflection",
                "append_note",
                "edit_note",
            ]
        );
        // 利用者の管理操作は出さない（05 1章）
        for hidden in [
            "agent_setup",
            "agent_history",
            "agent_revert",
            "export_handoff",
        ] {
            assert!(!exists(hidden), "{hidden}");
        }
    }

    /// スキーマに書いた引数の名前は、`Call::parse` が「知らない名前」と断らない。スキーマに無い名前は断る。
    #[test]
    fn schema_names_match_what_the_operations_accept() {
        for tool in list() {
            let name = tool["name"].as_str().unwrap();
            let properties = tool["inputSchema"]["properties"].as_object().unwrap();
            for key in properties.keys() {
                let mut params = Map::new();
                // 型が合わない値でよい（名前だけを確かめる）
                params.insert(key.clone(), json!(0));
                if let Err(Failure::Params(m)) = Call::parse(name, &params) {
                    assert!(!m.starts_with("unknown parameter"), "{name}.{key}: {m}");
                }
            }
            let mut params = Map::new();
            params.insert("no_such_parameter".into(), json!(0));
            match Call::parse(name, &params) {
                Err(Failure::Params(m)) => assert!(m.starts_with("unknown parameter"), "{name}"),
                other => panic!("{name}: {other:?}"),
            }
            // 必須と書いた引数はスキーマにあり、引数が何も無ければ `Call::parse` が断る
            let required = tool["inputSchema"]["required"].as_array();
            for key in required.into_iter().flatten() {
                let key = key.as_str().unwrap();
                assert!(properties.contains_key(key), "{name}.{key}");
            }
            if required.is_some() {
                assert!(Call::parse(name, &Map::new()).is_err(), "{name}");
            }
        }
    }

    #[test]
    fn read_tools_are_marked_read_only() {
        for tool in list() {
            let name = tool["name"].as_str().unwrap();
            let read = ["get_board", "open_board", "get_note", "search"].contains(&name);
            assert_eq!(tool["annotations"]["readOnlyHint"], read, "{name}");
        }
    }
}
