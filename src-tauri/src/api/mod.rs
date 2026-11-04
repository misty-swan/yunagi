//! ローカル API の待ち受け（アーキテクチャ設計書 4.4・4.6、MCP・CLI 仕様書 4.4）。
//!
//! アプリがロックを持ち、データベースを開き終えている間だけ、UNIX ドメインソケット（Windows は
//! 名前付きパイプ）で待ち受ける。接続ごとに、`hello`（版とトークンの確かめ）→ 依頼 1 つ → 応答 1 つ
//! で閉じる。依頼は [`dispatch`] が `AppCore` の入口（[`crate::AppCore::api_write`] など）へ渡す。
//! 通信路・トークン・プロトコルの型は共通のクレート `yunagi-api` にある。
//!
//! Tauri には依存しない。始める・止めるのは `AppCore`（`start_api`・`stop_api`）。

pub mod dispatch;

use std::cell::Cell;
use std::collections::HashMap;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use yunagi_api::token;
use yunagi_api::transport::{Interrupter, ListenError, Listener, Stream};
use yunagi_api::wire::{self, HelloParams, HelloResult, LineError};
use yunagi_api::{MAX_HELLO_LINE, MAX_REQUEST_LINE, PROTOCOL_VERSION};
use yunagi_core::domain::agent;
use yunagi_core::storage::DEFAULT_LOCK_WAIT;
use yunagi_core::storage::data_dir::DefaultDir;

use crate::app_core::AppCore;

/// 同時に受ける接続の数の上限。超えた接続は、何も読まずに閉じる（クライアントは試し直す）。
const MAX_CONNECTIONS: usize = 64;
/// 止めるとき、処理中の依頼が終わるのを待つ時間の上限。超えたら待たずに続ける（アプリの終了を
/// 止めない）。依頼の待ちは [`ApiOptions::db_wait`] までなので、ふつうはこれより前に終わる。
const DRAIN_LIMIT: Duration = Duration::from_secs(15);
/// 止めるとき、待ち受けのスレッドが終わるまで、起こし直す間隔。
const WAKE_RETRY: Duration = Duration::from_millis(200);
/// 止めるとき、待ち受けのスレッドが終わるのを待つ時間の上限。
const STOP_LIMIT: Duration = Duration::from_secs(20);

/// ローカル API の待ち時間の設定。既定は設計書 4.4 の値。試験で短くするために変えられる
/// （[`AppCore::set_api_options`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiOptions {
    /// データベースを使えるまで待つ時間の上限（保存先の変更・復元の最中の依頼。超えたら `busy`）。
    pub db_wait: Duration,
    /// 接続を受けてから、`hello` を受け取るまで待つ時間の上限。
    pub hello_timeout: Duration,
    /// `hello` の後、依頼の 1 行を受け取るまで待つ時間の上限。
    pub request_timeout: Duration,
    /// 応答を書く（相手が読む）のを待つ時間の上限。
    pub write_timeout: Duration,
}

impl Default for ApiOptions {
    fn default() -> Self {
        Self {
            db_wait: DEFAULT_LOCK_WAIT,
            hello_timeout: Duration::from_secs(3),
            request_timeout: Duration::from_secs(5),
            write_timeout: Duration::from_secs(5),
        }
    }
}

/// ローカル API の状態（[`AppCore::api_state`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiState {
    /// 待ち受けていない（データベースを開いていない間、閉じた後）。
    Stopped,
    /// 待ち受けている。`connections` は、いま処理中の接続の数。
    ///
    /// 接続は 1 依頼で閉じるので、ふつうはほとんど 0。「接続中のクライアントの一覧」は出さない
    /// （トークンを持つ相手に、ほかのクライアントを知らせる理由が無いため）。クライアントごとの
    /// 最後の接続は agent_client の `last_seen_at`（設定画面のエージェント区分が読む）。
    Listening { connections: usize },
    /// 待ち受けようとして、始められなかった。
    Unavailable(ApiUnavailable),
}

/// 待ち受けを始められなかった理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiUnavailable {
    pub reason: ApiUnavailableReason,
    /// ログ用の文。
    pub detail: String,
}

impl ApiUnavailable {
    pub(crate) fn other(e: &(dyn std::error::Error + 'static)) -> Self {
        Self {
            reason: ApiUnavailableReason::Other,
            detail: crate::app_core::chain(e),
        }
    }
}

/// 待ち受けを始められなかった理由の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiUnavailableReason {
    /// ソケットのパスが長すぎる（`sun_path` に収まらない）。クライアントも同じ判定で
    /// 決まった結果を返す。
    SocketPathTooLong,
    /// ソケットの場所に、ソケット以外のものがある。
    PathOccupied,
    /// パイプの名前をほかのプロセスが先に作っていた（Windows）。
    PipeNameTaken,
    /// ほかの失敗（権限を狭められない・bind できないなど）。
    Other,
}

impl From<ListenError> for ApiUnavailable {
    fn from(e: ListenError) -> Self {
        let reason = match &e {
            ListenError::SocketPathTooLong(_) => ApiUnavailableReason::SocketPathTooLong,
            ListenError::PathOccupied(_) => ApiUnavailableReason::PathOccupied,
            ListenError::PipeNameTaken(_) => ApiUnavailableReason::PipeNameTaken,
            ListenError::Io { .. } => ApiUnavailableReason::Other,
        };
        Self {
            reason,
            detail: crate::app_core::chain(&e),
        }
    }
}

/// 接続の進み具合。
struct Conn {
    /// 依頼を受け取る前に、別のスレッドから接続を閉じさせる道具。
    interrupter: Interrupter,
    /// 依頼を受け取って処理を始めた（止めるときも、最後まで行う）。
    working: bool,
}

#[derive(Default)]
struct Registry {
    /// 止め始めた（新しい依頼は始めない）。
    stopping: bool,
    next_id: u64,
    conns: HashMap<u64, Conn>,
}

/// 待ち受けのスレッドと接続のスレッドが共有するもの。
struct Shared {
    core: Weak<AppCore>,
    token: String,
    options: ApiOptions,
    registry: Mutex<Registry>,
    /// 接続が減ったとき。
    drained: Condvar,
    /// 待ち受けのスレッドが終わった（ソケットとトークンのファイルを消し終えた）。
    finished: (Mutex<bool>, Condvar),
    stopping: AtomicBool,
}

impl Shared {
    fn registry(&self) -> MutexGuard<'_, Registry> {
        self.registry.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// 動いている待ち受け。
pub struct ApiServer {
    shared: Arc<Shared>,
    waker: yunagi_api::transport::Waker,
}

impl ApiServer {
    /// 既定ディレクトリ `dir` で待ち受けを始める。トークンを作ってソケットより先に書く。
    pub fn start(core: &Arc<AppCore>, dir: &DefaultDir) -> Result<Self, ApiUnavailable> {
        let token = token::generate();
        let listener = Listener::open(dir, &token).map_err(ApiUnavailable::from)?;
        let waker = listener.waker();
        let shared = Arc::new(Shared {
            core: Arc::downgrade(core),
            token,
            options: core.api_options(),
            registry: Mutex::new(Registry::default()),
            drained: Condvar::new(),
            finished: (Mutex::new(false), Condvar::new()),
            stopping: AtomicBool::new(false),
        });
        let accept = Arc::clone(&shared);
        let spawned = thread::Builder::new()
            .name("yunagi-api".into())
            .spawn(move || accept_loop(&accept, listener));
        if let Err(e) = spawned {
            // listener は移った先（クロージャ）で捨てられ、ソケットとトークンのファイルが消える
            return Err(ApiUnavailable::other(&e));
        }
        Ok(Self { shared, waker })
    }

    /// いま処理中の接続の数。
    pub fn connections(&self) -> usize {
        self.shared.registry().conns.len()
    }

    /// 止める：新しい接続を受け付けなくし（ソケットのファイルをすぐ消す）、依頼をまだ受け取って
    /// いない接続は閉じ、処理中の依頼は最後まで行って応答を返す。そのあとトークンのファイルを消して
    /// 返る。ロックを離す前に呼ぶ。
    pub fn stop(self) {
        {
            let mut registry = self.shared.registry();
            registry.stopping = true;
            for conn in registry.conns.values().filter(|c| !c.working) {
                conn.interrupter.interrupt();
            }
        }
        self.shared.stopping.store(true, Ordering::Release);
        // 待ち受けのスレッドが、ソケットを消し、処理中の依頼の終わりを待ち、トークンを消して終わる。
        // 起こすための接続は、待ち行列が満杯・パイプがふさがっているときに失敗しうるので、終わるまで
        // 一定の間隔で起こし直す
        let (finished, changed) = &self.shared.finished;
        let mut done = finished.lock().unwrap_or_else(PoisonError::into_inner);
        let deadline = Instant::now() + STOP_LIMIT;
        while !*done {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                eprintln!("yunagi: the local API did not stop in time");
                return;
            };
            drop(done);
            self.waker.wake();
            done = finished.lock().unwrap_or_else(PoisonError::into_inner);
            if *done {
                break;
            }
            done = changed
                .wait_timeout(done, left.min(WAKE_RETRY))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

/// 接続を受けるスレッド。止める合図（`stopping`）を受けたら、ソケットを消し、処理中の依頼を待ち、
/// トークンのファイルを消して終わる。
fn accept_loop(shared: &Arc<Shared>, listener: Listener) {
    let mut failures = 0u32;
    loop {
        let accepted = listener.accept();
        if shared.stopping.load(Ordering::Acquire) {
            // 起こすための接続（か、止める直前に来た接続）。何も読まずに閉じる
            break;
        }
        let stream = match accepted {
            Ok(stream) => {
                failures = 0;
                stream
            }
            Err(e) => {
                // ファイル記述子が足りないなど。少し待って続ける
                failures = failures.saturating_add(1);
                if failures == 1 || failures.is_power_of_two() {
                    eprintln!("yunagi: the local API failed to accept a connection: {e}");
                }
                thread::sleep(Duration::from_millis(50));
                continue;
            }
        };
        spawn_connection(shared, stream);
    }
    // 新しいクライアントが「待ち受けが無い」と分かるよう、すぐソケットを消す
    listener.unlink();
    // 処理中の依頼を待つ
    {
        let deadline = Instant::now() + DRAIN_LIMIT;
        let mut registry = shared.registry();
        while !registry.conns.is_empty() {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                eprintln!("yunagi: the local API stopped with requests still running");
                break;
            };
            registry = shared
                .drained
                .wait_timeout(registry, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
    // ソケットとトークンのファイルを消す（トークンは最後）
    drop(listener);
    let (finished, changed) = &shared.finished;
    *finished.lock().unwrap_or_else(PoisonError::into_inner) = true;
    changed.notify_all();
}

/// 受けた接続を、登録して別のスレッドで処理する。
fn spawn_connection(shared: &Arc<Shared>, stream: Stream) {
    let Ok(interrupter) = stream.interrupter() else {
        return;
    };
    let id = {
        let mut registry = shared.registry();
        if registry.stopping || registry.conns.len() >= MAX_CONNECTIONS {
            return;
        }
        let id = registry.next_id;
        registry.next_id += 1;
        registry.conns.insert(
            id,
            Conn {
                interrupter,
                working: false,
            },
        );
        id
    };
    let handler = Arc::clone(shared);
    // 登録とストリームは 1 つにまとめて運ぶ。手放すとき、登録（`registration`）が先に消える
    // （フィールドは宣言の順に捨てられる）ので、スレッドを作れずに捨てられても、処理が panic しても、
    // 消した後のハンドルを止める側が中断することがない（Windows の `Interrupter` は生のハンドル）
    let connection = Connection {
        _registration: Registration {
            shared: Arc::clone(shared),
            id,
        },
        stream,
    };
    let _ = thread::Builder::new()
        .name("yunagi-api-conn".into())
        .spawn(move || {
            // 全体を取り込む（フィールドだけを取り込むと、登録がここに来る前に捨てられる）
            let connection = connection;
            serve(&handler, id, &connection.stream);
        });
}

/// 受けた接続：登録と、つながった相手。
struct Connection {
    _registration: Registration,
    stream: Stream,
}

/// 接続の登録。手放す（処理が終わる・panic する・スレッドを作れない）と登録を消し、止める側に
/// 知らせる。
struct Registration {
    shared: Arc<Shared>,
    id: u64,
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.shared.registry().conns.remove(&self.id);
        self.shared.drained.notify_all();
    }
}

/// 期限までしか待たない読み手。読みごとに、残りの時間を読みの時間切れにする。1 バイトずつ送って
/// 時間切れを延ばし続けるクライアントでも、1 行の合計の時間が期限を超えると読みが失敗する。
struct Timed<'a> {
    stream: &'a Stream,
    deadline: Cell<Instant>,
}

impl Read for Timed<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stream.set_read_deadline(self.deadline.get())?;
        (&*self.stream).read(buf)
    }
}

/// 1 つの接続を処理する：`hello` → 依頼 1 つ → 応答 1 つ。
fn serve(shared: &Shared, id: u64, stream: &Stream) {
    let options = shared.options;
    let wt = options.write_timeout;
    let mut reader = std::io::BufReader::new(Timed {
        stream,
        deadline: Cell::new(Instant::now() + options.hello_timeout),
    });
    // 1. hello
    let hello_line = match wire::read_line(&mut reader, MAX_HELLO_LINE) {
        Ok(Some(line)) => line,
        Ok(None) | Err(LineError::Io(_) | LineError::Truncated) => return,
        Err(LineError::TooLong) => {
            return reply(
                stream,
                wt,
                &wire::error_line(
                    &Value::Null,
                    wire::INVALID_REQUEST,
                    "Invalid Request: the line is too long",
                    None,
                ),
            );
        }
    };
    let client = match handshake(shared, &hello_line) {
        Ok((line, client)) => {
            if !reply_ok(stream, wt, &line) {
                return;
            }
            client
        }
        Err(line) => return reply(stream, wt, &line),
    };
    // 2. 依頼
    reader
        .get_ref()
        .deadline
        .set(Instant::now() + options.request_timeout);
    let read = wire::read_line(&mut reader, MAX_REQUEST_LINE);
    // 読み終えたら、読みの時間切れを外す。Windows では見張りが書きを中断しないように
    let _ = stream.set_read_timeout(None);
    let line = match read {
        Ok(Some(line)) => line,
        // 依頼が来る前の、時間切れ・止める合図・切断。クライアントが後から依頼を送って接続が切れた
        // ように見えると `result_unknown`（反映されたか分からない）になってしまうので、何も反映して
        // いないことを伝える `busy`（送り直してよい）を 1 行返して閉じる。相手が閉じていれば書けない
        // だけ
        Ok(None) | Err(LineError::Io(_) | LineError::Truncated) => {
            return reply(
                stream,
                wt,
                &wire::app_error_line(&Value::Null, &dispatch::busy_error()),
            );
        }
        Err(LineError::TooLong) => {
            return reply(
                stream,
                wt,
                &wire::error_line(
                    &Value::Null,
                    wire::INVALID_REQUEST,
                    "Invalid Request: the line is too long",
                    None,
                ),
            );
        }
    };
    // 止め始めていたら、始めずに断る（何も反映していない）。始めた依頼は、止めるときも最後まで行う
    let proceed = {
        let mut registry = shared.registry();
        if registry.stopping {
            false
        } else {
            if let Some(conn) = registry.conns.get_mut(&id) {
                conn.working = true;
            }
            true
        }
    };
    let response = match wire::parse_request(&line) {
        Err(bad) => bad.line(),
        Ok(request) if !proceed => wire::app_error_line(&request.id, &dispatch::busy_error()),
        Ok(request) => match shared.core.upgrade() {
            Some(core) => dispatch::handle(&core, client.as_deref(), &request),
            None => wire::app_error_line(&request.id, &dispatch::busy_error()),
        },
    };
    reply(stream, wt, &response);
}

/// `hello` を確かめる。通れば応答の行と、クライアントの識別子。通らなければ、返す失敗の行。
///
/// 確かめる順：形 → プロトコルの版 → トークン → 識別子の形。版を先にするのは、版が違うと
/// 以後の形も保証できないため（`hello` の形は版をまたいで変えない）。
fn handshake(shared: &Shared, line: &[u8]) -> Result<(String, Option<String>), String> {
    let request = wire::parse_request(line).map_err(|bad| bad.line())?;
    if request.method != wire::METHOD_HELLO {
        return Err(wire::error_line(
            &request.id,
            wire::INVALID_REQUEST,
            "Invalid Request: hello must come first",
            None,
        ));
    }
    let invalid =
        |message: &str| wire::error_line(&request.id, wire::INVALID_PARAMS, message, None);
    let params: HelloParams = serde_json::from_value(Value::Object(request.params.clone()))
        .map_err(|_| invalid("Invalid params: hello needs protocol and token"))?;
    let api_error = |reason: &str, message: &str, extra: Value| {
        let mut error = json!({"code": "api_error", "reason": reason, "message": message});
        if let (Some(object), Value::Object(extra)) = (error.as_object_mut(), extra) {
            object.extend(extra);
        }
        wire::app_error_line(&request.id, &error)
    };
    if params.protocol != PROTOCOL_VERSION {
        return Err(api_error(
            "version_mismatch",
            "The Yunagi app and this command speak different protocol versions. Ask the user to update Yunagi and the yunagi command to the same version.",
            json!({"server_protocol": PROTOCOL_VERSION}),
        ));
    }
    if !token::is_well_formed(&params.token) || !token::matches(&shared.token, &params.token) {
        return Err(api_error(
            "auth_failed",
            "The token was rejected. Ask the user to restart Yunagi.",
            json!({}),
        ));
    }
    if params
        .client
        .as_deref()
        .is_some_and(|name| !agent::is_valid_client_name(name))
    {
        return Err(invalid("Invalid params: client is not a valid identifier"));
    }
    let result = HelloResult {
        protocol: PROTOCOL_VERSION,
        app_version: yunagi_core::VERSION.to_owned(),
    };
    Ok((
        wire::result_line(&request.id, &json!(result)),
        params.client,
    ))
}

/// 応答の 1 行を書く（書けなければ諦める）。
fn reply_ok(stream: &Stream, timeout: Duration, line: &str) -> bool {
    use std::io::Write;
    // 相手が読まずに黙っていても、書きで止まり続けない（接続の枠を塞がない）
    if stream.set_write_timeout(Some(timeout)).is_err() {
        return false;
    }
    let mut writer = stream;
    writer
        .write_all(line.as_bytes())
        .and_then(|()| writer.flush())
        .is_ok()
}

/// 最後の応答を書いて、書き終えたことを相手に伝える。
fn reply(stream: &Stream, timeout: Duration, line: &str) {
    if reply_ok(stream, timeout, line) {
        stream.finish();
    }
}
