#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::CredentialSource;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock_guardrails::bedrock_post::GuardrailTransport;
use betterlite_bedrock_guardrails::bedrock_transport::{BedrockGuardrail, BedrockGuardrailInit};
use betterlite_bedrock_guardrails::guardrail_checks::ChecksInput;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_config::sections::content_filter_records::ConfidenceThreshold;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use reqwest::header::HeaderMap;
use secrecy::SecretString;
use serde_json::Number;

use crate::bedrock_checks::request::{
    ChecksEnvironment, ChecksInvocation, ChecksScan, invoke_checks,
};
use crate::bedrock_checks::violations::ChecksThresholds;
use crate::bedrock_message_selection::wire::{Message, ModelResponse};
use crate::bedrock_records::{ChecksMessage, ChecksResponse, ChecksViolation, Response, Source};
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};

pub(crate) const CHECKS_PATH: &str = "/guardrail-checks/invoke";
pub(crate) const RUNTIME_URL: &str = "https://bedrock-runtime.us-west-2.amazonaws.com";
pub(crate) const GUARDRAIL_NAME: &str = "bedrock-checks";
const RECORDS_KEY: &str = "standard_logging_guardrail_information";

pub(crate) fn checks_named(names: &[&str]) -> BTreeMap<String, GuardrailValue> {
    let mut checks = BTreeMap::new();
    for name in names {
        checks.insert((*name).to_owned(), GuardrailValue::Object(BTreeMap::new()));
    }
    checks
}

pub(crate) fn all_checks() -> BTreeMap<String, GuardrailValue> {
    checks_named(&["contentFilter", "promptAttack", "sensitiveInformation"])
}

pub(crate) fn aws_params() -> OpaqueParams {
    let pairs = [
        ("aws_access_key_id", "AKIAIOSFODNN7EXAMPLE"),
        (
            "aws_secret_access_key",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        ),
        ("aws_region_name", "us-west-2"),
    ];
    OpaqueParams(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), GuardrailValue::Text(value.to_owned())))
            .collect(),
    )
}

pub(crate) fn guardrail_init(checks: BTreeMap<String, GuardrailValue>) -> BedrockGuardrailInit {
    BedrockGuardrailInit {
        guardrail_identifier: None,
        guardrail_version: None,
        disable_exception_on_block: None,
        checks: Some(ChecksInput::Mapping(checks)),
        content_filter_threshold: None,
        prompt_attack_threshold: None,
        pii_confidence_threshold: None,
        chunk_budget_chars: 25_000,
        contextual_grounding: false,
        latest_role_message_only: false,
        mask_request_content: false,
        mask_response_content: false,
        buffer_until_moderated: None,
        streaming_sampling_rate: None,
        streaming_end_of_stream_only: None,
        buffer_release_on_scan: None,
        optional_params: aws_params(),
        timeout: None,
    }
}

pub(crate) fn built(init: BedrockGuardrailInit) -> BedrockGuardrail {
    BedrockGuardrail::__init__(init).expect("the test guardrail")
}

pub(crate) fn guardrail() -> BedrockGuardrail {
    built(guardrail_init(all_checks()))
}

pub(crate) fn base() -> GuardrailBase {
    let settings = GuardrailSettings {
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
    };
    GuardrailBase::new(settings, false).expect("the test base")
}

pub(crate) fn thresholds(
    content_filter: Option<f64>,
    prompt_attack: Option<f64>,
    sensitive_information: Option<f64>,
) -> ChecksThresholds {
    ChecksThresholds {
        content_filter: content_filter.map(ConfidenceThreshold),
        prompt_attack: prompt_attack.map(ConfidenceThreshold),
        sensitive_information: sensitive_information.map(ConfidenceThreshold),
    }
}

pub(crate) fn credentials() -> Credentials {
    Credentials {
        access_key_id: SecretString::from("AKIAIOSFODNN7EXAMPLE".to_owned()),
        secret_access_key: SecretString::from(
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_owned(),
        ),
        session_token: None,
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct StaticCredentials;

impl CredentialSource for StaticCredentials {
    fn resolve(
        &self,
        _settings: AwsSettings,
        _region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        ready(Ok(credentials()))
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MissingCredentials;

impl CredentialSource for MissingCredentials {
    fn resolve(
        &self,
        _settings: AwsSettings,
        _region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        ready(Err(CredentialError::MissingCredentials))
    }
}

#[derive(Clone)]
pub(crate) enum Reply {
    Answer { status: u16, body: String },
    Unreachable,
}

#[derive(Clone)]
pub(crate) struct ScriptedTransport {
    reply: Arc<Mutex<Option<Reply>>>,
    sent: Arc<Mutex<Vec<Sent>>>,
}

impl ScriptedTransport {
    pub(crate) fn replying(status: u16, body: &str) -> Self {
        Self {
            reply: Arc::new(Mutex::new(Some(Reply::Answer {
                status,
                body: body.to_owned(),
            }))),
            sent: Arc::default(),
        }
    }

    pub(crate) fn unreachable() -> Self {
        Self {
            reply: Arc::new(Mutex::new(Some(Reply::Unreachable))),
            sent: Arc::default(),
        }
    }

    pub(crate) fn sent(&self) -> Vec<Sent> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn answer(&self) -> Result<UpstreamResponse, TransportError> {
        let reply = self
            .reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        match reply {
            Some(Reply::Answer { status, body }) => Ok(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body: body.into_bytes(),
            }),
            Some(Reply::Unreachable) | None => Err(TransportError::Closed),
        }
    }
}

impl GuardrailTransport for ScriptedTransport {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Sent::from_request(request));
        ready(self.answer())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Sent {
    pub(crate) method: String,
    pub(crate) url: String,
    pub(crate) content_type: Option<String>,
    pub(crate) authorization: Option<String>,
    pub(crate) body: String,
}

impl Sent {
    fn from_request(request: OutboundRequest) -> Self {
        let body = match request.body {
            RequestBody::Json(bytes) => String::from_utf8(bytes).expect("the JSON body"),
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
                String::new()
            }
        };
        Self {
            method: request.method.to_string(),
            url: request.url.to_string(),
            content_type: header(&request.headers, "content-type"),
            authorization: header(&request.headers, "authorization"),
            body,
        }
    }
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .map(|value| value.to_str().expect("a visible header").to_owned())
}

pub(crate) fn fixed_now() -> SystemTime {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_hours(490_000))
        .expect("the fixed clock")
}

pub(crate) fn environment() -> ChecksEnvironment<'static> {
    ChecksEnvironment {
        runtime_endpoint: None,
        region_name: None,
        region: None,
        bearer_token: None,
    }
}

pub(crate) fn input_scan(messages: &[Message]) -> ChecksScan<'_> {
    ChecksScan {
        source: Source::Input,
        messages: Some(messages),
        response: None,
        logging_event_type: None,
    }
}

pub(crate) fn output_scan(response: &ModelResponse) -> ChecksScan<'_> {
    ChecksScan {
        source: Source::Output,
        messages: None,
        response: Some(response),
        logging_event_type: None,
    }
}

pub(crate) fn during_call(mut scan: ChecksScan<'_>) -> ChecksScan<'_> {
    scan.logging_event_type = Some(GuardrailEventHooks::DuringCall);
    scan
}

pub(crate) async fn run_checks<C, T>(
    guardrail: &BedrockGuardrail,
    request_data: &mut BTreeMap<String, GuardrailValue>,
    scan: ChecksScan<'_>,
    credentials: &C,
    transport: &T,
) -> Result<Response, GuardrailFailure>
where
    C: CredentialSource,
    T: GuardrailTransport,
{
    let invocation_base = base();
    let mut invocation = ChecksInvocation {
        guardrail,
        base: &invocation_base,
        request_data,
        log_target: None,
        environment: environment(),
        now: fixed_now,
    };
    invoke_checks(&mut invocation, scan, credentials, transport).await
}

pub(crate) fn messages(json: &str) -> Vec<Message> {
    serde_json::from_str(json).expect("the test messages")
}

pub(crate) fn model_response(json: &str) -> ModelResponse {
    serde_json::from_str(json).expect("the test response")
}

pub(crate) fn checks_response(json: &str) -> ChecksResponse {
    serde_json::from_str(json).expect("the test checks response")
}

pub(crate) fn blocks(message: &ChecksMessage) -> Vec<String> {
    let mut texts = Vec::new();
    if let Some(content) = message.content.as_ref() {
        for block in content {
            if let Some(text) = block.text.as_ref() {
                texts.push(text.clone());
            }
        }
    }
    texts
}

pub(crate) fn records(
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Vec<BTreeMap<String, GuardrailValue>> {
    let mut records = Vec::new();
    if let Some(GuardrailValue::Object(metadata)) = request_data.get("metadata")
        && let Some(GuardrailValue::Array(entries)) = metadata.get(RECORDS_KEY)
    {
        for entry in entries {
            if let GuardrailValue::Object(fields) = entry {
                records.push(fields.clone());
            }
        }
    }
    records
}

pub(crate) fn only_record(
    request_data: &BTreeMap<String, GuardrailValue>,
) -> BTreeMap<String, GuardrailValue> {
    let mut records = records(request_data);
    assert_eq!(records.len(), 1);
    records.remove(0)
}

pub(crate) fn field(fields: &BTreeMap<String, GuardrailValue>, key: &str) -> GuardrailValue {
    fields.get(key).expect("the record field").clone()
}

pub(crate) enum Label<'value> {
    Category(&'value str),
    TypeName(&'value str),
}

pub(crate) fn scored(check: &str, label: &Label<'_>, score: f64) -> ChecksViolation {
    let (category, type_name, severity_score, confidence_score) = match *label {
        Label::Category(category) => (
            Some(category.to_owned()),
            None,
            Number::from_f64(score),
            None,
        ),
        Label::TypeName(type_name) => (
            None,
            Some(type_name.to_owned()),
            None,
            Number::from_f64(score),
        ),
    };
    ChecksViolation {
        check: Some(check.to_owned()),
        category,
        type_name,
        severity_score,
        confidence_score,
        truncated: None,
        extra: BTreeMap::new(),
    }
}

pub(crate) fn truncated_violation() -> ChecksViolation {
    ChecksViolation {
        check: Some("sensitiveInformation".to_owned()),
        category: None,
        type_name: None,
        severity_score: None,
        confidence_score: None,
        truncated: Some(true),
        extra: BTreeMap::new(),
    }
}

pub(crate) fn as_object(value: &GuardrailValue) -> &BTreeMap<String, GuardrailValue> {
    let GuardrailValue::Object(fields) = value else {
        panic!("expected an object: {value:?}");
    };
    fields
}

pub(crate) fn as_array(value: &GuardrailValue) -> &[GuardrailValue] {
    let GuardrailValue::Array(items) = value else {
        panic!("expected an array: {value:?}");
    };
    items
}

pub(crate) fn as_text(value: &GuardrailValue) -> &str {
    let GuardrailValue::Text(text) = value else {
        panic!("expected text: {value:?}");
    };
    text
}
