//! 引数の読み取り（docs/design/05-mcp-cli-spec.md 3章・4.1）。
//!
//! 読むだけで、標準入力にもデータベースにも触れない。誤りは `invalid_argument`（終了コード 2）。
//! `--json` はどこに書いてもよい。オプションの値は `--date D` と `--date=D` のどちらでも書ける。

use std::io::Read;

use serde_json::{Map, Value, json};
use yunagi_core::storage::table::agent_client::Scope;

use crate::error::{self, ErrorObject};

/// 読んだ引数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// `--json`：機械可読な JSON を出す。
    pub json: bool,
    pub command: Command,
}

/// コマンド。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `--version`：版を表示する。
    Version,
    /// `--help`：使い方を表示する。
    Help,
    /// 読み取りの操作（ローカル API の操作の名前と引数）。
    Operation(Operation),
    /// `doctor`：診断情報を出す。
    Doctor,
    /// `mcp [--client ID]`：MCP サーバー（stdio）を動かす（05 4.5）。`client` は識別子で、無ければ
    /// `unknown` クライアントになる。形は読む段階で確かめてある。
    Mcp { client: Option<String> },
    /// `agent setup [--claude-code] [--codex]`：エージェントの設定に MCP サーバーを登録する（05 6章）。
    /// どちらの指定も無ければ、検出したエージェントすべて。
    AgentSetup { claude_code: bool, codex: bool },
    /// `agent permissions [CLIENT] [--allow S]... [--deny S]...`（05 2章・7章）：クライアントの権限を
    /// 見る。`--allow`・`--deny` があれば変える（変えるには、変更内容を見せて端末で確認する）。
    AgentPermissions {
        client: Option<String>,
        allow: Vec<Scope>,
        deny: Vec<Scope>,
    },
    /// `agent revert CHANGE_ID [--force]`：エージェントの変更を取り消す（05 7章。端末で確認する）。
    AgentRevert { change_id: String, force: bool },
}

/// ローカル API の操作の名前（05 3章のツール名）と引数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operation {
    pub method: &'static str,
    pub params: Map<String, Value>,
    /// 標準入力から読む本文を入れる引数の名前（`TEXT` か `--note` に `-` が指定されたとき。05 4.1）。
    /// 標準入力は引数を読む段階では読まず、依頼を組み立てるときに [`Operation::read_stdin`] で読む。
    pub stdin_key: Option<&'static str>,
}

impl Operation {
    fn new(method: &'static str, params: Map<String, Value>) -> Self {
        Self {
            method,
            params,
            stdin_key: None,
        }
    }

    /// 本文の引数 `key`（値は `-` で、あとで標準入力に置き換える）を持つ操作にする。
    fn with_stdin(mut self, key: &'static str) -> Self {
        if self.params.get(key).and_then(Value::as_str) == Some("-") {
            self.stdin_key = Some(key);
        }
        self
    }

    /// `-` が指定された本文を、標準入力 `stdin` から読んで引数に入れる（接続の前に呼ぶ。05 4.1）。
    pub fn read_stdin(&mut self, stdin: &mut dyn Read) -> Result<(), ErrorObject> {
        if let Some(key) = self.stdin_key.take() {
            let text = text_arg("-", stdin)?;
            self.params.insert(key.into(), Value::String(text));
        }
        Ok(())
    }
}

/// 使い方。
pub const USAGE: &str = "\
Usage: yunagi [--json] <command>

Commands:
  board [get] [--date D]            Show the board of a day (default: today)
  board open [--date D]             Show only open TODOs and unresolved blockers
  note get [--date D] [--heading H] Show the note (or the sections with a heading)
  search QUERY [--from D] [--to D] [--kind K]... [--limit N]
                                    Search TODOs, blockers, notes and reflections
  todo add TEXT [--date D]          Add a TODO
  todo edit ID TEXT                 Change the text of a TODO
  todo complete ID                  Mark a TODO done (alias: todo done)
  todo reopen ID                    Mark a done TODO open again
  blocker add TEXT [--date D]       Add a blocker
  blocker edit ID TEXT              Change the text of a blocker
  blocker resolve ID [--note TEXT]  Mark a blocker resolved (with a resolution note)
  reflection append TEXT [--date D] Append to the reflection (alias: reflect append)
  note append TEXT [--date D] [--heading [TITLE]]
                                    Append to the note (with a time heading)
  note edit --line N --expect TEXT (--replace TEXT | --delete | --move-to M) [--date D]
                                    Change a line of the note
  doctor                            Show diagnostic information (no work data)
  mcp [--client ID]                 Run the MCP server on standard input and output
                                    (for AI agents; ID is for example claude-code)
  agent setup [--claude-code] [--codex]
                                    Register the MCP server with Claude Code and Codex
                                    (shows the changes and asks first; needs a terminal)
  agent permissions [CLIENT] [--allow S]... [--deny S]...
                                    Show what each agent may do, or change it
                                    (changing shows the result and asks first; needs a terminal)
  agent history [--client NAME] [--unseen] [--limit N] [--before ID]
                                    List the changes agents made, newest first
  agent revert CHANGE_ID [--force]  Undo one change an agent made
                                    (shows what changes and asks first; needs a terminal)

Options:
  --json       Print JSON instead of text
  --version    Print the version
  -h, --help   Print this help

D is YYYY-MM-DD, today or yesterday. K is todo, blocker, note or reflection.
S is read, todo, blocker, append or note_edit (--allow todo,blocker also works).
TEXT (and --note) is read from standard input when it is -.
A value that starts with -- is written as --name=VALUE (for example --expect=---).
--heading=TITLE (or --heading TITLE) sets the title; a bare --heading adds just the time.
";

/// 引数の誤り。誤りの出し方（`--json` か）は、誤りがあっても読み終えた結果で決める（`--json` の
/// 判定を、読み取りの規則と食い違わせないため）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub error: ErrorObject,
    /// `--json` が指定されていたか。
    pub json: bool,
}

/// 引数 `args`（コマンド名を除く）を読む。最後まで読んでから、最初の誤りを返す。
///
/// オプションの値は `--name value` か `--name=value`。`--name` の次が `--` で始まる引数のときは、
/// 値が無いものとする（`--date --json` で `--json` を日付として読まない。`--` で始まる値は
/// `--name=--x` の形で書く）。
pub fn parse(args: &[String]) -> Result<Parsed, ParseError> {
    let mut json = false;
    let mut version = false;
    let mut help = false;
    let mut first_error: Option<ErrorObject> = None;
    let mut positionals: Vec<&str> = Vec::new();
    let mut options: Vec<(String, Option<String>)> = Vec::new();
    let mut iter = args.iter().peekable();
    let mut only_positionals = false;
    while let Some(arg) = iter.next() {
        if only_positionals || arg == "-" || !arg.starts_with('-') {
            positionals.push(arg);
            continue;
        }
        match arg.as_str() {
            "--" => only_positionals = true,
            "--json" => json = true,
            "--version" | "-V" => version = true,
            "--help" | "-h" => help = true,
            _ => {
                let Some(rest) = arg.strip_prefix("--") else {
                    first_error.get_or_insert_with(|| {
                        error::invalid_argument(format!(
                            "unknown option {arg} (to search for text that starts with '-', write it after --, for example: yunagi search -- {arg})"
                        ))
                    });
                    continue;
                };
                match rest.split_once('=') {
                    Some((name, value)) => {
                        options.push((name.to_owned(), Some(value.to_owned())));
                    }
                    // 値を取らない指定
                    None if matches!(
                        rest,
                        "delete" | "claude-code" | "codex" | "force" | "unseen"
                    ) =>
                    {
                        options.push((rest.to_owned(), None));
                    }
                    // 値を省略できる指定。次の引数が `-` で始まらなければ件名として読む
                    // （`-` や `--` で始まる件名は `--heading=TITLE` と書く）
                    None if rest == "heading" => {
                        let title = iter.next_if(|next| !next.starts_with('-')).cloned();
                        options.push((rest.to_owned(), title));
                    }
                    None => match iter.next_if(|next| !next.starts_with("--")) {
                        Some(value) => options.push((rest.to_owned(), Some(value.clone()))),
                        None => {
                            first_error.get_or_insert_with(|| {
                                error::invalid_argument(format!(
                                    "option --{rest} needs a value (a value that starts with -- is written as --{rest}=VALUE)"
                                ))
                            });
                        }
                    },
                }
            }
        }
    }
    // `yunagi mcp` の標準出力は MCP のメッセージだけにする：`--json` を付けても、引数の誤りは標準エラーに
    // テキストで出す（05 4.5）
    let json = json && positionals.first() != Some(&"mcp");
    let fail = |error| ParseError { error, json };
    if help {
        return Ok(Parsed {
            json,
            command: Command::Help,
        });
    }
    if version {
        return Ok(Parsed {
            json,
            command: Command::Version,
        });
    }
    if let Some(error) = first_error {
        return Err(fail(error));
    }
    let command = command(&positionals, &options).map_err(fail)?;
    // 確認の質問を含むコマンドは人が読む文だけを出す。`--json` は断る（権限を見るだけなら JSON も出せる）
    let asks = match &command {
        Command::AgentSetup { .. } => Some("agent setup"),
        Command::AgentPermissions { allow, deny, .. } if !(allow.is_empty() && deny.is_empty()) => {
            Some("agent permissions with --allow or --deny")
        }
        Command::AgentRevert { .. } => Some("agent revert"),
        _ => None,
    };
    if json && let Some(name) = asks {
        return Err(ParseError {
            error: error::invalid_argument(format!(
                "{name} prints text only (it asks a question); --json is not supported"
            )),
            json: false,
        });
    }
    Ok(Parsed { json, command })
}

/// 名詞（と動詞）、位置引数、オプションからコマンドを作る。
fn command(
    positionals: &[&str],
    options: &[(String, Option<String>)],
) -> Result<Command, ErrorObject> {
    let Some((&noun, rest)) = positionals.split_first() else {
        return Err(error::invalid_argument(format!(
            "no command given\n\n{USAGE}"
        )));
    };
    // `--heading`・`--delete` は `note` の操作だけの指定。ほかのコマンドでは、読み取りの段階で次の引数を
    // 値として飲み込むので、引数の数の誤りより先に、知らないオプションとして断る
    if noun != "note"
        && let Some((name, _)) = options
            .iter()
            .find(|(n, _)| n == "heading" || n == "delete")
    {
        return Err(error::invalid_argument(format!("unknown option --{name}")));
    }
    let mut opts = Options(options.to_vec());
    let command = match noun {
        "board" => {
            let open = match rest {
                [] | ["get"] => false,
                ["open"] => true,
                _ => return Err(unknown(positionals)),
            };
            let params = opts.date_only()?;
            if open {
                Command::Operation(Operation::new("open_board", params))
            } else {
                Command::Operation(Operation::new("get_board", params))
            }
        }
        "note" => match rest {
            ["get"] | ["show"] => {
                let mut params = Map::new();
                opts.take_into(&mut params, "date", "date")?;
                // 読み取りの `--heading` は、見出しの文字が要る
                match opts.take_optional("heading")? {
                    None => {}
                    Some(Some(heading)) => {
                        params.insert("heading".into(), json!(heading));
                    }
                    Some(None) => {
                        return Err(error::invalid_argument(
                            "option --heading needs a value (a heading from `yunagi board`, for example --heading \"10:00 sync\")",
                        ));
                    }
                }
                opts.finish()?;
                Command::Operation(Operation::new("get_note", params))
            }
            ["append", text] => {
                let mut params = text_date_params(&mut opts, text)?;
                // `--heading` は値を省略できる（省略は件名なしの `## HH:MM`）
                if let Some(title) = opts.take_optional("heading")? {
                    params.insert("heading".into(), json!(title.unwrap_or_default()));
                }
                opts.finish()?;
                Command::Operation(Operation::new("append_note", params).with_stdin("text"))
            }
            // TEXT が無い。`--heading` の直後に置いた TEXT は件名として読まれているので、そう伝える
            ["append"] => {
                let taken = match opts.take_optional("heading")? {
                    Some(Some(title)) => format!(
                        " (\"{title}\" was read as the --heading title; write --heading= for a heading with just the time, or put TEXT before --heading)"
                    ),
                    _ => String::new(),
                };
                return Err(error::invalid_argument(format!(
                    "note append needs TEXT{taken}"
                )));
            }
            ["edit"] => {
                let mut params = Map::new();
                opts.take_into(&mut params, "date", "date")?;
                let line = opts.take_number("line")?.ok_or_else(|| {
                    error::invalid_argument("note edit needs --line N (a line number, from 1)")
                })?;
                params.insert("line".into(), json!(line));
                // 行そのものを渡すので、`-` も文字として渡す（標準入力からは読まない）
                let expected = opts.take_value("expect")?.ok_or_else(|| {
                    error::invalid_argument(
                        "note edit needs --expect TEXT (the current text of the line; a value that starts with -- is written as --expect=VALUE)",
                    )
                })?;
                params.insert("expected_text".into(), json!(expected));
                let replace = opts.take_value("replace")?;
                let delete = opts.take_flag("delete")?;
                let move_to = opts.take_number("move-to")?;
                match (replace, delete, move_to) {
                    (Some(text), false, None) => {
                        params.insert("replace".into(), json!(text));
                    }
                    (None, true, None) => {
                        params.insert("delete".into(), json!(true));
                    }
                    (None, false, Some(to)) => {
                        params.insert("move_to".into(), json!(to));
                    }
                    _ => {
                        return Err(error::invalid_argument(
                            "note edit needs exactly one of --replace TEXT, --delete and --move-to M",
                        ));
                    }
                }
                opts.finish()?;
                Command::Operation(Operation::new("edit_note", params))
            }
            _ => return Err(unknown_or_usage(positionals, rest)),
        },
        "todo" => match rest {
            ["add", text] => {
                let params = text_date_params(&mut opts, text)?;
                opts.finish()?;
                Command::Operation(Operation::new("add_todo", params).with_stdin("text"))
            }
            ["edit", id, text] => item_text(&mut opts, "edit_todo", id, text)?,
            ["complete", id] | ["done", id] => item_id(&mut opts, "complete_todo", id)?,
            ["reopen", id] => item_id(&mut opts, "reopen_todo", id)?,
            _ => return Err(unknown_or_usage(positionals, rest)),
        },
        "blocker" => match rest {
            ["add", text] => {
                let params = text_date_params(&mut opts, text)?;
                opts.finish()?;
                Command::Operation(Operation::new("add_blocker", params).with_stdin("text"))
            }
            ["edit", id, text] => item_text(&mut opts, "edit_blocker", id, text)?,
            ["resolve", id] => {
                let mut params = Map::new();
                params.insert("id".into(), json!(id));
                opts.take_into(&mut params, "note", "note")?;
                opts.finish()?;
                Command::Operation(Operation::new("resolve_blocker", params).with_stdin("note"))
            }
            _ => return Err(unknown_or_usage(positionals, rest)),
        },
        "reflection" | "reflect" => match rest {
            ["append", text] => {
                let params = text_date_params(&mut opts, text)?;
                opts.finish()?;
                Command::Operation(Operation::new("append_reflection", params).with_stdin("text"))
            }
            _ => return Err(unknown_or_usage(positionals, rest)),
        },
        "search" => {
            let [query] = rest else {
                return Err(error::invalid_argument(
                    "search needs exactly one QUERY (quote it if it has spaces)",
                ));
            };
            let mut params = Map::new();
            params.insert("query".into(), json!(query));
            opts.take_into(&mut params, "from", "from")?;
            opts.take_into(&mut params, "to", "to")?;
            let kinds = opts.take_all("kind")?;
            if !kinds.is_empty() {
                params.insert("kinds".into(), json!(kinds));
            }
            if let Some(limit) = opts.take_value("limit")? {
                let n: u64 = limit.parse().map_err(|_| {
                    error::invalid_argument("--limit must be a positive whole number")
                })?;
                params.insert("limit".into(), json!(n));
            }
            opts.finish()?;
            Command::Operation(Operation::new("search", params))
        }
        "doctor" => {
            if !rest.is_empty() {
                return Err(unknown(positionals));
            }
            opts.finish()?;
            Command::Doctor
        }
        "mcp" => {
            if !rest.is_empty() {
                return Err(unknown(positionals));
            }
            let client = opts.take_value("client")?;
            opts.finish()?;
            // 識別子の形の誤りは、起動のときに一度だけ断る（ツールの呼び出しのたびには返さない。05 4.5）
            if let Some(name) = &client
                && !yunagi_core::domain::agent::is_valid_client_name(name)
            {
                return Err(error::invalid_argument(
                    "--client must start with a lowercase letter or digit and use only lowercase letters, digits, '-', '_' and '.' (up to 64 characters), for example --client claude-code",
                ));
            }
            Command::Mcp { client }
        }
        "agent" => match rest {
            ["setup"] => {
                let claude_code = opts.take_flag("claude-code")?;
                let codex = opts.take_flag("codex")?;
                opts.finish()?;
                Command::AgentSetup { claude_code, codex }
            }
            ["permissions"] | ["permissions", _] => {
                let client = match rest {
                    ["permissions", name] => Some((*name).to_owned()),
                    _ => None,
                };
                let allow = take_scopes(&mut opts, "allow")?;
                let deny = take_scopes(&mut opts, "deny")?;
                opts.finish()?;
                if client.is_none() && !(allow.is_empty() && deny.is_empty()) {
                    return Err(error::invalid_argument(
                        "agent permissions needs the CLIENT to change (see `yunagi agent permissions` for the names)",
                    ));
                }
                if let Some(both) = allow.iter().find(|s| deny.contains(s)) {
                    return Err(error::invalid_argument(format!(
                        "the scope {both} is both allowed and denied"
                    )));
                }
                Command::AgentPermissions {
                    client,
                    allow,
                    deny,
                }
            }
            ["history"] => {
                let mut params = Map::new();
                opts.take_into(&mut params, "client", "client")?;
                opts.take_into(&mut params, "before", "before")?;
                if opts.take_flag("unseen")? {
                    params.insert("unseen".into(), json!(true));
                }
                if let Some(limit) = opts.take_value("limit")? {
                    let n: u64 = limit.parse().map_err(|_| {
                        error::invalid_argument("--limit must be a positive whole number")
                    })?;
                    params.insert("limit".into(), json!(n));
                }
                opts.finish()?;
                Command::Operation(Operation::new("list_changes", params))
            }
            ["revert", change_id] => {
                let force = opts.take_flag("force")?;
                opts.finish()?;
                Command::AgentRevert {
                    change_id: (*change_id).to_owned(),
                    force,
                }
            }
            _ => return Err(unknown_or_usage(positionals, rest)),
        },
        _ => return Err(unknown(positionals)),
    };
    Ok(command)
}

/// `--allow`・`--deny` の値（繰り返せる。`todo,blocker` のようにコンマで区切ってもよい）をスコープにする。
fn take_scopes(opts: &mut Options, name: &str) -> Result<Vec<Scope>, ErrorObject> {
    let mut scopes = Vec::new();
    for value in opts.take_all(name)? {
        for part in value.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let scope = part.parse::<Scope>().map_err(|_| {
                error::invalid_argument(format!(
                    "unknown scope \"{part}\" for --{name} (use read, todo, blocker, append or note_edit)"
                ))
            })?;
            if !scopes.contains(&scope) {
                scopes.push(scope);
            }
        }
    }
    Ok(scopes)
}

fn unknown(positionals: &[&str]) -> ErrorObject {
    error::invalid_argument(format!(
        "unknown command: {}\n\n{USAGE}",
        positionals.join(" ")
    ))
}

/// 名詞は合っているが、動詞が無い・引数の数が合わない。
fn unknown_or_usage(positionals: &[&str], rest: &[&str]) -> ErrorObject {
    if rest.is_empty() {
        return unknown(positionals);
    }
    error::invalid_argument(format!(
        "wrong arguments for: {}\n\n{USAGE}",
        positionals.join(" ")
    ))
}

/// `TEXT` と `--date` を持つ操作（`add_todo` など）の引数。
fn text_date_params(opts: &mut Options, text: &str) -> Result<Map<String, Value>, ErrorObject> {
    let mut params = Map::new();
    params.insert("text".into(), json!(text));
    opts.take_into(&mut params, "date", "date")?;
    Ok(params)
}

/// `ID TEXT` を取る操作（`edit_todo`・`edit_blocker`）。
fn item_text(
    opts: &mut Options,
    method: &'static str,
    id: &str,
    text: &str,
) -> Result<Command, ErrorObject> {
    std::mem::take(opts).finish()?;
    let params = Map::from_iter([
        ("id".to_owned(), json!(id)),
        ("text".to_owned(), json!(text)),
    ]);
    Ok(Command::Operation(
        Operation::new(method, params).with_stdin("text"),
    ))
}

/// `ID` だけを取る操作（`complete_todo`・`reopen_todo`）。ID の項目の日を対象にするので `--date` は無い。
fn item_id(opts: &mut Options, method: &'static str, id: &str) -> Result<Command, ErrorObject> {
    std::mem::take(opts).finish()?;
    let params = Map::from_iter([("id".to_owned(), json!(id))]);
    Ok(Command::Operation(Operation::new(method, params)))
}

/// 読み終えていないオプション。値が `None` のものは、値を書かなかった指定（`--delete`、値を省略した
/// `--heading`）。
#[derive(Default)]
struct Options(Vec<(String, Option<String>)>);

impl Options {
    /// 名前 `name` の指定を取り出す。無ければ `None`、値を書かなかったなら `Some(None)`。同じ名前を
    /// 繰り返したら誤り（前の指定の誤りを、後ろの指定で黙って隠さないため。繰り返せるのは `--kind` だけ）。
    fn take_optional(&mut self, name: &str) -> Result<Option<Option<String>>, ErrorObject> {
        let mut found = Vec::new();
        self.0.retain(|(n, v)| {
            if n == name {
                found.push(v.clone());
                false
            } else {
                true
            }
        });
        if found.len() > 1 {
            return Err(error::invalid_argument(format!(
                "option --{name} is given more than once"
            )));
        }
        Ok(found.pop())
    }

    /// 値を取るオプション `name` の値を 1 つ取り出す。
    fn take_value(&mut self, name: &str) -> Result<Option<String>, ErrorObject> {
        match self.take_optional(name)? {
            None => Ok(None),
            Some(Some(value)) => Ok(Some(value)),
            Some(None) => Err(error::invalid_argument(format!(
                "option --{name} needs a value"
            ))),
        }
    }

    /// 値を取らないオプション `name`。あれば `true`（`--delete=x` のように値を付けたら誤り）。
    fn take_flag(&mut self, name: &str) -> Result<bool, ErrorObject> {
        match self.take_optional(name)? {
            None => Ok(false),
            Some(None) => Ok(true),
            Some(Some(_)) => Err(error::invalid_argument(format!(
                "option --{name} does not take a value"
            ))),
        }
    }

    /// 0 以上の整数を取るオプション `name`。
    fn take_number(&mut self, name: &str) -> Result<Option<u64>, ErrorObject> {
        match self.take_value(name)? {
            None => Ok(None),
            Some(value) => value.parse().map(Some).map_err(|_| {
                error::invalid_argument(format!("--{name} must be a whole number, from 0"))
            }),
        }
    }

    /// 名前 `name` のオプションの値をすべて取り出す。
    fn take_all(&mut self, name: &str) -> Result<Vec<String>, ErrorObject> {
        let mut taken = Vec::new();
        let mut missing = false;
        self.0.retain(|(n, v)| {
            if n == name {
                match v {
                    Some(v) => taken.push(v.clone()),
                    None => missing = true,
                }
                false
            } else {
                true
            }
        });
        if missing {
            return Err(error::invalid_argument(format!(
                "option --{name} needs a value"
            )));
        }
        Ok(taken)
    }

    /// オプション `name` があれば、引数の名前 `key` で `params` に入れる。
    fn take_into(
        &mut self,
        params: &mut Map<String, Value>,
        name: &str,
        key: &str,
    ) -> Result<(), ErrorObject> {
        if let Some(value) = self.take_value(name)? {
            params.insert(key.into(), json!(value));
        }
        Ok(())
    }

    /// `--date` だけを持つ操作の引数。
    fn date_only(&mut self) -> Result<Map<String, Value>, ErrorObject> {
        let mut params = Map::new();
        self.take_into(&mut params, "date", "date")?;
        std::mem::take(self).finish()?;
        Ok(params)
    }

    /// 使われなかったオプションがあれば誤り（綴りの間違いを黙って無視しない）。
    fn finish(self) -> Result<(), ErrorObject> {
        match self.0.first() {
            Some((name, _)) => Err(error::invalid_argument(format!("unknown option --{name}"))),
            None => Ok(()),
        }
    }
}

/// 標準入力から読む本文の大きさの上限（バイト）。依頼は JSON で送るので、制御文字 1 バイトが `\u00XX` の
/// 6 バイトになりうる。そのため、依頼の上限（`MAX_REQUEST_LINE`）から JSON の包みの余裕を引いた大きさの
/// 6 分の 1 にして、どんな本文でも依頼の 1 行が上限に収まるようにする。
pub const MAX_STDIN_TEXT: usize = (yunagi_api::MAX_REQUEST_LINE - 16 * 1024) / 6;

/// `TEXT` の引数 `arg` を、本文にする。`-` なら標準入力 `stdin` を最後まで読む（05 4.1。依頼を組み立て終えて
/// から接続するため、接続の前に呼ぶ）。標準入力が UTF-8 でない・大きすぎるときは `invalid_argument`。末尾の
/// 改行は除かない（前後の空白を除くのはコアの決まりで、同じコマンドのやり直しは同じ本文と判定される。
/// 05 3章）。
pub fn text_arg(arg: &str, stdin: &mut dyn Read) -> Result<String, ErrorObject> {
    if arg != "-" {
        return Ok(arg.to_owned());
    }
    let mut bytes = Vec::new();
    let limit = u64::try_from(MAX_STDIN_TEXT).unwrap_or(u64::MAX) + 1;
    stdin
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|_| error::invalid_argument("could not read the text from standard input"))?;
    if bytes.len() > MAX_STDIN_TEXT {
        return Err(error::invalid_argument(
            "the text from standard input is too large",
        ));
    }
    String::from_utf8(bytes)
        .map_err(|_| error::invalid_argument("the text from standard input is not UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Parsed, ErrorObject> {
        let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        parse(&args).map_err(|e| e.error)
    }

    #[test]
    fn a_missing_option_value_does_not_swallow_the_next_option() {
        let args: Vec<String> = ["board", "--date", "--json"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let e = parse(&args).unwrap_err();
        assert!(e.json);
        assert!(error::message(&e.error).contains("--date needs a value"));
        // `--json` が値として読まれる位置にあっても、誤りの出し方は `--json` の指定に従う
        let args: Vec<String> = ["search", "x", "--kind", "--json"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        assert!(parse(&args).unwrap_err().json);
        // `--` の後ろは位置引数で、`--json` の指定ではない
        let args: Vec<String> = ["search", "--", "--json"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let parsed = parse(&args).unwrap();
        assert!(!parsed.json);
        // 値が `--` で始まるときは `=` で書く
        assert_eq!(
            op(&["note", "get", "--heading=--x"]).params["heading"],
            "--x"
        );
    }

    #[test]
    fn the_stdin_limit_keeps_any_text_within_the_request_line() {
        // 制御文字だけの本文でも、JSON の 1 行が依頼の上限に収まる
        let text = "\u{1}".repeat(MAX_STDIN_TEXT);
        let line = json!({"jsonrpc": "2.0", "id": 1, "method": "add_todo", "params": {"text": text, "date": "yesterday"}}).to_string();
        assert!(line.len() < yunagi_api::MAX_REQUEST_LINE);
    }

    fn op(args: &[&str]) -> Operation {
        match parse_args(args).unwrap().command {
            Command::Operation(op) => op,
            other => panic!("not an operation: {other:?}"),
        }
    }

    #[test]
    fn board_and_its_alias() {
        assert_eq!(op(&["board"]).method, "get_board");
        assert_eq!(op(&["board", "get"]).method, "get_board");
        let open = op(&["board", "open", "--date", "yesterday"]);
        assert_eq!(open.method, "open_board");
        assert_eq!(open.params["date"], "yesterday");
        assert_eq!(
            op(&["board", "--date=2026-11-02"]).params["date"],
            "2026-11-02"
        );
    }

    #[test]
    fn note_get_and_its_alias_take_a_heading() {
        for verb in ["get", "show"] {
            let note = op(&["note", verb, "--heading", "10:00 定例", "--date", "today"]);
            assert_eq!(note.method, "get_note");
            assert_eq!(note.params["heading"], "10:00 定例");
            assert_eq!(note.params["date"], "today");
        }
    }

    #[test]
    fn search_takes_a_query_a_range_and_repeated_kinds() {
        let search = op(&[
            "search",
            "rate limit",
            "--from",
            "2026-11-01",
            "--to",
            "today",
            "--kind",
            "todo",
            "--kind",
            "note",
            "--limit",
            "10",
        ]);
        assert_eq!(search.method, "search");
        assert_eq!(search.params["query"], "rate limit");
        assert_eq!(search.params["from"], "2026-11-01");
        assert_eq!(search.params["kinds"], json!(["todo", "note"]));
        assert_eq!(search.params["limit"], 10);
    }

    #[test]
    fn agent_setup_takes_the_two_target_flags() {
        let setup = |claude_code, codex| Command::AgentSetup { claude_code, codex };
        assert_eq!(
            parse_args(&["agent", "setup"]).unwrap().command,
            setup(false, false)
        );
        assert_eq!(
            parse_args(&["agent", "setup", "--codex"]).unwrap().command,
            setup(false, true)
        );
        // 値を取らない指定なので、次の位置引数を値として飲み込まない
        assert_eq!(
            parse_args(&["agent", "--claude-code", "setup", "--codex"])
                .unwrap()
                .command,
            setup(true, true)
        );
        for bad in [
            &["agent"][..],
            &["agent", "setup", "extra"],
            &["agent", "setup", "--codex=1"],
            &["agent", "setup", "--client", "x"],
            &["agent", "revert"],
            &["board", "--codex"],
        ] {
            assert!(parse_args(bad).is_err(), "{bad:?}");
        }
        // 確認の質問があるので `--json` は断り、テキストで伝える
        let args: Vec<String> = ["--json", "agent", "setup"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let e = parse(&args).unwrap_err();
        assert!(!e.json);
        assert_eq!(error::code(&e.error), "invalid_argument");
    }

    #[test]
    fn mcp_takes_an_optional_client() {
        assert_eq!(
            parse_args(&["mcp"]).unwrap().command,
            Command::Mcp { client: None }
        );
        let with_client = Command::Mcp {
            client: Some("claude-code".into()),
        };
        assert_eq!(
            parse_args(&["mcp", "--client", "claude-code"])
                .unwrap()
                .command,
            with_client
        );
        assert_eq!(
            parse_args(&["mcp", "--client=claude-code"])
                .unwrap()
                .command,
            with_client
        );
    }

    #[test]
    fn mcp_ignores_json_so_that_errors_stay_off_standard_output() {
        assert!(!parse_args(&["mcp", "--json"]).unwrap().json);
        // `--json` や値を取るオプションが `mcp` の前にあっても、`mcp` が最初の位置引数なら同じ
        for args in [
            &["--json", "mcp", "--client", "Bad Name"][..],
            &["mcp", "--json", "--client", "Bad Name"],
            &["--json", "--client", "Bad", "mcp"],
        ] {
            let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
            let e = parse(&args).unwrap_err();
            assert!(!e.json, "{args:?}");
            assert_eq!(error::code(&e.error), "invalid_argument");
        }
    }

    #[test]
    fn json_can_be_given_anywhere() {
        assert!(parse_args(&["--json", "doctor"]).unwrap().json);
        assert!(parse_args(&["doctor", "--json"]).unwrap().json);
        assert!(!parse_args(&["doctor"]).unwrap().json);
    }

    #[test]
    fn version_and_help_win_over_the_rest() {
        assert_eq!(
            parse_args(&["--version"]).unwrap().command,
            Command::Version
        );
        assert_eq!(parse_args(&["board", "-h"]).unwrap().command, Command::Help);
    }

    #[test]
    fn mistakes_are_invalid_arguments() {
        let bad: &[&[&str]] = &[
            &[],
            &["nothing"],
            &["board", "frobnicate"],
            &["board", "--nope", "x"],
            &["board", "--date"],
            &["search"],
            &["search", "a", "b"],
            &["search", "a", "--limit", "x"],
            &["note"],
            &["doctor", "extra"],
            &["mcp", "extra"],
            &["mcp", "--client"],
            &["mcp", "--client", "Claude"],
            &["mcp", "--client", "a", "--client", "b"],
            &["mcp", "--nope", "x"],
            &["-x"],
        ];
        for args in bad {
            let e = parse_args(args).unwrap_err();
            assert_eq!(error::code(&e), "invalid_argument", "{args:?}");
        }
    }

    #[test]
    fn todo_and_blocker_commands_map_to_the_operations() {
        let add = op(&["todo", "add", "survey", "--date", "yesterday"]);
        assert_eq!(add.method, "add_todo");
        assert_eq!(add.params["text"], "survey");
        assert_eq!(add.params["date"], "yesterday");
        let edit = op(&["todo", "edit", "ID1", "new text"]);
        assert_eq!(edit.method, "edit_todo");
        assert_eq!(
            (&edit.params["id"], &edit.params["text"]),
            (&json!("ID1"), &json!("new text"))
        );
        for verb in ["complete", "done"] {
            assert_eq!(op(&["todo", verb, "ID1"]).method, "complete_todo");
        }
        assert_eq!(op(&["todo", "reopen", "ID1"]).method, "reopen_todo");
        assert_eq!(op(&["blocker", "add", "waiting"]).method, "add_blocker");
        assert_eq!(op(&["blocker", "edit", "ID1", "x"]).method, "edit_blocker");
        let resolve = op(&["blocker", "resolve", "ID1", "--note", "fixed"]);
        assert_eq!(resolve.method, "resolve_blocker");
        assert_eq!(resolve.params["note"], "fixed");
        assert!(
            !op(&["blocker", "resolve", "ID1"])
                .params
                .contains_key("note")
        );
    }

    #[test]
    fn reflection_and_note_append() {
        for noun in ["reflection", "reflect"] {
            let r = op(&[noun, "append", "good day", "--date", "today"]);
            assert_eq!(r.method, "append_reflection");
            assert_eq!(r.params["text"], "good day");
        }
        // `--heading` を付けなければ見出しを入れない
        assert!(
            !op(&["note", "append", "memo"])
                .params
                .contains_key("heading")
        );
        // 値を省略した `--heading` は件名なし（空の文字列）。文の前でも後でも同じ
        assert_eq!(
            op(&["note", "append", "memo", "--heading"]).params["heading"],
            ""
        );
        assert_eq!(
            op(&["note", "append", "--heading", "--date", "today", "memo"]).params["heading"],
            ""
        );
        assert_eq!(
            op(&["note", "append", "memo", "--heading="]).params["heading"],
            ""
        );
        // 件名は次の引数か `=` で渡す。`-` で始まる件名は `=` で書く
        assert_eq!(
            op(&["note", "append", "--heading", "retro", "memo"]).params["heading"],
            "retro"
        );
        assert_eq!(
            op(&["note", "append", "memo", "--heading=-x"]).params["heading"],
            "-x"
        );
        let dash = op(&["note", "append", "--heading", "-"]);
        assert_eq!(dash.params["text"], "-");
        assert_eq!(dash.params["heading"], "");
        assert_eq!(dash.stdin_key, Some("text"));
        // 読み取りの `--heading` は値が要る
        let bad = parse_args(&["note", "get", "--heading"]).unwrap_err();
        assert!(error::message(&bad).contains("--heading needs a value"));
    }

    #[test]
    fn note_edit_takes_exactly_one_change() {
        let replace = op(&[
            "note",
            "edit",
            "--line",
            "3",
            "--expect",
            "old",
            "--replace",
            "new",
        ]);
        assert_eq!(replace.method, "edit_note");
        assert_eq!(replace.params["line"], 3);
        assert_eq!(replace.params["expected_text"], "old");
        assert_eq!(replace.params["replace"], "new");
        // `--delete` は値を取らないので、後ろの指定を飲み込まない
        let delete = op(&[
            "note",
            "edit",
            "--line=2",
            "--expect=x",
            "--delete",
            "--date",
            "today",
        ]);
        assert_eq!(delete.params["delete"], true);
        assert_eq!(delete.params["date"], "today");
        let delete = op(&["note", "edit", "--line", "2", "--expect", "x", "--delete"]);
        assert_eq!(delete.params["delete"], true);
        let moved = op(&[
            "note",
            "edit",
            "--line",
            "2",
            "--expect",
            "x",
            "--move-to",
            "9",
        ]);
        assert_eq!(moved.params["move_to"], 9);
        // 空の行も、`-` も、`---` も、行そのものとして渡す（標準入力からは読まない）
        let blank = op(&["note", "edit", "--line", "2", "--expect=", "--replace", "-"]);
        assert_eq!(
            (&blank.params["expected_text"], &blank.params["replace"]),
            (&json!(""), &json!("-"))
        );
        assert_eq!(blank.stdin_key, None);
        assert_eq!(
            op(&["note", "edit", "--line", "2", "--expect=---", "--delete"]).params["expected_text"],
            "---"
        );
        let bad: &[&[&str]] = &[
            // 変え方が無い・2つ以上
            &["note", "edit", "--line", "2", "--expect", "x"],
            &[
                "note",
                "edit",
                "--line",
                "2",
                "--expect",
                "x",
                "--delete",
                "--move-to",
                "1",
            ],
            &[
                "note",
                "edit",
                "--line",
                "2",
                "--expect",
                "x",
                "--replace",
                "y",
                "--delete",
            ],
            // 行と --expect は必ず要る
            &["note", "edit", "--expect", "x", "--delete"],
            &["note", "edit", "--line", "2", "--delete"],
            // 数でない・値の誤り
            &["note", "edit", "--line", "two", "--expect", "x", "--delete"],
            &["note", "edit", "--line", "-1", "--expect", "x", "--delete"],
            &[
                "note",
                "edit",
                "--line",
                "2",
                "--expect",
                "x",
                "--delete=yes",
            ],
            // `---` は `--expect=---` と書く
            &["note", "edit", "--line", "2", "--expect", "---", "--delete"],
        ];
        for args in bad {
            let e = parse_args(args).unwrap_err();
            assert_eq!(error::code(&e), "invalid_argument", "{args:?}");
        }
    }

    #[test]
    fn a_heading_before_the_text_is_named_in_the_error_and_repeats_are_refused() {
        // `--heading` の直後の引数は件名として読まれる。TEXT が無くなったら、そのことを伝える
        let e = parse_args(&["note", "append", "--heading", "fixed the bug"]).unwrap_err();
        let message = error::message(&e);
        assert!(message.contains("note append needs TEXT"), "{message}");
        assert!(message.contains("\"fixed the bug\" was read as the --heading title"));
        assert!(!error::message(&parse_args(&["note", "append"]).unwrap_err()).contains("title"));
        // 同じオプションの繰り返しは、前の誤りを隠さないよう誤りにする（`--kind` だけは繰り返せる）
        for args in [
            &[
                "note",
                "edit",
                "--line",
                "2",
                "--expect",
                "x",
                "--delete=oops",
                "--delete",
            ][..],
            &[
                "note",
                "edit",
                "--line",
                "2",
                "--expect",
                "x",
                "--replace",
                "A",
                "--replace",
                "B",
            ],
            &["todo", "add", "x", "--date", "today", "--date", "yesterday"],
            &["note", "append", "x", "--heading", "--heading"],
        ] {
            let e = parse_args(args).unwrap_err();
            assert!(error::message(&e).contains("more than once"), "{args:?}");
        }
        assert!(parse_args(&["search", "x", "--kind", "todo", "--kind", "note"]).is_ok());
        // 繰り返した `--heading` の誤りは、TEXT が無いときも隠さない
        let e = parse_args(&["note", "append", "--heading", "A", "--heading", "B"]).unwrap_err();
        assert!(error::message(&e).contains("more than once"));
        // `note` 以外では、`--heading`・`--delete` が次の引数を飲み込まず、知らないオプションとして断る
        for args in [
            &["todo", "add", "--heading", "foo", "bar"][..],
            &["todo", "edit", "--heading", "ID1", "text"],
            &["search", "q", "--delete", "foo"],
        ] {
            let e = parse_args(args).unwrap_err();
            assert!(error::message(&e).contains("unknown option --"), "{args:?}");
        }
    }

    #[test]
    fn writes_by_id_take_no_date_and_a_text_needs_one_value() {
        let bad: &[&[&str]] = &[
            &["todo", "complete", "ID1", "--date", "today"],
            &["todo", "reopen", "ID1", "--date", "today"],
            &["todo", "edit", "ID1", "x", "--date", "today"],
            &["blocker", "resolve", "ID1", "--date", "today"],
            &["todo", "add"],
            &["todo", "add", "a", "b"],
            &["todo", "edit", "ID1"],
            &["todo"],
            &["todo", "frobnicate", "x"],
            &["blocker", "resolve", "ID1", "--note"],
            &["reflection", "append"],
            &["note", "append"],
        ];
        for args in bad {
            let e = parse_args(args).unwrap_err();
            assert_eq!(error::code(&e), "invalid_argument", "{args:?}");
        }
    }

    #[test]
    fn a_dash_marks_the_text_to_read_from_standard_input() {
        let mut add = op(&["todo", "add", "-"]);
        assert_eq!(add.stdin_key, Some("text"));
        let mut stdin: &[u8] = "from stdin\n".as_bytes();
        add.read_stdin(&mut stdin).unwrap();
        assert_eq!(add.params["text"], "from stdin\n");
        assert_eq!(add.stdin_key, None);
        assert_eq!(op(&["todo", "edit", "ID1", "-"]).stdin_key, Some("text"));
        assert_eq!(
            op(&["blocker", "resolve", "ID1", "--note", "-"]).stdin_key,
            Some("note")
        );
        assert_eq!(op(&["reflection", "append", "-"]).stdin_key, Some("text"));
        assert_eq!(op(&["todo", "add", "x"]).stdin_key, None);
        // 標準入力が大きすぎる・UTF-8 でないなら、接続の前に invalid_argument
        let mut bad = op(&["todo", "add", "-"]);
        let mut bytes: &[u8] = &[0xff];
        assert_eq!(
            error::code(&bad.read_stdin(&mut bytes).unwrap_err()),
            "invalid_argument"
        );
    }

    #[test]
    fn agent_permissions_history_and_revert_are_read() {
        let command = |args: &[&str]| parse_args(args).unwrap().command;
        assert_eq!(
            command(&["agent", "permissions"]),
            Command::AgentPermissions {
                client: None,
                allow: vec![],
                deny: vec![]
            }
        );
        // --allow はコンマ区切りでも繰り返しでもよく、重なりは1つにする
        assert_eq!(
            command(&[
                "agent",
                "permissions",
                "codex",
                "--allow",
                "todo, blocker",
                "--allow=todo",
                "--deny",
                "note_edit"
            ]),
            Command::AgentPermissions {
                client: Some("codex".into()),
                allow: vec![Scope::Todo, Scope::Blocker],
                deny: vec![Scope::NoteEdit]
            }
        );
        let message = |args: &[&str]| error::message(&parse_args(args).unwrap_err()).to_owned();
        assert!(
            message(&["agent", "permissions", "x", "--allow", "root"]).contains("unknown scope")
        );
        assert!(message(&["agent", "permissions", "--allow", "todo"]).contains("needs the CLIENT"));
        assert!(
            message(&[
                "agent",
                "permissions",
                "x",
                "--allow",
                "todo",
                "--deny",
                "todo"
            ])
            .contains("both allowed and denied")
        );
        // 確認の質問を含む形は --json を断る。見るだけなら JSON も出せる
        assert!(parse_args(&["agent", "permissions", "--json"]).is_ok());
        assert!(
            message(&["agent", "permissions", "x", "--deny", "read", "--json"])
                .contains("--json is not supported")
        );
        assert!(message(&["agent", "revert", "ID", "--json"]).contains("--json is not supported"));
        assert_eq!(
            command(&["agent", "revert", "ID", "--force"]),
            Command::AgentRevert {
                change_id: "ID".into(),
                force: true
            }
        );
        // --force は revert だけのオプション
        assert!(message(&["agent", "history", "--force"]).contains("unknown option --force"));
        let Command::Operation(history) = command(&[
            "agent", "history", "--client", "cli", "--unseen", "--limit", "5", "--before", "ID",
        ]) else {
            panic!("not an operation");
        };
        assert_eq!(history.method, "list_changes");
        assert_eq!(
            Value::Object(history.params),
            json!({"client": "cli", "unseen": true, "limit": 5, "before": "ID"})
        );
        assert!(message(&["agent", "history", "--limit", "x"]).contains("--limit"));
    }

    #[test]
    fn a_dash_reads_the_text_from_standard_input() {
        let mut stdin: &[u8] = "from stdin\n".as_bytes();
        assert_eq!(text_arg("-", &mut stdin).unwrap(), "from stdin\n");
        let mut unused: &[u8] = b"ignored";
        assert_eq!(text_arg("literal", &mut unused).unwrap(), "literal");
        let mut bad: &[u8] = &[0xff, 0xfe];
        assert_eq!(
            error::code(&text_arg("-", &mut bad).unwrap_err()),
            "invalid_argument"
        );
        let big = vec![b'a'; MAX_STDIN_TEXT + 1];
        let mut big: &[u8] = &big;
        assert_eq!(
            error::code(&text_arg("-", &mut big).unwrap_err()),
            "invalid_argument"
        );
    }
}
