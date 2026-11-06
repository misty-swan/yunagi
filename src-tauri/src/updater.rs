//! 自動更新：新しい版の確認と「更新して再起動」（docs/release/07-build-and-distribution.md 5章、
//! docs/design/03-ui-spec.md 3章・7章）。
//!
//! - 確認は Rust 側だけで行う（Tauri のプラグイン `tauri-plugin-updater` の Rust の API）。画面には、
//!   状態を読む `get_update_state`・イベント `update-state`・`install_update` だけを見せ、プラグインの
//!   JavaScript の API と権限（`capabilities/default.json` の `updater:*`）は渡さない。画面から配布元や
//!   署名の公開鍵を指定できないようにするため。
//! - 通信とインストールは [`Backend`] の後ろに置く。本物は [`PluginBackend`]、試験は偽物を使う。
//! - 更新の自動確認（設定 `update_check`）が無効のときは、起動時も定期の時刻が来ても通信しない。有効かは
//!   確認のたびにデータベースから読み直し（覚えておかない）、データベースを開く前は読めないので、確認しない
//!   （既定は有効なので、読めないうちに確認すると、無効にしている利用者にも通信が出てしまう）。
//! - 公開鍵と配布元は `tauri.conf.json` の `plugins.updater` に書く。空のままなら確認しない（エラーにも
//!   しない。[`is_configured`]）。プラグインを入れられなくても、アプリは起動する（[`install`]）。

use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use serde_json::Value;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::app_core::lock;
use crate::types::UpdateState;

/// 更新内容（`notes`）の最大の文字数。配布元の文なので、長すぎるものは切る。
pub const NOTES_MAX_CHARS: usize = 2000;

/// 確認の間隔（起動時の確認の次から、24 時間ごと）。
pub const CHECK_INTERVAL: Duration = Duration::from_secs(60 * 60 * 24);

/// 設定を読めない間（データベースを開く前）に、読み直す間隔。
pub const NOT_READY_RETRY: Duration = Duration::from_secs(5);

/// 再起動すると知らせてから、データベースを閉じるまでの待ち。画面は知らせを受けると、入力を受け付けなく
/// して、入力中の内容を保存し終える（閉じた後に打たれた文字が消えないように）。
pub const RESTART_GRACE: Duration = Duration::from_millis(1500);

/// 見つかった新しい版。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub version: String,
    /// 更新内容。信用しない文字列（プレーンテキストとして出す）。
    pub notes: Option<String>,
}

/// 更新の確認とインストール。本物（[`PluginBackend`]）と、試験の偽物がある。
pub trait Backend: Send + Sync {
    /// 使えるか：自動更新の対象の配布で、公開鍵と配布元が設定されている。使えないときは、確認もしない。
    fn available(&self) -> bool;
    /// 配布元に新しい版を尋ねる（通信する）。失敗したら理由（ログ用の文）を返す。
    fn check(&self) -> Result<Option<Found>, String>;
    /// 見つけた版 `version` をダウンロードして、署名を検証する（何も入れ替えない）。見つけている版が
    /// `version` と違えば（画面に出していた版と別の版を入れ替えないように）失敗する。`progress` に
    /// 0〜100 の割合（大きさが分からなければ `None`）を知らせる。
    fn download(&self, version: &str, progress: &mut dyn FnMut(Option<u8>)) -> Result<(), String>;
    /// ダウンロードした版を入れ替える。Windows では、入れ替えのためにアプリを終える。
    fn install(&self) -> Result<(), String>;
}

/// 外とのつなぎ目（アプリが渡し、試験は偽物を渡す）。
pub struct Hooks {
    /// 設定 `update_check` を、データベースから読み直す。データベースを開いていなければ `None`。
    pub update_check: Box<dyn Fn() -> Option<bool> + Send + Sync>,
    /// 状態が変わったことを画面に知らせる。
    pub notify: Box<dyn Fn(&UpdateState) + Send + Sync>,
    /// データベースを閉じる（入れ替えの前）。
    pub close: Box<dyn Fn() + Send + Sync>,
    /// アプリを再起動する。
    pub restart: Box<dyn Fn() + Send + Sync>,
}

/// 確認の間隔（試験は短くする）。
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub interval: Duration,
    pub not_ready_retry: Duration,
    /// 再起動すると知らせてから、データベースを閉じるまでの待ち。画面が、その間に入力中の内容を保存し終える。
    pub restart_grace: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            interval: CHECK_INTERVAL,
            not_ready_retry: NOT_READY_RETRY,
            restart_grace: RESTART_GRACE,
        }
    }
}

#[derive(Default)]
struct Schedule {
    stop: bool,
    kick: bool,
}

/// 自動更新の状態と、確認のスレッド（アプリに 1 つ）。
pub struct UpdateService {
    hooks: Hooks,
    timing: Timing,
    backend: Mutex<Option<Arc<dyn Backend>>>,
    state: Mutex<UpdateState>,
    schedule: Mutex<Schedule>,
    wake: Condvar,
}

impl UpdateService {
    /// 確認の手段はまだ無い状態（[`UpdateService::attach`] で使えるようにする）。
    pub fn new(hooks: Hooks) -> Arc<Self> {
        Self::with_timing(hooks, Timing::default())
    }

    pub fn with_timing(hooks: Hooks, timing: Timing) -> Arc<Self> {
        Arc::new(Self {
            hooks,
            timing,
            backend: Mutex::new(None),
            state: Mutex::new(UpdateState::None),
            schedule: Mutex::default(),
            wake: Condvar::new(),
        })
    }

    /// 確認の手段を使えるようにする。
    pub fn attach(&self, backend: Arc<dyn Backend>) {
        *lock(&self.backend) = Some(backend);
    }

    /// いまの状態。
    pub fn state(&self) -> UpdateState {
        lock(&self.state).clone()
    }

    /// 確認のスレッドを始める。始めたらすぐ 1 回確認する（有効で、設定を読めるとき）。
    pub fn start(self: &Arc<Self>) {
        let service = Arc::clone(self);
        if let Err(e) = thread::Builder::new()
            .name("yunagi-update".into())
            .spawn(move || service.run())
        {
            eprintln!("yunagi: failed to start the update thread: {e}");
        }
    }

    /// 確認のスレッドを止める（アプリの終了時）。
    pub fn stop(&self) {
        lock(&self.schedule).stop = true;
        self.wake.notify_all();
    }

    /// 設定 `update_check` が変わった。すぐ確かめ直す（有効にしたときは確認し、無効にしたときは表示を消す）。
    pub fn settings_changed(&self) {
        lock(&self.schedule).kick = true;
        self.wake.notify_all();
    }

    fn run(&self) {
        loop {
            let wait = self.tick();
            let mut schedule = lock(&self.schedule);
            if !schedule.stop && !schedule.kick {
                schedule = self
                    .wake
                    .wait_timeout_while(schedule, wait, |s| !s.stop && !s.kick)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
            if schedule.stop {
                return;
            }
            schedule.kick = false;
        }
    }

    /// 1 回分の確認。次の確認までの間隔を返す。
    ///
    /// 通信するのは、手段が使えて（[`Backend::available`]）、設定が有効だと読めたときだけ。
    pub fn tick(&self) -> Duration {
        let Some(backend) = lock(&self.backend).clone() else {
            return self.timing.interval;
        };
        if !backend.available() {
            return self.timing.interval;
        }
        match (self.hooks.update_check)() {
            // データベースを開く前は読めない。確認せず、読めるまで短い間隔で待つ
            None => return self.timing.not_ready_retry,
            Some(false) => {
                self.clear();
                return self.timing.interval;
            }
            Some(true) => {}
        }
        // ダウンロード中・再起動中は確認しない（表示も、入れ替える版も動かさない）
        if matches!(
            *lock(&self.state),
            UpdateState::Downloading { .. } | UpdateState::Restarting
        ) {
            return self.timing.interval;
        }
        let result = backend.check();
        // 確認している間に無効にされたら、結果を表示に使わない
        if (self.hooks.update_check)() == Some(true) {
            match result {
                Ok(Some(found)) => self.found(found),
                Ok(None) => self.clear(),
                // 確認の失敗は画面に出さない（次の確認でまた試す）
                Err(e) => eprintln!("yunagi: the update check failed: {e}"),
            }
        }
        self.timing.interval
    }

    /// 表示している「新しい版」を消す（ダウンロード中・再起動中は触らない）。
    fn clear(&self) {
        let mut state = lock(&self.state);
        if matches!(
            *state,
            UpdateState::Available { .. } | UpdateState::Failed { .. }
        ) {
            *state = UpdateState::None;
            (self.hooks.notify)(&state);
        }
    }

    fn found(&self, found: Found) {
        let mut state = lock(&self.state);
        if matches!(
            *state,
            UpdateState::Downloading { .. } | UpdateState::Restarting
        ) {
            return;
        }
        // ダウンロードに失敗した版が見つかり続けても、失敗の表示を消さない（利用者が選び直すか、
        // 配布元が取り下げるまで残す）
        if matches!(&*state, UpdateState::Failed { version } if *version == found.version) {
            return;
        }
        let next = UpdateState::Available {
            version: found.version,
            notes: found.notes,
        };
        if *state != next {
            *state = next;
            (self.hooks.notify)(&state);
        }
    }

    fn set(&self, next: UpdateState) {
        let mut state = lock(&self.state);
        *state = next;
        (self.hooks.notify)(&state);
    }

    /// 「更新して再起動」。新しい版があるとき（または前の更新に失敗したとき）だけ動く。
    ///
    /// 順序：ダウンロードして署名を検証する（失敗したら `failed` にして終わる。何も入れ替えていない）
    /// → データベースを閉じる → 入れ替える → 再起動する。入れ替えに失敗したときも、データベースを閉じた後
    /// なので、再起動する（古い版がデータベースを開き直す。選び直せば、もう一度更新できる）。
    ///
    /// 更新前のバックアップは、新しい版がデータベースを開くときのマイグレーション前のバックアップ
    /// （`backups/`。ストレージ設計）が受け持つ。
    pub fn install(&self) {
        let Some(backend) = lock(&self.backend).clone() else {
            return;
        };
        let version = {
            let mut state = lock(&self.state);
            let version = match &*state {
                UpdateState::Available { version, .. } | UpdateState::Failed { version } => {
                    version.clone()
                }
                // ダウンロード中・再起動中の重ねての頼み、見つかっていないときの頼みは何もしない
                _ => return,
            };
            *state = UpdateState::Downloading {
                version: version.clone(),
                progress: None,
            };
            (self.hooks.notify)(&state);
            version
        };
        let mut last = None;
        let downloaded = backend.download(&version, &mut |progress| {
            if progress != last {
                last = progress;
                self.set(UpdateState::Downloading {
                    version: version.clone(),
                    progress,
                });
            }
        });
        if let Err(e) = downloaded {
            eprintln!("yunagi: the update could not be downloaded: {e}");
            self.set(UpdateState::Failed { version });
            return;
        }
        self.set(UpdateState::Restarting);
        thread::sleep(self.timing.restart_grace);
        (self.hooks.close)();
        if let Err(e) = backend.install() {
            eprintln!("yunagi: the update could not be installed: {e}");
        }
        (self.hooks.restart)();
    }
}

/// `tauri.conf.json` の `plugins.updater` に、公開鍵と配布元が書かれているか。どちらかが空なら
/// `false`（確認しない）。
pub fn is_configured(config: Option<&Value>) -> bool {
    let Some(config) = config else {
        return false;
    };
    let pubkey = config
        .get("pubkey")
        .and_then(Value::as_str)
        .is_some_and(|k| !k.trim().is_empty());
    let endpoint = config
        .get("endpoints")
        .and_then(Value::as_array)
        .is_some_and(|e| {
            e.iter()
                .any(|u| u.as_str().is_some_and(|u| !u.trim().is_empty()))
        });
    pubkey && endpoint
}

/// 更新内容を、信用しない文字列として整える：前後の空白を除き、空なら `None`、長ければ
/// [`NOTES_MAX_CHARS`] 字で切る。
pub fn clip_notes(notes: Option<String>) -> Option<String> {
    let notes = notes?;
    let notes = notes.trim();
    if notes.is_empty() {
        return None;
    }
    Some(notes.chars().take(NOTES_MAX_CHARS).collect())
}

/// ダウンロードの割合（0〜100）。大きさが分からない・0 のときは `None`。
pub fn percent(downloaded: u64, total: Option<u64>) -> Option<u8> {
    let total = total.filter(|t| *t > 0)?;
    Some((downloaded.saturating_mul(100) / total).min(100) as u8)
}

/// この配布が自動更新の対象か：開発ビルドは対象外（動かしている実行ファイルを入れ替えないため）。
/// Linux は AppImage だけが対象（`.deb`・`.tar.gz` は再導入で更新する。07 2章）。
fn supported<R: Runtime>(app: &AppHandle<R>) -> bool {
    if cfg!(debug_assertions) {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        app.env().appimage.is_some()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
        true
    }
}

/// 確認の通信の時間切れ。止まった通信で確認のスレッドが長く塞がらないようにする。
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);

/// ダウンロードの通信の時間切れ（大きな配布物でも足りる長さ）。
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60 * 10);

#[derive(Default)]
struct Pending {
    /// 最後の確認で見つけた版。確認のたびに入れ替わる。
    update: Option<Update>,
    /// ダウンロードを始めた版と、そのバイト列。確認が入れ替えても変わらない（ダウンロードや入れ替えの
    /// 途中に確認が重なっても、ダウンロードした版を入れ替えられるように）。
    chosen: Option<Update>,
    bytes: Option<Vec<u8>>,
}

/// プラグインで確認・入れ替えをする本物。
pub struct PluginBackend<R: Runtime> {
    app: AppHandle<R>,
    pending: Mutex<Pending>,
}

impl<R: Runtime> PluginBackend<R> {
    pub fn new(app: AppHandle<R>) -> Self {
        Self {
            app,
            pending: Mutex::default(),
        }
    }
}

impl<R: Runtime> Backend for PluginBackend<R> {
    fn available(&self) -> bool {
        supported(&self.app) && is_configured(self.app.config().plugins.0.get("updater"))
    }

    fn check(&self) -> Result<Option<Found>, String> {
        let updater = self
            .app
            .updater_builder()
            .timeout(CHECK_TIMEOUT)
            .build()
            .map_err(|e| e.to_string())?;
        let update = tauri::async_runtime::block_on(updater.check()).map_err(|e| e.to_string())?;
        lock(&self.pending).update = update.clone();
        Ok(update.map(|u| Found {
            version: u.version,
            notes: clip_notes(u.body),
        }))
    }

    fn download(&self, version: &str, progress: &mut dyn FnMut(Option<u8>)) -> Result<(), String> {
        let update = {
            let mut pending = lock(&self.pending);
            let mut update = pending.update.clone().ok_or("no update has been found")?;
            if update.version != version {
                return Err(format!(
                    "the version found ({}) is not the one shown ({version})",
                    update.version
                ));
            }
            update.timeout = Some(DOWNLOAD_TIMEOUT);
            pending.chosen = Some(update.clone());
            pending.bytes = None;
            update
        };
        let mut downloaded = 0u64;
        let bytes = tauri::async_runtime::block_on(update.download(
            |chunk, total| {
                downloaded += chunk as u64;
                progress(percent(downloaded, total));
            },
            || {},
        ))
        .map_err(|e| e.to_string())?;
        lock(&self.pending).bytes = Some(bytes);
        Ok(())
    }

    fn install(&self) -> Result<(), String> {
        let (update, bytes) = {
            let mut pending = lock(&self.pending);
            (pending.chosen.clone(), pending.bytes.take())
        };
        let (Some(update), Some(bytes)) = (update, bytes) else {
            return Err("the update has not been downloaded".into());
        };
        update.install(bytes).map_err(|e| e.to_string())
    }
}

/// プラグインを入れて、確認の手段を使えるようにし、確認のスレッドを始める。入れられなくても、アプリは
/// 起動を続ける（更新の確認だけが使えなくなる）。
pub fn install<R: Runtime>(app: &AppHandle<R>) {
    match app.plugin(tauri_plugin_updater::Builder::new().build()) {
        Ok(()) => {
            let service = app.state::<Arc<UpdateService>>();
            service.attach(Arc::new(PluginBackend::new(app.clone())));
            service.start();
        }
        Err(e) => eprintln!("yunagi: updates are unavailable: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::json;

    use super::*;

    /// 偽の配布元：呼ばれた回数を数え、決めた答えを返す。
    struct Fake {
        available: bool,
        found: Mutex<Result<Option<Found>, String>>,
        checks: AtomicUsize,
        downloads: AtomicUsize,
        /// ダウンロードを頼まれた版（画面に出していた版を指しているか）。
        versions: Mutex<Vec<String>>,
        download_result: Mutex<Result<(), String>>,
        install_result: Mutex<Result<(), String>>,
        log: Arc<Mutex<Vec<&'static str>>>,
    }

    impl Fake {
        fn new(log: Arc<Mutex<Vec<&'static str>>>) -> Arc<Self> {
            Arc::new(Self {
                available: true,
                found: Mutex::new(Ok(Some(found("1.2.0")))),
                checks: AtomicUsize::new(0),
                downloads: AtomicUsize::new(0),
                versions: Mutex::default(),
                download_result: Mutex::new(Ok(())),
                install_result: Mutex::new(Ok(())),
                log,
            })
        }

        fn checks(&self) -> usize {
            self.checks.load(Ordering::SeqCst)
        }
    }

    impl Backend for Fake {
        fn available(&self) -> bool {
            self.available
        }

        fn check(&self) -> Result<Option<Found>, String> {
            self.checks.fetch_add(1, Ordering::SeqCst);
            self.found.lock().unwrap().clone()
        }

        fn download(
            &self,
            version: &str,
            progress: &mut dyn FnMut(Option<u8>),
        ) -> Result<(), String> {
            self.versions.lock().unwrap().push(version.to_owned());
            self.downloads.fetch_add(1, Ordering::SeqCst);
            self.log.lock().unwrap().push("download");
            progress(Some(40));
            progress(Some(40));
            progress(Some(100));
            self.download_result.lock().unwrap().clone()
        }

        fn install(&self) -> Result<(), String> {
            self.log.lock().unwrap().push("install");
            self.install_result.lock().unwrap().clone()
        }
    }

    fn found(version: &str) -> Found {
        Found {
            version: version.into(),
            notes: Some("fixes".into()),
        }
    }

    struct Rig {
        service: Arc<UpdateService>,
        fake: Arc<Fake>,
        update_check: Arc<Mutex<Option<bool>>>,
        notices: Arc<Mutex<Vec<UpdateState>>>,
        log: Arc<Mutex<Vec<&'static str>>>,
    }

    fn rig(update_check: Option<bool>, timing: Timing) -> Rig {
        let log = Arc::new(Mutex::new(Vec::new()));
        let setting = Arc::new(Mutex::new(update_check));
        let notices = Arc::new(Mutex::new(Vec::new()));
        let hooks = Hooks {
            update_check: {
                let s = Arc::clone(&setting);
                Box::new(move || *s.lock().unwrap())
            },
            notify: {
                let n = Arc::clone(&notices);
                Box::new(move |state| n.lock().unwrap().push(state.clone()))
            },
            close: {
                let l = Arc::clone(&log);
                Box::new(move || l.lock().unwrap().push("close"))
            },
            restart: {
                let l = Arc::clone(&log);
                Box::new(move || l.lock().unwrap().push("restart"))
            },
        };
        // 試験は、再起動の前の待ちを無くす
        let timing = Timing {
            restart_grace: Duration::ZERO,
            ..timing
        };
        let service = UpdateService::with_timing(hooks, timing);
        let fake = Fake::new(Arc::clone(&log));
        service.attach(fake.clone());
        Rig {
            service,
            fake,
            update_check: setting,
            notices,
            log,
        }
    }

    fn quick() -> Timing {
        Timing {
            interval: Duration::from_millis(20),
            not_ready_retry: Duration::from_millis(10),
            restart_grace: Duration::ZERO,
        }
    }

    fn available(version: &str) -> UpdateState {
        UpdateState::Available {
            version: version.into(),
            notes: Some("fixes".into()),
        }
    }

    #[test]
    fn a_disabled_setting_never_reaches_the_network() {
        let rig = rig(Some(false), Timing::default());
        for _ in 0..3 {
            assert_eq!(rig.service.tick(), CHECK_INTERVAL);
        }
        assert_eq!(rig.fake.checks(), 0);
        assert_eq!(rig.service.state(), UpdateState::None);
    }

    #[test]
    fn nothing_is_checked_before_the_setting_can_be_read() {
        let rig = rig(None, Timing::default());
        assert_eq!(rig.service.tick(), NOT_READY_RETRY);
        assert_eq!(rig.fake.checks(), 0);
        // 読めるようになり、有効なら、次の確認で通信する
        *rig.update_check.lock().unwrap() = Some(true);
        rig.service.tick();
        assert_eq!(rig.fake.checks(), 1);
    }

    #[test]
    fn an_unconfigured_or_unsupported_distribution_never_reaches_the_network() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut fake = Arc::try_unwrap(Fake::new(log)).ok().unwrap();
        fake.available = false;
        let fake = Arc::new(fake);
        let rig = rig(Some(true), Timing::default());
        rig.service.attach(fake.clone());
        assert_eq!(rig.service.tick(), CHECK_INTERVAL);
        assert_eq!(fake.checks(), 0);
        assert_eq!(rig.service.state(), UpdateState::None);
    }

    #[test]
    fn an_enabled_setting_checks_once_per_tick_and_shows_the_new_version() {
        let rig = rig(Some(true), Timing::default());
        rig.service.tick();
        assert_eq!(rig.fake.checks(), 1);
        assert_eq!(rig.service.state(), available("1.2.0"));
        // 同じ版が見つかり続けても、知らせは 1 回だけ
        rig.service.tick();
        assert_eq!(rig.fake.checks(), 2);
        assert_eq!(*rig.notices.lock().unwrap(), [available("1.2.0")]);
    }

    #[test]
    fn a_failed_check_shows_nothing_and_keeps_what_was_shown() {
        let rig = rig(Some(true), Timing::default());
        *rig.fake.found.lock().unwrap() = Err("offline".into());
        rig.service.tick();
        assert_eq!(rig.service.state(), UpdateState::None);
        *rig.fake.found.lock().unwrap() = Ok(Some(found("1.2.0")));
        rig.service.tick();
        *rig.fake.found.lock().unwrap() = Err("offline".into());
        rig.service.tick();
        assert_eq!(rig.service.state(), available("1.2.0"));
    }

    #[test]
    fn disabling_hides_the_notice_and_stops_checking() {
        let rig = rig(Some(true), Timing::default());
        rig.service.tick();
        assert_eq!(rig.service.state(), available("1.2.0"));
        *rig.update_check.lock().unwrap() = Some(false);
        rig.service.tick();
        assert_eq!(rig.service.state(), UpdateState::None);
        assert_eq!(rig.fake.checks(), 1);
        rig.service.tick();
        assert_eq!(rig.fake.checks(), 1);
    }

    #[test]
    fn a_withdrawn_release_clears_the_notice() {
        let rig = rig(Some(true), Timing::default());
        rig.service.tick();
        *rig.fake.found.lock().unwrap() = Ok(None);
        rig.service.tick();
        assert_eq!(rig.service.state(), UpdateState::None);
    }

    fn wait_for(what: &str, f: impl Fn() -> bool) {
        for _ in 0..200 {
            if f() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for {what}");
    }

    #[test]
    fn the_thread_checks_at_start_again_on_the_interval_and_stops() {
        let rig = rig(Some(true), quick());
        rig.service.start();
        wait_for("three checks", || rig.fake.checks() >= 3);
        rig.service.stop();
        // 止めた後は数が増えない（走っていた 1 回が終わるのを待つ）
        thread::sleep(Duration::from_millis(60));
        let after = rig.fake.checks();
        thread::sleep(Duration::from_millis(80));
        assert_eq!(rig.fake.checks(), after);
    }

    #[test]
    fn the_thread_waits_for_the_database_and_never_checks_while_disabled() {
        let rig = rig(None, quick());
        rig.service.start();
        thread::sleep(Duration::from_millis(80));
        assert_eq!(rig.fake.checks(), 0);
        *rig.update_check.lock().unwrap() = Some(false);
        thread::sleep(Duration::from_millis(80));
        assert_eq!(rig.fake.checks(), 0);
        rig.service.stop();
    }

    #[test]
    fn enabling_the_setting_checks_right_away() {
        let rig = rig(
            Some(false),
            Timing {
                interval: Duration::from_secs(3600),
                not_ready_retry: Duration::from_secs(3600),
                restart_grace: Duration::ZERO,
            },
        );
        rig.service.start();
        thread::sleep(Duration::from_millis(50));
        assert_eq!(rig.fake.checks(), 0);
        *rig.update_check.lock().unwrap() = Some(true);
        rig.service.settings_changed();
        wait_for("the check after enabling", || rig.fake.checks() == 1);
        rig.service.stop();
    }

    #[test]
    fn installing_downloads_then_closes_the_database_then_installs_then_restarts() {
        let rig = rig(Some(true), Timing::default());
        rig.service.tick();
        rig.notices.lock().unwrap().clear();
        rig.service.install();
        assert_eq!(
            *rig.log.lock().unwrap(),
            ["download", "close", "install", "restart"]
        );
        let version = || "1.2.0".to_string();
        assert_eq!(
            *rig.notices.lock().unwrap(),
            [
                UpdateState::Downloading {
                    version: version(),
                    progress: None
                },
                UpdateState::Downloading {
                    version: version(),
                    progress: Some(40)
                },
                UpdateState::Downloading {
                    version: version(),
                    progress: Some(100)
                },
                UpdateState::Restarting,
            ]
        );
    }

    #[test]
    fn a_failed_download_leaves_the_app_running_and_can_be_retried() {
        let rig = rig(Some(true), Timing::default());
        rig.service.tick();
        *rig.fake.download_result.lock().unwrap() = Err("bad signature".into());
        rig.service.install();
        assert_eq!(
            rig.service.state(),
            UpdateState::Failed {
                version: "1.2.0".into()
            }
        );
        // データベースを閉じず、入れ替えも再起動もしない
        assert_eq!(*rig.log.lock().unwrap(), ["download"]);
        *rig.fake.download_result.lock().unwrap() = Ok(());
        rig.service.install();
        assert_eq!(rig.fake.downloads.load(Ordering::SeqCst), 2);
        // 頼むのは、画面に出していた版
        assert_eq!(*rig.fake.versions.lock().unwrap(), ["1.2.0", "1.2.0"]);
        assert_eq!(rig.log.lock().unwrap().last(), Some(&"restart"));
    }

    #[test]
    fn a_failure_stays_shown_while_the_same_version_is_found_and_goes_when_withdrawn() {
        let rig = rig(Some(true), Timing::default());
        rig.service.tick();
        *rig.fake.download_result.lock().unwrap() = Err("offline".into());
        rig.service.install();
        let failed = UpdateState::Failed {
            version: "1.2.0".into(),
        };
        rig.service.tick();
        assert_eq!(rig.service.state(), failed);
        // 別の版が見つかったら、その版の通知に替わる
        *rig.fake.found.lock().unwrap() = Ok(Some(found("1.3.0")));
        rig.service.tick();
        assert_eq!(rig.service.state(), available("1.3.0"));
        // 取り下げられたら消える
        *rig.fake.found.lock().unwrap() = Ok(None);
        rig.service.tick();
        assert_eq!(rig.service.state(), UpdateState::None);
    }

    #[test]
    fn a_failed_install_restarts_the_old_version() {
        let rig = rig(Some(true), Timing::default());
        rig.service.tick();
        *rig.fake.install_result.lock().unwrap() = Err("denied".into());
        rig.service.install();
        assert_eq!(
            *rig.log.lock().unwrap(),
            ["download", "close", "install", "restart"]
        );
    }

    #[test]
    fn installing_without_a_new_version_does_nothing() {
        let rig = rig(Some(true), Timing::default());
        rig.service.install();
        assert!(rig.log.lock().unwrap().is_empty());
        // 重ねての頼み（ダウンロード中・再起動中）も何もしない
        rig.service.tick();
        rig.service.set(UpdateState::Restarting);
        rig.service.install();
        assert!(rig.log.lock().unwrap().is_empty());
    }

    #[test]
    fn a_check_does_not_overwrite_a_download_in_progress() {
        let rig = rig(Some(true), Timing::default());
        rig.service.tick();
        let downloading = UpdateState::Downloading {
            version: "1.2.0".into(),
            progress: Some(10),
        };
        rig.service.set(downloading.clone());
        // ダウンロード中は、確認の通信もしない（入れ替える版を動かさないため）
        rig.service.tick();
        assert_eq!(rig.fake.checks(), 1);
        assert_eq!(rig.service.state(), downloading);
        *rig.update_check.lock().unwrap() = Some(false);
        rig.service.tick();
        assert_eq!(rig.service.state(), downloading);
    }

    #[test]
    fn configuration_needs_both_a_public_key_and_an_endpoint() {
        assert!(!is_configured(None));
        assert!(!is_configured(Some(
            &json!({"pubkey": "", "endpoints": []})
        )));
        assert!(!is_configured(Some(
            &json!({"pubkey": "k", "endpoints": []})
        )));
        assert!(!is_configured(Some(
            &json!({"pubkey": " ", "endpoints": ["https://example.test/latest.json"]})
        )));
        assert!(!is_configured(Some(
            &json!({"pubkey": "", "endpoints": ["https://example.test/latest.json"]})
        )));
        assert!(!is_configured(Some(
            &json!({"pubkey": "k", "endpoints": [""]})
        )));
        assert!(is_configured(Some(
            &json!({"pubkey": "k", "endpoints": ["https://example.test/latest.json"]})
        )));
    }

    /// 配る設定ファイルの `plugins.updater` を、プラグインが読める（読めないとプラグインの初期化に
    /// 失敗し、更新が使えなくなる。窓の無い試験はプラグインを入れないので、ここで確かめる）。
    /// 署名に版を入れさせる設定と、Windows の入れ替え方も確かめる。
    #[test]
    fn the_shipped_updater_config_is_readable_by_the_plugin() {
        let conf: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let updater = conf["plugins"]["updater"].clone();
        let config: tauri_plugin_updater::Config = serde_json::from_value(updater).unwrap();
        assert!(config.require_signed_version);
        assert!(!config.allow_downgrades);
        assert!(!config.dangerous_insecure_transport_protocol);
        assert!(!config.dangerous_accept_invalid_certs);
        assert!(!config.dangerous_accept_invalid_hostnames);
        assert!(config.windows.is_some());
        // 設定があるなら、配布元は https だけ（プラグインの読み込みが検査する）
        for url in &config.endpoints {
            assert_eq!(url.scheme(), "https");
        }
    }

    #[test]
    fn notes_are_trimmed_and_capped() {
        assert_eq!(clip_notes(None), None);
        assert_eq!(clip_notes(Some("  \n ".into())), None);
        assert_eq!(clip_notes(Some(" a ".into())), Some("a".into()));
        let long = "あ".repeat(NOTES_MAX_CHARS + 10);
        assert_eq!(
            clip_notes(Some(long)).unwrap().chars().count(),
            NOTES_MAX_CHARS
        );
    }

    #[test]
    fn download_progress_is_a_percentage_or_unknown() {
        assert_eq!(percent(50, Some(200)), Some(25));
        assert_eq!(percent(300, Some(200)), Some(100));
        assert_eq!(percent(50, None), None);
        assert_eq!(percent(50, Some(0)), None);
    }
}
