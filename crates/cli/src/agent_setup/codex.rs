//! Codex への登録（docs/design/05-mcp-cli-spec.md 6章）。
//!
//! `~/.codex/config.toml`（`CODEX_HOME` があれば `$CODEX_HOME/config.toml`）の `[mcp_servers.yunagi]` に、
//! `command`・`args`・`env_vars`（環境変数の**名前**だけ。値は書かない）を書く。`codex mcp add` は使わない：
//! 名前の転送（`env_vars`）を指定する引数が無く（`--env KEY=VALUE` のみ）、同じ名前があると確認なしに
//! 置き換えて `env_vars` を消すため（2026-11-04、codex-cli 0.166.0 で確かめた）。
//!
//! 書き換えは `toml_edit` で、ほかの表・コメント・並びを変えずに、`yunagi` の表の 3 つの欄だけを足す・直す
//! （その表のほかの欄、たとえば `enabled`・`startup_timeout_sec` は残す）。TOML として読めないファイルは、
//! 書かずに止める。

use std::fs;
use std::path::PathBuf;

use toml_edit::{Array, DocumentMut, Item, Table, TableLike, Value};

use super::{Change, Prepared, SERVER_NAME, SetupEnv, files};

const SERVER_COMMAND: &str = "yunagi";
const SERVER_ARGS: [&str; 3] = ["mcp", "--client", "codex"];

/// Codex を使っていると見てよいか：`codex` コマンドがあるか、設定のフォルダがある。
pub fn detected(setup: &SetupEnv) -> bool {
    setup.find("codex").is_some() || codex_dir(setup).is_some_and(|d| d.is_dir())
}

/// 設定のフォルダ。`CODEX_HOME` を先に見る（2026-11-04 に確かめた）。
fn codex_dir(setup: &SetupEnv) -> Option<PathBuf> {
    setup
        .codex_home
        .clone()
        .or_else(|| setup.home.as_ref().map(|h| h.join(".codex")))
}

/// 登録の中身（変更内容の表示と同じ形）。
fn block(names: &[&str]) -> Vec<String> {
    let quote = |items: &[&str]| {
        items
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(", ")
    };
    vec![
        format!("[mcp_servers.{SERVER_NAME}]"),
        format!("command = \"{SERVER_COMMAND}\""),
        format!("args = [{}]", quote(&SERVER_ARGS)),
        format!("env_vars = [{}]", quote(names)),
    ]
}

/// `mcp_servers.yunagi` の今の状態。
enum Current {
    /// 無い。
    Missing,
    /// 同じ内容。
    Same,
    /// 違う内容（`command`・`args`・`env_vars` の今の値を 1 行ずつにしたもの）。
    Different(Vec<String>),
}

fn current(doc: &DocumentMut, names: &[&str]) -> Result<Current, String> {
    let Some(servers) = doc.get("mcp_servers") else {
        return Ok(Current::Missing);
    };
    let servers = servers
        .as_table_like()
        .ok_or("`mcp_servers` in the file is not a table.")?;
    let Some(entry) = servers.get(SERVER_NAME) else {
        return Ok(Current::Missing);
    };
    let entry = entry
        .as_table_like()
        .ok_or("`mcp_servers.yunagi` in the file is not a table.")?;
    let strings = |key: &str| -> Option<Vec<String>> {
        entry
            .get(key)?
            .as_array()?
            .iter()
            .map(|v| v.as_str().map(str::to_owned))
            .collect()
    };
    let same = entry.get("command").and_then(Item::as_str) == Some(SERVER_COMMAND)
        && strings("args").is_some_and(|a| a == SERVER_ARGS)
        && strings("env_vars").is_some_and(|a| a == names);
    if same {
        return Ok(Current::Same);
    }
    // 今の値を表示する（`env` は値を含みうるので出さない）
    let shown = ["command", "args", "env_vars"]
        .iter()
        .filter_map(|key| {
            entry
                .get(key)
                .map(|item| format!("{key} = {}", item.to_string().trim()))
        })
        .collect();
    Ok(Current::Different(shown))
}

/// `mcp_servers.yunagi` が `enabled = false` か。
fn is_disabled(doc: &DocumentMut) -> bool {
    doc.get("mcp_servers")
        .and_then(Item::as_table_like)
        .and_then(|servers| servers.get(SERVER_NAME))
        .and_then(Item::as_table_like)
        .and_then(|entry| entry.get("enabled"))
        .and_then(Item::as_bool)
        == Some(false)
}

/// `mcp_servers.yunagi` の `command`・`args`・`env_vars` を書く。ほかの表・コメント・欄は変えない。
fn edit(doc: &mut DocumentMut, names: &[&str], blank_line_before: bool) -> Result<(), String> {
    let root = doc.as_table_mut();
    if !root.contains_key("mcp_servers") {
        let mut table = Table::new();
        // `[mcp_servers]` の見出しは出さず、`[mcp_servers.yunagi]` だけを出す
        table.set_implicit(true);
        root.insert("mcp_servers", Item::Table(table));
    }
    let servers = root
        .get_mut("mcp_servers")
        .and_then(Item::as_table_like_mut)
        .ok_or("`mcp_servers` in the file is not a table.")?;
    if !servers.contains_key(SERVER_NAME) {
        let mut table = Table::new();
        if blank_line_before {
            table.decor_mut().set_prefix("\n");
        }
        servers.insert(SERVER_NAME, Item::Table(table));
    }
    let entry = servers
        .get_mut(SERVER_NAME)
        .and_then(Item::as_table_like_mut)
        .ok_or("`mcp_servers.yunagi` in the file is not a table.")?;
    set(entry, "command", Value::from(SERVER_COMMAND));
    set(entry, "args", strings(&SERVER_ARGS));
    set(entry, "env_vars", strings(names));
    Ok(())
}

fn strings(items: &[&str]) -> Value {
    Value::Array(items.iter().copied().collect::<Array>())
}

/// 欄 `key` に値を書く。すでにある欄は、行頭の字下げや行末のコメントを残して値だけ置き換える。
fn set(entry: &mut dyn TableLike, key: &str, mut value: Value) {
    match entry.get_mut(key) {
        Some(item) => {
            if let Some(old) = item.as_value() {
                *value.decor_mut() = old.decor().clone();
            }
            *item = Item::Value(value);
        }
        None => {
            entry.insert(key, Item::Value(value));
        }
    }
}

/// 編集の元にする文字列。最後が改行で終わっていないファイルには改行を足す（足さないと、足した表の見出しが
/// 前の行に続いてしまう）。
fn working_text(text: &str) -> String {
    if !text.is_empty() && !text.ends_with('\n') {
        format!("{text}\n")
    } else {
        text.to_owned()
    }
}

/// 書き出す文字列。元のファイルが CRLF なら、足した行も CRLF にそろえる。
fn render(doc: &DocumentMut, original: Option<&str>) -> String {
    let text = doc.to_string();
    if original.is_some_and(|o| o.contains("\r\n")) {
        text.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        text
    }
}

/// TOML の誤りを、ファイルの内容（トークンなどを含みうる）を出さずに、行番号と理由だけで伝える。
fn parse_error(path: &std::path::Path, text: &str, e: &toml_edit::TomlError) -> String {
    let line = e
        .span()
        .map(|span| text[..span.start.min(text.len())].matches('\n').count() + 1);
    let at = line.map(|l| format!(" at line {l}")).unwrap_or_default();
    format!(
        "{} is not valid TOML{at} ({}). Fix it first; the file was not touched.",
        path.display(),
        e.message()
    )
}

/// 変更内容を作る（何も書かない）。
pub fn prepare(setup: &SetupEnv) -> Prepared<Plan> {
    let Some(path) = codex_dir(setup).map(|d| d.join("config.toml")) else {
        return Prepared::Failed(
            "Your home folder could not be found. Set CODEX_HOME to the folder with config.toml."
                .to_owned(),
        );
    };
    let original = match fs::read(&path) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => Some(text),
            Err(_) => {
                return Prepared::Failed(format!("{} is not valid UTF-8.", path.display()));
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            return Prepared::Failed(format!("Could not read {}: {e}.", path.display()));
        }
    };
    let text = working_text(original.as_deref().unwrap_or(""));
    let text = text.as_str();
    let mut doc: DocumentMut = match text.parse() {
        Ok(doc) => doc,
        Err(e) => return Prepared::Failed(parse_error(&path, text, &e)),
    };
    let names = setup.dir_vars();
    let old = match current(&doc, names) {
        Ok(Current::Same) => {
            // 利用者が無効にしている（`enabled = false`）登録は変えない。ただし、使えないことは伝える
            let disabled = if is_disabled(&doc) {
                " It is turned off (enabled = false), so Codex does not start it; remove that line to use it."
            } else {
                ""
            };
            return Prepared::Unchanged(format!(
                "already registered in {} (same command and env_vars). Nothing to change.{disabled}",
                path.display()
            ));
        }
        Ok(Current::Missing) => None,
        Ok(Current::Different(shown)) => Some(shown),
        Err(msg) => return Prepared::Failed(format!("{} — {msg}", path.display())),
    };
    let blank_line_before = old.is_none() && !text.trim().is_empty();
    if let Err(msg) = edit(&mut doc, names, blank_line_before) {
        return Prepared::Failed(format!("{} — {msg}", path.display()));
    }
    let new_text = render(&doc, original.as_deref());
    Prepared::Change(Plan {
        path,
        names: names.iter().map(|s| (*s).to_owned()).collect(),
        old,
        original,
        new_text,
        stamp: setup.stamp(),
        yunagi_on_path: setup.find(SERVER_COMMAND).is_some(),
    })
}

#[derive(Debug)]
pub struct Plan {
    path: PathBuf,
    names: Vec<String>,
    /// 今の登録の `command`・`args`・`env_vars`（無ければ `None`）。
    old: Option<Vec<String>>,
    /// 読んだときのファイルの中身（ファイルが無ければ `None`）。書く前に、変わっていないか確かめる。
    original: Option<String>,
    new_text: String,
    stamp: String,
    yunagi_on_path: bool,
}

impl Change for Plan {
    fn describe(&self) -> Vec<String> {
        let names: Vec<&str> = self.names.iter().map(String::as_str).collect();
        let mut lines = Vec::new();
        let file = if self.original.is_some() {
            "the existing file"
        } else {
            "a new file"
        };
        match &self.old {
            None => lines.push(format!(
                "add this table to {} ({file}; other tables and comments are not changed):",
                self.path.display()
            )),
            Some(old) => {
                lines.push(format!("registered now in {}:", self.path.display()));
                lines.extend(old.iter().map(|l| format!("  {l}")));
                lines.push(
                    "replace these three fields with the following (other fields, tables and \
                     comments are not changed):"
                        .to_owned(),
                );
            }
        }
        lines.extend(block(&names).into_iter().map(|l| format!("  {l}")));
        lines.push(
            "env_vars holds variable names only; Codex passes their values from its own \
             environment when it starts the server, so no value is stored."
                .to_owned(),
        );
        if self.original.is_some() {
            lines.push(format!(
                "backup before the change: {}.yunagi-backup-{} (next to the file; to undo, copy it back)",
                self.path.display(),
                self.stamp
            ));
        }
        if !self.yunagi_on_path {
            lines.push(
                "note: `yunagi` is not on PATH in this terminal; the agent needs it on its PATH to start the server."
                    .to_owned(),
            );
        }
        lines
    }

    fn apply(&self) -> Result<Vec<String>, String> {
        // 確認を待つ間に、ほかが書き換えていないか
        let now = match fs::read(&self.path) {
            Ok(bytes) => String::from_utf8(bytes).ok(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("Could not read {}: {e}.", self.path.display())),
        };
        if now != self.original {
            return Err(format!(
                "{} changed while waiting for the confirmation, so nothing was written. Run \
                 `yunagi agent setup` again.",
                self.path.display()
            ));
        }
        let mut done = Vec::new();
        if let Some(original) = &self.original {
            let backup = files::create_backup(&self.path, original.as_bytes(), &self.stamp)
                .map_err(|e| format!("Could not make the backup, so nothing was changed: {e}"))?;
            done.push(format!("backup: {}", backup.display()));
        }
        files::write_atomic(&self.path, self.new_text.as_bytes())
            .map_err(|e| format!("Could not write {}: {e}.", self.path.display()))?;
        done.push(format!("wrote {}", self.path.display()));
        done.push(
            "undo: copy the backup back, or delete the [mcp_servers.yunagi] table".to_owned(),
        );
        Ok(done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: [&str; 2] = ["XDG_DATA_HOME", "HOME"];

    /// `text` に登録を足した結果。
    fn added(text: &str) -> String {
        let mut doc: DocumentMut = working_text(text).parse().unwrap();
        let blank = !text.trim().is_empty();
        edit(&mut doc, &NAMES, blank).unwrap();
        render(&doc, Some(text))
    }

    const BLOCK: &str = "[mcp_servers.yunagi]\ncommand = \"yunagi\"\nargs = [\"mcp\", \"--client\", \"codex\"]\nenv_vars = [\"XDG_DATA_HOME\", \"HOME\"]\n";

    #[test]
    fn an_empty_file_gets_only_the_table() {
        assert_eq!(added(""), BLOCK);
    }

    #[test]
    fn other_tables_and_comments_are_kept_byte_for_byte() {
        let text = "# my codex settings\nmodel = \"x\"   # keep me\n\n[profiles.fast]\nmodel = \"y\"\n\n# other server\n[mcp_servers.other]\ncommand = \"o\"   # why\nargs = [ \"a\",\"b\" ]\n";
        let result = added(text);
        // 元の文字列が、そのまま先頭に残る（足したのは末尾だけ）
        assert!(result.starts_with(text), "{result}");
        assert_eq!(result[text.len()..], format!("\n{BLOCK}"));
    }

    #[test]
    fn a_file_without_a_final_newline_is_handled() {
        let result = added("model = \"x\"");
        assert!(result.contains("model = \"x\"\n"), "{result:?}");
        let doc: DocumentMut = result.parse().unwrap();
        assert_eq!(
            doc["mcp_servers"]["yunagi"]["command"].as_str(),
            Some("yunagi")
        );
    }

    #[test]
    fn an_existing_table_keeps_its_other_fields_and_comments() {
        let text = "[mcp_servers.yunagi]\n# the old registration\ncommand = \"/old/yunagi\"  # path\n  args = [\"mcp\"]\nenabled = false\nstartup_timeout_sec = 30\nenv = { A = \"1\" }\n\n[mcp_servers.other]\ncommand = \"o\"\n";
        let result = added(text);
        assert!(result.contains("# the old registration\n"), "{result}");
        assert!(
            result.contains("command = \"yunagi\"  # path\n"),
            "{result}"
        );
        assert!(
            result.contains("  args = [\"mcp\", \"--client\", \"codex\"]\n"),
            "{result}"
        );
        assert!(
            result.contains("enabled = false\nstartup_timeout_sec = 30\nenv = { A = \"1\" }\n")
        );
        assert!(result.contains("env_vars = [\"XDG_DATA_HOME\", \"HOME\"]\n"));
        assert!(result.contains("\n[mcp_servers.other]\ncommand = \"o\"\n"));
        // 足したのは env_vars の 1 行だけ
        assert_eq!(result.lines().count(), text.lines().count() + 1);
    }

    #[test]
    fn inline_and_dotted_forms_are_not_broken() {
        for text in [
            "mcp_servers = { other = { command = \"o\" } }\n",
            "[mcp_servers]\nother = { command = \"o\" }\n",
            "[mcp_servers]\nyunagi.command = \"old\"\n",
            "[mcp_servers]\nyunagi = { command = \"old\", args = [] }\n",
        ] {
            let result = added(text);
            let doc: DocumentMut = result.parse().unwrap_or_else(|e| panic!("{e}\n{result}"));
            let entry = &doc["mcp_servers"]["yunagi"];
            assert_eq!(entry["command"].as_str(), Some("yunagi"), "{result}");
            assert_eq!(entry["args"].as_array().unwrap().len(), 3, "{result}");
            assert_eq!(entry["env_vars"].as_array().unwrap().len(), 2, "{result}");
            if text.contains("other") {
                assert_eq!(doc["mcp_servers"]["other"]["command"].as_str(), Some("o"));
            }
        }
    }

    #[test]
    fn a_crlf_file_stays_crlf() {
        let text = "model = \"x\"\r\n\r\n[a]\r\nb = 1\r\n";
        let result = added(text);
        assert!(result.starts_with(text));
        assert!(!result.replace("\r\n", "").contains('\n'), "{result:?}");
        assert!(result.contains("[mcp_servers.yunagi]\r\n"));
    }

    #[test]
    fn current_state_is_told_apart() {
        let parse = |t: &str| t.parse::<DocumentMut>().unwrap();
        assert!(matches!(current(&parse(""), &NAMES), Ok(Current::Missing)));
        assert!(matches!(
            current(&parse("[mcp_servers.other]\ncommand=\"o\"\n"), &NAMES),
            Ok(Current::Missing)
        ));
        assert!(matches!(current(&parse(BLOCK), &NAMES), Ok(Current::Same)));
        // 名前の並びや中身が違えば違う登録
        let other_order = BLOCK.replace("\"XDG_DATA_HOME\", \"HOME\"", "\"HOME\"");
        assert!(matches!(
            current(&parse(&other_order), &NAMES),
            Ok(Current::Different(_))
        ));
        // 表でないものは、書かずに止める
        assert!(current(&parse("mcp_servers = 1\n"), &NAMES).is_err());
        assert!(current(&parse("[mcp_servers]\nyunagi = \"x\"\n"), &NAMES).is_err());
    }

    #[test]
    fn showing_the_old_registration_hides_env_values() {
        let doc: DocumentMut =
            "[mcp_servers.yunagi]\ncommand = \"x\"\nenv = { TOKEN = \"secret\" }\n"
                .parse()
                .unwrap();
        let Ok(Current::Different(shown)) = current(&doc, &NAMES) else {
            panic!("expected a different registration");
        };
        assert_eq!(shown, ["command = \"x\""]);
    }

    mod flow {
        use super::super::*;
        use super::BLOCK;
        use crate::agent_setup::test_setup;
        use std::fs;

        fn setup(dir: &std::path::Path) -> SetupEnv {
            SetupEnv {
                codex_home: Some(dir.join("codex")),
                ..test_setup()
            }
        }

        fn change(prepared: Prepared<Plan>) -> Plan {
            match prepared {
                Prepared::Change(plan) => plan,
                _ => panic!("expected a change"),
            }
        }

        #[test]
        fn a_missing_file_and_folder_are_created_without_a_backup() {
            let dir = tempfile::tempdir().unwrap();
            let setup = setup(dir.path());
            let plan = change(prepare(&setup));
            let lines = plan.describe().join("\n");
            assert!(
                lines.contains("a new file") && !lines.contains("backup before"),
                "{lines}"
            );
            plan.apply().unwrap();
            let written = fs::read_to_string(dir.path().join("codex/config.toml")).unwrap();
            assert_eq!(written, BLOCK);
            let names: Vec<_> = fs::read_dir(dir.path().join("codex"))
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            assert_eq!(names, ["config.toml"]);
            // 登録した後に調べ直すと、同じ内容と分かる
            assert!(matches!(prepare(&setup), Prepared::Unchanged(_)));
        }

        #[test]
        fn an_existing_file_is_backed_up_next_to_it_and_never_overwritten() {
            let dir = tempfile::tempdir().unwrap();
            let setup = setup(dir.path());
            let config = dir.path().join("codex/config.toml");
            fs::create_dir_all(config.parent().unwrap()).unwrap();
            let original = "# mine\nmodel = \"x\"\n";
            fs::write(&config, original).unwrap();
            // 同じ名前のバックアップがすでにあっても、上書きしない
            let taken = dir
                .path()
                .join("codex/config.toml.yunagi-backup-20261104-120000");
            fs::write(&taken, "precious").unwrap();
            let plan = change(prepare(&setup));
            let done = plan.apply().unwrap().join("\n");
            assert_eq!(fs::read_to_string(&taken).unwrap(), "precious");
            let second = dir
                .path()
                .join("codex/config.toml.yunagi-backup-20261104-120000-2");
            assert_eq!(fs::read_to_string(&second).unwrap(), original);
            assert!(done.contains("-2"), "{done}");
            let written = fs::read_to_string(&config).unwrap();
            assert!(
                written.starts_with(original) && written.ends_with(BLOCK),
                "{written}"
            );
        }

        #[test]
        fn a_file_changed_while_waiting_is_not_overwritten() {
            let dir = tempfile::tempdir().unwrap();
            let setup = setup(dir.path());
            let config = dir.path().join("codex/config.toml");
            fs::create_dir_all(config.parent().unwrap()).unwrap();
            fs::write(&config, "a = 1\n").unwrap();
            let plan = change(prepare(&setup));
            fs::write(&config, "a = 2\n").unwrap();
            let e = plan.apply().unwrap_err();
            assert!(e.contains("changed while waiting"), "{e}");
            assert_eq!(fs::read_to_string(&config).unwrap(), "a = 2\n");
        }

        #[test]
        fn broken_toml_stops_without_touching_the_file_or_leaking_it() {
            let dir = tempfile::tempdir().unwrap();
            let setup = setup(dir.path());
            let config = dir.path().join("codex/config.toml");
            fs::create_dir_all(config.parent().unwrap()).unwrap();
            fs::write(&config, "ok = 1\ntoken = \"sk-secret\nbroken = [\n").unwrap();
            let Prepared::Failed(msg) = prepare(&setup) else {
                panic!("expected a failure");
            };
            assert!(
                msg.contains("not valid TOML") && msg.contains("line"),
                "{msg}"
            );
            assert!(!msg.contains("sk-secret"), "{msg}");
            assert_eq!(
                fs::read_to_string(&config).unwrap(),
                "ok = 1\ntoken = \"sk-secret\nbroken = [\n"
            );
        }

        #[test]
        fn a_disabled_registration_is_left_alone_but_reported() {
            let dir = tempfile::tempdir().unwrap();
            let setup = setup(dir.path());
            let config = dir.path().join("codex/config.toml");
            fs::create_dir_all(config.parent().unwrap()).unwrap();
            fs::write(&config, format!("{BLOCK}enabled = false\n")).unwrap();
            let Prepared::Unchanged(line) = prepare(&setup) else {
                panic!("expected no change");
            };
            assert!(line.contains("enabled = false"), "{line}");
        }

        #[test]
        fn the_variable_names_follow_the_platform() {
            use yunagi_core::storage::data_dir::Platform;
            let dir = tempfile::tempdir().unwrap();
            for (platform, expected) in [
                (Platform::Linux, "env_vars = [\"XDG_DATA_HOME\", \"HOME\"]"),
                (Platform::MacOs, "env_vars = [\"HOME\"]"),
                (Platform::Windows, "env_vars = [\"LOCALAPPDATA\"]"),
            ] {
                let setup = SetupEnv {
                    platform,
                    ..setup(dir.path())
                };
                let plan = change(prepare(&setup));
                assert!(plan.new_text.contains(expected), "{}", plan.new_text);
                // 値は書かない
                assert!(!plan.new_text.contains('/'), "{}", plan.new_text);
            }
        }
    }
}
