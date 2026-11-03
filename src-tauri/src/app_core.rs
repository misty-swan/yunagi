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

use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use std::{fs, io};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use rusqlite::Connection;
use yunagi_core::domain::agent::Agent;
use yunagi_core::domain::day;
use yunagi_core::domain::item;
use yunagi_core::domain::logical_date::LogicalDate;
use yunagi_core::domain::text_doc;
use yunagi_core::error::ErrorChain;
use yunagi_core::storage::{
    DEFAULT_LOCK_WAIT, Database, DatabaseNotFound, DbLock, LockError, OpenError, StorageError,
    Ulid, data_dir,
};

use crate::screen::ScreenState;
use crate::types::{
    BackupFailed, ChangeSource, Changed, CommandError, ItemOutcome, SplitResult, Startup,
    StartupFailure, TextSave, TodayChanged,
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
}

/// 知らせ先。どのスレッドからも呼ばれる。
pub type Notify = Box<dyn Fn(Notice) + Send + Sync>;

/// 今日の論理日付が変わったかを確かめる間隔（[`AppCore::start`]）。切り替わりを知るのは最大でこれだけ
/// 遅れる。時計を見るだけでなく設定の切り替わり時刻も読み直すので、設定の変更にも追いつく。
pub const TODAY_CHECK_INTERVAL: Duration = Duration::from_secs(30);

/// データベースの状態。
enum Slot {
    Opening,
    Open(Database),
    NotFound(DatabaseNotFound),
    Failed {
        reason: StartupFailure,
        detail: String,
    },
    /// アプリの終了で閉じた。
    Closed,
}

/// アプリの中のコア。Tauri の状態（`State<Arc<AppCore>>`）として持つ。
pub struct AppCore {
    db: Mutex<Slot>,
    screen: ScreenState,
    /// 最後に確かめた今日の論理日付（変わったかを見るため）。
    today: Mutex<Option<LogicalDate>>,
    /// 定期バックアップを作っている間に持つ。保存先の変更と復元（画面の作業で作る）も、これを
    /// 持ってから行い、バックアップと同時に走らないようにする（`DailyBackup` の説明）。
    maintenance: Mutex<()>,
    /// 最後に始めた定期バックアップのスレッド（[`AppCore::wait_for_backup`]）。
    backup_thread: Mutex<Option<JoinHandle<()>>>,
    notify: Notify,
}

/// ロックの毒（ほかのスレッドが持ったまま止まった）は無視して中身を使う。中身はどれも、
/// 書きかけで止まっても使える値（データベースは SQLite がトランザクションで守る）。
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 原因までつないだ1行の文（ログ用）。
pub fn chain(e: &(dyn std::error::Error + 'static)) -> String {
    ErrorChain(e).to_string()
}

/// 端末のタイムゾーン。求められなければ UTC（日付の行に記録するタイムゾーンもそれになる）。
pub fn system_tz() -> TimeZone {
    TimeZone::try_system().unwrap_or(TimeZone::UTC)
}

/// 今の時刻（ミリ秒まで。コアが記録する精度）。
pub fn now() -> Timestamp {
    yunagi_core::storage::table::now()
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

impl AppCore {
    /// まだデータベースを開いていないコア。知らせは `notify` に渡す。
    pub fn new(notify: Notify) -> Arc<Self> {
        Arc::new(Self {
            db: Mutex::new(Slot::Opening),
            screen: ScreenState::default(),
            today: Mutex::new(None),
            maintenance: Mutex::new(()),
            backup_thread: Mutex::new(None),
            notify,
        })
    }

    /// 別のスレッドでデータベースを開き（[`AppCore::open`]）、今日の論理日付が変わったかを
    /// [`TODAY_CHECK_INTERVAL`] ごとに確かめ始める（[`AppCore::check_today`]）。呼んだスレッドは
    /// 止めない。アプリの起動時に1回呼ぶ。
    pub fn start(self: &Arc<Self>) {
        let core = Arc::clone(self);
        spawn("yunagi-open", move || core.open());
        let weak = Arc::downgrade(self);
        spawn("yunagi-today", move || watch_today(&weak));
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
        {
            let mut slot = lock(&self.db);
            if !matches!(*slot, Slot::NotFound(_)) {
                return Err(CommandError::InvalidState);
            }
            // 作っている間に、もう一度作ろうとしても断る
            *slot = Slot::Opening;
        }
        self.open_with(Database::create);
        Ok(self.startup())
    }

    fn open_with(self: &Arc<Self>, open: fn(DbLock) -> Result<Database, OpenError>) {
        let slot = open_database(open);
        let ready = matches!(slot, Slot::Open(_));
        *lock(&self.db) = slot;
        self.notify(Notice::Startup(self.startup()));
        if ready {
            self.check_today(now());
        }
    }

    /// データベースを開く処理の状態。
    pub fn startup(&self) -> Startup {
        match &*lock(&self.db) {
            Slot::Opening => Startup::Opening,
            Slot::Open(_) => Startup::Ready,
            Slot::NotFound(nf) => Startup::NotFound {
                path: nf.path.display().to_string(),
                data_dir: nf.data_dir.display().to_string(),
                configured_in: nf.configured_in.as_ref().map(|p| p.display().to_string()),
                data_dir_exists: nf.data_dir_exists,
            },
            Slot::Failed { reason, detail } => Startup::Failed {
                reason: *reason,
                detail: detail.clone(),
            },
            Slot::Closed => Startup::Failed {
                reason: StartupFailure::Storage,
                detail: "the database was closed".into(),
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
    /// - 書き込みに失敗したら（[`CommandError::Storage`]）、次に成功するまで、画面の保存の失敗中と
    ///   して記録する（その間、エージェントの書き込みは `storage_unavailable`。アーキテクチャ設計書
    ///   4.1）。引数の誤りなど、保存先に関係なく失敗したものは数えない。
    pub fn screen_write<T: Changes, E: Into<CommandError>>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, E>,
    ) -> Result<T, CommandError> {
        let result = self.read(f);
        match &result {
            Ok(r) => {
                if r.committed() {
                    self.screen.set_save_failing(false);
                }
                if r.changed() {
                    self.notify_changed(ChangeSource::Screen);
                }
            }
            Err(CommandError::Storage { .. }) => self.screen.set_save_failing(true),
            Err(_) => {}
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
        let agent = Agent {
            client_id,
            ui: &self.screen,
        };
        let result = self.with_db(|db| Ok(f(db.connection(), &agent)))?;
        if result.as_ref().is_ok_and(Changes::changed) {
            self.notify_changed(ChangeSource::Agent);
        }
        Ok(result)
    }

    /// 画面の入力中の状態。画面から知らされたものを [`ScreenState::report`] で置き換える。
    pub fn screen(&self) -> &ScreenState {
        &self.screen
    }

    /// 瞬間 `now` の今日の論理日付を求め、前に確かめたときから変わっていたら
    /// [`Notice::TodayChanged`] を知らせて、その日の定期バックアップを始める。開いた直後の1回目は、
    /// 知らせずにバックアップだけを始める（アーキテクチャ設計書 6.2）。
    ///
    /// データベースを開いていなければ何もしない。
    pub fn check_today(self: &Arc<Self>, now: Timestamp) {
        let Ok(today) = self.read(|conn| day::today(conn, &system_tz(), now)) else {
            return;
        };
        let previous = lock(&self.today).replace(today);
        if previous == Some(today) {
            return;
        }
        if previous.is_some() {
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

    /// `today` の定期バックアップを、別のスレッドで作る（無ければ作る）。
    ///
    /// 写すのはデータベースのファイルを別に開いた接続からなので、作っている間も画面のコマンドは
    /// 待たずに動く（`DailyBackup`）。作れなかったときは、標準エラーに書き、
    /// [`Notice::BackupFailed`] を知らせる（失敗にはせず、次に日付が変わったとき・次の起動で作る）。
    fn start_daily_backup(self: &Arc<Self>, today: LogicalDate) {
        let job = match self.with_db(|db| db.daily_backup().map_err(|e| CommandError::storage(&e)))
        {
            Ok(job) => job,
            Err(CommandError::Storage { detail }) => return self.backup_failed(detail),
            Err(_) => return,
        };
        let core = Arc::clone(self);
        let handle = spawn("yunagi-backup", move || {
            let _maintenance = lock(&core.maintenance);
            if let Err(e) = job.ensure(today) {
                core.backup_failed(chain(&e));
            }
        });
        *lock(&self.backup_thread) = handle;
    }

    fn backup_failed(&self, detail: String) {
        eprintln!("yunagi: the daily backup failed: {detail}");
        self.notify(Notice::BackupFailed(BackupFailed { detail }));
    }

    /// 最後に始めた定期バックアップが終わるまで待つ（テストと、終了の前に使う）。
    pub fn wait_for_backup(&self) {
        let handle = lock(&self.backup_thread).take();
        if let Some(handle) = handle {
            // バックアップのスレッドが止まっても（不具合）、待つ側は続ける
            let _ = handle.join();
        }
    }

    /// データベースを閉じ、ロックを離す（アプリの終了時）。閉じた後のコマンドは
    /// [`CommandError::NotReady`]。
    pub fn close(&self) {
        let slot = std::mem::replace(&mut *lock(&self.db), Slot::Closed);
        if let Slot::Open(db) = slot
            && let Err(e) = db.close()
        {
            eprintln!("yunagi: failed to close the database: {}", chain(&e));
        }
    }

    fn notify_changed(&self, by: ChangeSource) {
        self.notify(Notice::Changed(Changed { by }));
    }

    fn notify(&self, notice: Notice) {
        (self.notify)(notice);
    }
}

/// `weak` のコアが残っている間、[`TODAY_CHECK_INTERVAL`] ごとに今日の論理日付を確かめる。
fn watch_today(weak: &Weak<AppCore>) {
    loop {
        thread::sleep(TODAY_CHECK_INTERVAL);
        let Some(core) = weak.upgrade() else { return };
        core.check_today(now());
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

/// 既定ディレクトリを求めて作り、`yunagi.lock` を取ってから `open` で開く。開けたか、開けなかった
/// 理由の状態を返す。
fn open_database(open: fn(DbLock) -> Result<Database, OpenError>) -> Slot {
    match try_open_database(open) {
        Ok(db) => Slot::Open(db),
        Err(slot) => *slot,
    }
}

fn try_open_database(
    open: fn(DbLock) -> Result<Database, OpenError>,
) -> Result<Database, Box<Slot>> {
    let failed = |reason, e: &(dyn std::error::Error + 'static)| {
        Box::new(Slot::Failed {
            reason,
            detail: chain(e),
        })
    };
    let dir = data_dir::default_dir().map_err(|e| failed(StartupFailure::DefaultDir, &e))?;
    // 既定ディレクトリはアプリの置き場所で、データの保存先ではない（ロック・config.json を置く）。
    // 初めて起動したときは無いので作る（保存先を作らない決まり（6.3）とは別。6章）
    fs::create_dir_all(dir.path()).map_err(|e| {
        failed(
            StartupFailure::DefaultDir,
            &io::Error::new(
                e.kind(),
                format!("failed to create {}: {e}", dir.path().display()),
            ),
        )
    })?;
    let lock = DbLock::acquire(&dir, DEFAULT_LOCK_WAIT).map_err(|e| match e {
        LockError::Busy { .. } => failed(StartupFailure::LockBusy, &e),
        _ => failed(StartupFailure::Lock, &e),
    })?;
    open(lock).map_err(|e| match e {
        OpenError::NotFound(nf) => Box::new(Slot::NotFound(nf)),
        OpenError::DataDir(_) => failed(StartupFailure::Config, &e),
        OpenError::AlreadyExists(_) => failed(StartupFailure::AlreadyExists, &e),
        OpenError::Storage(StorageError::Lock(_)) => failed(StartupFailure::Lock, &e),
        OpenError::Storage(_) => failed(StartupFailure::Storage, &e),
    })
}
