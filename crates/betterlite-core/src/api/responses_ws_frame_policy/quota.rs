use std::sync::Arc;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_error::error::{ErrorClass, class_message};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::websocket::ResponsesClientWebSocket;
use indexmap::IndexMap;
use serde::Serialize;

use super::{CHARS_PER_TOKEN_ESTIMATE, FramePolicy, NO_MAX_OUTPUT_TOKENS_FLOOR};
use crate::api::responses_stream_events::ProjectQuotaCallback;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameQuotaEstimate {
    pub input_tokens: i64,
    pub output_tokens: i64,
}

#[derive(Serialize)]
struct RejectionFrame {
    #[serde(rename = "type")]
    kind: &'static str,
    error: RejectionError,
}

#[derive(Serialize)]
struct RejectionError {
    #[serde(rename = "type")]
    kind: &'static str,
    message: String,
}

// an empty nested response falls back to the top level, while restoring and authorizing still treat it as nested
#[must_use]
pub fn frame_quota_estimate(frame: &IndexMap<String, Json>) -> FrameQuotaEstimate {
    let params = if let Some(Json::Object(nested)) = frame.get("response")
        && !nested.is_empty()
    {
        nested
    } else {
        frame
    };
    let total_chars = frame_text_chars(params);
    let quotient = total_chars
        .checked_div(CHARS_PER_TOKEN_ESTIMATE)
        .unwrap_or(0);
    let input_tokens = if total_chars == 0 { 0 } else { quotient.max(1) };
    // Python's bool is an int, so max_output_tokens=true is an explicit cap of one token
    let output_tokens = match params.get("max_output_tokens") {
        Some(Json::Number(number)) => number.as_i64().unwrap_or(NO_MAX_OUTPUT_TOKENS_FLOOR),
        Some(Json::Bool(flag)) => i64::from(*flag),
        None | Some(Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            NO_MAX_OUTPUT_TOKENS_FLOOR
        }
    };
    FrameQuotaEstimate {
        input_tokens: i64::try_from(input_tokens).unwrap_or(i64::MAX),
        output_tokens,
    }
}

// a work stack rather than recursion: instructions pops before input, and an item's text before its content
fn frame_text_chars(params: &IndexMap<String, Json>) -> usize {
    let mut pending = vec![params.get("input"), params.get("instructions")];
    let mut total = 0_usize;
    while let Some(value) = pending.pop() {
        let items = match value {
            Some(Json::Text(text)) => {
                total = total.saturating_add(text.chars().count());
                continue;
            }
            Some(Json::List(items)) => items,
            None | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_)) => continue,
        };
        for item in items {
            match item {
                Json::Text(text) => total = total.saturating_add(text.chars().count()),
                Json::Object(fields) => pending.extend([fields.get("content"), fields.get("text")]),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {}
            }
        }
    }
    total
}

pub async fn enforce_frame_project_quota(
    quota_callbacks: &[Arc<dyn ProjectQuotaCallback>],
    user_api_key_dict: Option<&UserAPIKeyAuth>,
    model: Option<&str>,
    raw_message: &str,
) -> Result<(), ErrorClass> {
    if quota_callbacks.is_empty() {
        return Ok(());
    }
    let Ok(Json::Object(frame)) = serde_json::from_str::<Json>(raw_message) else {
        return Ok(());
    };
    if !matches!(frame.get("type"), Some(Json::Text(kind)) if kind == "response.create") {
        return Ok(());
    }
    let estimate = frame_quota_estimate(&frame);
    for callback in quota_callbacks {
        callback
            .enforce_frame_quota(
                user_api_key_dict,
                model,
                estimate.input_tokens,
                estimate.output_tokens,
            )
            .await?;
    }
    Ok(())
}

// a rejected frame is dropped and the connection stays open, so the client can retry once the window resets
pub async fn enforce_or_reject_frame<Guard, Client>(
    policy: &FramePolicy<'_, Guard>,
    client: &Client,
    message: &str,
) -> Result<bool, ErrorClass>
where
    Guard: Sync,
    Client: ResponsesClientWebSocket + Sync,
{
    let outcome = enforce_frame_project_quota(
        policy.quota_callbacks,
        policy.user_api_key_dict,
        policy.authorized_model,
        message,
    )
    .await;
    let Err(error) = outcome else {
        return Ok(true);
    };
    if !matches!(error, ErrorClass::RateLimit(_)) {
        return Err(error);
    }
    let rejection = RejectionFrame {
        kind: "error",
        error: RejectionError {
            kind: "rate_limit_exceeded",
            message: class_message(&error),
        },
    };
    if let Ok(text) = serde_json::to_string(&rejection) {
        // the client may already be gone; the frame is dropped either way
        let _sent = client.send_text(&text).await;
    }
    Ok(false)
}
