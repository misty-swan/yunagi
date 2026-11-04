//! `yunagi agent permissions`・`yunagi agent revert`（A-08。docs/design/05-mcp-cli-spec.md 2章・7章）。
//!
//! - `agent permissions [CLIENT]`：エージェントごとの権限を見る（確認は要らない）。`--allow`・`--deny` が
//!   あれば変える。
//! - `agent revert CHANGE_ID [--force]`：エージェントの変更を 1 件取り消す。
//!
//! 変える・取り消すは、**対話端末（TTY）での確認**でしか確定できない（標準入力と標準出力の両方が端末の
//! とき）。端末でない実行（エージェントのシェル、git フックなど）では、何も書かずに `permission_denied`
//! （`required_scope` なし。終了コード 3）を返す。確認は誤って確定しないための確認で、防御の境界ではない
//! （標準入力を擬似端末につなぐなどすれば、エージェントのシェルからも通せる。05 2章。`agent setup` と同じ）。
//! 確認を得た後の処理は、ローカル API の管理の操作（`set_agent_scopes`・`preview_revert`・`revert_change`。
//! 05 4.4）に頼む。
//!
//! `agent history` は読み取りなので、ここには無い（ふつうの操作として `output` が整える）。

use serde_json::{Map, Value, json};
use yunagi_core::domain::agent_admin;
use yunagi_core::storage::Ulid;
use yunagi_core::storage::table::agent_client::Scope;

use crate::error::{self, ErrorObject};
use crate::output;
use crate::{Env, Io};

/// 確認の質問の答え。
enum Answer {
    Yes,
    No,
    /// 答えを得られなかった（入力が閉じている・読めない）。何も書かずに失敗にする。
    Unavailable,
}

/// `[y/N]` の質問をして、答えを読む。`y`・`yes`（大文字小文字は問わない）だけを承諾とする。
fn confirm(io: &mut Io<'_>, question: &str) -> Answer {
    let _ = write!(io.stdout, "\n{question} [y/N] ");
    let _ = io.stdout.flush();
    let mut answer = String::new();
    match io.stdin.read_line(&mut answer) {
        Ok(0) => {
            let _ = writeln!(io.stdout);
            let _ = writeln!(
                io.stderr,
                "No answer was given (the input is closed). Nothing was changed."
            );
            Answer::Unavailable
        }
        Err(e) => {
            let _ = writeln!(io.stdout);
            let _ = writeln!(
                io.stderr,
                "Could not read the answer ({e}). Nothing was changed."
            );
            Answer::Unavailable
        }
        Ok(_) => {
            if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
                Answer::Yes
            } else {
                Answer::No
            }
        }
    }
}

fn out(io: &mut Io<'_>, line: &str) {
    let _ = writeln!(io.stdout, "{line}");
}

/// 端末でないので確定できない（`required_scope` なし。終了コード 3）。
fn needs_terminal(what: &str, io: &mut Io<'_>) -> i32 {
    crate::fail(
        &error::permission_denied(format!(
            "Confirmation needs a terminal. Nothing was changed. Run `{what}` yourself in a \
             terminal, or use the Yunagi app (Settings > Agents)."
        )),
        false,
        io,
    )
}

fn scope_list(scopes: &[Scope]) -> String {
    if scopes.is_empty() {
        "(none)".to_owned()
    } else {
        scopes
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn names(value: &Value) -> Vec<Scope> {
    value
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .filter_map(|s| s.as_str()?.parse().ok())
        .collect()
}

/// `agent permissions [CLIENT] [--allow S]... [--deny S]...`。
pub fn permissions(
    client: Option<&str>,
    allow: &[Scope],
    deny: &[Scope],
    json: bool,
    env: &Env,
    io: &mut Io<'_>,
) -> i32 {
    if allow.is_empty() && deny.is_empty() {
        return list_permissions(client, json, env, io);
    }
    let Some(client) = client else {
        // 引数の読み取りが先に断る。ここには来ない
        return crate::fail(&error::internal("no client given"), false, io);
    };
    // 識別子のない接続（`unknown`）に足せるのは `read` だけ。確認の後で断るより先に断る
    if let Some(scope) = agent_admin::forbidden_scope(client, allow) {
        return crate::fail(
            &error::invalid_argument(format!(
                "The unknown client (connections without an identifier) can only have the read \
                 scope, not {scope}. Register the agent with `yunagi agent setup` so that it has \
                 its own name."
            )),
            false,
            io,
        );
    }
    if !env.setup.interactive {
        return needs_terminal("yunagi agent permissions", io);
    }
    // 一覧と変更で同じ道具を使う（環境を分けている形の警告を、1 回だけ出すため）
    let api = crate::ApiClient::new(env);
    let listed = match api.call(io, "list_agents", Map::new()) {
        Ok(v) => v,
        Err(e) => return crate::fail(&e, false, io),
    };
    let Some(current) = listed["agents"]
        .as_array()
        .and_then(|a| a.iter().find(|a| a["name"].as_str() == Some(client)))
    else {
        return crate::fail(
            &error::not_found(format!(
                "No client named \"{client}\" is registered. A client is registered when it first \
                 connects. Run `yunagi agent permissions` to see the names."
            )),
            false,
            io,
        );
    };
    let before = names(&current["scopes"]);
    let after = agent_admin::apply_scopes(&before, allow, deny);
    if after == before {
        out(
            io,
            &format!(
                "{client} already has these scopes: {}. Nothing to change.",
                scope_list(&before)
            ),
        );
        return 0;
    }
    out(io, &format!("Client: {client}"));
    out(io, &format!("  now:   {}", scope_list(&before)));
    out(io, &format!("  after: {}", scope_list(&after)));
    let gained: Vec<&str> = after
        .iter()
        .filter(|s| !before.contains(s))
        .map(|s| meaning(*s))
        .collect();
    if !gained.is_empty() {
        out(
            io,
            &format!("  This lets the agent: {}.", gained.join("; ")),
        );
    }
    match confirm(io, "Apply this change?") {
        Answer::Yes => {}
        Answer::No => {
            out(io, "Cancelled. Nothing was changed.");
            return 0;
        }
        Answer::Unavailable => return 1,
    }
    let params = Map::from_iter([
        ("client".to_owned(), json!(client)),
        (
            "allow".to_owned(),
            json!(allow.iter().map(|s| s.as_str()).collect::<Vec<_>>()),
        ),
        (
            "deny".to_owned(),
            json!(deny.iter().map(|s| s.as_str()).collect::<Vec<_>>()),
        ),
        // 見せた「いま」から変わっていたら、書かずに止める
        (
            "expect".to_owned(),
            json!(before.iter().map(|s| s.as_str()).collect::<Vec<_>>()),
        ),
    ]);
    match api.call(io, "set_agent_scopes", params) {
        Ok(v) => {
            let now = names(&v["scopes"]);
            out(
                io,
                &format!("Updated. {client} now has: {}", scope_list(&now)),
            );
            0
        }
        Err(e) => crate::fail(&e, false, io),
    }
}

/// スコープで何ができるか（確認の表示。05 2章の表）。
fn meaning(scope: Scope) -> &'static str {
    match scope {
        Scope::Read => "read boards, items, notes and reflections, and search",
        Scope::Todo => "add, edit, complete and reopen TODOs",
        Scope::Blocker => "add, edit and resolve blockers",
        Scope::Append => "append to the note and the reflection",
        Scope::NoteEdit => "rewrite, delete and move existing note lines",
    }
}

fn list_permissions(client: Option<&str>, json: bool, env: &Env, io: &mut Io<'_>) -> i32 {
    let listed = match crate::call_api(env, io, "list_agents", Map::new()) {
        Ok(v) => v,
        Err(e) => return crate::fail(&e, json, io),
    };
    if json {
        let agents: Vec<&Value> = listed["agents"]
            .as_array()
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .filter(|a| client.is_none_or(|name| a["name"].as_str() == Some(name)))
            .collect();
        if agents.is_empty() {
            return crate::fail(&not_registered(client), true, io);
        }
        out(io, &json!({"agents": agents}).to_string());
        return 0;
    }
    match output::agents(&listed, client) {
        Some(text) => {
            let _ = io.stdout.write_all(text.as_bytes());
            0
        }
        None => crate::fail(&not_registered(client), false, io),
    }
}

fn not_registered(client: Option<&str>) -> ErrorObject {
    error::not_found(format!(
        "No client named \"{}\" is registered. Run `yunagi agent permissions` to see the names.",
        client.unwrap_or_default()
    ))
}

/// `agent revert CHANGE_ID [--force]`。
pub fn revert(change_id: &str, force: bool, env: &Env, io: &mut Io<'_>) -> i32 {
    // ID の形の誤りは、端末の確認や接続の前に断る（05 3.2 の例外と同じ考え方）
    if change_id.parse::<Ulid>().is_err() {
        return crate::fail(
            &error::invalid_argument(
                "CHANGE_ID is not a valid change ID. Use an ID from `yunagi agent history`.",
            ),
            false,
            io,
        );
    }
    if !env.setup.interactive {
        return needs_terminal("yunagi agent revert", io);
    }
    let params = Map::from_iter([("change_id".to_owned(), json!(change_id))]);
    let api = crate::ApiClient::new(env);
    let plan = match api.call(io, "preview_revert", params) {
        Ok(v) => v,
        Err(e) => return crate::fail(&e, false, io),
    };
    let state = plan["state"].as_str().unwrap_or("");
    let drift: Vec<&str> = plan["changed_since"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .filter_map(Value::as_str)
        .collect();
    out(
        io,
        &format!(
            "Change {change_id}: {} by {} at {}",
            plan["operation"].as_str().unwrap_or(""),
            plan["client"].as_str().unwrap_or("?"),
            output::local_time(plan["created_at"].as_str().unwrap_or(""))
        ),
    );
    match state {
        "already_reverted" => {
            out(io, "Already reverted. Nothing changed.");
            return 0;
        }
        "already_restored" => {
            out(
                io,
                "The target already has the earlier values. Nothing needs to be written.",
            );
        }
        // --force は、失う内容が全部見えるように、差分を切り詰めずに出す
        _ => describe_effect(&plan["effect"], force, io),
    }
    if state == "apply" && !drift.is_empty() {
        if !force {
            return crate::fail(
                &json!({
                    "code": "conflict",
                    "reason": "changed",
                    "changed_fields": drift,
                    "message": format!(
                        "The target was changed after this record (changed: {}), so it was not \
                         reverted. Nothing was changed. Check the difference above, then run again \
                         with --force to overwrite it.",
                        drift.join(", ")
                    ),
                }),
                false,
                io,
            );
        }
        out(
            io,
            &format!(
                "--force: this overwrites changes made after the record ({}).",
                drift.join(", ")
            ),
        );
    }
    // 値を書かない（すでに戻っている）ときは、記録を取り消し済みにするだけなので、聞かない
    if state == "apply" {
        match confirm(io, "Revert this change?") {
            Answer::Yes => {}
            Answer::No => {
                out(io, "Cancelled. Nothing was changed.");
                return 0;
            }
            Answer::Unavailable => return 1,
        }
    }
    let mut params = Map::from_iter([
        ("change_id".to_owned(), json!(change_id)),
        ("force".to_owned(), json!(force)),
    ]);
    // 見せた状態から変わっていたら、force でも書かない
    if let Some(token) = plan["token"].as_str() {
        params.insert("expect".into(), json!(token));
    }
    match api.call(io, "revert_change", params) {
        Ok(v) => {
            report(&v, io);
            0
        }
        Err(e) => crate::fail(&e, false, io),
    }
}

/// 取り消しで書き換わる内容の表示（確認の前）。
fn describe_effect(effect: &Value, show_all: bool, io: &mut Io<'_>) {
    let shown = if show_all { usize::MAX } else { 20 };
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    match effect["type"].as_str().unwrap_or("") {
        "delete_item" => {
            out(
                io,
                &format!(
                    "This deletes the {} added on {}: {:?}",
                    s(effect, "kind"),
                    s(effect, "date"),
                    s(effect, "text")
                ),
            );
            if let (Some(id), Some(date)) = (
                effect["carried_to_id"].as_str(),
                effect["carried_to_date"].as_str(),
            ) {
                out(
                    io,
                    &format!("The copy carried over to {date} ({id}) is kept as it is."),
                );
            }
        }
        "fields" => {
            out(
                io,
                &format!(
                    "This restores the {} on {}: {:?}",
                    s(effect, "kind"),
                    s(effect, "date"),
                    s(effect, "text")
                ),
            );
            for f in effect["fields"].as_array().map_or(&[][..], Vec::as_slice) {
                let mark = if f["changed_since"].as_bool() == Some(true) {
                    "   (changed after the record)"
                } else {
                    ""
                };
                out(
                    io,
                    &format!(
                        "  {}: {} -> {}{mark}",
                        s(f, "name"),
                        show(&f["current"]),
                        show(&f["restore"])
                    ),
                );
            }
        }
        "text" => {
            out(
                io,
                &format!(
                    "This restores the {} of {}:",
                    s(effect, "doc"),
                    s(effect, "date")
                ),
            );
            lines(io, '-', &effect["removed"], shown);
            lines(io, '+', &effect["added"], shown);
            if effect["truncated"].as_bool() == Some(true) {
                out(io, "  ... (the difference is longer than shown)");
            }
        }
        _ => {}
    }
}

/// 差分の行を最大 `shown` 行まで出す。
fn lines(io: &mut Io<'_>, mark: char, list: &Value, shown: usize) {
    let list = list.as_array().map_or(&[][..], Vec::as_slice);
    for line in list.iter().take(shown) {
        out(io, &format!("  {mark} {}", line.as_str().unwrap_or("")));
    }
    if list.len() > shown {
        out(
            io,
            &format!("  {mark} ... ({} more lines)", list.len() - shown),
        );
    }
}

fn show(v: &Value) -> String {
    match v {
        Value::String(s) => format!("{s:?}"),
        other => other.to_string(),
    }
}

/// 取り消しの結果の表示。
fn report(v: &Value, io: &mut Io<'_>) {
    match v["state"].as_str().unwrap_or("") {
        "reverted" => out(io, "Reverted."),
        "already_reverted" => out(io, "Already reverted. Nothing changed."),
        _ => out(
            io,
            "Already back to the earlier values. The change is marked as reverted.",
        ),
    }
}
