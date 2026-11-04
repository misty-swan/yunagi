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
use yunagi_core::domain::{archive, board, carryover, export, item, note_send, settings, text_doc};
use yunagi_core::storage::cloud_sync;
use yunagi_core::storage::search::{self as storage_search, SearchQuery};

use crate::app_core::{AppCore, Changes, ScreenWrite, now, system_tz};
use crate::types::{
    Archive, BackupEntry, Board, CarryoverConfirmed, CarryoverDecision, CarryoverList,
    CarryoverStatus, CloudService, CommandError, DataDirChanged, Date, Deleted, Id, Item,
    ItemFields, ItemKind, ItemOutcome, NoteMark, Overview, Position, SearchHit, SendResult,
    Settings, SettingsPatch, Snapshot, SplitResult, Startup, TextDoc, TextSave, UiReport,
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

/// 画面の入力中の状態を知らせる（変わるたびに全体を知らせ直す。アーキテクチャ設計書 4.1）。
#[tauri::command]
pub async fn report_ui_state(core: Core<'_>, state: UiReport) -> Result<(), CommandError> {
    blocking(&core, move |core| core.screen().report(state)).await
}
