//! コンソール用バイナリ `yunagi`（docs/design/01-architecture.md 2章、
//! docs/design/05-mcp-cli-spec.md）。本体は `yunagi_cli::run`。

use std::process::ExitCode;

fn main() -> ExitCode {
    let (mut stdin, mut stdout, mut stderr) =
        (std::io::stdin(), std::io::stdout(), std::io::stderr());
    let mut io = yunagi_cli::Io {
        stdin: &mut stdin,
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    // UTF-8 でない引数は、panic せずに引数の誤り（終了コード 2）にする
    let code = match std::env::args_os()
        .skip(1)
        .map(|a| a.into_string())
        .collect::<Result<Vec<String>, _>>()
    {
        Ok(args) => yunagi_cli::run(&args, &yunagi_cli::Env::default(), &mut io),
        Err(_) => {
            let json = std::env::args_os().any(|a| a == "--json");
            yunagi_cli::invalid_utf8_argument(json, &mut io)
        }
    };
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
