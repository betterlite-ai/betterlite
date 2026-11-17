use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_openai::output_limits::build::{
    RecoveredBody, Recovery, build_output_token_limit_response,
};
use betterlite_openai::output_limits::raw_send::{send_raw_chat_async, send_raw_chat_sync};
use betterlite_openai::output_limits::tokens::TokenizerPromptTokens;
use betterlite_openai::output_limits::{
    ErrorBody, drop_params_from_unprocessable_entity_error, is_output_token_limit_error,
};
use betterlite_oracle_helper_api::helper::HelperResult;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::project::{
    body_of, encoded, failure, header_map, header_of, projected_chunk, projected_completion,
    raw_failure,
};
use super::transports::{Scripted, recorded};
use super::{fixed_clock, fixed_id, no_elapsed};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DropCase {
    pub body: Value,
    pub data: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MarkerCase {
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BuildCase {
    pub data: Map<String, Value>,
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AzureCase {
    pub asynchronous: bool,
    pub data: Map<String, Value>,
    pub timeout: f64,
    pub outcome: AzureOutcome,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum AzureOutcome {
    Reply {
        headers: BTreeMap<String, String>,
        body: Value,
    },
    BadRequest {
        headers: BTreeMap<String, String>,
        body: Value,
    },
    Timeout,
}

// a string body is the raw response text of an httpx status error; anything else is the SDK exception's body
pub(crate) fn drop_case(case: &DropCase) -> HelperResult {
    let body = match &case.body {
        Value::String(text) => ErrorBody::Http(text.as_bytes()),
        other => ErrorBody::Sdk(other),
    };
    let dropped = drop_params_from_unprocessable_entity_error(body, &case.data)
        .map_err(|error| failure("RejectedFieldsError", None, error.to_string()))?;
    Ok(Value::Object(dropped))
}

pub(crate) fn marker_case(case: &MarkerCase) -> HelperResult {
    Ok(Value::Bool(is_output_token_limit_error(&case.message)))
}

pub(crate) async fn build_case(case: &BuildCase) -> HelperResult {
    let tokens = TokenizerPromptTokens;
    let recovery = Recovery {
        clock: fixed_clock,
        next_id: fixed_id,
        tokens: &tokens,
    };
    let response =
        build_output_token_limit_response(&case.data, header_map(&case.headers), &recovery)
            .await
            .map_err(|error| failure("PromptCountError", None, error.to_string()))?;
    let body = match &response.body {
        RecoveredBody::Completion(completion) => projected_completion(&encoded(completion)?),
        RecoveredBody::Chunk(chunk) => projected_chunk(&encoded(chunk)?),
    };
    Ok(json!({
        "header": header_of(&response.headers, "x-request-id"),
        "body": body,
    }))
}

pub(crate) async fn azure_case(case: &AzureCase) -> HelperResult {
    let transport = Scripted {
        seen: Mutex::new(Vec::new()),
        outcome: case.outcome.clone(),
    };
    let tokens = TokenizerPromptTokens;
    let recovery = Recovery {
        clock: fixed_clock,
        next_id: fixed_id,
        tokens: &tokens,
    };
    let response = if case.asynchronous {
        send_raw_chat_async(&transport, &case.data, case.timeout, no_elapsed, &recovery).await
    } else {
        send_raw_chat_sync(&transport, &case.data, case.timeout, &recovery).await
    };
    let requests = recorded(&transport);
    let response = response.map_err(raw_failure)?;
    Ok(json!({
        "requests": requests,
        "header": header_of(&response.headers, "x-ms-request-id"),
        "body": body_of(&response.body, &case.data),
    }))
}
