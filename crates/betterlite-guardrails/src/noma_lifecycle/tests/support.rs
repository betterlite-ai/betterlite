use std::collections::{BTreeMap, BTreeSet};
use std::future::IntoFuture as _;
use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Bytes;
use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_http::proxy::base_builder;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio_util::task::TaskTracker;
use tracing::{Level, Subscriber};
use tracing_subscriber::fmt;

use crate::noma_lifecycle::config::{NomaDependencies, NomaParams};
use crate::noma_lifecycle::error::NomaLifecycleError;
use crate::noma_lifecycle::hooks::{PostCallHookInput, RequestHookInput};
use crate::noma_lifecycle::{NomaGuardrail, RequestData};

pub(crate) const SAFE_REPLY: &str =
    r#"{"aggregatedScanResult": false, "scanResult": [{"role": "user", "results": {}}]}"#;
pub(crate) const BLOCKING_REPLY: &str = r#"{"aggregatedScanResult": true, "scanResult": [{"role": "user", "results": {"harmfulContent": {"result": true}}}]}"#;
pub(crate) const ANONYMIZING_USER_REPLY: &str = r#"{"aggregatedScanResult": false, "scanResult": [{"role": "user", "results": {"anonymizedContent": {"anonymized": "clean"}}}]}"#;
pub(crate) const ANONYMIZING_ASSISTANT_REPLY: &str = r#"{"aggregatedScanResult": false, "scanResult": [{"role": "assistant", "results": {"anonymizedContent": {"anonymized": "clean"}}}]}"#;
pub(crate) const USER_MESSAGES: &str = r#"{"messages": [{"role": "user", "content": "hello"}]}"#;
pub(crate) const MODEL_RESPONSE: &str =
    r#"{"id": "chatcmpl-1", "choices": [{"message": {"role": "assistant", "content": "hello"}}]}"#;

pub(crate) struct MockNoma {
    pub(crate) base: String,
    pub(crate) seen: Arc<Mutex<Vec<Bytes>>>,
    _server: JoinSet<IoResult<()>>,
}

pub(crate) async fn mock_noma(status: StatusCode, reply: &str) -> MockNoma {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&seen);
    let canned = reply.to_owned();
    let answer = move |body: Bytes| {
        let captured = Arc::clone(&recorded);
        let reply_body = canned.clone();
        async move {
            captured.lock().unwrap().push(body);
            (status, [("content-type", "application/json")], reply_body)
        }
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, Router::new().fallback(answer)).into_future());
    MockNoma {
        base,
        seen,
        _server: server,
    }
}

// The text of the one input item a scan sent
pub(crate) fn scanned_text(mock: &MockNoma, index: usize) -> String {
    let seen = mock.seen.lock().unwrap();
    let body: GuardrailValue = serde_json::from_slice(&seen[index]).unwrap();
    let GuardrailValue::Object(fields) = body else {
        panic!("the scan body is not an object");
    };
    let Some(GuardrailValue::Array(items)) = fields.get("input") else {
        panic!("the scan body has no input");
    };
    let GuardrailValue::Object(item) = &items[0] else {
        panic!("the input item is not an object");
    };
    let Some(GuardrailValue::Array(parts)) = item.get("content") else {
        panic!("the input item has no content");
    };
    let GuardrailValue::Object(part) = &parts[0] else {
        panic!("the content part is not an object");
    };
    let Some(GuardrailValue::Text(text)) = part.get("text") else {
        panic!("the content part has no text");
    };
    text.clone()
}

pub(crate) fn request_count(mock: &MockNoma) -> usize {
    mock.seen.lock().unwrap().len()
}

pub(crate) fn data(text: &str) -> RequestData {
    let parsed = serde_json::from_str(text).unwrap();
    serde_json::from_value(parsed).unwrap()
}

pub(crate) fn value(text: &str) -> GuardrailValue {
    let parsed = serde_json::from_str(text).unwrap();
    serde_json::from_value(parsed).unwrap()
}

pub(crate) fn fixed_now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000))
        .unwrap()
}

pub(crate) fn environment(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, text)| ((*name).to_owned(), (*text).to_owned()))
        .collect()
}

pub(crate) fn params(api_base: &str) -> NomaParams {
    NomaParams {
        guardrail_name: Some(String::from("noma-test")),
        api_key: Some(SecretString::from(String::from("test-api-key"))),
        api_base: Some(api_base.to_owned()),
        application_id: Some(String::from("test-app")),
        monitor_mode: Some(false),
        block_failures: Some(true),
        anonymize_input: Some(false),
        timeout: None,
        event_hook: None,
        default_on: false,
    }
}

pub(crate) fn hooked(api_base: &str, hook: &str) -> NomaParams {
    let mut hooked = params(api_base);
    hooked.event_hook = Some(LitellmParamsMode::One(hook.to_owned()));
    // a configured hook only runs for a requested guardrail unless it is on by default
    hooked.default_on = true;
    hooked
}

pub(crate) fn build_with(
    configured: &NomaParams,
    env: &BTreeMap<String, String>,
) -> Result<NomaGuardrail, NomaLifecycleError> {
    let dependencies = NomaDependencies {
        client: base_builder().build().unwrap(),
        now: fixed_now,
        env,
        tasks: TaskTracker::new(),
        strict_modes: true,
    };
    NomaGuardrail::new(configured, dependencies)
}

pub(crate) fn build(configured: &NomaParams) -> NomaGuardrail {
    build_with(configured, &BTreeMap::new()).unwrap()
}

pub(crate) async fn pre_call(
    guardrail: &NomaGuardrail,
    request_data: &mut RequestData,
    call_type: &str,
) -> Result<(), NomaLifecycleError> {
    let input = RequestHookInput {
        user_auth: &BTreeMap::new(),
        data: request_data,
        call_type,
        suppressed: &BTreeSet::new(),
    };
    guardrail.async_pre_call_hook(input).await
}

pub(crate) async fn moderation(
    guardrail: &NomaGuardrail,
    request_data: &mut RequestData,
    call_type: &str,
) -> Result<(), NomaLifecycleError> {
    let input = RequestHookInput {
        user_auth: &BTreeMap::new(),
        data: request_data,
        call_type,
        suppressed: &BTreeSet::new(),
    };
    guardrail.async_moderation_hook(input).await
}

pub(crate) async fn post_call(
    guardrail: &NomaGuardrail,
    request_data: &mut RequestData,
    response: &mut GuardrailValue,
) -> Result<(), NomaLifecycleError> {
    let input = PostCallHookInput {
        user_auth: &BTreeMap::new(),
        data: request_data,
        response,
        suppressed: &BTreeSet::new(),
    };
    guardrail.async_post_call_success_hook(input).await
}

pub(crate) fn litellm_params(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).unwrap()
}

pub(crate) fn records(request_data: &RequestData) -> Vec<GuardrailValue> {
    let bucket = request_data
        .get("litellm_metadata")
        .filter(|found| !matches!(found, GuardrailValue::Null))
        .or_else(|| request_data.get("metadata"));
    let Some(GuardrailValue::Object(fields)) = bucket else {
        return Vec::new();
    };
    let Some(GuardrailValue::Array(found)) = fields.get("standard_logging_guardrail_information")
    else {
        return Vec::new();
    };
    found.clone()
}

pub(crate) fn field<'value>(
    record: &'value GuardrailValue,
    name: &str,
) -> Option<&'value GuardrailValue> {
    let GuardrailValue::Object(fields) = record else {
        return None;
    };
    fields.get(name)
}

#[derive(Clone, Default)]
pub(crate) struct CapturedLog(Arc<Mutex<Vec<u8>>>);

impl CapturedLog {
    pub(crate) fn text(&self) -> String {
        let bytes = self.0.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }

    pub(crate) fn subscriber(&self, level: Level) -> impl Subscriber + use<> {
        let writer = self.clone();
        fmt()
            .with_max_level(level)
            .with_writer(move || writer.clone())
            .finish()
    }
}

impl Write for CapturedLog {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}
