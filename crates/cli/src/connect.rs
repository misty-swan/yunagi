//! 接続の切り替え：ローカル API と直接モードのどちらで操作を処理するかを決めて処理する
//! （docs/design/01-architecture.md 4.4・4.5・4.6、docs/design/05-mcp-cli-spec.md 4.3）。
//!
//! CLI と `yunagi mcp`（同じバイナリ）が同じ [`Connector`] を使う。MCP サーバーは常駐するので、操作ごとに
//! [`Connector::call`] を呼び、そのたびに選び直す（接続を使い回さない）。
//!
//! 手順（操作ごと）：次を最大 5 秒、50 ミリ秒ごとに繰り返す。
//!
//! 1. ローカル API に接続して認証する。つながれば以後はこの接続で行う。待ち受けが無ければ 2 へ。一時的に
//!    つながらない（または認証に答えない）なら、直接モードに切り替えず、次の繰り返しで試し直す。待ち受けは
//!    あるが使えない（認証の失敗・版の不一致・権限）なら `api_error`。
//! 2. `yunagi.lock` を待たずに 1 回だけ取ってみる。取れたら、別の候補の既定ディレクトリを 1 回だけ調べ
//!    （[`yunagi_core::storage::dir_mismatch`]）、問題が無ければ直接モードで処理する。取れなければ、期限なら
//!    `busy`、そうでなければ 1 に戻る。
//!
//! 依頼を送った後に接続が切れたら、書き込みは送り直さず `result_unknown`、読み取りは 1 回だけ手順 1 から
//! やり直す。応答を待つ時間には上限を設けない（上限で切って送り直すと、反映済みの書き込みが二重になる）。

use std::cell::Cell;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};
use yunagi_api::client::{self, CallError, HelloError};
use yunagi_api::ops::{self, Call, Failure};
use yunagi_core::domain::agent::Agent;
use yunagi_core::error::ErrorChain;
use yunagi_core::storage::data_dir::{self, DataDirError, DefaultDir, Platform};
use yunagi_core::storage::dir_mismatch::{self, Candidate, CandidateInfo, Judgement, OwnStorage};
use yunagi_core::storage::{DEFAULT_LOCK_WAIT, Database, DbLock, LockError, OpenError};

use crate::error::{self, ErrorObject};

/// 接続の手順の時間（試験で短くできる）。
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// 接続とロックを待つ期限（4.4 の 5 秒）。
    pub wait: Duration,
    /// 試し直しの間隔（50 ミリ秒）。
    pub interval: Duration,
    /// 1 回の接続と認証の上限（1 秒）。期限の残りが短ければそちらまで。
    pub connect: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            wait: DEFAULT_LOCK_WAIT,
            interval: Duration::from_millis(50),
            connect: yunagi_api::CLIENT_CONNECT_TIMEOUT,
        }
    }
}

/// 別の候補の既定ディレクトリを求めるためのホーム（OS に尋ねる。試験では差し替える）。
#[derive(Debug, Clone)]
pub enum AltHome {
    /// OS に尋ねる（`getpwuid`・既知のフォルダの API）。本番はこれ。
    Os,
    /// 決めた値（試験が、本物のホームではなく一時フォルダで確かめるため）。
    Fixed(Option<PathBuf>),
}

/// 接続の手順を行う道具。
pub struct Connector {
    dir: DefaultDir,
    client: String,
    alt_home: AltHome,
    timing: Timing,
    warn: Box<dyn Fn(&str) + Send + Sync>,
    warned: AtomicBool,
}

impl Connector {
    /// 既定ディレクトリ `dir` で、クライアントの識別子 `client`（CLI は `cli`、MCP は `--client` の値。
    /// 無ければ `unknown`）として接続する。`warn` は、環境を分けている形（4.5 の 3）のときの警告を
    /// 出す先（プロセスごとに 1 回）。
    pub fn new(
        dir: DefaultDir,
        client: impl Into<String>,
        alt_home: AltHome,
        warn: impl Fn(&str) + Send + Sync + 'static,
    ) -> Self {
        Self {
            dir,
            client: client.into(),
            alt_home,
            timing: Timing::default(),
            warn: Box::new(warn),
            warned: AtomicBool::new(false),
        }
    }

    /// 動いている環境の既定ディレクトリ（環境変数から求める）で作る。求められなければ
    /// `storage_unavailable`（求めた環境変数の名前つき）。
    pub fn from_env(
        client: impl Into<String>,
        warn: impl Fn(&str) + Send + Sync + 'static,
    ) -> Result<Self, ErrorObject> {
        match data_dir::default_dir() {
            Ok(dir) => Ok(Self::new(dir, client, AltHome::Os, warn)),
            Err(e) => Err(default_dir_failure(&e)),
        }
    }

    /// 別の候補を求めるホームを変える（試験）。
    pub fn with_alt_home(mut self, alt_home: AltHome) -> Self {
        self.alt_home = alt_home;
        self
    }

    /// 手順の時間を変える（試験）。
    pub fn with_timing(mut self, timing: Timing) -> Self {
        self.timing = timing;
        self
    }

    /// 自分の既定ディレクトリ。
    pub fn default_dir(&self) -> &DefaultDir {
        &self.dir
    }

    /// 別の候補の既定ディレクトリ（`doctor` も使う）。
    pub fn candidate(&self) -> Option<Candidate> {
        match &self.alt_home {
            AltHome::Os => dir_mismatch::alternative(&self.dir),
            AltHome::Fixed(home) => {
                dir_mismatch::alternative_from_home(&self.dir, Platform::current(), home.clone())
            }
        }
    }

    pub(crate) fn timing(&self) -> Timing {
        self.timing
    }

    pub(crate) fn client(&self) -> &str {
        &self.client
    }

    /// 操作 `method` を、引数 `params`（05 4.4 の引数の名前）で行い、成功の返り値（05 4.2・4.4）か、
    /// エラー（05 4.3）を返す。引数の誤り（ID・日付の形など）は、接続する前に `invalid_argument` にする。
    pub fn call(&self, method: &str, params: Map<String, Value>) -> Result<Value, ErrorObject> {
        let call = match Call::parse(method, &params) {
            Ok(call) => call,
            Err(Failure::App(error)) => return Err(error),
            Err(Failure::Params(message)) => return Err(error::invalid_argument(message)),
            Err(Failure::NoMethod) => {
                return Err(error::internal(format!("unknown operation {method}")));
            }
        };
        let is_read = matches!(call, Call::Read(_));
        let params = Value::Object(params);
        let mut retried = false;
        loop {
            let deadline = Instant::now() + self.timing.wait;
            match self.attempt(method, &params, &call, deadline) {
                Attempt::Done(result) => return result,
                // 読み取りは何も変えないので、手順 1 から 1 回だけやり直す
                Attempt::ConnectionLost if is_read && !retried => retried = true,
                Attempt::ConnectionLost => return Err(error::result_unknown()),
            }
        }
    }

    /// 手順 1・2 を、期限 `deadline` まで繰り返す。
    fn attempt(&self, method: &str, params: &Value, call: &Call, deadline: Instant) -> Attempt {
        let mut socket_path_too_long = false;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let connect_within = left.min(self.timing.connect);
            match client::connect(&self.dir, Some(&self.client), connect_within) {
                Ok(session) => {
                    return match session.call(method, params) {
                        Ok(value) => Attempt::Done(Ok(value)),
                        Err(CallError::Rpc(e)) => Attempt::Done(Err(rpc_failure(&e))),
                        Err(CallError::ConnectionLost) => Attempt::ConnectionLost,
                        // 依頼を送る前の失敗：何も送っていないので、手順 1 の結果の扱いに従う
                        Err(CallError::NotSent(_)) => {
                            if self.wait_or_busy(deadline) {
                                continue;
                            }
                            Attempt::Done(Err(error::busy()))
                        }
                        Err(CallError::Protocol(message)) => {
                            eprintln!("yunagi: unexpected response from Yunagi: {message}");
                            Attempt::Done(Err(error::api_error(
                                "unexpected",
                                "Yunagi's response could not be understood. Ask the user to update Yunagi and the yunagi command to the same version.",
                            )))
                        }
                    };
                }
                Err(HelloError::NoListener) => {}
                Err(HelloError::SocketPathTooLong(_)) => socket_path_too_long = true,
                // 待ち受けはあるが一時的につながらない・認証に答えない：直接モードには切り替えない
                Err(HelloError::Transient(_) | HelloError::NotAnswering) => {
                    if self.wait_or_busy(deadline) {
                        continue;
                    }
                    return Attempt::Done(Err(error::busy()));
                }
                Err(HelloError::Denied(_)) => return Attempt::Done(Err(error::denied())),
                Err(HelloError::AuthFailed) => return Attempt::Done(Err(error::auth_failed())),
                Err(HelloError::VersionMismatch { .. }) => {
                    return Attempt::Done(Err(error::version_mismatch()));
                }
                Err(HelloError::Io(e)) => {
                    eprintln!(
                        "yunagi: could not talk to the local API: {}",
                        ErrorChain(&e)
                    );
                    return Attempt::Done(Err(error::api_error(
                        "unexpected",
                        "Could not talk to Yunagi's local API. Nothing was changed. Ask the user to restart Yunagi.",
                    )));
                }
            }
            // 手順 2：ロックを待たずに 1 回だけ取ってみる
            match DbLock::acquire(&self.dir, Duration::ZERO) {
                Ok(lock) => return Attempt::Done(self.direct(lock, call)),
                Err(LockError::Busy { .. }) => {
                    if socket_path_too_long {
                        // 待っても直らず、`busy` の「再起動して」の案内でも直らない
                        return Attempt::Done(Err(error::socket_path_too_long()));
                    }
                    if !self.wait_or_busy(deadline) {
                        return Attempt::Done(Err(error::busy()));
                    }
                }
                Err(LockError::Io { source, .. }) => {
                    return Attempt::Done(Err(self.lock_failure(&source)));
                }
            }
        }
    }

    /// 期限までなら、試し直しの間隔だけ待って `true`。期限を過ぎていれば `false`。
    fn wait_or_busy(&self, deadline: Instant) -> bool {
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        thread::sleep(self.timing.interval.min(deadline - now));
        true
    }

    /// 自分の既定ディレクトリのロックが開けなかった。既定ディレクトリが無いときは、別の候補を調べて
    /// から `storage_unavailable`（4.5 の 4）。そのほか（権限など）は、調べずに読み書きの失敗。
    fn lock_failure(&self, source: &io::Error) -> ErrorObject {
        let missing = matches!(
            source.kind(),
            io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
        ) && !self.dir.path().is_dir();
        if !missing {
            eprintln!("yunagi: could not lock yunagi.lock: {}", ErrorChain(source));
            return error::storage_failed();
        }
        match self.judge() {
            (_, Some(info), Judgement::Mismatch { .. }) => {
                error::default_dir_mismatch(self.dir.path(), &info.dir)
            }
            _ => error::default_dir_missing(
                &self.dir,
                dir_mismatch::default_dir_vars(Platform::current()),
            ),
        }
    }

    /// 別の候補を調べて、4.5 の判定を行う（操作ごとに 1 回）。
    fn judge(&self) -> (OwnStorage, Option<CandidateInfo>, Judgement) {
        let own = dir_mismatch::own_storage(&self.dir);
        let info = self.candidate().map(|c| c.probe());
        let judgement = dir_mismatch::judge(&own, info.as_ref());
        (own, info, judgement)
    }

    /// ロックを取れたので、直接モード（アーキテクチャ設計書 4章 2）で処理する。
    fn direct(&self, lock: DbLock, call: &Call) -> Result<Value, ErrorObject> {
        let (own, info, judgement) = self.judge();
        match (judgement, &info) {
            (Judgement::Mismatch { .. }, Some(info)) => {
                // ロックを離し、データベースを開かず、何も書かない
                drop(lock);
                return Err(error::default_dir_mismatch(self.dir.path(), &info.dir));
            }
            // 環境を分けている形：止めず、プロセスごとに 1 回だけ警告する
            (Judgement::Separate, Some(info)) if !self.warned.swap(true, Ordering::SeqCst) => {
                (self.warn)(&format!(
                    "yunagi: Yunagi is also running with another data folder ({}); this command uses {}",
                    info.dir.display(),
                    self.dir.path().display()
                ));
            }
            _ => {}
        }
        // CLI・MCP は、データベースを新しく作らない（6.3。作るのはアプリだけ）
        // 保存先を求められないとき（`config.json` が壊れている・読めない）も、開かずに止める
        let Some(data_dir) = &own.data_dir else {
            drop(lock);
            if let Err(e) = data_dir::resolve_data_dir(&self.dir) {
                eprintln!("yunagi: could not find the data folder: {}", ErrorChain(&e));
            }
            return Err(error::storage_failed());
        };
        if !own.has_database {
            drop(lock);
            return Err(error::database_not_found(&data_dir::database_path(
                data_dir,
            )));
        }
        let db = match Database::open(lock) {
            Ok(db) => db,
            Err(OpenError::NotFound(nf)) => return Err(error::database_not_found(&nf.path)),
            Err(e) => {
                eprintln!("yunagi: could not open the database: {}", ErrorChain(&e));
                return Err(error::storage_failed());
            }
        };
        let result = self.run_direct(&db, call);
        let closed = db.close();
        match (result, closed) {
            (Ok(value), Ok(())) => Ok(value),
            (Ok(_), Err(e)) => {
                eprintln!("yunagi: could not close the database: {}", ErrorChain(&e));
                Err(error::storage_failed())
            }
            (Err(e), _) => Err(e),
        }
    }

    fn run_direct(&self, db: &Database, call: &Call) -> Result<Value, ErrorObject> {
        let conn = db.connection();
        let registered = Cell::new(false);
        let client_id =
            ops::resolve_client(conn, Some(&self.client), &registered).map_err(failure_error)?;
        match call {
            Call::Read(read) => read.execute(conn, client_id).map_err(failure_error),
            Call::Write(write) => {
                let agent = Agent::without_ui(client_id);
                write
                    .execute(conn, &agent, &ops::system_tz(), ops::now())
                    .map(|reply| reply.value)
                    .map_err(failure_error)
            }
        }
    }
}

/// 1 回の試行の結果。
enum Attempt {
    Done(Result<Value, ErrorObject>),
    /// 依頼を送った後、応答の前に接続が切れた。
    ConnectionLost,
}

/// 操作の失敗（`ops::Failure`）をエラーのオブジェクトにする。
fn failure_error(failure: Failure) -> ErrorObject {
    match failure {
        Failure::App(error) => error,
        Failure::Params(message) => error::invalid_argument(message),
        Failure::NoMethod => error::internal("unknown operation"),
    }
}

/// サーバーが返した失敗（JSON-RPC の `error`）をエラーのオブジェクトにする。アプリのエラー
/// （`data.error`）はそのまま返す。
fn rpc_failure(e: &client::RpcError) -> ErrorObject {
    match e.app_error() {
        Some(app) => app.clone(),
        None => error::internal(format!("Yunagi rejected the request: {}", e.message)),
    }
}

/// 既定ディレクトリを求められなかった。
fn default_dir_failure(e: &DataDirError) -> ErrorObject {
    let vars = dir_mismatch::default_dir_vars(Platform::current());
    match e {
        DataDirError::NoDefaultDir { .. } => error::default_dir_unset(vars),
        _ => {
            eprintln!("yunagi: {}", ErrorChain(e));
            error::storage_failed()
        }
    }
}
