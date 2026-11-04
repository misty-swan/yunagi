//! 引数の読み取り（docs/design/05-mcp-cli-spec.md 3章・4.1）。
//!
//! 読むだけで、標準入力にもデータベースにも触れない。誤りは `invalid_argument`（終了コード 2）。
//! `--json` はどこに書いてもよい。オプションの値は `--date D` と `--date=D` のどちらでも書ける。

use std::io::Read;

use serde_json::{Map, Value, json};

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
}

/// ローカル API の操作の名前（05 3章のツール名）と引数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operation {
    pub method: &'static str,
    pub params: Map<String, Value>,
}

impl Operation {
    fn new(method: &'static str, params: Map<String, Value>) -> Self {
        Self { method, params }
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
  doctor                            Show diagnostic information (no work data)

Options:
  --json       Print JSON instead of text
  --version    Print the version
  -h, --help   Print this help

D is YYYY-MM-DD, today or yesterday. K is todo, blocker, note or reflection.
";

/// 引数 `args`（コマンド名を除く）を読む。
pub fn parse(args: &[String]) -> Result<Parsed, ErrorObject> {
    let mut json = false;
    let mut version = false;
    let mut help = false;
    let mut positionals: Vec<&str> = Vec::new();
    let mut options: Vec<(String, String)> = Vec::new();
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
                    return Err(error::invalid_argument(format!("unknown option {arg}")));
                };
                let (name, value) = match rest.split_once('=') {
                    Some((name, value)) => (name, value.to_owned()),
                    None => match iter.next() {
                        Some(value) => (rest, value.clone()),
                        None => {
                            return Err(error::invalid_argument(format!(
                                "option --{rest} needs a value"
                            )));
                        }
                    },
                };
                options.push((name.to_owned(), value));
            }
        }
    }
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
    let command = command(&positionals, &options)?;
    Ok(Parsed { json, command })
}

/// 名詞（と動詞）、位置引数、オプションからコマンドを作る。
fn command(positionals: &[&str], options: &[(String, String)]) -> Result<Command, ErrorObject> {
    let Some((&noun, rest)) = positionals.split_first() else {
        return Err(error::invalid_argument(format!(
            "no command given\n\n{USAGE}"
        )));
    };
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
                opts.take_into(&mut params, "date", "date");
                opts.take_into(&mut params, "heading", "heading");
                opts.finish()?;
                Command::Operation(Operation::new("get_note", params))
            }
            _ => return Err(unknown(positionals)),
        },
        "search" => {
            let [query] = rest else {
                return Err(error::invalid_argument(
                    "search needs exactly one QUERY (quote it if it has spaces)",
                ));
            };
            let mut params = Map::new();
            params.insert("query".into(), json!(query));
            opts.take_into(&mut params, "from", "from");
            opts.take_into(&mut params, "to", "to");
            let kinds = opts.take_all("kind");
            if !kinds.is_empty() {
                params.insert("kinds".into(), json!(kinds));
            }
            if let Some(limit) = opts.take("limit") {
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
        _ => return Err(unknown(positionals)),
    };
    Ok(command)
}

fn unknown(positionals: &[&str]) -> ErrorObject {
    error::invalid_argument(format!(
        "unknown command: {}\n\n{USAGE}",
        positionals.join(" ")
    ))
}

/// 読み終えていないオプション。
#[derive(Default)]
struct Options(Vec<(String, String)>);

impl Options {
    /// 名前 `name` のオプションの値を 1 つ取り出す（同じ名前を繰り返したら、最後のもの）。
    fn take(&mut self, name: &str) -> Option<String> {
        let all = self.take_all(name);
        all.into_iter().last()
    }

    /// 名前 `name` のオプションの値をすべて取り出す。
    fn take_all(&mut self, name: &str) -> Vec<String> {
        let mut taken = Vec::new();
        self.0.retain(|(n, v)| {
            if n == name {
                taken.push(v.clone());
                false
            } else {
                true
            }
        });
        taken
    }

    /// オプション `name` があれば、引数の名前 `key` で `params` に入れる。
    fn take_into(&mut self, params: &mut Map<String, Value>, name: &str, key: &str) {
        if let Some(value) = self.take(name) {
            params.insert(key.into(), json!(value));
        }
    }

    /// `--date` だけを持つ操作の引数。
    fn date_only(&mut self) -> Result<Map<String, Value>, ErrorObject> {
        let mut params = Map::new();
        self.take_into(&mut params, "date", "date");
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

/// 標準入力から読む本文の大きさの上限（バイト。ローカル API の依頼の上限の半分。残りは JSON の包みと
/// 余裕）。
pub const MAX_STDIN_TEXT: usize = yunagi_api::MAX_REQUEST_LINE / 2;

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
        parse(&args)
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
            &["-x"],
        ];
        for args in bad {
            let e = parse_args(args).unwrap_err();
            assert_eq!(error::code(&e), "invalid_argument", "{args:?}");
        }
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
