//! サポーターライセンスのキーの確認（画面設計書 7章「ライセンス区分と『このアプリについて』」、
//! アーキテクチャ設計書 2.1・4.3、セキュリティ方針 4.6）。
//!
//! - 確認は、画面の「確認する」（`verify_license`）が呼ばれたときに 1 回だけ、販売サービス（Polar）の
//!   API に問い合わせる。起動時・定期には通信しない。自動ではやり直さない。
//! - 送るのは、キーと販売元の識別子（[`SELLER`]）だけ。送り先（[`ENDPOINT`]）と識別子は、ビルドの
//!   ときに決まる固定の値で、画面・環境変数・設定のどれからも替えられない。
//! - 通信は [`Backend`] の後ろに置く。本物は [`HttpBackend`]（`https` だけ・リダイレクトは追わない・
//!   15 秒で時間切れ・応答は 64 KiB まで）、試験は偽物を使う。
//! - 確認できたときだけ、setting の `license` に、キーの末尾と時刻を書く（コアの
//!   `domain::license`）。キーの全体は保存せず、ログにも出さない。
//! - 販売元の識別子のどちらかが空のビルドでは、通信せずに「この版では使えない」を返す。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};
use yunagi_core::domain::license::{self, License};

use crate::app_core::{AppCore, chain, lock, now};
use crate::types::{CommandError, LicenseState, VerifyResult};

/// 送り先（Polar の、認証の要らない確認の API。画面設計書 7章「送り先」）。
pub const ENDPOINT: &str = "https://api.polar.sh/v1/customer-portal/license-keys/validate";

/// 時間切れ（つながるまでと、応答を読み終えるまでの全体）。
pub const TIMEOUT: Duration = Duration::from_secs(15);

/// 応答の本文を読む上限（バイト）。超えたら読むのをやめ、本文は読めなかったものとする。
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// User-Agent（アプリの名前だけ。版・OS は入れない）。
pub const USER_AGENT: &str = "Yunagi";

/// 販売元の識別子とキーの接頭辞（ビルドのときに決まる固定の値。秘密ではない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seller {
    /// Polar の組織の ID（`organization_id`）。
    pub organization_id: &'static str,
    /// サポーターライセンスの特典の ID（`benefit_id`）。ほかの商品のキーを通さないために付ける。
    pub benefit_id: &'static str,
    /// キーの接頭辞（Polar の特典に設定した値）。空なら接頭辞を見ない。
    pub key_prefix: &'static str,
}

impl Seller {
    /// 識別子が 2 つともあるか。無ければ確認できない（「この版では使えない」）。
    pub fn configured(&self) -> bool {
        !self.organization_id.is_empty() && !self.benefit_id.is_empty()
    }
}

/// このビルドの販売元の識別子。
///
/// オーナーが Polar に組織とサポーターライセンスの商品（ライセンスキーの特典）を作った後に、値を入れる
/// （手順は画面設計書 7章「販売元の識別子を入れる」）。値が入るまでは空のままで、確認の通信をせず、
/// 「この版では使えない」を出す。
pub const SELLER: Seller = Seller {
    organization_id: "",
    benefit_id: "",
    key_prefix: "",
};

/// 送る本文（JSON）。キーを持つので `Debug` を付けない（ログに出さないため）。
#[derive(Serialize)]
struct ValidateRequest<'a> {
    key: &'a str,
    organization_id: &'a str,
    benefit_id: &'a str,
}

/// 販売サービスの応答。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// つながらない・名前を引けない・時間切れ・TLS の失敗。中はログ用の文（キーを含まない）。
    Unreachable(String),
    /// 応答があった。`body` は、上限（[`MAX_RESPONSE_BYTES`]）を超えたときは `None`（読み切らない）。
    /// 本文の途中で止まった・切れたときは、[`Response::Unreachable`] にする。
    Http { status: u16, body: Option<Vec<u8>> },
}

/// 販売サービスへの問い合わせ。本物（[`HttpBackend`]）と、試験の偽物がある。
pub trait Backend: Send + Sync {
    /// 本文 `body`（JSON）を送り先へ POST する（1 回だけ。やり直さない）。
    fn validate(&self, body: &[u8]) -> Response;
}

/// 応答の分け方（画面設計書 7章「結果の分け方」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// 200 で `status` が `granted`。
    Granted,
    /// 404、または 200 で `status` が `granted` でない。
    Rejected,
    /// 通信そのものの失敗。
    Unreachable,
    /// そのほか（5xx・429・422・リダイレクト・読めない本文）。
    ServiceError,
}

/// 応答の本文のうち、読むのは `status` だけ（購入者の情報などは読まずに捨てる）。
#[derive(Deserialize)]
struct ValidatedKey {
    status: String,
}

/// 応答を分ける。
pub fn classify(response: &Response) -> Outcome {
    match response {
        Response::Unreachable(_) => Outcome::Unreachable,
        Response::Http { status: 404, .. } => Outcome::Rejected,
        Response::Http {
            status: 200,
            body: Some(body),
        } => match serde_json::from_slice::<ValidatedKey>(body) {
            Ok(key) if key.status == "granted" => Outcome::Granted,
            Ok(_) => Outcome::Rejected,
            Err(_) => Outcome::ServiceError,
        },
        Response::Http { .. } => Outcome::ServiceError,
    }
}

/// 確認の状態と手段（アプリに 1 つ。Tauri の状態 `State<Arc<LicenseService>>`）。
pub struct LicenseService {
    seller: Seller,
    backend: Arc<dyn Backend>,
    /// 確認の途中か（同時に 1 本だけ）。
    verifying: Mutex<bool>,
}

/// 確認の途中の印。手放すと（panic でも）、途中でなくなる。
struct Verifying<'a>(&'a Mutex<bool>);

impl Drop for Verifying<'_> {
    fn drop(&mut self) {
        *lock(self.0) = false;
    }
}

impl LicenseService {
    pub fn new(seller: Seller, backend: Arc<dyn Backend>) -> Arc<Self> {
        Arc::new(Self {
            seller,
            backend,
            verifying: Mutex::new(false),
        })
    }

    /// このビルドの識別子と、本物の通信の部品。
    pub fn built_in() -> Arc<Self> {
        Self::new(SELLER, Arc::new(HttpBackend::polar()))
    }

    /// いまの状態（`get_license`）。確認済み → 確認中 → 使えない → 未確認の順に見る。通信しない。
    pub fn state(&self, core: &AppCore) -> Result<LicenseState, CommandError> {
        if let Some(l) = core.read(license::load)? {
            return Ok(LicenseState::Verified { license: l.into() });
        }
        Ok(if *lock(&self.verifying) {
            LicenseState::Verifying
        } else if !self.seller.configured() {
            LicenseState::Unavailable
        } else {
            LicenseState::Unverified
        })
    }

    /// 入力 `input` を確かめる（`verify_license`）。通信するのは、識別子があり、形の検査を通り、ほかの
    /// 確認の途中でないときの 1 回だけ。確認できたときだけ、確かめ始めたときのデータベースに保存する。
    pub fn verify(&self, core: &AppCore, input: &str) -> Result<VerifyResult, CommandError> {
        if !self.seller.configured() {
            return Ok(VerifyResult::Unavailable);
        }
        let Some(key) = license::check_key(input, self.seller.key_prefix) else {
            return Ok(VerifyResult::InvalidFormat);
        };
        let _verifying = {
            let mut verifying = lock(&self.verifying);
            if *verifying {
                return Ok(VerifyResult::Busy);
            }
            *verifying = true;
            Verifying(&self.verifying)
        };
        let Some(generation) = core.database_generation() else {
            return Err(CommandError::NotReady);
        };
        let body = serde_json::to_vec(&ValidateRequest {
            key,
            organization_id: self.seller.organization_id,
            benefit_id: self.seller.benefit_id,
        })
        .expect("strings are serializable");
        let response = self.backend.validate(&body);
        drop(body);
        let outcome = classify(&response);
        if let Response::Unreachable(detail) = &response {
            eprintln!("yunagi: could not reach the license service: {detail}");
        } else if outcome == Outcome::ServiceError
            && let Response::Http { status, .. } = &response
        {
            // 本文はログに出さない（購入者の情報が入りうる）
            eprintln!("yunagi: the license service answered with status {status}");
        }
        if outcome != Outcome::Granted {
            // 確かめている間にデータベースが替わっていたら、失敗の結果も捨てる（画面は読み込み直す）
            if core.database_generation() != Some(generation) {
                return Ok(VerifyResult::Discarded);
            }
            return Ok(match outcome {
                Outcome::Rejected => VerifyResult::Rejected,
                Outcome::Unreachable => VerifyResult::Unreachable,
                _ => VerifyResult::ServiceError,
            });
        }
        let verified = License::new(key, now());
        match core.write_if_same_database(generation, |conn| license::save(conn, &verified)) {
            Ok(Some(())) => Ok(VerifyResult::Verified {
                license: verified.into(),
            }),
            Ok(None) => Ok(VerifyResult::Discarded),
            Err(e) => {
                if let CommandError::Storage { detail } = &e {
                    eprintln!("yunagi: could not save the verified license: {detail}");
                }
                Ok(VerifyResult::SaveFailed)
            }
        }
    }

    /// 確認の印を消す（`remove_license`）。通信しない。
    pub fn remove(&self, core: &AppCore) -> Result<(), CommandError> {
        core.quiet_write(license::remove).map(|_| ())
    }
}

/// 本物の通信の部品（reqwest。自動更新のプラグインと同じもの）。
pub struct HttpBackend {
    url: &'static str,
    timeout: Duration,
    https_only: bool,
    /// OS・環境変数のプロキシの設定に従うか（本物は従う。手元の試験のサーバーには使わない）。
    system_proxy: bool,
}

impl HttpBackend {
    /// 販売サービス（[`ENDPOINT`]）への部品。
    pub fn polar() -> Self {
        Self {
            url: ENDPOINT,
            timeout: TIMEOUT,
            https_only: true,
            system_proxy: true,
        }
    }

    fn client(&self) -> reqwest::Result<reqwest::Client> {
        ensure_crypto_provider();
        let builder = reqwest::Client::builder();
        let builder = if self.system_proxy {
            builder
        } else {
            builder.no_proxy()
        };
        builder
            .user_agent(USER_AGENT)
            .redirect(reqwest::redirect::Policy::none())
            .https_only(self.https_only)
            .timeout(self.timeout)
            .build()
    }

    async fn send(&self, body: Vec<u8>) -> Response {
        let client = match self.client() {
            Ok(c) => c,
            Err(e) => return Response::Unreachable(chain(&e)),
        };
        let response = client
            .post(self.url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::ACCEPT, "application/json")
            .body(body)
            .send()
            .await;
        let mut response = match response {
            Ok(r) => r,
            Err(e) => return Response::Unreachable(chain(&e)),
        };
        let status = response.status().as_u16();
        let mut read = Vec::new();
        let body = loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    if read.len() + chunk.len() > MAX_RESPONSE_BYTES {
                        break None;
                    }
                    read.extend_from_slice(&chunk);
                }
                Ok(None) => break Some(read),
                // 本文の途中で止まった・切れた（時間切れを含む）：通信そのものの失敗
                Err(e) => return Response::Unreachable(chain(&e)),
            }
        };
        Response::Http { status, body }
    }
}

impl Backend for HttpBackend {
    fn validate(&self, body: &[u8]) -> Response {
        tauri::async_runtime::block_on(self.send(body.to_vec()))
    }
}

/// TLS の暗号の部品（ring）を、まだ無ければ入れる。自動更新のプラグインも、確認の前に同じことをする。
fn ensure_crypto_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        // 失敗するのは、ほかが先に入れたときだけ（それを使う）
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

/// アプリに確認の手段を持たせる（`State<Arc<LicenseService>>`）。窓の無い試験は、自分で偽物を持たせる。
pub fn install<R: Runtime>(app: &AppHandle<R>) {
    app.manage(LicenseService::built_in());
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Instant;

    use super::*;

    fn http(status: u16, body: &str) -> Response {
        Response::Http {
            status,
            body: Some(body.as_bytes().to_vec()),
        }
    }

    #[test]
    fn responses_are_sorted_as_the_ui_spec_says() {
        let granted =
            r#"{"id":"x","status":"granted","customer":{"email":"a@example.com","name":"A"}}"#;
        assert_eq!(classify(&http(200, granted)), Outcome::Granted);
        assert_eq!(classify(&http(404, "")), Outcome::Rejected);
        assert_eq!(
            classify(&Response::Http {
                status: 404,
                body: None
            }),
            Outcome::Rejected
        );
        // 200 でも granted でないものは、キーが確認できない
        for status in [r#"{"status":"revoked"}"#, r#"{"status":"disabled"}"#] {
            assert_eq!(classify(&http(200, status)), Outcome::Rejected);
        }
        assert_eq!(
            classify(&Response::Unreachable("timed out".into())),
            Outcome::Unreachable
        );
        for r in [
            http(500, ""),
            http(502, "<html>"),
            http(429, ""),
            http(422, r#"{"detail":[]}"#),
            http(400, ""),
            http(301, ""),
            http(302, ""),
            http(200, "not json"),
            http(200, r#"{"status":3}"#),
            http(200, r#"{"id":"x"}"#),
            Response::Http {
                status: 200,
                body: None,
            },
        ] {
            assert_eq!(classify(&r), Outcome::ServiceError, "{r:?}");
        }
    }

    #[test]
    fn the_built_in_endpoint_is_fixed_https() {
        let b = HttpBackend::polar();
        assert_eq!(b.url, ENDPOINT);
        assert!(ENDPOINT.starts_with("https://api.polar.sh/"));
        assert!(b.https_only);
        assert_eq!(b.timeout, Duration::from_secs(15));
    }

    /// 手元の HTTP サーバー（1 回だけ受ける）。受けた依頼の全体を返す。
    fn serve(
        reply: impl FnOnce(&mut std::net::TcpStream) + Send + 'static,
    ) -> (String, mpsc::Receiver<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/validate", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_millis(500)))
                .unwrap();
            let mut request = Vec::new();
            let mut buf = [0; 4096];
            // 見出しと本文（Content-Length の分）を読む
            loop {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => request.extend_from_slice(&buf[..n]),
                }
                if let Some(end) = find(&request, b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&request[..end]).to_lowercase();
                    let len = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if request.len() >= end + 4 + len {
                        break;
                    }
                }
            }
            tx.send(request).unwrap();
            reply(&mut stream);
        });
        (url, rx)
    }

    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    fn local(url: String, timeout: Duration) -> HttpBackend {
        HttpBackend {
            url: Box::leak(url.into_boxed_str()),
            timeout,
            https_only: false,
            system_proxy: false,
        }
    }

    fn respond(stream: &mut std::net::TcpStream, head: &str, body: &[u8]) {
        let _ = stream.write_all(
            format!(
                "{head}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        );
        let _ = stream.write_all(body);
    }

    #[test]
    fn the_request_carries_only_the_key_and_the_seller_ids() {
        let (url, rx) = serve(|s| respond(s, "HTTP/1.1 200 OK", br#"{"status":"granted"}"#));
        let body = br#"{"key":"YUNAGI_aaaaaaaa","organization_id":"org","benefit_id":"ben"}"#;
        let response = local(url, TIMEOUT).validate(body);
        assert_eq!(classify(&response), Outcome::Granted);
        let request = String::from_utf8(rx.recv().unwrap()).unwrap();
        let (head, sent) = request.split_once("\r\n\r\n").unwrap();
        assert_eq!(sent.as_bytes(), body);
        assert!(head.starts_with("POST /validate HTTP/1.1\r\n"), "{head}");
        let lower = head.to_lowercase();
        assert!(lower.contains("\r\nuser-agent: yunagi\r\n"), "{head}");
        assert!(
            lower.contains("\r\ncontent-type: application/json\r\n"),
            "{head}"
        );
        // 版・OS は送らない（User-Agent はアプリの名前だけ）
        for os in ["linux", "windows", "macos", "darwin"] {
            assert!(!lower.contains(os), "{head}");
        }
    }

    #[test]
    fn redirects_are_not_followed() {
        let (url, _rx) = serve(|s| {
            respond(
                s,
                "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/elsewhere",
                b"",
            )
        });
        let response = local(url, TIMEOUT).validate(b"{}");
        assert_eq!(
            response,
            Response::Http {
                status: 302,
                body: Some(Vec::new())
            }
        );
        assert_eq!(classify(&response), Outcome::ServiceError);
    }

    #[test]
    fn a_response_over_the_limit_is_not_read_through() {
        let (url, _rx) = serve(|s| {
            let big = format!(
                r#"{{"status":"granted","pad":"{}"}}"#,
                "x".repeat(MAX_RESPONSE_BYTES)
            );
            respond(s, "HTTP/1.1 200 OK", big.as_bytes());
        });
        let response = local(url, TIMEOUT).validate(b"{}");
        assert_eq!(
            response,
            Response::Http {
                status: 200,
                body: None
            }
        );
        assert_eq!(classify(&response), Outcome::ServiceError);
    }

    #[test]
    fn a_silent_server_times_out_as_unreachable() {
        let (url, _rx) = serve(|_| thread::sleep(Duration::from_secs(3)));
        let started = Instant::now();
        let response = local(url, Duration::from_millis(300)).validate(b"{}");
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(matches!(response, Response::Unreachable(_)), "{response:?}");
        assert_eq!(classify(&response), Outcome::Unreachable);
    }

    #[test]
    fn a_body_that_stops_midway_times_out_as_unreachable() {
        // 見出しと本文の一部だけを返して黙る
        let (url, _rx) = serve(|s| {
            let _ = s.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{\"status\":",
            );
            let _ = s.flush();
            thread::sleep(Duration::from_secs(3));
        });
        let started = Instant::now();
        let response = local(url, Duration::from_millis(500)).validate(b"{}");
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(matches!(response, Response::Unreachable(_)), "{response:?}");
    }

    #[test]
    fn nothing_listening_is_unreachable() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/validate", listener.local_addr().unwrap());
        drop(listener);
        let response = local(url, TIMEOUT).validate(b"{}");
        assert_eq!(classify(&response), Outcome::Unreachable);
    }

    #[test]
    fn plain_http_is_refused_by_the_built_in_settings() {
        // 本物の部品は https だけを使う（http の送り先には、通信せずに失敗する）
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/validate", listener.local_addr().unwrap());
        let backend = HttpBackend {
            https_only: true,
            ..local(url, TIMEOUT)
        };
        listener.set_nonblocking(true).unwrap();
        assert_eq!(classify(&backend.validate(b"{}")), Outcome::Unreachable);
        assert!(listener.accept().is_err(), "nothing should have connected");
    }

    #[test]
    fn both_seller_ids_are_needed() {
        assert!(
            !Seller {
                organization_id: "org",
                benefit_id: "",
                key_prefix: ""
            }
            .configured()
        );
        assert!(
            Seller {
                organization_id: "org",
                benefit_id: "ben",
                key_prefix: ""
            }
            .configured()
        );
    }
}
