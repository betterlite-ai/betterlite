#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::io;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::CredentialSource;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock_guardrails::bedrock_post::GuardrailTransport;
use betterlite_bedrock_guardrails::bedrock_transport::{BedrockGuardrail, BedrockGuardrailInit};
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
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;

use super::batch::{BatchInvocation, apply_guardrail};
use crate::bedrock_apply::ApiInvocation;
use crate::bedrock_checks::request::ChecksEnvironment;
use crate::generic_guardrail_contracts::{GuardrailInputType, GuardrailMessage};
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::guardrail_scan_cache::{ScanCacheStore, scan_cache_key, scanned_text_hash};

pub(super) const GUARDRAIL_NAME: &str = "bedrock-incremental";
pub(super) const SESSION: &str = "session-1";
pub(super) const TTL_SECONDS: i64 = 86400;
pub(super) const RECORDS_KEY: &str = "standard_logging_guardrail_information";

const REGION: &str = "us-west-2";
const ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
const IDENTIFIER: &str = "gr-abc123";
const VERSION: &str = "1";
const FIXED_NOW_SECONDS: u64 = 1_000_000;
const ENVIRONMENT: ChecksEnvironment<'static> = ChecksEnvironment {
    runtime_endpoint: None,
    region_name: None,
    region: None,
    bearer_token: None,
};

pub(super) struct Reply {
    status: u16,
    body: Vec<u8>,
}

impl Reply {
    pub(super) fn ok(body: &str) -> Self {
        Self::text(200, body)
    }

    pub(super) fn failed(status: u16, body: &str) -> Self {
        Self::text(status, body)
    }

    fn text(status: u16, text: &str) -> Self {
        Self {
            status,
            body: text.as_bytes().to_vec(),
        }
    }
}

pub(super) fn passing() -> Reply {
    Reply::ok(r#"{"action": "NONE", "usage": {"textUnits": 3}}"#)
}

pub(super) fn masked(text: &str) -> Reply {
    let body = format!(
        r#"{{"action": "NONE", "outputs": [{{"text": {}}}]}}"#,
        quoted(text)
    );
    Reply::ok(&body)
}

pub(super) fn blocking() -> Reply {
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

pub(super) struct Scripted {
    replies: Mutex<VecDeque<Reply>>,
    sent: Mutex<Vec<OutboundRequest>>,
    closed: AtomicBool,
}

impl Scripted {
    pub(super) fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            sent: Mutex::new(Vec::new()),
            closed: AtomicBool::new(false),
        }
    }

    pub(super) fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
    }

    pub(super) fn count(&self) -> usize {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub(super) fn bodies(&self) -> Vec<Json> {
        let sent = self.sent.lock().unwrap_or_else(PoisonError::into_inner);
        sent.iter().map(body_of).collect()
    }

    pub(super) fn urls(&self) -> Vec<String> {
        let sent = self.sent.lock().unwrap_or_else(PoisonError::into_inner);
        sent.iter().map(|request| request.url.to_string()).collect()
    }
}

impl GuardrailTransport for Scripted {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        if self.closed.load(Ordering::Relaxed) {
            return ready(Err(TransportError::Closed));
        }
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

pub(super) struct StaticCredentials;

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

fn body_of(request: &OutboundRequest) -> Json {
    match &request.body {
        RequestBody::Json(bytes) => serde_json::from_slice(bytes).expect("the body is JSON"),
        RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
            Json::Null
        }
    }
}

pub(super) fn guardrail() -> BedrockGuardrail {
    build(false)
}

pub(super) fn guardrail_without_block_exception() -> BedrockGuardrail {
    build(true)
}

fn build(disable_exception_on_block: bool) -> BedrockGuardrail {
    let mut params = BTreeMap::new();
    let pairs = [
        ("aws_access_key_id", ACCESS_KEY),
        ("aws_secret_access_key", SECRET_KEY),
        ("aws_region_name", REGION),
    ];
    for (key, value) in pairs {
        params.insert(key.to_owned(), GuardrailValue::Text(value.to_owned()));
    }
    BedrockGuardrail::__init__(BedrockGuardrailInit {
        guardrail_identifier: Some(IDENTIFIER.to_owned()),
        guardrail_version: Some(VERSION.to_owned()),
        disable_exception_on_block: Some(disable_exception_on_block),
        checks: None,
        content_filter_threshold: None,
        prompt_attack_threshold: None,
        pii_confidence_threshold: None,
        chunk_budget_chars: 25,
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

pub(super) fn base() -> GuardrailBase {
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
            switches: GuardrailSwitches::default(),
        },
        mode: None,
    }
}

pub(super) struct MemoryCache {
    pub(super) entries: Mutex<BTreeMap<String, Vec<String>>>,
    pub(super) ttl_writes: Mutex<Vec<i64>>,
    pub(super) fail_reads: bool,
}

impl Default for MemoryCache {
    fn default() -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
            ttl_writes: Mutex::new(Vec::new()),
            fail_reads: false,
        }
    }
}

impl MemoryCache {
    pub(super) fn remember(&self, session: &str, texts: &[&str]) {
        let hashes = texts.iter().map(|text| scanned_text_hash(text)).collect();
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(scan_cache_key(Some(GUARDRAIL_NAME), session), hashes);
    }

    pub(super) fn hashes(&self, session: &str) -> Vec<String> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&scan_cache_key(Some(GUARDRAIL_NAME), session))
            .cloned()
            .unwrap_or_default()
    }

    pub(super) fn ttls(&self) -> Vec<i64> {
        self.ttl_writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ScanCacheStore for MemoryCache {
    type Error = io::Error;

    fn get(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<Vec<String>>, io::Error>> + Send {
        if self.fail_reads {
            return ready(Err(io::Error::other("the cache is down")));
        }
        ready(Ok(self
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()))
    }

    fn set(
        &self,
        key: &str,
        hashes: Vec<String>,
        ttl_seconds: i64,
    ) -> impl Future<Output = Result<(), io::Error>> + Send {
        self.ttl_writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ttl_seconds);
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), hashes);
        ready(Ok(()))
    }
}

pub(super) struct Harness {
    pub(super) guardrail: BedrockGuardrail,
    pub(super) base: GuardrailBase,
    pub(super) data: BTreeMap<String, GuardrailValue>,
    pub(super) transport: Scripted,
    pub(super) cache: MemoryCache,
    pub(super) ttl_seconds: i64,
    pub(super) tool_results_only: bool,
}

impl Harness {
    pub(super) fn new(replies: Vec<Reply>) -> Self {
        Self {
            guardrail: guardrail(),
            base: base(),
            data: BTreeMap::new(),
            transport: Scripted::new(replies),
            cache: MemoryCache::default(),
            ttl_seconds: TTL_SECONDS,
            tool_results_only: false,
        }
    }

    pub(super) fn with_session(self) -> Self {
        let mut harness = self;
        harness.data.insert(
            "litellm_session_id".to_owned(),
            GuardrailValue::Text(SESSION.to_owned()),
        );
        harness
    }

    pub(super) fn incremental(&mut self) {
        self.base.settings.only_scan_new_messages = true;
    }

    pub(super) fn latest_user_only(&mut self) {
        self.guardrail.latest_role_message_only = true;
    }

    pub(super) async fn apply(
        &mut self,
        inputs: GuardrailApiInputs,
        input_type: GuardrailInputType,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        let catalog = BundledCatalog::default().get();
        let mut batch = BatchInvocation {
            api: ApiInvocation {
                guardrail: &self.guardrail,
                base: &self.base,
                request_data: &mut self.data,
                log_target: None,
                environment: ENVIRONMENT,
                catalog: catalog.as_ref(),
                now: fixed_now,
            },
            cache: &self.cache,
            cache_ttl_seconds: self.ttl_seconds,
            tool_results_only: self.tool_results_only,
        };
        apply_guardrail(
            &mut batch,
            inputs,
            input_type,
            &StaticCredentials,
            &self.transport,
        )
        .await
    }
}

fn fixed_now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(FIXED_NOW_SECONDS))
        .unwrap_or(UNIX_EPOCH)
}

pub(super) fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(super) fn inputs(values: &[&str]) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: Some(strings(values)),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    }
}

pub(super) fn texts_of(inputs: &GuardrailApiInputs) -> Vec<String> {
    inputs.texts.clone().unwrap_or_default()
}

pub(super) fn data_messages(json: &str) -> GuardrailValue {
    serde_json::from_str(json).expect("the messages parse")
}

pub(super) fn structured_messages(json: &str) -> Vec<GuardrailMessage> {
    serde_json::from_str(json).expect("the structured messages parse")
}

pub(super) fn field_of(value: &Json, key: &str) -> Json {
    let Json::Object(fields) = value else {
        return Json::Null;
    };
    fields.get(key).cloned().unwrap_or(Json::Null)
}

pub(super) fn content_texts(body: &Json) -> Vec<String> {
    let Json::List(items) = field_of(body, "content") else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match field_of(item, "text") {
            Json::Object(fields) => match fields.get("text") {
                Some(Json::Text(text)) => Some(text.clone()),
                Some(_) | None => None,
            },
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        })
        .collect()
}

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn expected(source: &str) -> Json {
    serde_json::from_str(source).expect("the expected JSON parses")
}

pub(super) fn quoted(value: &str) -> String {
    serde_json::to_string(value).expect("the text encodes as a JSON string")
}

pub(super) fn records_of(data: &BTreeMap<String, GuardrailValue>) -> Vec<Json> {
    let Some(GuardrailValue::Object(metadata)) = data.get("metadata") else {
        return Vec::new();
    };
    let Some(records) = metadata.get(RECORDS_KEY) else {
        return Vec::new();
    };
    let text = serde_json::to_string(records).expect("the records encode");
    serde_json::from_str(&text).expect("the records are a list")
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Captured {
    pub(super) fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("tracing output is utf8")
    }
}

pub(super) fn capture(level: tracing::Level) -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = fmt()
        .with_max_level(level)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let guard = set_default(subscriber);
    (captured, guard)
}
