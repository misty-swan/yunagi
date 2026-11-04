//! アプリの中のコアの入口（アーキテクチャ設計書 2章・4章）。
//!
//! - 起動時に `yunagi.lock` を取り、データベースを開く（4章 4、6.3）。開くまで（ロックを最大 5 秒
//!   待つ）画面を止めないよう、[`AppCore::start`] は別のスレッドで開く。
//! - 画面とエージェントの書き込みは、どちらもここを通り（[`AppCore::screen_write`]・
//!   [`AppCore::agent_write`]）、変えたら [`Notice::Changed`] を知らせる（4章 6）。
//! - 画面からの保存が失敗してから次に成功するまでを記録し、エージェントの書き込みの判定に使う
//!   （4.1「保存に失敗している間」）。入力中の欄・守る行は画面から受け取る（[`ScreenState`]）。
//! - 開いた直後と、論理日付が変わったときに、定期バックアップを別のスレッドで作る（6.2）。
//!
//! Tauri には依存しない（知らせ先は [`Notify`] で受け取る）。Tauri とのつなぎは `crate::run`。

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, TryLockError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use rusqlite::Connection;
use yunagi_core::domain::agent::Agent;
use yunagi_core::domain::day;
use yunagi_core::domain::item;
use yunagi_core::domain::logical_date::LogicalDate;
use yunagi_core::domain::text_doc;
use yunagi_core::error::ErrorChain;
use yunagi_core::storage::backup::{self, BackupError};
use yunagi_core::storage::data_dir::DataDirError;
use yunagi_core::storage::{
    DEFAULT_LOCK_WAIT, Database, DatabaseNotFound, DbLock, LockError, OpenError, RecoverError,
    StorageError, Ulid, data_dir,
};

use crate::api::{ApiOptions, ApiServer, ApiState, ApiUnavailable};
use crate::screen::ScreenState;
use crate::types::{
    BackupEntry, BackupFailed, ChangeSource, Changed, CommandError, ItemOutcome, RejectReason,
    ShowRequested, SplitResult, Startup, StartupFailure, TextSave, TodayChanged,
};

/// コアから画面への知らせ（Tauri のイベントになる。`crate::run`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// データベースを開く処理の状態が変わった（`startup`）。
    Startup(Startup),
    /// データが変わった（`changed`）。
    Changed(Changed),
    /// 今日の論理日付が変わった（`today-changed`）。
    TodayChanged(TodayChanged),
    /// 定期バックアップを作れなかった（`backup-failed`）。
    BackupFailed(BackupFailed),
    /// 窓を前面に出す頼み（ローカル API の `show_app`。`yunagi show`）。受けた側（`crate::run`）が、窓を
    /// 前面に出し（最小化を解き、表示し、フォーカスを移す）、画面に `show-requested` を知らせる。
    ShowRequested(ShowRequested),
}

/// 知らせ先。どのスレッドからも呼ばれる。
pub type Notify = Box<dyn Fn(Notice) + Send + Sync>;

/// 今日の論理日付が変わったかを確かめる間隔（[`AppCore::start`]）。切り替わりを知るのは最大でこれだけ
/// 遅れる。時計を見るだけでなく設定の切り替わり時刻も読み直すので、設定の変更にも追いつく。
pub const TODAY_CHECK_INTERVAL: Duration = Duration::from_secs(30);

/// ローカル API の依頼を、待っても処理できなかった（保存先の変更・復元の最中、またはデータベースを
/// 開いていない）。何も反映していない（CLI・MCP には `busy`。CLI・MCP 仕様書 4.3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Busy;

/// データベースの使用を待つ間に、取り直しを試みる間隔（[`AppCore::api_write`]）。
const DB_RETRY_INTERVAL: Duration = Duration::from_millis(5);
/// 取り直しの間隔の上限。
const DB_RETRY_MAX_INTERVAL: Duration = Duration::from_millis(50);

/// ローカル API の待ち受けの状態（[`AppCore`] が持つ）。
enum ApiSlot {
    /// 待ち受けていない（データベースを開いていない間、閉じた後）。
    Stopped,
    Running(ApiServer),
    /// 待ち受けようとして、始められなかった。
    Unavailable(ApiUnavailable),
}

/// データベースの状態。
enum Slot {
    Opening,
    Open(Database),
    NotFound(DatabaseNotFound),
    Failed {
        reason: StartupFailure,
        detail: String,
        info: FailureInfo,
    },
    /// アプリの終了で閉じた。
    Closed,
}

/// 開けなかったときに、画面の案内に使う場所。
#[derive(Default)]
struct FailureInfo {
    /// 開こうとした `yunagi.db`（保存先が分かったとき）。
    database: Option<PathBuf>,
    /// マイグレーション前のバックアップ（マイグレーションに失敗したとき）。
    backup: Option<PathBuf>,
    /// `config.json`（読めなかったとき）。
    config: Option<PathBuf>,
}

/// アプリの中のコア。Tauri の状態（`State<Arc<AppCore>>`）として持つ。
pub struct AppCore {
    db: Mutex<Slot>,
    screen: ScreenState,
    /// 最後に確かめた今日の論理日付（変わったかを見るため）。
    today: Mutex<Option<LogicalDate>>,
    /// 定期バックアップの進み具合。保存先の変更と復元（画面の作業で作る）は、バックアップと同時に
    /// 走らせない（`DailyBackup` の説明）。作るときは、作っている途中のものを
    /// [`AppCore::wait_for_backup`] で待ち、その間に次が始まらないようにする。
    backup: Mutex<BackupState>,
    /// 起動時にバックアップから戻した後、利用者が確かめる（[`AppCore::acknowledge_restore`]）まで
    /// 持つ、退避した前のデータベースの場所（無かったときは中が `None`）。持っている間、開いた状態は
    /// [`Startup::Restored`] として知らせる（画面が読み込み直しても、場所を示せるように）。
    restored: Mutex<Option<Option<PathBuf>>>,
    /// 動いている間の保存先の変更・選び直し・復元の最中か（[`AppCore::begin_maintenance`]）。
    /// 最中は、定期バックアップを始めない。
    maintenance: AtomicBool,
    /// ローカル API の待ち受け（アーキテクチャ設計書 4.4・4.6）。データベースを開き終えている間
    /// だけ動く。
    api: Mutex<ApiSlot>,
    api_options: Mutex<ApiOptions>,
    /// 閉じ始めた（[`AppCore::close`]）。以後、ローカル API の待ち受けを始めない。
    closing: AtomicBool,
    /// ノートモードで前面に出す頼み（`yunagi show --note`）のうち、画面がまだ受け取っていないもの
    /// （[`AppCore::request_show`]・[`AppCore::take_show_note`]）。起動した直後は、待ち受けが開いても
    /// 画面がまだ準備できておらず、知らせ（`show-requested`）を取りこぼすため、画面が準備できたときに
    /// 取り出せるよう覚えておく。
    show_note: AtomicBool,
    notify: Notify,
}

/// 動いている間の保存先の変更・選び直し・復元の最中であることの印（手放すと終わる）。
struct Maintenance<'a>(&'a AtomicBool);

impl Drop for Maintenance<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// 定期バックアップの進み具合。作るのは一度に1つだけ。
#[derive(Default)]
struct BackupState {
    /// 作り終えた（すでにあった場合も含む）論理日付。
    done_for: Option<LogicalDate>,
    /// 作れなかったと知らせた論理日付（同じ日の作り直しの失敗では、もう知らせない）。
    failed_for: Option<LogicalDate>,
    /// 作っているスレッド（[`AppCore::wait_for_backup`]）。
    running: Option<JoinHandle<()>>,
}

/// ロックの毒（ほかのスレッドが持ったまま止まった）は無視して中身を使う。中身はどれも、
/// 書きかけで止まっても使える値（データベースは SQLite がトランザクションで守る）。
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 原因までつないだ1行の文（ログ用）。
pub fn chain(e: &(dyn std::error::Error + 'static)) -> String {
    ErrorChain(e).to_string()
}

/// 端末のタイムゾーン。求められなければ UTC（日付の行に記録するタイムゾーンもそれになる）。
///
/// 呼ぶたびに求めるが、jiff が5分間は覚えているので、毎回ファイルを読むことはない。アプリで
/// 覚え続けると、起動中に端末のタイムゾーンを変えたとき（移動など）に追いつけないので、覚えない。
pub fn system_tz() -> TimeZone {
    TimeZone::try_system().unwrap_or(TimeZone::UTC)
}

/// 今の時刻（ミリ秒まで。コアが記録する精度）。
pub fn now() -> Timestamp {
    yunagi_core::storage::table::now()
}

/// 画面からの書き込みの種類（[`AppCore::screen_write`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenWrite {
    /// 保存：利用者が入力・選んだ内容の書き込み（項目の追加・本文・状態・解消メモ・分割、
    /// ノート・振り返り。ノートの本文の保存を兼ねる行送りも）。画面は失敗したら内容を持ったまま
    /// 再試行する（画面設計書 3章）ので、
    /// 保存の失敗中（アーキテクチャ設計書 4.1）を始め・終える。
    Save,
    /// それ以外の操作（削除・削除の取り消し・並べ替え）。保存の失敗中を変えない。
    Operation,
}

/// コマンドの結果が、データを変えたか（変えたら画面に知らせる）。
pub trait Changes {
    /// データを変えたか。
    fn changed(&self) -> bool;

    /// 書き込みまで進んだか（画面の保存の失敗中を終えるか）。保存の行き違いで書かなかったときだけ
    /// `false`。
    fn committed(&self) -> bool {
        true
    }
}

impl Changes for item::Item {
    fn changed(&self) -> bool {
        true
    }
}

impl Changes for item::Outcome {
    fn changed(&self) -> bool {
        self.updated
    }
}

impl Changes for item::Resolution {
    fn changed(&self) -> bool {
        self.updated()
    }
}

impl Changes for text_doc::Written {
    fn changed(&self) -> bool {
        self.updated
    }
}

impl Changes for crate::types::Item {
    fn changed(&self) -> bool {
        true
    }
}

impl Changes for ItemOutcome {
    fn changed(&self) -> bool {
        self.updated
    }
}

impl Changes for SplitResult {
    fn changed(&self) -> bool {
        true
    }
}

impl Changes for crate::types::Deleted {
    fn changed(&self) -> bool {
        true
    }
}

impl Changes for TextSave {
    fn changed(&self) -> bool {
        matches!(self, Self::Saved { updated: true, .. })
    }

    fn committed(&self) -> bool {
        matches!(self, Self::Saved { .. })
    }
}

impl Changes for crate::types::CarryoverConfirmed {
    /// 項目を作らなくても（「持ち越さない」）、持ち越しを終えた記録を書くので、変えたとして知らせる
    /// （ほかの窓は持ち越しの状態を読み直す）。
    fn changed(&self) -> bool {
        true
    }
}

impl AppCore {
    /// まだデータベースを開いていないコア。知らせは `notify` に渡す。
    pub fn new(notify: Notify) -> Arc<Self> {
        Arc::new(Self {
            db: Mutex::new(Slot::Opening),
            screen: ScreenState::default(),
            today: Mutex::new(None),
            backup: Mutex::new(BackupState::default()),
            restored: Mutex::new(None),
            maintenance: AtomicBool::new(false),
            api: Mutex::new(ApiSlot::Stopped),
            api_options: Mutex::new(ApiOptions::default()),
            closing: AtomicBool::new(false),
            show_note: AtomicBool::new(false),
            notify,
        })
    }

    /// 別のスレッドでデータベースを開き（[`AppCore::open`]）、今日の論理日付が変わったかを
    /// [`TODAY_CHECK_INTERVAL`] ごとに確かめ始める（[`AppCore::check_today`]）。呼んだスレッドは
    /// 止めない。アプリの起動時に1回呼ぶ。
    pub fn start(self: &Arc<Self>) {
        let core = Arc::clone(self);
        spawn("yunagi-open", move || core.open());
        let core = Arc::clone(self);
        spawn("yunagi-today", move || watch_today(&core));
    }

    /// 既定ディレクトリの `yunagi.lock` を取り、データベースを開く（アーキテクチャ設計書 4章 4、
    /// 6.3）。終わったら [`Notice::Startup`] を知らせ、開けたら定期バックアップを始める。
    ///
    /// ロックを最大 5 秒待つので、画面を動かすスレッドからは呼ばない（[`AppCore::start`] を使う）。
    pub fn open(self: &Arc<Self>) {
        self.open_with(Database::open);
    }

    /// データベースが見つからなかったとき（[`Startup::NotFound`]）に、新しく作って開く
    /// （利用者が「新しく作る」を選んだとき。アーキテクチャ設計書 6.3）。それ以外の状態では、
    /// 何もせずに [`CommandError::InvalidState`]。結果は [`AppCore::open`] と同じく知らせる。
    pub fn create_database(self: &Arc<Self>) -> Result<Startup, CommandError> {
        self.begin(|s| matches!(s, Slot::NotFound(_)))?;
        self.open_with(Database::create);
        Ok(self.startup())
    }

    /// 見つからなかった・開けなかったときに、もう一度開く（同期が終わった、ドライブを繋いだ、
    /// CLI・MCP の操作が終わったなど。画面設計書 10.1）。開いている途中・開いた
    /// 後は [`CommandError::InvalidState`]。結果は [`AppCore::open`] と同じく知らせる。
    pub fn retry_open(self: &Arc<Self>) -> Result<Startup, CommandError> {
        self.begin(|s| matches!(s, Slot::NotFound(_) | Slot::Failed { .. }))?;
        self.open_with(Database::open);
        Ok(self.startup())
    }

    /// 見つからなかったとき（[`Startup::NotFound`]）に、データベースのある既存のフォルダ `dir` を
    /// 保存先として記録してから開く（保存先の選び直し。アーキテクチャ設計書 6章・6.3）。
    ///
    /// `dir` を使えなければ、何も変えずに [`CommandError::Rejected`]（状態は見つからないまま）。
    /// 記録した後に開けなかったときは、`Ok` で開けなかった状態を返す。ロックが取れないなど、選び直す
    /// 前に止まったときは [`CommandError::NotReady`]（状態は [`Notice::Startup`] で届く）。それ以外の
    /// 状態では [`CommandError::InvalidState`]。
    pub fn choose_data_dir(self: &Arc<Self>, dir: &Path) -> Result<Startup, CommandError> {
        let lock = self.begin_locked(|s| matches!(s, Slot::NotFound(_)))?;
        let chosen = data_dir::use_existing_data_dir(&lock, dir);
        // 記録できなかったときも、同じロックで開き直して、見つからない状態に戻す
        self.finish(open_slot(lock, Database::open));
        match chosen {
            Ok(()) => Ok(self.startup()),
            Err(e) => {
                eprintln!(
                    "yunagi: could not use {} as the data directory: {}",
                    dir.display(),
                    chain(&e)
                );
                Err(data_dir_rejected(&e))
            }
        }
    }

    /// 見つからなかったとき（[`Startup::NotFound`]）と、データベースが壊れていて開けなかったとき
    /// （[`StartupFailure::Corrupted`]）に、バックアップ `backup` で置き換えて開く（アーキテクチャ
    /// 設計書 6.2「開けないデータベースからの回復」・6.3）。それまでのデータベースは消さずに
    /// `backups/` に退避する。戻したら、利用者が確かめる（[`AppCore::acknowledge_restore`]）まで、状態は
    /// 退避した場所を持つ [`Startup::Restored`] になる。
    ///
    /// 戻せなかったときは、開き直して元の状態に戻してから、エラーを返す（使えないバックアップは
    /// [`CommandError::Rejected`]、前のデータベースを元の場所に戻せなかったときは
    /// [`CommandError::PutBackFailed`]）。ロックが取れないなど、戻す前に止まったときは
    /// [`CommandError::NotReady`]（状態は [`Notice::Startup`] で届く）。それ以外の状態では
    /// [`CommandError::InvalidState`]。
    pub fn restore_at_startup(self: &Arc<Self>, backup: &Path) -> Result<Startup, CommandError> {
        let db_lock = self.begin_locked(|s| {
            matches!(
                s,
                Slot::NotFound(_)
                    | Slot::Failed {
                        reason: StartupFailure::Corrupted,
                        ..
                    }
            )
        })?;
        match Database::open_from_backup(db_lock, backup) {
            Ok(recovered) => {
                // 開いた知らせより先に記録する（知らせは Restored で届く）
                *lock(&self.restored) = Some(recovered.set_aside);
                self.finish(Slot::Open(recovered.database));
                Ok(self.startup())
            }
            Err(e) => {
                eprintln!(
                    "yunagi: could not restore from {}: {}",
                    backup.display(),
                    chain(&e)
                );
                // ロックは離れている。取り直して、今の状態（壊れている・見つからない）を求め直す
                self.open_with(Database::open);
                Err(recover_error(&e))
            }
        }
    }

    /// 保存先の `backups/` にあるバックアップの一覧（新しいものが先）。開いた後と、見つからない・
    /// 保存先が分かっていて開けなかったときに使える（起動時の「バックアップから戻す」と設定画面）。
    /// 保存先が分からないときは [`CommandError::InvalidState`]。
    pub fn backups(&self) -> Result<Vec<BackupEntry>, CommandError> {
        let data_dir = match &*lock(&self.db) {
            Slot::Open(db) => db.path().parent().map(Path::to_path_buf),
            Slot::NotFound(nf) => Some(nf.data_dir.clone()),
            Slot::Failed { info, .. } => info
                .database
                .as_deref()
                .and_then(Path::parent)
                .map(Path::to_path_buf),
            Slot::Opening | Slot::Closed => None,
        }
        .ok_or(CommandError::InvalidState)?;
        let list = backup::list_backups(&data_dir).map_err(|e| CommandError::storage(&e))?;
        Ok(list.into_iter().map(BackupEntry::from).collect())
    }

    /// バックアップから戻した結果（[`Startup::Restored`]）を、利用者が確かめた。状態を
    /// [`Startup::Ready`] にして知らせる。戻した後でなければ [`CommandError::InvalidState`]。
    pub fn acknowledge_restore(&self) -> Result<Startup, CommandError> {
        if !matches!(*lock(&self.db), Slot::Open(_)) || lock(&self.restored).take().is_none() {
            return Err(CommandError::InvalidState);
        }
        let startup = self.startup();
        self.notify(Notice::Startup(startup.clone()));
        Ok(startup)
    }

    // ---- 動いている間の保存先の変更・選び直し・復元（設定画面のデータ区分。画面設計書 7章） ----
    //
    // 起動時の操作（上）と違い、データベースを開いたまま画面が動いている。画面を消さないよう、
    // 状態は「開いた」のまま変えず（[`Notice::Startup`] も出さない）、データベースを使う間は
    // `db` のロックを持ち続けて、ほかのコマンドを待たせる（途中の状態を見せない）。定期バックアップとは
    // 重ねない（アーキテクチャ設計書 6.2）。

    /// 保存先の変更・選び直し・復元を始める。ほかのそれが最中なら [`CommandError::InvalidState`]。
    /// 作っている途中の定期バックアップを待ち、手放すまで新しい定期バックアップを始めさせない。
    fn begin_maintenance(&self) -> Result<Maintenance<'_>, CommandError> {
        self.maintenance
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| CommandError::InvalidState)?;
        let guard = Maintenance(&self.maintenance);
        self.wait_for_backup();
        Ok(guard)
    }

    /// 開いているデータベースを、`yunagi.lock` を離さずに閉じて `f` を行い、同じロックで開き直す。
    /// 開き直せなかったときは、開けなかった状態を知らせる（画面は起動時の画面に戻る）。
    ///
    /// 開いていなければ [`CommandError::NotReady`]。閉じるのに失敗したときは、データベースを元に戻して
    /// 何もせずに [`CommandError::Storage`]。
    fn with_database_closed<T>(
        &self,
        f: impl FnOnce(&DbLock) -> Result<T, CommandError>,
    ) -> Result<T, CommandError> {
        let _maintenance = self.begin_maintenance()?;
        let mut slot = lock(&self.db);
        if !matches!(*slot, Slot::Open(_)) {
            return Err(CommandError::NotReady);
        }
        let Slot::Open(db) = std::mem::replace(&mut *slot, Slot::Opening) else {
            unreachable!("checked above");
        };
        let held = match db.close_keeping_lock() {
            Ok(held) => held,
            Err(e) => {
                let e = *e;
                *slot = Slot::Open(e.db);
                return Err(CommandError::storage(&e.error));
            }
        };
        // `f` が panic しても、開き直してから伝える（データベースを開けないまま残さない）
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&held)));
        let reopened = open_slot(held, Database::open);
        let opened = matches!(reopened, Slot::Open(_));
        *slot = reopened;
        drop(slot);
        if !opened {
            // 開き直せず、ロックも離れた。待ち受けを止める（待っていた依頼は、データベースが開いて
            // いないので何も反映せずに `busy` で終わる）
            self.stop_api();
            self.notify(Notice::Startup(self.startup()));
        }
        match result {
            Err(panic) => std::panic::resume_unwind(panic),
            // 開き直せなかったときは、成功を返さない（画面は起動時の画面に移っている）
            Ok(Ok(_)) if !opened => Err(CommandError::NotReady),
            Ok(result) => result,
        }
    }

    /// いまの保存先（データベースの `yunagi.db` と `backups/` を置くフォルダ）。開いていなければ
    /// [`CommandError::NotReady`]。
    pub fn data_dir(&self) -> Result<String, CommandError> {
        self.with_db(|db| {
            db.path()
                .parent()
                .map(|p| p.display().to_string())
                .ok_or(CommandError::NotReady)
        })
    }

    /// 保存先を `dir` に変える（データベースと `backups/` を移し、`config.json` に記録する。アーキテクチャ
    /// 設計書 6章）。使えない保存先は何も変えずに [`CommandError::Rejected`]。結果は、移した後に消せなかった
    /// 元のファイル。
    pub fn change_data_dir(&self, dir: &Path) -> Result<Vec<PathBuf>, CommandError> {
        self.with_database_closed(|held| {
            data_dir::change_data_dir(held, dir)
                .map(|report| report.leftovers)
                .map_err(|e| {
                    eprintln!(
                        "yunagi: could not change the data directory to {}: {}",
                        dir.display(),
                        chain(&e)
                    );
                    data_dir_rejected(&e)
                })
        })
    }

    /// データベースのある既存のフォルダ `dir` を、データを移さずに保存先として記録して開き直す
    /// （保存先の選び直し。アーキテクチャ設計書 6章）。使えないフォルダは何も変えずに
    /// [`CommandError::Rejected`]。開くデータベースが別のものになるので、画面は読み込み直す。
    pub fn use_data_dir(&self, dir: &Path) -> Result<(), CommandError> {
        self.with_database_closed(|held| {
            data_dir::use_existing_data_dir(held, dir).map_err(|e| {
                eprintln!(
                    "yunagi: could not use {} as the data directory: {}",
                    dir.display(),
                    chain(&e)
                );
                data_dir_rejected(&e)
            })?;
            // 開くデータベースは別のものなので、その日の定期バックアップを作り直す（最中の印を持っている
            // 間に戻す。手放した後だと、始まった定期バックアップと重なって2回作る）
            lock(&self.backup).done_for = None;
            Ok(())
        })?;
        self.notify_changed(ChangeSource::Screen);
        Ok(())
    }

    /// バックアップ `backup`（`list_backups` の `path`）から復元する（アーキテクチャ設計書 6.2）。
    /// 今のデータは復元前のバックアップに退避する。使えないバックアップは何も変えずに
    /// [`CommandError::Rejected`]。画面は、復元の後に読み込み直す。
    pub fn restore_backup(&self, backup: &Path) -> Result<(), CommandError> {
        let _maintenance = self.begin_maintenance()?;
        {
            let mut slot = lock(&self.db);
            let Slot::Open(db) = &mut *slot else {
                return Err(CommandError::NotReady);
            };
            db.restore_backup(backup).map_err(|e| {
                eprintln!(
                    "yunagi: could not restore from {}: {}",
                    backup.display(),
                    chain(&e)
                );
                backup_error(&e)
            })?;
        }
        // 中身が変わったので、その日の定期バックアップを作り直す
        lock(&self.backup).done_for = None;
        self.notify_changed(ChangeSource::Screen);
        Ok(())
    }

    /// [`AppCore::begin`] の後で `yunagi.lock` を取る（選び直し・戻すとき）。取れなければ、開けなかった
    /// 状態を記録して知らせ、[`CommandError::NotReady`] を返す（操作は始めていない）。
    fn begin_locked(
        self: &Arc<Self>,
        allowed: impl FnOnce(&Slot) -> bool,
    ) -> Result<DbLock, CommandError> {
        self.begin(allowed)?;
        acquire_lock().map_err(|slot| {
            self.finish(*slot);
            CommandError::NotReady
        })
    }

    /// 状態が `allowed` なら、開いている途中にする（その間に、ほかの起動時の操作を断る）。
    /// それ以外なら [`CommandError::InvalidState`]。
    fn begin(&self, allowed: impl FnOnce(&Slot) -> bool) -> Result<(), CommandError> {
        let mut slot = lock(&self.db);
        if !allowed(&slot) {
            return Err(CommandError::InvalidState);
        }
        *slot = Slot::Opening;
        Ok(())
    }

    /// ロックを取ってから `open` で開き、結果を記録する（[`AppCore::finish`]）。
    fn open_with(self: &Arc<Self>, open: fn(DbLock) -> Result<Database, OpenError>) {
        let slot = match acquire_lock() {
            Ok(lock) => open_slot(lock, open),
            Err(slot) => *slot,
        };
        self.finish(slot);
    }

    /// 開いた結果を、まだ開いている途中のとき（[`Slot::Opening`]）だけ記録し、知らせる。開いている
    /// 間にアプリが終わった（[`AppCore::close`]）ときは、開いたデータベースをすぐ閉じ、知らせもしない。
    fn finish(self: &Arc<Self>, slot: Slot) {
        let ready = matches!(slot, Slot::Open(_));
        {
            let mut current = lock(&self.db);
            if !matches!(*current, Slot::Opening) {
                drop(current);
                close_slot(slot);
                return;
            }
            *current = slot;
        }
        // ロックを持ち、データベースを開き終えた：ここから待ち受ける（4.4）
        if ready {
            self.start_api();
        }
        self.notify(Notice::Startup(self.startup()));
        if ready {
            self.check_today(now());
        }
    }

    /// データベースを開く処理の状態。
    pub fn startup(&self) -> Startup {
        let text = |p: &Option<PathBuf>| p.as_ref().map(|p| p.display().to_string());
        match &*lock(&self.db) {
            Slot::Opening => Startup::Opening,
            Slot::Open(_) => match &*lock(&self.restored) {
                Some(set_aside) => Startup::Restored {
                    set_aside: text(set_aside),
                },
                None => Startup::Ready,
            },
            Slot::NotFound(nf) => Startup::NotFound {
                path: nf.path.display().to_string(),
                data_dir: nf.data_dir.display().to_string(),
                configured_in: nf.configured_in.as_ref().map(|p| p.display().to_string()),
                data_dir_exists: nf.data_dir_exists,
            },
            Slot::Failed {
                reason,
                detail,
                info,
            } => Startup::Failed {
                reason: *reason,
                detail: detail.clone(),
                database: text(&info.database),
                backup: text(&info.backup),
                config: text(&info.config),
            },
            Slot::Closed => Startup::Failed {
                reason: StartupFailure::Storage,
                detail: "the database was closed".into(),
                database: None,
                backup: None,
                config: None,
            },
        }
    }

    /// 開いたデータベースで `f` を行う。開いていなければ [`CommandError::NotReady`]。
    /// データベースを使う処理は1つずつ順に行う（接続は1つ）。
    fn with_db<T>(
        &self,
        f: impl FnOnce(&Database) -> Result<T, CommandError>,
    ) -> Result<T, CommandError> {
        match &*lock(&self.db) {
            Slot::Open(db) => f(db),
            _ => Err(CommandError::NotReady),
        }
    }

    /// 読み取り。
    pub fn read<T, E: Into<CommandError>>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, E>,
    ) -> Result<T, CommandError> {
        self.with_db(|db| f(db.connection()).map_err(Into::into))
    }

    /// 画面からの書き込み。
    ///
    /// - 変えたら [`Notice::Changed`]（`by: screen`）を知らせる。
    /// - `kind` が [`ScreenWrite::Save`] のとき：書き込みに失敗したら（[`CommandError::Storage`]）、
    ///   次に保存が成功するまで、画面の保存の失敗中として記録する（その間、エージェントの書き込みは
    ///   `storage_unavailable`。アーキテクチャ設計書 4.1・4.3）。引数の誤りなど、保存先に関係なく
    ///   失敗したものは数えない。保存の行き違い（[`TextSave::Stale`]）は成功に数えない。
    ///
    /// 保存の失敗中かは、データベースを使っている間に記録する（2つの保存の結果が、書いた順と
    /// 逆に記録されないように）。
    pub fn screen_write<T: Changes, E: Into<CommandError>>(
        &self,
        kind: ScreenWrite,
        f: impl FnOnce(&Connection) -> Result<T, E>,
    ) -> Result<T, CommandError> {
        let result = self.with_db(|db| {
            let result = f(db.connection()).map_err(Into::into);
            if kind == ScreenWrite::Save {
                match &result {
                    Ok(r) if r.committed() => self.screen.set_save_failing(false),
                    Err(CommandError::Storage { .. }) => self.screen.set_save_failing(true),
                    _ => {}
                }
            }
            result
        });
        if result.as_ref().is_ok_and(Changes::changed) {
            self.notify_changed(ChangeSource::Screen);
        }
        result
    }

    /// エージェント・CLI からの書き込み（ローカル API が使う。アーキテクチャ設計書 4章 1）。
    /// `f` には、画面の状態（入力中・保存の失敗中）を持つ呼び出し元 `agent` が渡る。
    /// 変えたら [`Notice::Changed`]（`by: agent`）を知らせる（画面からの書き込みと同じ経路。4章 6）。
    ///
    /// データベースを開いていなければ `Err(`[`CommandError::NotReady`]`)`。`f` の結果は
    /// そのまま `Ok` の中に返す。
    pub fn agent_write<T: Changes, E>(
        &self,
        client_id: Ulid,
        f: impl FnOnce(&Connection, &Agent<'_>) -> Result<T, E>,
    ) -> Result<Result<T, E>, CommandError> {
        // ローカル API の書き込み（`api_write`）と同じ経路。待つ期限だけが無い（ほぼ無期限）
        let far = Instant::now() + Duration::from_secs(60 * 60 * 24 * 365);
        self.api_write(far, |_| Ok(client_id), f)
            .map_err(|Busy| CommandError::NotReady)
    }

    /// 初めての識別子の登録（設定画面のエージェントの一覧が変わる）を画面に知らせる。データの
    /// 書き込みではないので、`changed` の `by` はエージェント。
    pub fn notify_agent_registered(&self) {
        self.notify_changed(ChangeSource::Agent);
    }

    /// 窓を前面に出すよう頼む（ローカル API の `show_app`。`note` が真ならノートモードで）。データベース
    /// にも錠にも触れないので、保存先の変更・復元の最中でも待たずに効く。窓を操作する部分は、知らせの
    /// 受け手（`crate::run`）が持つ（窓の無い試験では知らせを受け取るだけ）。
    pub fn request_show(&self, note: bool) {
        if note {
            self.show_note.store(true, Ordering::Release);
        }
        self.notify(Notice::ShowRequested(ShowRequested { note }));
    }

    /// ノートモードで前面に出す頼みが、画面にまだ渡っていなければ、受け取って忘れる（画面が準備できた
    /// ときと、`show-requested` を受けたときに呼ぶ）。
    pub fn take_show_note(&self) -> bool {
        self.show_note.swap(false, Ordering::AcqRel)
    }

    /// [`AppCore::agent_write`] の、ローカル API 用の版。データベースを使うのを待つのは最大
    /// [`ApiOptions::db_wait`]（既定 5 秒）までで、待っても使えなければ（保存先の変更・復元の最中、
    /// またはデータベースを開いていない）何も反映せずに [`Busy`]（CLI・MCP には `busy`）。
    /// それ以外は [`AppCore::agent_write`] と同じ（変えたら `changed`（`by: agent`）を知らせる）。
    ///
    /// `deadline` は依頼ごとに [`AppCore::api_deadline`] で 1 回だけ求める。呼び出し元のクライアントを
    /// 求める処理 `client` も、同じ錠を持つ間に（`f` の前に）行う（錠を 2 回取って待ちが 2 倍になり、
    /// 5 秒を超えないように）。`client` が失敗したら `f` は行わない。
    pub fn api_write<T: Changes, E>(
        &self,
        deadline: Instant,
        client: impl FnOnce(&Connection) -> Result<Ulid, E>,
        f: impl FnOnce(&Connection, &Agent<'_>) -> Result<T, E>,
    ) -> Result<Result<T, E>, Busy> {
        let result = self.with_db_within(deadline, |db| {
            let client_id = client(db.connection())?;
            let agent = Agent {
                client_id,
                ui: &self.screen,
            };
            f(db.connection(), &agent)
        })?;
        self.notify_agent_write(&result);
        Ok(result)
    }

    /// ローカル API の読み取り。待ちは [`AppCore::api_write`] と同じ。
    pub fn api_read<T>(
        &self,
        deadline: Instant,
        f: impl FnOnce(&Connection) -> T,
    ) -> Result<T, Busy> {
        self.with_db_within(deadline, |db| f(db.connection()))
    }

    /// 依頼がデータベースを使えるまで待つ期限（いまから [`ApiOptions::db_wait`] 後）。
    pub fn api_deadline(&self) -> Instant {
        let now = Instant::now();
        now.checked_add(self.api_options().db_wait)
            .unwrap_or_else(|| now + Duration::from_secs(60 * 60 * 24 * 365))
    }

    /// 開いたデータベースで `f` を行う。ロックを待つのは `deadline` まで。待っても取れないか、開いて
    /// いなければ [`Busy`]（`f` は行わない）。ロックを離す前の待ち受けの停止が、ここで待つ依頼を
    /// 巻き込んで止まらないよう、ロックの待ちに期限を付ける。
    ///
    /// 標準の錠には期限付きの取得が無いので、取り直しで待つ。間隔は 5 ミリ秒から 50 ミリ秒まで
    /// 倍々に延ばす（長い作業の間に多くの依頼が待っても、起きる回数を抑えるため）。
    fn with_db_within<T>(
        &self,
        deadline: Instant,
        f: impl FnOnce(&Database) -> T,
    ) -> Result<T, Busy> {
        let mut interval = DB_RETRY_INTERVAL;
        loop {
            let guard = match self.db.try_lock() {
                Ok(guard) => guard,
                Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
                Err(TryLockError::WouldBlock) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(Busy);
                    }
                    thread::sleep(interval.min(deadline - now));
                    interval = (interval * 2).min(DB_RETRY_MAX_INTERVAL);
                    continue;
                }
            };
            return match &*guard {
                Slot::Open(db) => Ok(f(db)),
                _ => Err(Busy),
            };
        }
    }

    /// エージェントの書き込みが変えていたら `changed`（`by: agent`）を知らせる。
    fn notify_agent_write<T: Changes, E>(&self, result: &Result<T, E>) {
        if result.as_ref().is_ok_and(Changes::changed) {
            self.notify_changed(ChangeSource::Agent);
        }
    }

    /// ローカル API の待ち受けの設定（待ち時間など）。
    pub fn api_options(&self) -> ApiOptions {
        *lock(&self.api_options)
    }

    /// ローカル API の待ち受けの設定を変える（試験で待ち時間を短くする）。次に待ち受けを始めるときと、
    /// 以後の依頼から効く。
    pub fn set_api_options(&self, options: ApiOptions) {
        *lock(&self.api_options) = options;
    }

    /// ローカル API の状態（動作中か、接続中のクライアント数。設定画面のエージェント区分が使う。
    /// アーキテクチャ設計書 4.6）。
    pub fn api_state(&self) -> ApiState {
        match &*lock(&self.api) {
            ApiSlot::Stopped => ApiState::Stopped,
            ApiSlot::Running(server) => ApiState::Listening {
                connections: server.connections(),
            },
            ApiSlot::Unavailable(u) => ApiState::Unavailable(u.clone()),
        }
    }

    /// ローカル API の待ち受けを始める（ロックを持ち、データベースを開き終えた直後。4.4）。すでに
    /// 動いていれば何もしない。始められなければ、理由を標準エラーと [`AppCore::api_state`] に残す
    /// （アプリは止めない。CLI・MCP は待ち受けが無いものとして直接モードを試す）。
    fn start_api(self: &Arc<Self>) {
        let mut slot = lock(&self.api);
        if matches!(*slot, ApiSlot::Running(_)) {
            return;
        }
        // 開き終えてから始めるまでの間に閉じ始めていたら（アプリの終了）、始めない。ロックを離した
        // 後で待ち受けが始まらないようにする。`close` は、先に `closing` を立ててから待ち受けを止める
        // （この `api` の錠を取る）ので、ここの確かめと止めるのが行き違わない
        if self.closing.load(Ordering::Acquire) || !matches!(*lock(&self.db), Slot::Open(_)) {
            return;
        }
        *slot = match data_dir::default_dir()
            .map_err(|e| ApiUnavailable::other(&e))
            .and_then(|dir| ApiServer::start(self, &dir))
        {
            Ok(server) => ApiSlot::Running(server),
            Err(unavailable) => {
                eprintln!(
                    "yunagi: the local API did not start: {}",
                    unavailable.detail
                );
                ApiSlot::Unavailable(unavailable)
            }
        };
    }

    /// ローカル API の待ち受けを止める：新しい接続を受け付けなくし、処理中の依頼を最後まで行って
    /// 応答を返し、接続を閉じて、ソケットとトークンのファイルを消す。止め終えてから返る。
    /// ロックを離す前（アプリの終了、開けなかった状態に移るとき）に呼ぶ。動いていなければ何もしない。
    fn stop_api(&self) {
        let slot = std::mem::replace(&mut *lock(&self.api), ApiSlot::Stopped);
        if let ApiSlot::Running(server) = slot {
            server.stop();
        }
    }

    /// 画面の入力中の状態。画面から知らされたものを [`ScreenState::report`] で置き換える。
    pub fn screen(&self) -> &ScreenState {
        &self.screen
    }

    /// 瞬間 `now` の今日の論理日付を求め、前に確かめたときから変わっていたら
    /// [`Notice::TodayChanged`] を知らせる（開いた直後の1回目は知らせない）。その日の定期バックアップを
    /// まだ作り終えていなければ、作り始める（アーキテクチャ設計書 6.2。作れなかったときは、次に
    /// 確かめたときに作り直す）。
    ///
    /// データベースを開いていなければ何もしない。
    pub fn check_today(self: &Arc<Self>, now: Timestamp) {
        let Ok(today) = self.read(|conn| day::today(conn, &system_tz(), now)) else {
            return;
        };
        let previous = lock(&self.today).replace(today);
        if previous.is_some_and(|p| p != today) {
            self.notify(Notice::TodayChanged(TodayChanged {
                today: crate::types::Date(today),
            }));
        }
        self.start_daily_backup(today);
    }

    /// 今日の論理日付（端末の時計とタイムゾーン、設定の切り替わり時刻で求める）。
    pub fn today(&self) -> Result<LogicalDate, CommandError> {
        self.read(|conn| day::today(conn, &system_tz(), now()))
    }

    /// `today` の定期バックアップを、別のスレッドで作る（無ければ作る）。その日の分を作り終えているか、
    /// ほかのバックアップを作っている途中なら何もしない（作っている途中に日付が変わったときは、次に
    /// 確かめたときに作る）。
    ///
    /// 写すのはデータベースのファイルを別に開いた接続からなので、作っている間も画面のコマンドは
    /// 待たずに動く（`DailyBackup`）。作れなかったときは、標準エラーに書き、その日に初めて失敗した
    /// ときだけ [`Notice::BackupFailed`] を知らせる（失敗にはせず、次に確かめたときに作り直す）。
    fn start_daily_backup(self: &Arc<Self>, today: LogicalDate) {
        let mut state = lock(&self.backup);
        let running = state.running.as_ref().is_some_and(|h| !h.is_finished());
        // 保存先の変更・選び直し・復元の最中は始めない（ファイルを差し替えるため）。終わった後の
        // 次の確かめ（30 秒後）で作る
        if running || state.done_for == Some(today) || self.maintenance.load(Ordering::Acquire) {
            return;
        }
        let job = match self.with_db(|db| db.daily_backup().map_err(|e| CommandError::storage(&e)))
        {
            Ok(job) => job,
            Err(CommandError::Storage { detail }) => {
                return self.backup_failed(&mut state, today, &detail);
            }
            Err(_) => return,
        };
        let core = Arc::clone(self);
        state.running = spawn("yunagi-backup", move || {
            let result = job.ensure(today);
            let mut state = lock(&core.backup);
            match result {
                Ok(_) => state.done_for = Some(today),
                Err(e) => core.backup_failed(&mut state, today, &chain(&e)),
            }
        });
    }

    fn backup_failed(&self, state: &mut BackupState, today: LogicalDate, detail: &str) {
        eprintln!("yunagi: the daily backup for {today} failed: {detail}");
        if state.failed_for.replace(today) != Some(today) {
            self.notify(Notice::BackupFailed(BackupFailed {
                detail: detail.to_owned(),
            }));
        }
    }

    /// 作っている途中の定期バックアップが終わるまで待つ（テストと、終了の前に使う）。
    pub fn wait_for_backup(&self) {
        let handle = lock(&self.backup).running.take();
        if let Some(handle) = handle {
            // バックアップのスレッドが止まっても（不具合）、待つ側は続ける
            let _ = handle.join();
        }
    }

    /// 作っている途中の定期バックアップを待ってから、データベースを閉じ、ロックを離す（アプリの
    /// 終了時）。待たずに離すと、写している途中で CLI・MCP がロックを取ってデータベースを書き換えうる
    /// ため。閉じた後のコマンドは [`CommandError::NotReady`]。
    pub fn close(&self) {
        // ロックを離す前に、ローカル API を止める（4.4）。処理中の依頼は、データベースが開いている
        // うちに最後まで行って応答を返す。先に印を立てて、止めた後に待ち受けが始まらないようにする
        self.closing.store(true, Ordering::Release);
        self.stop_api();
        // 先に閉じた状態にして、次のバックアップが始まらないようにしてから待つ。データベース
        // （ロック）は、待ち終わるまで手元に持つ
        let slot = std::mem::replace(&mut *lock(&self.db), Slot::Closed);
        self.wait_for_backup();
        close_slot(slot);
    }

    /// 閉じたか（[`AppCore::close`]）。
    fn is_closed(&self) -> bool {
        matches!(*lock(&self.db), Slot::Closed)
    }

    fn notify_changed(&self, by: ChangeSource) {
        self.notify(Notice::Changed(Changed { by }));
    }

    fn notify(&self, notice: Notice) {
        (self.notify)(notice);
    }
}

/// コアを閉じるまで、[`TODAY_CHECK_INTERVAL`] ごとに今日の論理日付を確かめる。
fn watch_today(core: &Arc<AppCore>) {
    loop {
        thread::sleep(TODAY_CHECK_INTERVAL);
        if core.is_closed() {
            return;
        }
        core.check_today(now());
    }
}

/// 開いていれば閉じ、ロックを離す。閉じるのに失敗したら標準エラーに書く。
fn close_slot(slot: Slot) {
    if let Slot::Open(db) = slot
        && let Err(e) = db.close()
    {
        eprintln!("yunagi: failed to close the database: {}", chain(&e));
    }
}

/// 名前を付けたスレッドで `f` を動かす。スレッドを作れなければ、標準エラーに書いて `None`。
fn spawn(name: &str, f: impl FnOnce() + Send + 'static) -> Option<JoinHandle<()>> {
    match thread::Builder::new().name(name.into()).spawn(f) {
        Ok(handle) => Some(handle),
        Err(e) => {
            eprintln!("yunagi: failed to start the {name} thread: {e}");
            None
        }
    }
}

/// 既定ディレクトリを求めて作り、`yunagi.lock` を取る。取れなければ、開けなかった状態を返す。
fn acquire_lock() -> Result<DbLock, Box<Slot>> {
    let failed = |reason, e: &(dyn std::error::Error + 'static)| {
        Box::new(Slot::Failed {
            reason,
            detail: chain(e),
            info: FailureInfo::default(),
        })
    };
    let dir = data_dir::default_dir().map_err(|e| failed(StartupFailure::DefaultDir, &e))?;
    // 既定ディレクトリはアプリの置き場所で、データの保存先ではない（ロック・config.json を置く）。
    // 初めて起動したときは無いので作る（保存先を作らない決まり（6.3）とは別。6章）
    // UNIX 系では本人だけが入れる権限（0700）で作る（ローカル API のソケットとトークンを置くため）
    data_dir::create_default_dir(&dir).map_err(|e| {
        failed(
            StartupFailure::DefaultDir,
            &io::Error::new(
                e.kind(),
                format!("failed to create {}: {e}", dir.path().display()),
            ),
        )
    })?;
    DbLock::acquire(&dir, DEFAULT_LOCK_WAIT).map_err(|e| match e {
        LockError::Busy { .. } => failed(StartupFailure::LockBusy, &e),
        _ => failed(StartupFailure::Lock, &e),
    })
}

/// ロック `lock` を持って `open` で開き、開けたか、開けなかった理由の状態を返す。
fn open_slot(lock: DbLock, open: fn(DbLock) -> Result<Database, OpenError>) -> Slot {
    // 案内に使う場所は、開く前に求めておく（開けなかったときはロックが離れている）
    let database = lock.database_path().ok();
    let config = lock.default_dir().path().join(data_dir::CONFIG_FILE_NAME);
    match open(lock) {
        Ok(db) => Slot::Open(db),
        Err(OpenError::NotFound(nf)) => Slot::NotFound(nf),
        Err(e) => {
            let mut info = FailureInfo {
                database,
                ..FailureInfo::default()
            };
            let reason = match &e {
                OpenError::DataDir(_) => {
                    info.config = Some(config);
                    StartupFailure::Config
                }
                OpenError::AlreadyExists(_) => StartupFailure::AlreadyExists,
                OpenError::Storage(e) => storage_failure(e, &mut info),
                OpenError::NotFound(_) => unreachable!("handled above"),
            };
            Slot::Failed {
                reason,
                detail: chain(&e),
                info,
            }
        }
    }
}

/// 開く・マイグレーションの失敗の理由。マイグレーション前のバックアップがあれば `info` に書く。
fn storage_failure(e: &StorageError, info: &mut FailureInfo) -> StartupFailure {
    match e {
        StorageError::Lock(_) => StartupFailure::Lock,
        StorageError::MigrationFailed { backup, .. } => {
            info.backup = Some(backup.clone());
            StartupFailure::MigrationFailed
        }
        StorageError::RestoreFailed { backup, .. } => {
            info.backup = Some(backup.clone());
            StartupFailure::MigrationRestoreFailed
        }
        StorageError::UnsupportedSchema { .. } => StartupFailure::UnsupportedSchema,
        StorageError::BackupCorrupted { .. } => StartupFailure::Corrupted,
        StorageError::Sqlite(e) if is_corruption(e) => StartupFailure::Corrupted,
        _ => StartupFailure::Storage,
    }
}

/// SQLite が、データベースが壊れている・データベースでないと答えた失敗か。
fn is_corruption(e: &rusqlite::Error) -> bool {
    matches!(
        e.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase)
    )
}

/// 選んだバックアップで戻せなかった理由（[`AppCore::restore_at_startup`]）。
fn recover_error(e: &RecoverError) -> CommandError {
    match e {
        RecoverError::Backup(b) => backup_error(b),
        // 前のデータベースが退避した場所に残っている。画面はその場所を示す
        RecoverError::PutBackFailed { set_aside, .. } => CommandError::PutBackFailed {
            set_aside: set_aside.display().to_string(),
            detail: chain(e),
        },
        _ => CommandError::storage(e),
    }
}

/// 選んだバックアップを使えなかった理由（起動時の回復と、設定画面の復元）。使えない（確かめて断った）
/// ものは `rejected`、それ以外は `storage`。
fn backup_error(e: &BackupError) -> CommandError {
    let reject = |reason| CommandError::Rejected { reason };
    match e {
        BackupError::Corrupted { .. } => reject(RejectReason::Corrupted),
        BackupError::NotYunagiDatabase(_) => reject(RejectReason::NotYunagi),
        BackupError::Storage(StorageError::UnsupportedSchema { .. }) => {
            reject(RejectReason::NewerSchema)
        }
        BackupError::Sqlite(_) | BackupError::NotABackup(_) => reject(RejectReason::Unreadable),
        _ => CommandError::storage(e),
    }
}

/// 選んだ保存先を使えなかった理由（[`AppCore::choose_data_dir`]・[`AppCore::change_data_dir`]）。
fn data_dir_rejected(e: &DataDirError) -> CommandError {
    let reject = |reason| CommandError::Rejected { reason };
    match e {
        DataDirError::SameDirectory(_) => reject(RejectReason::SameDirectory),
        DataDirError::InsideBackups(_) => reject(RejectReason::InsideBackups),
        DataDirError::TargetHasDatabase(_) | DataDirError::TargetHasBackups(_) => {
            reject(RejectReason::TargetHasData)
        }
        DataDirError::NotAbsolute(_)
        | DataDirError::NotNormalized(_)
        | DataDirError::NotUtf8(_) => reject(RejectReason::InvalidPath),
        DataDirError::NotADirectory(_) => reject(RejectReason::NotADirectory),
        DataDirError::NoDatabase(_) => reject(RejectReason::NoDatabase),
        DataDirError::Inspect { .. } => reject(RejectReason::Unreadable),
        DataDirError::DatabaseCorrupted { .. } => reject(RejectReason::Corrupted),
        DataDirError::NotYunagiDatabase(_) => reject(RejectReason::NotYunagi),
        DataDirError::UnsupportedSchema { .. } => reject(RejectReason::NewerSchema),
        _ => CommandError::storage(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reason(e: StorageError) -> (StartupFailure, Option<PathBuf>) {
        let mut info = FailureInfo::default();
        let reason = storage_failure(&e, &mut info);
        (reason, info.backup)
    }

    #[test]
    fn storage_failures_become_startup_reasons() {
        let backup = PathBuf::from("/d/backups/pre-migration-v1-to-v2-1.db");
        assert_eq!(
            reason(StorageError::MigrationFailed {
                version: 2,
                name: "x",
                source: rusqlite::Error::InvalidQuery,
                backup: backup.clone(),
            }),
            (StartupFailure::MigrationFailed, Some(backup.clone()))
        );
        assert_eq!(
            reason(StorageError::RestoreFailed {
                version: 2,
                name: "x",
                source: rusqlite::Error::InvalidQuery,
                restore_error: Box::new(StorageError::WalUnavailable {
                    mode: "delete".into()
                }),
                backup: backup.clone(),
            }),
            (StartupFailure::MigrationRestoreFailed, Some(backup))
        );
        assert_eq!(
            reason(StorageError::UnsupportedSchema {
                found: 9,
                supported: 3
            }),
            (StartupFailure::UnsupportedSchema, None)
        );
        assert_eq!(
            reason(StorageError::BackupCorrupted {
                path: "/d/backups/x.db".into(),
                result: "bad".into()
            }),
            (StartupFailure::Corrupted, None)
        );
        for code in [rusqlite::ffi::SQLITE_CORRUPT, rusqlite::ffi::SQLITE_NOTADB] {
            let e = rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None);
            assert_eq!(
                reason(StorageError::Sqlite(e)),
                (StartupFailure::Corrupted, None)
            );
        }
        let full = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL),
            None,
        );
        assert_eq!(
            reason(StorageError::Sqlite(full)),
            (StartupFailure::Storage, None)
        );
        assert_eq!(
            reason(StorageError::WalUnavailable {
                mode: "memory".into()
            }),
            (StartupFailure::Storage, None)
        );
    }

    #[test]
    fn a_failed_put_back_keeps_where_the_previous_database_is() {
        let e = RecoverError::PutBackFailed {
            source: Box::new(RecoverError::NoDataDir("/d".into())),
            put_back_error: io::Error::other("busy"),
            set_aside: "/d/backups/damaged-1.db".into(),
        };
        match recover_error(&e) {
            CommandError::PutBackFailed { set_aside, detail } => {
                assert_eq!(set_aside, "/d/backups/damaged-1.db");
                assert!(detail.contains("busy"), "{detail}");
            }
            other => panic!("{other:?}"),
        }
    }
}
