//! 画面から呼ぶ Tauri コマンド（アーキテクチャ設計書 2章）。
//!
//! どれも非同期のコマンドにして、データベースの読み書きは止めてよいスレッド
//! （`spawn_blocking`）で行う。Tauri は同期のコマンドを画面のスレッドで動かすため。
//! 引数の名前は、画面からは camelCase で渡す（`baseRev` → `base_rev`。Tauri の決まり）。
//! 型は `crate::types`（画面の `src/lib/bindings.ts`）。

// コアのエラー（`ItemError` など）は大きいが、1回の呼び出しで1度返すだけなので、箱に入れても得が無い
#![allow(clippy::result_large_err)]

use std::path::Path;
use std::sync::Arc;

use tauri::State;
use yunagi_core::domain::{board, export, item, note_send, text_doc};

use crate::app_core::{AppCore, Changes, ScreenWrite, now, system_tz};
use crate::types::{
    BackupEntry, Board, CommandError, Date, Deleted, Id, Item, ItemFields, ItemKind, ItemOutcome,
    NoteMark, Overview, Position, SendResult, Snapshot, SplitResult, Startup, TextDoc, TextSave,
    UiReport,
};

type Core<'a> = State<'a, Arc<AppCore>>;

/// `f` を、止めてよいスレッドで動かす。
async fn blocking<T: Send + 'static>(
    core: &Core<'_>,
    f: impl FnOnce(&Arc<AppCore>) -> Result<T, CommandError> + Send + 'static,
) -> Result<T, CommandError> {
    let core = Arc::clone(core.inner());
    tauri::async_runtime::spawn_blocking(move || f(&core))
        .await
        .map_err(|e| CommandError::Internal {
            detail: e.to_string(),
        })?
}

/// データベースを開く処理の状態。画面は、`startup` イベントを聞き始めてからこれを呼ぶ
/// （聞き始める前に開き終わっても取りこぼさないため）。
#[tauri::command]
pub async fn startup_status(core: Core<'_>) -> Result<Startup, CommandError> {
    blocking(&core, |core| Ok(core.startup())).await
}

/// データベースが見つからなかったときに、新しく作って開く（`Startup::NotFound` のときだけ）。
#[tauri::command]
pub async fn create_database(core: Core<'_>) -> Result<Startup, CommandError> {
    blocking(&core, |core| core.create_database()).await
}

/// 見つからなかった・開けなかったときに、もう一度開く。
#[tauri::command]
pub async fn retry_open(core: Core<'_>) -> Result<Startup, CommandError> {
    blocking(&core, |core| core.retry_open()).await
}

/// 見つからなかったときに、データベースのある既存のフォルダ `dir` を保存先として記録してから開く
/// （保存先の選び直し）。
#[tauri::command]
pub async fn choose_data_dir(core: Core<'_>, dir: String) -> Result<Startup, CommandError> {
    blocking(&core, move |core| core.choose_data_dir(Path::new(&dir))).await
}

/// 見つからなかった・壊れていて開けなかったときに、バックアップ `backup`（`list_backups` の
/// `path`）で置き換えて開く。
#[tauri::command]
pub async fn restore_at_startup(core: Core<'_>, backup: String) -> Result<Startup, CommandError> {
    blocking(&core, move |core| {
        core.restore_at_startup(Path::new(&backup))
    })
    .await
}

/// バックアップから戻した結果（`Startup::Restored`）を、利用者が確かめた。
#[tauri::command]
pub async fn acknowledge_restore(core: Core<'_>) -> Result<Startup, CommandError> {
    blocking(&core, |core| core.acknowledge_restore()).await
}

/// 保存先の `backups/` にあるバックアップの一覧（新しいものが先）。
#[tauri::command]
pub async fn list_backups(core: Core<'_>) -> Result<Vec<BackupEntry>, CommandError> {
    blocking(&core, |core| core.backups()).await
}

/// 今日の論理日付。変わると `today-changed` イベントが届く。
#[tauri::command]
pub async fn today(core: Core<'_>) -> Result<Date, CommandError> {
    blocking(&core, |core| core.today().map(Date)).await
}

/// その日のボード。
#[tauri::command]
pub async fn get_board(core: Core<'_>, date: Date) -> Result<Board, CommandError> {
    blocking(&core, move |core| {
        core.read(|conn| board::get_screen_board(conn, date.0).map(Board::from))
    })
    .await
}

/// 画面の骨組み（サイドバー・上部のバー・凪メーター）に出す件数：表示している日 `date` の件数、
/// 今日と Recent の日の件数、エージェントによる未確認の変更の数。
#[tauri::command]
pub async fn get_overview(core: Core<'_>, date: Date) -> Result<Overview, CommandError> {
    blocking(&core, move |core| {
        let today = core.today()?;
        core.read(|conn| board::overview(conn, date.0, today).map(Overview::new))
    })
    .await
}

/// 項目を足す（`origin = human`）。
#[tauri::command]
pub async fn add_item(
    core: Core<'_>,
    date: Date,
    kind: ItemKind,
    text: String,
    position: Position,
) -> Result<Item, CommandError> {
    blocking(&core, move |core| {
        core.screen_write(ScreenWrite::Save, |conn| {
            item::add(
                conn,
                date.0,
                kind.into(),
                &text,
                position.into(),
                &system_tz(),
                now(),
            )
            .map(Item::from)
        })
    })
    .await
}

/// 項目を保存する。`fields` は利用者が変えた欄だけ（無い欄には触れない。アーキテクチャ設計書 4.1
/// 「項目の保存」）。状態を変えたときは、利用者とエージェントが同じ状態にした行き違いも判定する。
#[tauri::command]
pub async fn save_item(
    core: Core<'_>,
    id: Id,
    fields: ItemFields,
) -> Result<ItemOutcome, CommandError> {
    blocking(&core, move |core| {
        let fields = fields.into();
        core.screen_write(ScreenWrite::Save, |conn| {
            item::save(conn, id.0, &fields, now()).map(ItemOutcome::from)
        })
    })
    .await
}

/// 項目を削除する。結果は削除の取り消し（`restore_item`）に渡す。
#[tauri::command]
pub async fn delete_item(core: Core<'_>, id: Id) -> Result<Deleted, CommandError> {
    blocking(&core, move |core| {
        core.screen_write(ScreenWrite::Operation, |conn| {
            item::delete(conn, id.0, now()).map(Deleted::from)
        })
    })
    .await
}

/// 削除を取り消す（Mod+Z）。`deleted` は `delete_item` の結果。
#[tauri::command]
pub async fn restore_item(core: Core<'_>, deleted: Deleted) -> Result<Item, CommandError> {
    blocking(&core, move |core| {
        let source = deleted.source.map(|s| s.into_core());
        core.screen_write(ScreenWrite::Operation, |conn| {
            item::restore(conn, deleted.id.0, source.as_ref(), now()).map(Item::from)
        })
    })
    .await
}

/// 項目を同じ日の並びの中で動かす。
#[tauri::command]
pub async fn move_item(
    core: Core<'_>,
    id: Id,
    position: Position,
) -> Result<ItemOutcome, CommandError> {
    blocking(&core, move |core| {
        core.screen_write(ScreenWrite::Operation, |conn| {
            item::move_item(conn, id.0, position.into(), now()).map(ItemOutcome::from)
        })
    })
    .await
}

/// 項目を2つに分ける（`first` を元の項目の本文に、`second` を新しい項目の本文にする）。
#[tauri::command]
pub async fn split_item(
    core: Core<'_>,
    id: Id,
    first: String,
    second: String,
) -> Result<SplitResult, CommandError> {
    blocking(&core, move |core| {
        core.screen_write(ScreenWrite::Save, |conn| {
            item::split(conn, id.0, &first, &second, now()).map(|(original, created)| SplitResult {
                original: original.into(),
                created: created.into(),
            })
        })
    })
    .await
}

/// その日のノートか振り返りの本文と版。
#[tauri::command]
pub async fn get_text_doc(
    core: Core<'_>,
    doc: TextDoc,
    date: Date,
) -> Result<Snapshot, CommandError> {
    blocking(&core, move |core| {
        core.read(|conn| text_doc::get(conn, doc.into(), date.0).map(Snapshot::from))
    })
    .await
}

/// ノートか振り返りを保存する。`baseRev` はもとにした版。古ければ書かずに最新を返す
/// （`TextSave::Stale`。アーキテクチャ設計書 4.1）。
#[tauri::command]
pub async fn save_text_doc(
    core: Core<'_>,
    doc: TextDoc,
    date: Date,
    body: String,
    base_rev: i64,
) -> Result<TextSave, CommandError> {
    blocking(&core, move |core| {
        core.screen_write(ScreenWrite::Save, |conn| {
            match text_doc::save(
                conn,
                doc.into(),
                date.0,
                &body,
                base_rev,
                &system_tz(),
                now(),
            ) {
                Ok(w) => Ok(TextSave::Saved {
                    rev: w.rev,
                    updated: w.updated,
                }),
                Err(text_doc::TextDocError::Stale { latest }) => Ok(TextSave::Stale {
                    body: latest.body,
                    rev: latest.rev,
                }),
                Err(e) => Err(CommandError::from(e)),
            }
        })
    })
    .await
}

/// 行送りの結果と、データを変えたか（本文を書いたか、項目を作ったか）。
struct SendOutcome {
    result: SendResult,
    changed: bool,
}

impl Changes for SendOutcome {
    fn changed(&self) -> bool {
        self.changed
    }
}

/// ノートの `from` 行目から `to` 行目まで（1始まり。両端を含む）を、1行ごとに `kind` の項目として
/// 送る（Mod+Shift+T・Mod+Shift+B。操作仕様書 2章）。`body` はエディタの本文で、`baseRev` はその
/// もとにした版。コアは本文を保存し、同じトランザクションで項目と送った記録を作る。版が古ければ
/// 何も書かずに最新を返す（`SendResult::Stale`。画面は取り込んでから送り直す）。行の番号の誤りは
/// `invalid_argument`（`note_line`）。
///
/// 再試行を待つ入力の保存ではない（失敗しても画面は送り直さず、本文はふだんの自動保存が送る）ので、
/// 保存の失敗中（アーキテクチャ設計書 4.1）を始めも終えもしない。
#[tauri::command]
pub async fn send_note_lines(
    core: Core<'_>,
    date: Date,
    body: String,
    base_rev: i64,
    from: usize,
    to: usize,
    kind: ItemKind,
) -> Result<SendResult, CommandError> {
    blocking(&core, move |core| {
        core.screen_write(ScreenWrite::Operation, |conn| {
            let request = note_send::SendLines {
                date: date.0,
                body: &body,
                base_rev,
                lines: from..=to,
                kind: kind.into(),
            };
            match note_send::send(conn, &request, &system_tz(), now()) {
                Ok(sent) => Ok(SendOutcome {
                    changed: !sent.sent.is_empty() || sent.rev != base_rev,
                    result: sent.into(),
                }),
                Err(note_send::SendError::Stale { latest }) => Ok(SendOutcome {
                    result: SendResult::Stale {
                        body: latest.body,
                        rev: latest.rev,
                    },
                    changed: false,
                }),
                Err(e) => Err(CommandError::from(e)),
            }
        })
        .map(|o| o.result)
    })
    .await
}

/// その日のノートの本文 `body`（エディタの本文。まだ保存していない入力を含んでよい）で、送信済みの
/// 印を付ける行（行の順。データモデル設計書 3章 note_send の照合）。
#[tauri::command]
pub async fn note_marks(
    core: Core<'_>,
    date: Date,
    body: String,
) -> Result<Vec<NoteMark>, CommandError> {
    blocking(&core, move |core| {
        core.read(|conn| {
            note_send::marks(conn, date.0, &body)
                .map(|marks| marks.into_iter().map(NoteMark::from).collect())
        })
    })
    .await
}

/// `from` から `to` まで（両端を含む。逆でもよい）の日ごとの Markdown を、フォルダ `dir`（利用者が
/// 選んだ、ある絶対パスのフォルダ）に `YYYY-MM-DD.md` として書き出し、書いたファイルのパスを日付の順に
/// 返す（アーキテクチャ設計書 6.4）。書き出すものが無い日は書かない（何も無ければ空）。
///
/// 書けなかったとき（フォルダが無い・絶対パスでない・書き込めないなど）は `Storage`（`detail` に
/// ファイルのパス・置き換え終えたファイルの数・原因）。読み取りと同じく、保存の失敗中（アーキテクチャ
/// 設計書 4.1）には数えない。書き出し先の誤りを別の種類にするかは、画面を作るときに決める（画面設計書
/// 11章）。
#[tauri::command]
pub async fn export_markdown(
    core: Core<'_>,
    from: Date,
    to: Date,
    dir: String,
) -> Result<Vec<String>, CommandError> {
    blocking(&core, move |core| {
        // データベースを使うのは読む間だけにし、文字列にしてファイルを書く間はほかのコマンドを待たせない
        let boards = core.read(|conn| export::read_between(conn, from.0, to.0))?;
        let paths = export::write_files(Path::new(&dir), boards.into_days())
            .map_err(|e| CommandError::storage(&e))?;
        Ok(paths.iter().map(|p| p.display().to_string()).collect())
    })
    .await
}

/// 画面の入力中の状態を知らせる（変わるたびに全体を知らせ直す。アーキテクチャ設計書 4.1）。
#[tauri::command]
pub async fn report_ui_state(core: Core<'_>, state: UiReport) -> Result<(), CommandError> {
    blocking(&core, move |core| core.screen().report(state)).await
}
