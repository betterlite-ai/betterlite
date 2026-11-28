#![cfg(test)]

mod chunking;
mod dispatch;
mod logging;
mod posting;
mod request;
mod retry;
mod signing;

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::CredentialSource;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock_guardrails::bedrock_post::GuardrailTransport;
use betterlite_bedrock_guardrails::bedrock_transport::{BedrockGuardrail, BedrockGuardrailInit};
use betterlite_bedrock_guardrails::guardrail_checks::ChecksInput;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use reqwest::header::HeaderMap;
use secrecy::SecretString;
use serde::Serialize;
use serde_json::Number;

use super::*;
use crate::bedrock_checks::request::ChecksEnvironment;
use crate::bedrock_message_selection::wire::{Message, ModelResponse};
use crate::bedrock_records::{Response, Source};
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};

const REGION: &str = "us-west-2";
const ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
const IDENTIFIER: &str = "gr-abc123";
const VERSION: &str = "1";
const GUARDRAIL_NAME: &str = "bedrock-apply";
const RECORDS_KEY: &str = "standard_logging_guardrail_information";
const FIXED_NOW_SECONDS: u64 = 1_000_000;
const ENVIRONMENT: ChecksEnvironment<'static> = ChecksEnvironment {
    runtime_endpoint: None,
    region_name: None,
    region: None,
    bearer_token: None,
};

struct Reply {
    status: u16,
    body: Vec<u8>,
}

impl Reply {
    fn ok(body: &str) -> Self {
        Self::text(200, body)
    }

    fn failed(status: u16, body: &str) -> Self {
        Self::text(status, body)
    }

    fn text(status: u16, text: &str) -> Self {
        Self {
            status,
            body: text.as_bytes().to_vec(),
        }
    }
}

fn too_large() -> Reply {
    Reply::failed(
        429,
        r#"{ "message": "Input is too long for requested model." }"#,
    )
}

fn passing(text_units: i64) -> Reply {
    let body = format!(r#"{{"action": "NONE", "usage": {{"textUnits": {text_units}}}}}"#);
    Reply::ok(&body)
}

fn masked(text: &str) -> Reply {
    let body = format!(
        r#"{{"action": "NONE", "outputs": [{{"text": {}}}]}}"#,
        quoted(text)
    );
    Reply::ok(&body)
}

fn blocking() -> Reply {
    Reply::ok(
        r#"{
            "action": "GUARDRAIL_INTERVENED",
            "assessments": [{
                "topicPolicy": { "topics": [{ "name": "denied", "type": "DENY", "action": "BLOCKED" }] }
            }],
            "outputs": [{ "text": "blocked text" }],
            "usage": { "textUnits": 7 }
        }"#,
    )
}

struct Scripted {
    replies: Mutex<VecDeque<Reply>>,
    sent: Mutex<Vec<OutboundRequest>>,
}

impl Scripted {
    fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            sent: Mutex::new(Vec::new()),
        }
    }

    fn count(&self) -> usize {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    fn urls(&self) -> Vec<String> {
        let sent = self.sent.lock().unwrap_or_else(PoisonError::into_inner);
        sent.iter().map(|request| request.url.to_string()).collect()
    }

    fn bodies(&self) -> Vec<Json> {
        let sent = self.sent.lock().unwrap_or_else(PoisonError::into_inner);
        sent.iter().map(body_of).collect()
    }

    fn header_values(&self, name: &str) -> Vec<Option<String>> {
        let sent = self.sent.lock().unwrap_or_else(PoisonError::into_inner);
        sent.iter()
            .map(|request| header_of(request, name))
            .collect()
    }
}

fn body_of(request: &OutboundRequest) -> Json {
    match &request.body {
        RequestBody::Json(bytes) => serde_json::from_slice(bytes).expect("the body is JSON"),
        RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
            Json::Null
        }
    }
}

fn header_of(request: &OutboundRequest, name: &str) -> Option<String> {
    request
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

impl GuardrailTransport for Scripted {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let reply = {
            let mut replies = self.replies.lock().unwrap_or_else(PoisonError::into_inner);
            replies.pop_front()
        };
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let outcome = reply.map(|given| UpstreamResponse {
            status: given.status,
            headers: HeaderMap::new(),
            body: given.body,
        });
        ready(outcome.ok_or(TransportError::Closed))
    }
}

struct Closed;

impl GuardrailTransport for Closed {
    fn send(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(Err(TransportError::Closed))
    }
}

struct StaticCredentials;

impl CredentialSource for StaticCredentials {
    fn resolve(
        &self,
        _settings: AwsSettings,
        _region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        ready(Ok(Credentials {
            access_key_id: SecretString::from(ACCESS_KEY.to_owned()),
            secret_access_key: SecretString::from(SECRET_KEY.to_owned()),
            session_token: None,
        }))
    }
}

fn guardrail() -> BedrockGuardrail {
    build(Some(IDENTIFIER), 25, None, false)
}

fn guardrail_with_budget(budget: u64) -> BedrockGuardrail {
    build(Some(IDENTIFIER), budget, None, false)
}

fn guardrail_with_checks() -> BedrockGuardrail {
    let mut checks = BTreeMap::new();
    checks.insert(
        "contentFilter".to_owned(),
        GuardrailValue::Object(BTreeMap::new()),
    );
    build(None, 25, Some(ChecksInput::Mapping(checks)), false)
}

fn anonymous_guardrail() -> BedrockGuardrail {
    build(None, 25, None, false)
}

fn guardrail_without_block_exception() -> BedrockGuardrail {
    build(Some(IDENTIFIER), 25, None, true)
}

fn build(
    identifier: Option<&str>,
    budget: u64,
    checks: Option<ChecksInput>,
    disable_exception_on_block: bool,
) -> BedrockGuardrail {
    let mut params = BTreeMap::new();
    let pairs = [
        ("aws_access_key_id", ACCESS_KEY),
        ("aws_secret_access_key", SECRET_KEY),
        ("aws_region_name", REGION),
    ];
    for (key, value) in pairs {
        params.insert(key.to_owned(), GuardrailValue::Text(value.to_owned()));
    }
    let version = identifier.map(|_| VERSION.to_owned());
    BedrockGuardrail::__init__(BedrockGuardrailInit {
        guardrail_identifier: identifier.map(str::to_owned),
        guardrail_version: version,
        disable_exception_on_block: Some(disable_exception_on_block),
        checks,
        content_filter_threshold: None,
        prompt_attack_threshold: None,
        pii_confidence_threshold: None,
        chunk_budget_chars: budget,
        contextual_grounding: false,
        latest_role_message_only: false,
        mask_request_content: false,
        mask_response_content: false,
        buffer_until_moderated: None,
        streaming_sampling_rate: None,
        streaming_end_of_stream_only: None,
        buffer_release_on_scan: None,
        optional_params: OpaqueParams(params),
        timeout: None,
    })
    .expect("the guardrail builds")
}

fn base() -> GuardrailBase {
    GuardrailBase {
        settings: GuardrailSettings {
            guardrail_name: Some(GUARDRAIL_NAME.to_owned()),
            supported_event_hooks: None,
            event_hook: None,
            default_on: false,
            mask_request_content: false,
            mask_response_content: false,
            violation_message_template: None,
            end_session_after_n_fails: None,
            on_violation: None,
            realtime_violation_message: None,
            on_sensitive_data: None,
            sensitive_data_route_to_model: None,
            sticky_session_routing: true,
            run_in_parallel: false,
            scan_raw_request: false,
            only_scan_new_messages: false,
            timeout: None,
            switches: GuardrailSwitches {
                use_native_during_call_hook: false,
                use_native_lifecycle_hooks: false,
                records_own_information: false,
                uses_apply_guardrail_interface: false,
                deny_text_on_error: false,
            },
        },
        mode: None,
    }
}

fn fixed_now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(FIXED_NOW_SECONDS))
        .unwrap_or(UNIX_EPOCH)
}

fn message(role: &str, text: &str) -> Message {
    structured_message(role, &quoted(text))
}

// the content arrives as JSON text, so a grounded turn can pass its own query and guard-content items
fn structured_message(role: &str, content: &str) -> Message {
    let wire = format!(r#"{{"role":{},"content":{content}}}"#, quoted(role));
    serde_json::from_str(&wire).expect("the message parses")
}

fn model_response(text: &str) -> ModelResponse {
    let wire = format!(
        r#"{{"choices":[{{"message":{{"role":"assistant","content":{}}}}}]}}"#,
        quoted(text)
    );
    serde_json::from_str(&wire).expect("the response parses")
}

fn input_scan(messages: &[Message]) -> ChecksScan<'_> {
    ChecksScan {
        source: Source::Input,
        messages: Some(messages),
        response: None,
        logging_event_type: None,
    }
}

fn output_scan<'scan>(
    messages: Option<&'scan [Message]>,
    response: &'scan ModelResponse,
) -> ChecksScan<'scan> {
    ChecksScan {
        source: Source::Output,
        messages,
        response: Some(response),
        logging_event_type: None,
    }
}

async fn run<T>(
    guardrail: &BedrockGuardrail,
    data: &mut BTreeMap<String, GuardrailValue>,
    scan: ChecksScan<'_>,
    transport: &T,
) -> Result<Response, GuardrailFailure>
where
    T: GuardrailTransport,
{
    run_at(guardrail, data, scan, transport, fixed_now).await
}

async fn run_at<T>(
    guardrail: &BedrockGuardrail,
    data: &mut BTreeMap<String, GuardrailValue>,
    scan: ChecksScan<'_>,
    transport: &T,
    clock: fn() -> SystemTime,
) -> Result<Response, GuardrailFailure>
where
    T: GuardrailTransport,
{
    let settings = base();
    let catalog = BundledCatalog::default().get();
    let mut invocation = ApiInvocation {
        guardrail,
        base: &settings,
        request_data: data,
        log_target: None,
        environment: ENVIRONMENT,
        catalog: catalog.as_ref(),
        now: clock,
    };
    let credentials = StaticCredentials;
    make_bedrock_api_request(&mut invocation, scan, &credentials, transport).await
}

fn records_of(data: &BTreeMap<String, GuardrailValue>) -> Vec<Json> {
    let Some(GuardrailValue::Object(metadata)) = data.get("metadata") else {
        return Vec::new();
    };
    let Some(records) = metadata.get(RECORDS_KEY) else {
        return Vec::new();
    };
    let text = serde_json::to_string(records).expect("the records encode");
    serde_json::from_str(&text).expect("the records are a list")
}

fn expected(source: &str) -> Json {
    serde_json::from_str(source).expect("the expected JSON parses")
}

fn wire_of<T>(value: &T) -> Json
where
    T: Serialize,
{
    let text = serde_json::to_string(value).expect("the wire value encodes");
    serde_json::from_str(&text).expect("the wire value parses")
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: u64) -> Json {
    Json::Number(Number::from(value))
}

fn quoted(value: &str) -> String {
    serde_json::to_string(value).expect("the text encodes as a JSON string")
}

fn field_of(value: &Json, key: &str) -> Json {
    let Json::Object(fields) = value else {
        return Json::Null;
    };
    fields.get(key).cloned().unwrap_or(Json::Null)
}

fn has_field(value: &Json, key: &str) -> bool {
    matches!(value, Json::Object(fields) if fields.contains_key(key))
}

fn usage_units(record: &Json, counter: &str) -> Json {
    let usage = field_of(record, "guardrail_usage");
    field_of(&usage, counter)
}
