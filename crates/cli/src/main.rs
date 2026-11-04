//! コンソール用バイナリ `yunagi`（docs/design/01-architecture.md 2章、
//! docs/design/05-mcp-cli-spec.md）。本体は `yunagi_cli::run`。

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut stdin, mut stdout, mut stderr) =
        (std::io::stdin(), std::io::stdout(), std::io::stderr());
    let code = yunagi_cli::run(
        &args,
        &yunagi_cli::Env::default(),
        &mut yunagi_cli::Io {
            stdin: &mut stdin,
            stdout: &mut stdout,
            stderr: &mut stderr,
        },
    );
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
