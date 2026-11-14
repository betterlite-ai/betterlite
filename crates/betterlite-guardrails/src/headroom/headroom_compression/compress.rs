use std::collections::BTreeSet;

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Number;
use thiserror::Error;

use super::HeadroomGuardrail;
use crate::headroom_failure::{
    CompressFailure, CompressResult, build_compress_failure_detail, handle_compress_failure,
    read_ccr_hashes,
};
use crate::headroom_settings::{HeadroomSettingsError, UnreachableFallback};
use crate::primitives::json_value::JsonValue;

pub(crate) const BODY_PREVIEW_CHARS: usize = 500;
pub(crate) const COUNT_ERROR: &str = "Headroom compression service changed the message count";
pub(crate) const EMPTY_MESSAGES_ERROR: &str =
    "Headroom compression service returned empty message list";
const MESSAGES_KEY: &str = "messages";
pub(crate) const MISSING_MESSAGES_ERROR: &str =
    "Headroom compression service response missing 'messages'";
pub(crate) const NON_JSON_ERROR: &str = "Headroom compression service returned non-JSON response";
pub(crate) const SERVICE_ERROR: &str = "Headroom compression service returned an error";
pub(crate) const SHAPE_ERROR: &str =
    "Headroom compression service returned unexpected response shape";
const TOKENS_AFTER: &str = "tokens_after";
const TOKENS_BEFORE: &str = "tokens_before";
const TOKENS_SAVED: &str = "tokens_saved";
pub(crate) const UNREACHABLE_ERROR: &str = "Headroom compression service unreachable";
const STAT_KEYS: [&str; 5] = [
    TOKENS_BEFORE,
    TOKENS_AFTER,
    TOKENS_SAVED,
    "compression_ratio",
    "transforms_applied",
];

#[derive(Debug, Error)]
pub(crate) enum CompressError {
    #[error(transparent)]
    Headers(#[from] HeadroomSettingsError),
    #[error(transparent)]
    Failure(#[from] CompressFailure),
}

#[derive(Serialize)]
struct CompressPayload<'a> {
    messages: &'a [JsonValue],
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<&'a str>,
}

pub(crate) struct CompressRequest<'a> {
    pub(crate) rows: &'a [JsonValue],
    pub(crate) model: Option<&'a str>,
}

impl HeadroomGuardrail {
    pub(crate) async fn call_compress(
        &self,
        request: CompressRequest<'_>,
    ) -> Result<CompressResult, CompressError> {
        let headers = self.settings.request_headers()?;
        let payload = CompressPayload {
            messages: request.rows,
            model: request.model,
        };
        let url = format!("{}/v1/compress", self.settings.api_base);
        let fallback = self.settings.unreachable_fallback;
        let sent = self
            .client
            .post(url)
            .headers(headers)
            .timeout(self.settings.timeout.total)
            .json(&payload)
            .send()
            .await;
        let response = match sent {
            Ok(response) => response,
            Err(error) => return unreachable(request.rows, &error.to_string(), fallback),
        };
        let status = response.status().as_u16();
        let text = match response.text().await {
            Ok(text) => text,
            Err(error) => return unreachable(request.rows, &error.to_string(), fallback),
        };
        if status != 200 {
            return rejected(status, &text, request.rows, fallback);
        }
        parse_body(&text, request.rows, fallback).map_err(CompressError::Failure)
    }
}

fn unreachable(
    rows: &[JsonValue],
    error: &str,
    fallback: UnreachableFallback,
) -> Result<CompressResult, CompressError> {
    let detail = IndexMap::from([("detail".to_owned(), JsonValue::String(error.to_owned()))]);
    failure(fallback, rows, UNREACHABLE_ERROR, detail).map_err(CompressError::Failure)
}

fn rejected(
    status: u16,
    body: &str,
    rows: &[JsonValue],
    fallback: UnreachableFallback,
) -> Result<CompressResult, CompressError> {
    let detail = build_compress_failure_detail(status, body);
    failure(fallback, rows, SERVICE_ERROR, detail).map_err(CompressError::Failure)
}

fn failure(
    fallback: UnreachableFallback,
    rows: &[JsonValue],
    error: &str,
    detail: IndexMap<String, JsonValue>,
) -> Result<CompressResult, CompressFailure> {
    let messages = handle_compress_failure(fallback, rows, error, &detail)?;
    let result = CompressResult {
        messages,
        succeeded: false,
        stats: IndexMap::new(),
        ccr_hashes: BTreeSet::new(),
    };
    Ok(result)
}

fn parse_body(
    text: &str,
    rows: &[JsonValue],
    fallback: UnreachableFallback,
) -> Result<CompressResult, CompressFailure> {
    let Ok(body) = serde_json::from_str::<JsonValue>(text) else {
        return failure(
            fallback,
            rows,
            NON_JSON_ERROR,
            body_detail(text, BODY_PREVIEW_CHARS),
        );
    };
    let JsonValue::Object(fields) = &body else {
        return failure(
            fallback,
            rows,
            SHAPE_ERROR,
            body_detail(text, BODY_PREVIEW_CHARS),
        );
    };
    let Some(JsonValue::Array(items)) = fields.get(MESSAGES_KEY) else {
        return failure(
            fallback,
            rows,
            MISSING_MESSAGES_ERROR,
            body_detail(text, text.chars().count()),
        );
    };
    let filtered: Vec<JsonValue> = items
        .iter()
        .filter(|item| matches!(item, JsonValue::Object(_)))
        .cloned()
        .collect();
    if filtered.is_empty() {
        return failure(
            fallback,
            rows,
            EMPTY_MESSAGES_ERROR,
            body_detail(text, text.chars().count()),
        );
    }
    if filtered.len() != rows.len() {
        return failure(
            fallback,
            rows,
            COUNT_ERROR,
            count_detail(rows.len(), filtered.len()),
        );
    }
    let result = CompressResult {
        messages: filtered,
        succeeded: true,
        stats: statistics(fields),
        ccr_hashes: read_ccr_hashes(&body),
    };
    Ok(result)
}

fn body_detail(text: &str, limit: usize) -> IndexMap<String, JsonValue> {
    let preview: String = text.chars().take(limit).collect();
    let mut detail = IndexMap::new();
    detail.insert("body".to_owned(), JsonValue::String(preview));
    detail
}

fn count_detail(sent: usize, returned: usize) -> IndexMap<String, JsonValue> {
    IndexMap::from([
        ("sent".to_owned(), JsonValue::Number(Number::from(sent))),
        (
            "returned".to_owned(),
            JsonValue::Number(Number::from(returned)),
        ),
    ])
}

fn statistics(fields: &IndexMap<String, JsonValue>) -> IndexMap<String, JsonValue> {
    let mut stats = IndexMap::new();
    for key in STAT_KEYS {
        if let Some(value) = fields.get(key) {
            stats.insert(key.to_owned(), value.clone());
        }
    }
    derive_tokens_saved(&mut stats);
    stats
}

fn derive_tokens_saved(stats: &mut IndexMap<String, JsonValue>) {
    if stats.contains_key(TOKENS_SAVED) {
        return;
    }
    let Some(JsonValue::Number(before)) = stats.get(TOKENS_BEFORE) else {
        return;
    };
    let Some(JsonValue::Number(after)) = stats.get(TOKENS_AFTER) else {
        return;
    };
    let Some(derived) = subtract(before, after) else {
        return;
    };
    stats.insert(TOKENS_SAVED.to_owned(), JsonValue::Number(derived));
}

fn subtract(before: &Number, after: &Number) -> Option<Number> {
    match (before.as_i64(), after.as_i64()) {
        (Some(left), Some(right)) => {
            i64::try_from(i128::from(left).checked_sub(i128::from(right))?)
                .ok()
                .map(Number::from)
        }
        _ => {
            let left = before.as_f64()?;
            let right = after.as_f64()?;
            Number::from_f64(left - right)
        }
    }
}
