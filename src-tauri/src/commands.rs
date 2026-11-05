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
use yunagi_core::domain::{
    agent_admin, archive, board, carryover, export, handoff, history, item, note_send, settings,
    text_doc,
};
use yunagi_core::storage::cloud_sync;
use yunagi_core::storage::search::{self as storage_search, SearchQuery};

use crate::app_core::{AppCore, Changes, ScreenWrite, now, system_tz};
use crate::clipboard::HandoffClipboard;
use crate::global_shortcut::GlobalShortcutController;
use crate::ime::ImeWorker;
use crate::types::{
    AgentRow, AgentScope, ApiStatus, Archive, BackupEntry, Board, CarryoverConfirmed,
    CarryoverDecision, CarryoverList, CarryoverStatus, ChangeEntry, ChangeList, CloudService,
    CommandError, DataDirChanged, Date, Deleted, GlobalShortcutStatus, Id, ImeStatus, Item,
    ItemFields, ItemKind, ItemOutcome, NoteMark, Overview, Position, RevertPlan, Reverted,
    ScopesChanged, SearchHit, SendResult, Settings, SettingsPatch, Snapshot, SplitResult, Startup,
    TextDoc, TextSave, UiReport,
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

/// ふりかえり（S-08）の内容：選んだ日 `date`（無ければ、今日より前で記録のある最も新しい日。今日より
/// 後は今日にする）を含む週の件数、記録のある直近の日、選んだ日のプレビュー。
#[tauri::command]
pub async fn get_archive(core: Core<'_>, date: Option<Date>) -> Result<Archive, CommandError> {
    blocking(&core, move |core| {
        let today = core.today()?;
        core.read(|conn| archive::get_archive(conn, date.map(|d| d.0), today).map(Archive::from))
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

    fn committed(&self) -> bool {
        matches!(self.result, SendResult::Sent { .. })
    }
}

/// ノートの `from` 行目から `to` 行目まで（1始まり。両端を含む）を、1行ごとに `kind` の項目として
/// 送る（Mod+Shift+T・Mod+Shift+B。操作仕様書 2章）。`body` はエディタの本文で、`baseRev` はその
/// もとにした版。コアは本文を保存し、同じトランザクションで項目と送った記録を作る。版が古ければ
/// 何も書かずに最新を返す（`SendResult::Stale`。画面は取り込んでから送り直す）。行の番号の誤りは
/// `invalid_argument`（`note_line`）。
///
/// エディタの本文の保存を兼ねるので、ノートの保存と同じく保存として数える（アーキテクチャ設計書 4.1
/// 「保存に失敗している間」）：書けたら保存の失敗中を終え、保存先の問題で失敗したら始める（行送りは
/// 送り直さないが、本文はふだんの自動保存が内容を持ったまま再試行する）。
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
        core.screen_write(ScreenWrite::Save, |conn| {
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
/// 書き出し先に書けなかったとき（フォルダが無い・絶対パスでない・書き込めないなど）は `ExportFailed`
/// （置き換え終えたファイルの数 `replaced` と、ログ用の `detail`）。データベースの読み取りの失敗は
/// `Storage`。どちらも、保存の失敗中（アーキテクチャ設計書 4.1）には数えない（2026-11-04 決定。
/// アーキテクチャ設計書 4.3・6.4）。
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
        let paths = export::write_files(Path::new(&dir), boards.into_days())?;
        Ok(paths.iter().map(|p| p.display().to_string()).collect())
    })
    .await
}

/// 設定（既定値を当てた後のもの。画面設計書 7章）。起動のとき、外枠を出す前に読む。
#[tauri::command]
pub async fn get_settings(core: Core<'_>) -> Result<Settings, CommandError> {
    blocking(&core, |core| {
        core.read(|conn| settings::load(conn).map(Settings::from))
    })
    .await
}

/// 設定の保存結果と、変わったか（変わったら画面に知らせる）。
struct SettingsSaved {
    settings: Settings,
    changed: bool,
}

impl Changes for SettingsSaved {
    /// 設定は項目・ノートなどのデータではないので、`changed` は出さない（画面は設定を自分で当てる。
    /// 出すと、ボードと件数を読み直し、入力中の画面を乱す）。
    fn changed(&self) -> bool {
        false
    }
}

/// 設定を保存する。`patch` は変える欄だけ。誤った値（凪メーターの始業が終業より前でない、フォント
/// サイズが 13〜19 でない、見直しの回数が 1 未満、`HH:MM` でない時刻）は、何も書かずに
/// `invalid_argument`。保存した後の設定を返す。
///
/// 設定の保存は、保存の失敗中（アーキテクチャ設計書 4.1）を始め・終えない（再試行を待つ入力では
/// なく、失敗は画面に出して選び直すため）。日付の切り替わり時刻を変えたときは、すぐに今日を求め直す。
#[tauri::command]
pub async fn save_settings(core: Core<'_>, patch: SettingsPatch) -> Result<Settings, CommandError> {
    blocking(&core, move |core| {
        let moves_today = patch.day_boundary.is_some();
        let saved = core.screen_write(ScreenWrite::Operation, |conn| {
            // 片方だけの始業・終業にもう片方の今の値を当てるのは、書く処理と同じ接続の中で行う
            // （続けて届いた保存が、古い値と組にならないように。接続は1つで順に使う）
            let patch = patch.clone().into_core(&settings::load(conn)?)?;
            let changed = settings::save(conn, &patch)?;
            Ok::<_, CommandError>(SettingsSaved {
                settings: Settings::from(settings::load(conn)?),
                changed,
            })
        })?;
        if moves_today && saved.changed {
            core.check_today(now());
        }
        Ok(saved.settings)
    })
    .await
}

/// いまの保存先のフォルダ（設定画面のデータ区分に示す）。
#[tauri::command]
pub async fn get_data_dir(core: Core<'_>) -> Result<String, CommandError> {
    blocking(&core, |core| core.data_dir()).await
}

/// 保存先の候補 `dir` がクラウド同期フォルダ（の中）と判定できるか（画面設計書 7章。警告を出すため）。
/// 判定できなければ `null`。
#[tauri::command]
pub async fn check_data_dir(
    core: Core<'_>,
    dir: String,
) -> Result<Option<CloudService>, CommandError> {
    blocking(&core, move |_| {
        Ok(cloud_sync::detect_cloud_sync(Path::new(&dir)).map(CloudService::from))
    })
    .await
}

/// 保存先を `dir`（絶対パスのフォルダ）に変える：データベースと `backups/` を移し、`config.json` に
/// 記録する（アーキテクチャ設計書 6章）。使えないときは何も変えずに `rejected`。移した後に消せなかった
/// 元のファイルは `leftovers` で返す。
#[tauri::command]
pub async fn change_data_dir(core: Core<'_>, dir: String) -> Result<DataDirChanged, CommandError> {
    blocking(&core, move |core| {
        let leftovers = core.change_data_dir(Path::new(&dir))?;
        Ok(DataDirChanged {
            leftovers: leftovers.iter().map(|p| p.display().to_string()).collect(),
        })
    })
    .await
}

/// データベースのある既存のフォルダ `dir` を、データを移さずに保存先として記録して開き直す。使えない
/// ときは何も変えずに `rejected`。開くデータベースが別のものになるので、画面は読み込み直す。
#[tauri::command]
pub async fn use_data_dir(core: Core<'_>, dir: String) -> Result<(), CommandError> {
    blocking(&core, move |core| core.use_data_dir(Path::new(&dir))).await
}

/// バックアップ `backup`（`list_backups` の `path`）から復元する。今のデータは復元前のバックアップに
/// 退避する。使えないバックアップは何も変えずに `rejected`。画面は、復元の後に読み込み直す。
#[tauri::command]
pub async fn restore_backup(core: Core<'_>, backup: String) -> Result<(), CommandError> {
    blocking(&core, move |core| core.restore_backup(Path::new(&backup))).await
}

/// 検索の件数の上限の上限（画面が大きな値を渡しても、ノートの本文をまとめて読み過ぎないため）。
pub const MAX_SEARCH_LIMIT: u32 = 200;

/// 全日付の TODO・障害・ノート・振り返りを `text` で探す（データモデル設計書 4.1。パレット S-04、
/// 画面設計書 6章）。並びは日付の新しい順で、`limit` 件まで（[`MAX_SEARCH_LIMIT`] を超える値はそこまで）。
/// 一致の決まり（前後の空白を除いて全体を部分一致・空なら空）はコアの `storage::search` に従う。
#[tauri::command]
pub async fn search(
    core: Core<'_>,
    text: String,
    limit: u32,
) -> Result<Vec<SearchHit>, CommandError> {
    blocking(&core, move |core| {
        let query = SearchQuery {
            text: &text,
            from: None,
            to: None,
            kinds: &[],
            limit: limit.min(MAX_SEARCH_LIMIT) as usize,
        };
        core.read(|conn| {
            storage_search::search(conn, &query)
                .map(|hits| hits.into_iter().map(SearchHit::from).collect())
        })
    })
    .await
}

/// 選んだ TODO `id` の依頼文を作り、クリップボードに書く（Mod+Shift+A。操作仕様書 2章、MCP・CLI 仕様書
/// 5章）。画面の操作なので、権限は確かめない。何も変えず、change_log にも記録しない。保存の失敗中でも
/// 使える（読み取りだけ）。クリップボードに書けなかったら `clipboard_failed`、項目が無ければ `not_found`、
/// 障害の ID なら `invalid_argument`（`wrong_kind`）。
///
/// 依頼文を作るのとクリップボードへの書き込みを 1 つのコマンドにして Rust 側で行う（`crate::clipboard`）。
#[tauri::command]
pub async fn copy_handoff(
    core: Core<'_>,
    clipboard: State<'_, Arc<HandoffClipboard>>,
    id: Id,
) -> Result<(), CommandError> {
    let markdown = blocking(&core, move |core| {
        core.read(|conn| handoff::build(conn, id.0, &system_tz(), now()).map(|h| h.markdown))
    })
    .await?;
    let clipboard = Arc::clone(&clipboard);
    // OS の応答を待つことがあるので、止めてよいスレッドで書く
    tauri::async_runtime::spawn_blocking(move || clipboard.write(&markdown))
        .await
        .map_err(|e| CommandError::Internal {
            detail: e.to_string(),
        })?
}

/// 持ち越しの後回し・再表示の記録の結果。`changed` は書いたか（ほかの窓が持ち越しの状態を読み直す
/// ように、書いたら知らせる）。
struct CarryoverMarked<T> {
    value: T,
    changed: bool,
}

impl<T> Changes for CarryoverMarked<T> {
    fn changed(&self) -> bool {
        self.changed
    }
}

/// 今日の持ち越しの状態（画面設計書 5章）。持ち越しを終えたか、前回の日に持ち越す項目が無ければ
/// `null`（画面を出さず、パレットの「持ち越しを開く」も出さない）。
#[tauri::command]
pub async fn carryover_status(core: Core<'_>) -> Result<Option<CarryoverStatus>, CommandError> {
    blocking(&core, |core| {
        let today = core.today()?;
        core.read(|conn| carryover::status(conn, today).map(|s| s.map(CarryoverStatus::from)))
    })
    .await
}

/// 持ち越しの画面に出す項目（前回の日の、まだ持ち越し済みでない未完了の TODO と未解消の障害）。
/// 前回の日が無ければ `null`。
#[tauri::command]
pub async fn carryover_candidates(core: Core<'_>) -> Result<Option<CarryoverList>, CommandError> {
    blocking(&core, |core| {
        let today = core.today()?;
        core.read(|conn| carryover::candidates(conn, today).map(|c| c.map(CarryoverList::from)))
    })
    .await
}

/// 持ち越しを後回しにした（Esc）ことを記録する。すでに後回しにしていれば最初の時刻のまま、持ち越しを
/// 終えていれば何もしない。
///
/// 持ち越しの書き込み（この3つ）は、保存の失敗中（アーキテクチャ設計書 4.1）を始め・終えない
/// （画面は内容を持ったまま再試行せず、失敗を画面に出して利用者が選び直すため）。
#[tauri::command]
pub async fn postpone_carryover(core: Core<'_>) -> Result<(), CommandError> {
    blocking(&core, |core| {
        let today = core.today()?;
        // 最初の時刻のまま変えないときも、書いたかをコアは返さないので、知らせる（読み直すだけで害は無い）
        core.screen_write(ScreenWrite::Operation, |conn| {
            carryover::postpone(conn, today, &system_tz(), now()).map(|()| CarryoverMarked {
                value: (),
                changed: true,
            })
        })
        .map(|r| r.value)
    })
    .await
}

/// 後回しにした持ち越しを、前面に出したときに自動で再表示したことを記録する。記録したら `true`
/// （画面はそのときだけ出す）。後回しにしていない・再表示を記録済み・持ち越しを終えていれば `false`。
#[tauri::command]
pub async fn mark_carryover_reshown(core: Core<'_>) -> Result<bool, CommandError> {
    blocking(&core, |core| {
        let today = core.today()?;
        core.screen_write(ScreenWrite::Operation, |conn| {
            carryover::mark_reshown(conn, today, now()).map(|written| CarryoverMarked {
                value: written,
                changed: written,
            })
        })
        .map(|r| r.value)
    })
    .await
}

/// 持ち越しを確定する（画面設計書 5章）。確定の時点で持ち越しの対象のものだけを扱う。
/// 画面に出していた前回の日が今と違えば `conflict`（`previous_day_changed`）、今日の持ち越しを
/// 終えていれば `conflict`（`carryover_done`）で、どちらも何も書かない。
#[tauri::command]
pub async fn confirm_carryover(
    core: Core<'_>,
    decision: CarryoverDecision,
) -> Result<CarryoverConfirmed, CommandError> {
    blocking(&core, move |core| {
        let today = core.today()?;
        let decision = decision.into();
        core.screen_write(ScreenWrite::Operation, |conn| {
            carryover::confirm(conn, today, &decision, &system_tz(), now())
                .map(CarryoverConfirmed::from)
        })
    })
    .await
}

/// この環境で IME の切替ができるか（設定画面の制約の表示。操作仕様書 4.4 の 4）。コマンドを呼んで調べる
/// ので、止めてよいスレッドで行う。
#[tauri::command]
pub async fn ime_status() -> Result<ImeStatus, CommandError> {
    tauri::async_runtime::spawn_blocking(crate::ime::status)
        .await
        .map_err(|e| CommandError::Internal {
            detail: e.to_string(),
        })
}

/// OS 全体のショートカット（Yunagi を前面に呼び出す）の割当を `binding` に変えて OS に登録し直す。
/// `None` は登録を外す（割当が使えないとき）。登録できなかった理由は、返す状態で知らせる（失敗にしない。
/// 操作仕様書 1章「割当の変更」）。プラグインの呼び出しはメインスレッドの完了を待つので、止めてよい
/// スレッドで行う。
#[tauri::command]
pub async fn set_global_shortcut(
    shortcut: State<'_, Arc<GlobalShortcutController>>,
    binding: Option<String>,
) -> Result<GlobalShortcutStatus, CommandError> {
    let shortcut = Arc::clone(&shortcut);
    tauri::async_runtime::spawn_blocking(move || {
        shortcut.set(binding.as_deref(), cfg!(target_os = "macos"))
    })
    .await
    .map_err(|e| CommandError::Internal {
        detail: e.to_string(),
    })
}

/// ノートモードで前面に出す頼み（`yunagi show --note`）が、まだ渡っていなければ受け取る（`true`）。
/// 画面が準備できたとき（起動した直後は、頼みが画面の準備より先に届く）と、`show-requested` を受けたときに
/// 呼ぶ。受け取ると忘れる。
#[tauri::command]
pub fn take_show_note(core: State<'_, Arc<AppCore>>) -> bool {
    // コアを持たせる前に届いた頼み（`crate::EARLY_SHOW_NOTE`）も受け取る
    let early = crate::EARLY_SHOW_NOTE.swap(false, std::sync::atomic::Ordering::AcqRel);
    core.take_show_note() || early
}

/// Vim の挿入モードを抜けた：IME を英数入力に切り替える（操作仕様書 4.4 の 1）。切替は専用のスレッドが
/// 順に行うので、待たずに返る。切り替えられなくても失敗にしない（ベストエフォート）。
///
/// この 2 つのコマンドだけは同期のコマンドにする（画面のスレッドで、届いた順に動く）。非同期のコマンドは
/// 別々のスレッドで動き、Esc の直後の `i` で、入ったことの知らせが抜けたことの知らせを追い越しうるため。
/// 中身は専用のスレッドへ知らせを送るだけで、待たない。
#[tauri::command]
pub fn ime_leave_insert(ime: State<'_, ImeWorker>) -> Result<(), CommandError> {
    ime.leave_insert();
    Ok(())
}

/// Vim の挿入モードに入った：覚えていた入力状態を捨て、`restore`（設定 `vim_ime_restore`）なら抜けた
/// 時点の入力状態に戻す（操作仕様書 4.4 の 3）。
#[tauri::command]
pub fn ime_enter_insert(ime: State<'_, ImeWorker>, restore: bool) -> Result<(), CommandError> {
    ime.enter_insert(restore);
    Ok(())
}

// ---- エージェントの管理（設定のエージェント区分・変更履歴 S-06。MCP・CLI 仕様書 4.4・7章） ----
//
// ローカル API の管理の操作と同じコア（`domain::agent_admin`・`domain::history`）を、画面向けの型で
// 呼ぶ。確認は画面が行い、ここは確認を済ませた後の変更を行う。呼び出し元のスコープは確かめない（画面は
// アプリ自身の画面だけで、外部の内容は読み込まない。CSP は `tauri.conf.json`。外から呼べるのは、トークンで守る
// ローカル API の管理の操作で、こちらではない）。
// 書き込みは `AppCore::admin_write`（画面の入力中・保存の失敗中を取り消しに見せ、変えたら `changed`）。

/// 登録されているエージェント（クライアント）と、許可した権限・最後に接続した時刻・消せるか
/// （登録した順。固定の `cli`・`unknown` が先）。
#[tauri::command]
pub async fn list_agents(core: Core<'_>) -> Result<Vec<AgentRow>, CommandError> {
    blocking(&core, |core| {
        core.read(|conn| {
            agent_admin::list_entries(conn)
                .map(|entries| entries.into_iter().map(AgentRow::from).collect())
        })
    })
    .await
}

/// エージェント `client` の権限を、`allow` を足し、`deny` を外して変える。`expect` は確認の画面で見せた
/// 変更前の権限（いまと違えば、書かずに `conflict`（`scopes_changed`））。足すものも外すものも無い・
/// 同じものを両方に指定するのは画面の不具合（`internal`）。`unknown` に `read` 以外は足せない
/// （`invalid_argument`）。変えたら `changed`（`by: agent`）を知らせる。
#[tauri::command]
pub async fn set_agent_scopes(
    core: Core<'_>,
    client: String,
    allow: Vec<AgentScope>,
    deny: Vec<AgentScope>,
    expect: Vec<AgentScope>,
) -> Result<ScopesChanged, CommandError> {
    blocking(&core, move |core| {
        let allow: Vec<_> = allow.into_iter().map(Into::into).collect();
        let deny: Vec<_> = deny.into_iter().map(Into::into).collect();
        let expect: Vec<_> = expect.into_iter().map(Into::into).collect();
        core.admin_write(|conn, _| {
            agent_admin::change_scopes(conn, &client, &allow, &deny, Some(&expect)).map(|c| {
                ScopesChanged {
                    updated: c.updated,
                    scopes: c.client.scopes.into_iter().map(Into::into).collect(),
                }
            })
        })
    })
    .await
}

/// 消した登録（結果は値を持たない）。
struct Removed;

impl Changes for Removed {
    fn changed(&self) -> bool {
        true
    }
}

/// 登録を消す（登録の数の上限に達したときの整理。MCP・CLI 仕様書 9章）。消せるのは、固定の
/// `cli`・`unknown` ではなく、変更履歴にも項目にも指されていない登録だけ（`invalid_argument` の
/// `fixed_client`、`conflict` の `client_in_use`）。画面だけの口で、ローカル API・CLI には出さない。
#[tauri::command]
pub async fn remove_agent(core: Core<'_>, client: String) -> Result<(), CommandError> {
    blocking(&core, move |core| {
        core.admin_write(|conn, _| agent_admin::remove_client(conn, &client).map(|()| Removed))
            .map(|Removed| ())
    })
    .await
}

/// ローカル API の状態（動作中か・始められなかった理由の種類・処理中の接続数。アーキテクチャ設計書 4.6）。
/// データベースを開いていなくても答える。
#[tauri::command]
pub async fn api_status(core: Core<'_>) -> Result<ApiStatus, CommandError> {
    blocking(&core, |core| Ok(core.api_state().into())).await
}

/// 変更履歴を新しい順に返す（MCP・CLI 仕様書 7.2）。確認済みの印は付けない。`client` はそのエージェントの
/// 記録だけ（登録が無ければ `not_found`）、`unseen_only` は未確認のものだけ、`before` はその記録より古い
/// ものだけ（続きを読む目印）。`limit` は 1 以上（200 を超えれば 200）。
#[tauri::command]
pub async fn list_changes(
    core: Core<'_>,
    client: Option<String>,
    unseen_only: bool,
    before: Option<Id>,
    limit: usize,
) -> Result<ChangeList, CommandError> {
    blocking(&core, move |core| {
        // 続きがあるかを決めるため、1 件多く読む（ちょうど `limit` 件で終わるときに、空の続きを出さない）。
        // コアの上限（200）で打ち切られたときは、読めた件数が `limit` と同じなら続きがあるものとする
        let filter = history::ChangeFilter {
            client,
            unseen_only,
            before: before.map(|i| i.0),
            limit: if limit == 0 {
                0
            } else {
                limit.saturating_add(1)
            },
        };
        core.read(|conn| {
            let mut entries = history::list_changes(conn, &filter)?;
            let shown = limit.min(history::MAX_LIMIT);
            let more =
                entries.len() > shown || (shown == history::MAX_LIMIT && entries.len() == shown);
            entries.truncate(shown);
            let next_before = more.then(|| entries.last().map(|e| Id(e.log.id))).flatten();
            Ok::<_, history::HistoryError>(ChangeList {
                changes: entries.iter().map(ChangeEntry::from).collect(),
                next_before,
            })
        })
    })
    .await
}

/// 変更の取り消しで何が起きるかを返す（何も書かない。MCP・CLI 仕様書 7.3）。確認の画面に見せる。
#[tauri::command]
pub async fn preview_revert(core: Core<'_>, change_id: Id) -> Result<RevertPlan, CommandError> {
    blocking(&core, move |core| {
        core.read(|conn| history::preview(conn, change_id.0).map(|p| RevertPlan::from(&p)))
    })
    .await
}

/// 変更を取り消す（MCP・CLI 仕様書 7.3。判定の順もそこ）。`expect` は `preview_revert` の `token`
/// （見せたときと状態が違えば、`force` でも書かずに `conflict`（`changed`、`afterPreview`））。`force` は
/// 人があとから変えた欄を、見せた値で上書きする。取り消したときと、すでに戻っていた記録を取り消し済みに
/// したときは `changed`（`by: agent`）を知らせる。
#[tauri::command]
pub async fn revert_change(
    core: Core<'_>,
    change_id: Id,
    force: bool,
    expect: String,
) -> Result<Reverted, CommandError> {
    blocking(&core, move |core| {
        core.admin_write(|conn, screen| {
            let request = history::RevertRequest {
                force,
                expect: Some(&expect),
            };
            history::revert(conn, change_id.0, &request, screen, &system_tz(), now()).map(|r| {
                Reverted {
                    updated: r.updated,
                    state: r.plan.state.into(),
                    marked: r.updated || r.plan.state == history::PlanState::AlreadyRestored,
                }
            })
        })
    })
    .await
}

/// 確認した件数（印を付けた記録の数）。付けた記録があれば、上部のバーの件数が変わる。
struct Marked(usize);

impl Changes for Marked {
    fn changed(&self) -> bool {
        self.0 > 0
    }
}

/// 変更履歴の画面で見せた記録 `ids` に、確認済みの印を付ける（MCP・CLI 仕様書 7.2）。見せていない記録には
/// 付けない（一覧を読んでから届いた変更を、見せずに確認済みにしないため）。付けた件数を返す。
#[tauri::command]
pub async fn mark_changes_seen(core: Core<'_>, ids: Vec<Id>) -> Result<usize, CommandError> {
    blocking(&core, move |core| {
        let ids: Vec<_> = ids.into_iter().map(|i| i.0).collect();
        core.admin_write(|conn, _| history::mark_seen(conn, &ids, now()).map(Marked))
            .map(|Marked(n)| n)
    })
    .await
}

/// 画面の入力中の状態を知らせる（変わるたびに全体を知らせ直す。アーキテクチャ設計書 4.1）。
#[tauri::command]
pub async fn report_ui_state(core: Core<'_>, state: UiReport) -> Result<(), CommandError> {
    blocking(&core, move |core| core.screen().report(state)).await
}
