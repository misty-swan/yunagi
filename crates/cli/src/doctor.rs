//! `yunagi doctor`：診断情報（docs/design/05-mcp-cli-spec.md 3.1、アーキテクチャ設計書 4.4・4.5）。
//!
//! 版・OS・既定ディレクトリと保存先・ローカル API の状態・別の候補の既定ディレクトリの状態を出す。
//! 業務データの内容は含めない（パスと状態だけ）。ローカル API の状態は、接続の手順と同じ試し方で
//! 1 回だけ調べる（待たない。一時的につながらないときだけ 1 秒まで試し直す）。取ったロックはすぐ離す。

use std::fmt::Write as _;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use yunagi_api::PROTOCOL_VERSION;
use yunagi_api::client::{self, HelloError};
use yunagi_core::storage::data_dir::Platform;
use yunagi_core::storage::dir_mismatch::{self, CandidateStatus, Judgement, LockState};

use crate::connect::Connector;

/// 一時的につながらないときに、試し直す時間の上限。
const RETRY_WITHIN: Duration = Duration::from_secs(1);

/// 診断情報を集める。
pub fn diagnose(connector: &Connector) -> Value {
    let dir = connector.default_dir();
    let vars = dir_mismatch::default_dir_vars(Platform::current());
    let own = dir_mismatch::own_storage(dir);
    let candidate = connector.candidate().map(|c| c.probe());
    let judgement = dir_mismatch::judge(&own, candidate.as_ref());
    let (status, app_version) = local_api_status(connector, judgement);
    let other = candidate.as_ref().map(|info| {
        let status = match (info.status, judgement) {
            (CandidateStatus::InUse, Judgement::Separate) => "in_use_separate",
            (CandidateStatus::InUse, _) => "in_use",
            (CandidateStatus::HasData, _) => "has_data",
            (CandidateStatus::Absent, _) => "absent",
            (CandidateStatus::Unknown, _) => "unknown",
        };
        json!({"path": info.dir, "status": status})
    });
    json!({
        "version": yunagi_core::VERSION,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "protocol": PROTOCOL_VERSION,
        "default_dir": {
            "path": dir.path(),
            "env_vars": vars,
            "exists": own.default_dir_exists,
        },
        "data_dir": own.data_dir.as_ref().map(|path| json!({
            "path": path,
            "database_exists": own.has_database,
        })),
        "local_api": {"status": status, "app_version": app_version},
        "other_default_dir": other,
    })
}

/// 既定ディレクトリを求められなかったとき（環境変数が無い）の診断情報。ローカル API の状態は調べられない。
pub fn diagnose_without_dir(message: &str) -> Value {
    json!({
        "version": yunagi_core::VERSION,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "protocol": PROTOCOL_VERSION,
        "default_dir": {
            "path": null,
            "env_vars": dir_mismatch::default_dir_vars(Platform::current()),
            "exists": false,
            "error": message,
        },
        "data_dir": null,
        "local_api": null,
        "other_default_dir": null,
    })
}

/// ローカル API の状態（05 3.1）と、つながったときのアプリの版。
fn local_api_status(connector: &Connector, judgement: Judgement) -> (&'static str, Option<String>) {
    let timing = connector.timing();
    let deadline = Instant::now() + RETRY_WITHIN;
    let mut too_long = false;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match client::connect(
            connector.default_dir(),
            Some(connector.client()),
            left.min(timing.connect),
        ) {
            Ok(session) => return ("connected", Some(session.server.app_version.clone())),
            Err(HelloError::NoListener) => break,
            Err(HelloError::SocketPathTooLong(_)) => {
                too_long = true;
                break;
            }
            Err(e @ (HelloError::Transient(_) | HelloError::NotAnswering)) => {
                if Instant::now() < deadline {
                    thread::sleep(timing.interval.min(deadline - Instant::now()));
                    continue;
                }
                return (
                    if matches!(e, HelloError::Transient(_)) {
                        "listener_busy"
                    } else {
                        "starting_or_unresponsive"
                    },
                    None,
                );
            }
            Err(HelloError::Denied(_)) => return ("denied", None),
            Err(HelloError::AuthFailed) => return ("auth_failed", None),
            Err(HelloError::VersionMismatch { .. }) => return ("version_mismatch", None),
            Err(HelloError::Io(_)) => return ("starting_or_unresponsive", None),
        }
    }
    // 待ち受けが無い：ロックを 1 回だけ取ってみて、すぐ離す（ファイルは作らない）
    match dir_mismatch::lock_state_of(connector.default_dir()) {
        LockState::Held => (
            if too_long {
                "socket_path_too_long"
            } else {
                "starting_or_unresponsive"
            },
            None,
        ),
        // 取れた、またはファイルもフォルダも無い（ロックを持つ者はいない）
        LockState::Free | LockState::NoFile => (
            if matches!(judgement, Judgement::Mismatch { .. }) {
                "default_dir_mismatch"
            } else {
                "no_listener"
            },
            None,
        ),
        // 権限が無いなどでロックファイルを調べられない：待ち受けが無いとは言えない
        LockState::Unknown => ("unknown", None),
    }
}

/// テキスト出力（`key: value` の行）。
pub fn render_text(report: &Value) -> String {
    let mut out = String::new();
    let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
    let _ = writeln!(out, "version: {}", s(&report["version"]));
    let _ = writeln!(out, "os: {} ({})", s(&report["os"]), s(&report["arch"]));
    let _ = writeln!(out, "protocol: {}", report["protocol"]);
    let dir = &report["default_dir"];
    match dir["error"].as_str() {
        Some(error) => {
            let _ = writeln!(out, "default directory: unknown ({error})");
        }
        None => {
            let _ = writeln!(
                out,
                "default directory: {} ({})",
                s(&dir["path"]),
                if dir["exists"].as_bool().unwrap_or(false) {
                    "exists"
                } else {
                    "not found"
                }
            );
        }
    }
    let vars: Vec<String> = dir["env_vars"]
        .as_array()
        .map(|a| a.iter().map(&s).collect())
        .unwrap_or_default();
    let _ = writeln!(out, "  environment variables: {}", vars.join(", "));
    match report["data_dir"].as_object() {
        Some(data) => {
            let _ = writeln!(
                out,
                "data directory: {} (database {})",
                s(&data["path"]),
                if data["database_exists"].as_bool().unwrap_or(false) {
                    "found"
                } else {
                    "not found"
                }
            );
        }
        None => {
            let _ = writeln!(out, "data directory: unknown");
        }
    }
    let api = &report["local_api"];
    match (api["status"].as_str(), api["app_version"].as_str()) {
        (Some(status), Some(version)) => {
            let _ = writeln!(out, "local API: {status} (Yunagi {version})");
        }
        (Some(status), None) => {
            let _ = writeln!(out, "local API: {status}");
        }
        (None, _) => {
            let _ = writeln!(out, "local API: unknown");
        }
    }
    match report["other_default_dir"].as_object() {
        Some(other) => {
            let _ = writeln!(
                out,
                "other default directory: {} ({})",
                s(&other["path"]),
                s(&other["status"])
            );
        }
        None => {
            let _ = writeln!(out, "other default directory: none");
        }
    }
    out
}
