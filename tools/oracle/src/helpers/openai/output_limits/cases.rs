use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_openai::output_limits::build::{Recovery, build_limit_response};
use betterlite_openai::output_limits::raw_send::{send_raw_chat_async, send_raw_chat_sync};
use betterlite_openai::output_limits::{ErrorBody, RejectedFieldsError, without_rejected_fields};
use betterlite_oracle_helper_api::helper::HelperResult;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::project::{failure, header_map, raw_failure, reply_report};
use super::transports::{Scripted, recorded};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DropCase {
    pub channel: DropChannel,
    pub body: Value,
    pub data: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DropChannel {
    Http,
    Sdk,
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
    // boxed so it stays within 3x of MarkerCase, the limit variant_size_differences enforces
    pub outcome: Box<AzureOutcome>,
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
    // any other APIStatusError, which the wrappers rethrow with its status and response intact
    Status {
        status: u16,
        headers: BTreeMap<String, String>,
        body: Value,
    },
    Timeout,
}

// the HTTP channel is the raw response text of an httpx status error, the SDK channel the exception's body
pub(crate) fn drop_case(case: &DropCase) -> HelperResult {
    let body = match case.channel {
        DropChannel::Http => ErrorBody::Http(case.body.as_str().unwrap_or_default().as_bytes()),
        DropChannel::Sdk => ErrorBody::Sdk(&case.body),
    };
    // LiteLLM decodes the reply as JSON, then reads the message and each detail entry through attribute access, so a non-JSON reply fails as JSONDecodeError and every other failure here as Python's AttributeError
    match without_rejected_fields(body, &case.data) {
        Ok(dropped) => Ok(Value::Object(dropped)),
        Err(error @ RejectedFieldsError::Json { .. }) => {
            Err(failure("JSONDecodeError", None, error.to_string()))
        }
        Err(error) => Err(failure("AttributeError", None, error.to_string())),
    }
}

pub(crate) async fn build_case(case: &BuildCase, recovery: &Recovery<'_>) -> HelperResult {
    let response = build_limit_response(&case.data, header_map(&case.headers), recovery)
        .await
        .map_err(|error| failure("PromptCountError", None, error.to_string()))?;
    let report = reply_report(
        &response.headers,
        "x-request-id",
        &response.body,
        &case.data,
    )?;
    Ok(Value::Object(report))
}

pub(crate) async fn azure_case(case: &AzureCase, recovery: &Recovery<'_>) -> HelperResult {
    let transport = Scripted {
        seen: Mutex::new(Vec::new()),
        outcome: case.outcome.clone(),
    };
    let response = if case.asynchronous {
        send_raw_chat_async(
            &transport,
            &case.data,
            case.timeout,
            || Duration::ZERO,
            recovery,
        )
        .await
    } else {
        send_raw_chat_sync(&transport, &case.data, case.timeout, recovery).await
    };
    let requests = recorded(&transport);
    let answered = response.map_err(raw_failure)?;
    let mut report = reply_report(
        &answered.headers,
        "x-ms-request-id",
        &answered.body,
        &case.data,
    )?;
    report.insert(String::from("requests"), requests);
    Ok(Value::Object(report))
}
