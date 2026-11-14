use std::collections::BTreeSet;

use indexmap::IndexMap;
use thiserror::Error;

use crate::headroom_settings::UnreachableFallback;
use crate::primitives::json_value::JsonValue;

mod tests;

pub const FAILURE_STATUS: u16 = 502;

pub const NOT_FOUND_HINT: &str = "The Headroom compression endpoint returned HTTP 404. Verify that the configured Headroom endpoint is correct and that the compression endpoint is available. If you are using a self-hosted deployment, some deployments require enabling remote compression (for example, HEADROOM_COMPRESS_ALLOW_REMOTE=1).";

#[derive(Debug, Error)]
#[error("headroom compression failed with status {status}")]
pub struct CompressFailure {
    pub status: u16,
    pub body: IndexMap<String, JsonValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompressResult {
    pub messages: Vec<JsonValue>,
    pub succeeded: bool,
    pub stats: IndexMap<String, JsonValue>,
    pub ccr_hashes: BTreeSet<String>,
}

#[must_use]
pub fn build_compress_failure_detail(status: u16, body: &str) -> IndexMap<String, JsonValue> {
    let mut detail = IndexMap::new();
    detail.insert(
        "status_code".to_owned(),
        JsonValue::Number(serde_json::Number::from(status)),
    );
    detail.insert("body".to_owned(), JsonValue::String(body.to_owned()));
    if status == 404 {
        detail.insert(
            "hint".to_owned(),
            JsonValue::String(NOT_FOUND_HINT.to_owned()),
        );
    }
    detail
}

pub fn handle_compress_failure(
    fallback: UnreachableFallback,
    rows: &[JsonValue],
    error: &str,
    detail: &IndexMap<String, JsonValue>,
) -> Result<Vec<JsonValue>, CompressFailure> {
    match fallback {
        UnreachableFallback::FailOpen => {
            tracing::error!(
                "Headroom: {error}; fail_open configured, forwarding request uncompressed. detail={detail:?}"
            );
            Ok(rows.to_vec())
        }
        UnreachableFallback::FailClosed => {
            let mut body =
                IndexMap::from([("error".to_owned(), JsonValue::String(error.to_owned()))]);
            body.extend(detail.clone());
            Err(CompressFailure {
                status: FAILURE_STATUS,
                body,
            })
        }
    }
}

#[must_use]
pub fn read_ccr_hashes(body: &JsonValue) -> BTreeSet<String> {
    let JsonValue::Object(fields) = body else {
        return BTreeSet::new();
    };
    let Some(JsonValue::Array(hashes)) = fields.get("ccr_hashes") else {
        return BTreeSet::new();
    };
    hashes.iter().filter_map(ccr_hash).collect()
}

fn ccr_hash(value: &JsonValue) -> Option<String> {
    let JsonValue::String(text) = value else {
        return None;
    };
    let lowered = text.to_ascii_lowercase();
    let length_ok = (12..=24).contains(&lowered.len());
    let hex = lowered
        .bytes()
        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'));
    (length_ok && hex).then_some(lowered)
}
