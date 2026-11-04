//! 操作 `show_app`：アプリの窓を前面に出す（`yunagi show`。docs/design/05-mcp-cli-spec.md 3.1・4.4、
//! docs/design/01-architecture.md 4.4・4.6）。
//!
//! データの操作（[`crate::ops`]）とは別に置く。次の違いがあるため。
//!
//! - データベースにも呼び出し元のスコープにも触れない（窓を操作するだけ）。アプリは、データベースを
//!   使えるまで待たず、クライアントの識別子も登録しない。
//! - 直接モードが無い（窓はアプリにしか無い）。MCP にも出さない（`tools/list` に無く、直接モードの
//!   `ops::Call::parse` は知らない操作として断る）。
//!
//! アプリの待ち受け（`src-tauri/src/api/dispatch.rs`）と CLI（`crates/cli`）が、操作の名前・引数・
//! 返り値の形を食い違わせないよう、同じものを使う。

use serde_json::{Map, Value, json};

/// 操作の名前。
pub const METHOD: &str = "show_app";

/// 引数の名前：ノートモードで出す（省略は `false`）。
pub const PARAM_NOTE: &str = "note";

/// 引数 `params` を読む。戻り値は、ノートモードで出すか。形が誤っていれば、その理由。
///
/// 名前は `note`（真偽）だけ。知らない名前は、綴りの誤りを黙って無視しないよう断る（05 4.4）。
/// `params` がオブジェクトでないことは、依頼を読む段階で断っている（[`crate::wire::parse_request`]）。
pub fn parse_params(params: &Map<String, Value>) -> Result<bool, String> {
    for key in params.keys() {
        if key != PARAM_NOTE {
            return Err(format!("unknown parameter {key}"));
        }
    }
    match params.get(PARAM_NOTE) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(note)) => Ok(*note),
        Some(_) => Err(format!("{PARAM_NOTE} must be a boolean")),
    }
}

/// 依頼の引数。
pub fn params(note: bool) -> Map<String, Value> {
    let mut map = Map::new();
    if note {
        map.insert(PARAM_NOTE.into(), Value::Bool(true));
    }
    map
}

/// 成功の返り値。窓を前面に出すよう頼んだことを表す。OS が別のアプリからのフォーカスの移動を
/// 止めることがある（Wayland のコンポジターなど）ので、前面に出たことまでは約束しない。
pub fn result() -> Value {
    json!({"requested": true})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(value: Value) -> Result<bool, String> {
        let Value::Object(map) = value else {
            panic!("not an object");
        };
        parse_params(&map)
    }

    #[test]
    fn params_are_read_strictly() {
        assert_eq!(parse(json!({})), Ok(false));
        assert_eq!(parse(json!({"note": false})), Ok(false));
        assert_eq!(parse(json!({"note": true})), Ok(true));
        assert_eq!(parse(json!({"note": null})), Ok(false));
        assert!(parse(json!({"note": "yes"})).is_err());
        assert!(parse(json!({"nte": true})).is_err());
    }

    #[test]
    fn the_request_round_trips() {
        assert_eq!(parse_params(&params(true)), Ok(true));
        assert_eq!(parse_params(&params(false)), Ok(false));
        assert_eq!(result(), json!({"requested": true}));
    }
}
