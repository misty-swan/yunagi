//! 画面と共有する型の TypeScript の定義（`src/lib/bindings.ts`）を作り、置いてあるものと
//! 同じかを確かめる（`pnpm check` で走る）。
//!
//! 型（`crate::types`）やイベントの名前を変えたら、`UPDATE_BINDINGS=1 cargo test -p yunagi-app bindings`
//! で作り直す（作り直したファイルも確かめる）。

use std::path::Path;

use ts_rs::{Config, TS};

use crate::types::*;

/// 作り直すときに設定する環境変数。
const UPDATE_ENV: &str = "UPDATE_BINDINGS";

/// 置き場所（リポジトリ直下からの相対）。
const RELATIVE_PATH: &str = "src/lib/bindings.ts";

/// 1つの型の定義（説明があれば付ける）。
fn decl<T: TS>(cfg: &Config) -> String {
    let docs = T::docs().unwrap_or_default();
    format!("{docs}export {}\n", T::decl(cfg))
}

/// ファイル全体。
fn render() -> String {
    let cfg = Config::new();
    let decls = [
        decl::<Id>(&cfg),
        decl::<Date>(&cfg),
        decl::<Millis>(&cfg),
        decl::<ItemKind>(&cfg),
        decl::<ItemStatus>(&cfg),
        decl::<CloseReason>(&cfg),
        decl::<Origin>(&cfg),
        decl::<ItemField>(&cfg),
        decl::<TextDoc>(&cfg),
        decl::<Item>(&cfg),
        decl::<Position>(&cfg),
        decl::<ItemOutcome>(&cfg),
        decl::<SplitResult>(&cfg),
        decl::<Deleted>(&cfg),
        decl::<SourceState>(&cfg),
        decl::<CarriedTo>(&cfg),
        decl::<BoardItem>(&cfg),
        decl::<TimeHeading>(&cfg),
        decl::<NoteOutline>(&cfg),
        decl::<Board>(&cfg),
        decl::<Snapshot>(&cfg),
        decl::<TextSave>(&cfg),
        decl::<UiReport>(&cfg),
        decl::<EditingItem>(&cfg),
        decl::<EditorLines>(&cfg),
        decl::<Startup>(&cfg),
        decl::<StartupFailure>(&cfg),
        decl::<Changed>(&cfg),
        decl::<ChangeSource>(&cfg),
        decl::<TodayChanged>(&cfg),
        decl::<BackupFailed>(&cfg),
        decl::<CommandError>(&cfg),
        decl::<InvalidReason>(&cfg),
        decl::<Conflict>(&cfg),
        decl::<RestoreRefused>(&cfg),
    ];
    let events = format!(
        "/** コアが出すイベントの名前（src-tauri/src/lib.rs）。 */\n\
         export const EVENTS = {{\n  \
         startup: \"{}\",\n  \
         changed: \"{}\",\n  \
         todayChanged: \"{}\",\n  \
         backupFailed: \"{}\",\n\
         }} as const;\n",
        crate::EVENT_STARTUP,
        crate::EVENT_CHANGED,
        crate::EVENT_TODAY_CHANGED,
        crate::EVENT_BACKUP_FAILED,
    );
    format!(
        "// このファイルは src-tauri/src/bindings.rs が Rust の型から作る。手で書き換えない。\n\
         // 作り直すには：UPDATE_BINDINGS=1 cargo test -p yunagi-app bindings\n\n{}\n{events}",
        decls.join("\n")
    )
}

#[test]
fn bindings_are_up_to_date() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(RELATIVE_PATH);
    let expected = render();
    if std::env::var_os(UPDATE_ENV).is_some() {
        std::fs::write(&path, &expected).unwrap();
    }
    let actual = std::fs::read_to_string(&path)
        .unwrap_or_default()
        // Windows で改行が CRLF になって取り出されても同じとみなす
        .replace("\r\n", "\n");
    assert!(
        actual == expected,
        "{RELATIVE_PATH} が Rust の型と食い違っている。{UPDATE_ENV}=1 cargo test -p yunagi-app bindings で作り直す"
    );
}
