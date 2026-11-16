use std::collections::{BTreeMap, BTreeSet};
use std::future::IntoFuture as _;
use std::io;
use std::net::TcpListener as StdTcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode, Uri};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_records::error::GuardrailSpanError;
use betterlite_guardrails::guardrail_records::invocation::{
    GuardrailHookName, GuardrailInvocation,
};
use betterlite_guardrails::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailSpanSink,
};
use betterlite_guardrails::repello_lifecycle::RepelloAILifecycle;
use betterlite_guardrails::repello_lifecycle::config::{RepelloAIParams, RepelloAISettings};
use betterlite_guardrails::repello_lifecycle::hooks::{
    HookContext, post_call_success_hook, pre_call_hook, streaming_iterator_hook,
};
use betterlite_guardrails::repello_lifecycle::initialize::{
    CallbackRegistry, guardrail_settings, initialize_guardrail,
};
use betterlite_guardrails::repello_lifecycle::{DEFAULT_REQUEST_TIMEOUT, Data};
use betterlite_http::proxy::base_builder;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tracing::{Level, Subscriber};
use tracing_subscriber::fmt;

pub(crate) const API_KEY: &str = "repello-test-key";
pub(crate) const ASSET_ID: &str = "asset-123";
pub(crate) const GUARDRAIL_NAME: &str = "repello-test";
pub(crate) const PASSED_REPLY: &str = r#"{"verdict": "passed"}"#;
pub(crate) const BLOCKED_REPLY: &str = r#"{"verdict": "blocked", "policies_violated": [{"policy_name": "pii", "action_taken": "block"}]}"#;
pub(crate) const FLAGGED_REPLY: &str =
    r#"{"verdict": "flagged", "policies_violated": [{"policy_name": "toxicity"}]}"#;

pub(crate) struct Seen {
    pub(crate) path: String,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Bytes,
}

pub(crate) struct MockRepello {
    pub(crate) base: String,
    pub(crate) seen: Arc<Mutex<Vec<Seen>>>,
    _server: JoinSet<io::Result<()>>,
}

pub(crate) async fn mock_repello(status: StatusCode, reply: &str) -> MockRepello {
    mock_repello_after(Duration::ZERO, status, reply).await
}

pub(crate) async fn mock_repello_after(
    delay: Duration,
    status: StatusCode,
    reply: &str,
) -> MockRepello {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&seen);
    let canned = reply.to_owned();
    let answer = move |uri: Uri, headers: HeaderMap, body: Bytes| {
        let captured = Arc::clone(&recorded);
        let answer = canned.clone();
        async move {
            captured.lock().unwrap().push(Seen {
                path: uri.path().to_owned(),
                headers,
                body,
            });
            sleep(delay).await;
            (status, [("content-type", "application/json")], answer)
        }
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, Router::new().fallback(answer)).into_future());
    MockRepello {
        base,
        seen,
        _server: server,
    }
}

pub(crate) fn request_count(mock: &MockRepello) -> usize {
    mock.seen.lock().unwrap().len()
}

pub(crate) fn captured_path(mock: &MockRepello) -> String {
    mock.seen.lock().unwrap().first().unwrap().path.clone()
}

pub(crate) fn captured_body(mock: &MockRepello) -> BTreeMap<String, GuardrailValue> {
    let seen = mock.seen.lock().unwrap();
    serde_json::from_slice(seen.first().unwrap().body.as_ref()).unwrap()
}

pub(crate) fn captured_headers(mock: &MockRepello) -> HeaderMap {
    mock.seen.lock().unwrap().first().unwrap().headers.clone()
}

pub(crate) fn closed_base() -> String {
    let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{address}")
}

pub(crate) fn data(text: &str) -> Data {
    let parsed = serde_json::from_str(text).unwrap();
    serde_json::from_value(parsed).unwrap()
}

pub(crate) fn value(text: &str) -> GuardrailValue {
    let parsed = serde_json::from_str(text).unwrap();
    serde_json::from_value(parsed).unwrap()
}

pub(crate) fn prompt_data() -> Data {
    data(r#"{"model": "gpt-4o", "messages": [{"role": "user", "content": "hello"}]}"#)
}

pub(crate) fn response_data() -> Data {
    data(r#"{"choices": [{"index": 0, "message": {"role": "assistant", "content": "answer"}}]}"#)
}

pub(crate) fn empty_env() -> BTreeMap<String, String> {
    BTreeMap::default()
}

pub(crate) fn lookup(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, text)| ((*name).to_owned(), (*text).to_owned()))
        .collect()
}

pub(crate) fn params() -> RepelloAIParams {
    RepelloAIParams {
        guardrail_name: Some(GUARDRAIL_NAME.to_owned()),
        api_key: None,
        api_base: None,
        asset_id: Some(ASSET_ID.to_owned()),
        unreachable_fallback: UnreachableFallback::FailClosed,
        timeout: None,
        mode: LitellmParamsMode::One(String::from("pre_call")),
        default_on: true,
        strict_modes: true,
    }
}

pub(crate) fn settings(base: &str) -> RepelloAISettings {
    RepelloAISettings {
        api_key: SecretString::from(String::from(API_KEY)),
        api_base: base.to_owned(),
        asset_id: ASSET_ID.to_owned(),
        unreachable_fallback: UnreachableFallback::FailClosed,
        timeout: None,
        request_timeout: DEFAULT_REQUEST_TIMEOUT,
    }
}

pub(crate) fn guardrail(settings: RepelloAISettings) -> RepelloAILifecycle {
    guardrail_with(&params(), settings)
}

pub(crate) fn guardrail_with(
    configured: &RepelloAIParams,
    settings: RepelloAISettings,
) -> RepelloAILifecycle {
    let redactor = betterlite_redact::secret::Redactor::new(
        betterlite_redact::secret::MINIMUM_CUSTOM_KEY_LENGTH,
    )
    .unwrap();
    RepelloAILifecycle {
        base: GuardrailBase::new(
            guardrail_settings(configured, configured.mode.clone()),
            true,
        )
        .unwrap(),
        settings,
        client: base_builder().build().unwrap(),
        clock: fixed_now,
        hooks: betterlite_logging::verbose_hooks::VerboseHooks {
            logger: betterlite_guardrails::repello_lifecycle::stream::SilentLogger,
            console: betterlite_guardrails::repello_lifecycle::stream::SilentConsole,
            set_verbose: betterlite_redact::structured::Payload::Bool(false),
            redactor: Arc::new(redactor),
        },
    }
}

pub(crate) fn fixed_now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000))
        .unwrap()
}

pub(crate) struct NoSink;

impl GuardrailSpanSink for NoSink {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

pub(crate) struct HookRun {
    pub(crate) result: Result<(), GuardrailFailure>,
    pub(crate) data: Data,
}

pub(crate) fn decoded_params(body: &str) -> RepelloAIParams {
    let params = betterlite_config::sections::guardrail_params::decode::decode_litellm_params(
        body.as_bytes(),
    )
    .unwrap();
    RepelloAIParams::from_litellm(&params, GUARDRAIL_NAME, true).unwrap()
}

pub(crate) fn parsed(
    text: &str,
) -> betterlite_guardrails::repello_content::wire::RepelloAIAnalyzeResponse {
    serde_json::from_str(text).unwrap()
}

pub(crate) async fn run_pre_call_suppressed(
    guardrail: &RepelloAILifecycle,
    mut data: Data,
    suppressed: &BTreeSet<String>,
) -> HookRun {
    let mut invocation =
        GuardrailInvocation::begin(GuardrailHookName::PreCall, &data, None, fixed_now()).unwrap();
    let sink = NoSink;
    let mut context = HookContext {
        suppressed,
        invocation: &mut invocation,
        sink: &sink,
        log_target: None,
    };
    let result = pre_call_hook(guardrail, &mut data, &mut context).await;
    HookRun { result, data }
}

pub(crate) async fn run_pre_call(guardrail: &RepelloAILifecycle, mut data: Data) -> HookRun {
    let mut invocation =
        GuardrailInvocation::begin(GuardrailHookName::PreCall, &data, None, fixed_now()).unwrap();
    let suppressed = BTreeSet::new();
    let sink = NoSink;
    let mut context = HookContext {
        suppressed: &suppressed,
        invocation: &mut invocation,
        sink: &sink,
        log_target: None,
    };
    let result = pre_call_hook(guardrail, &mut data, &mut context).await;
    HookRun { result, data }
}

pub(crate) async fn run_post_call(
    guardrail: &RepelloAILifecycle,
    mut data: Data,
    response: &Data,
) -> HookRun {
    let mut invocation =
        GuardrailInvocation::begin(GuardrailHookName::PostCallSuccess, &data, None, fixed_now())
            .unwrap();
    let suppressed = BTreeSet::new();
    let sink = NoSink;
    let mut context = HookContext {
        suppressed: &suppressed,
        invocation: &mut invocation,
        sink: &sink,
        log_target: None,
    };
    let result = post_call_success_hook(guardrail, &mut data, response, &mut context).await;
    HookRun { result, data }
}

pub(crate) struct StreamRun {
    pub(crate) result: Result<(), GuardrailFailure>,
    pub(crate) output: Vec<GuardrailValue>,
    pub(crate) data: Data,
}

pub(crate) async fn run_stream(
    guardrail: &RepelloAILifecycle,
    mut data: Data,
    chunks: &[&str],
) -> StreamRun {
    let (input_tx, input_rx) = mpsc::channel(16);
    for chunk in chunks {
        input_tx.send(value(chunk)).await.unwrap();
    }
    drop(input_tx);
    let (output_tx, mut output_rx) = mpsc::channel(16);
    let mut invocation = GuardrailInvocation::begin(
        GuardrailHookName::StreamingIterator,
        &data,
        None,
        fixed_now(),
    )
    .unwrap();
    let suppressed = BTreeSet::new();
    let sink = NoSink;
    let mut context = HookContext {
        suppressed: &suppressed,
        invocation: &mut invocation,
        sink: &sink,
        log_target: None,
    };
    let result =
        streaming_iterator_hook(guardrail, &mut data, &mut context, input_rx, output_tx).await;
    let mut output = Vec::new();
    while let Some(chunk) = output_rx.recv().await {
        output.push(chunk);
    }
    StreamRun {
        result,
        output,
        data,
    }
}

pub(crate) fn chat_chunks() -> Vec<&'static str> {
    vec![
        r#"{"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o", "choices": [{"index": 0, "delta": {"role": "assistant", "content": "hello"}}]}"#,
        r#"{"choices": [{"index": 0, "delta": {"content": " world"}}]}"#,
        r#"{"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}"#,
    ]
}

pub(crate) fn text_chunks() -> Vec<&'static str> {
    vec![r#"{"choices": [{"index": 0, "text": "Solo"}]}"#]
}

pub(crate) fn records(data: &Data) -> Vec<GuardrailValue> {
    let bucket = data
        .get("litellm_metadata")
        .filter(|value| !matches!(value, GuardrailValue::Null))
        .or_else(|| data.get("metadata"));
    let Some(GuardrailValue::Object(fields)) = bucket else {
        return Vec::new();
    };
    if let Some(GuardrailValue::Array(records)) =
        fields.get("standard_logging_guardrail_information")
    {
        return records.clone();
    }
    Vec::new()
}

pub(crate) fn applied(data: &Data) -> Vec<GuardrailValue> {
    let bucket = data
        .get("litellm_metadata")
        .filter(|value| !matches!(value, GuardrailValue::Null))
        .or_else(|| data.get("metadata"));
    let Some(GuardrailValue::Object(fields)) = bucket else {
        return Vec::new();
    };
    match fields.get("applied_guardrails") {
        Some(GuardrailValue::Array(names)) => names.clone(),
        _ => Vec::new(),
    }
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

pub(crate) fn text_field(record: &GuardrailValue, name: &str) -> Option<String> {
    let GuardrailValue::Text(text) = field(record, name)? else {
        return None;
    };
    Some(text.clone())
}

#[derive(Default)]
pub(crate) struct Registry {
    pub(crate) callbacks: Vec<Arc<RepelloAILifecycle>>,
}

impl CallbackRegistry for Registry {
    fn add_litellm_callback(&mut self, guardrail: Arc<RepelloAILifecycle>) {
        self.callbacks.push(guardrail);
    }
}

pub(crate) fn initialize(
    configured: &RepelloAIParams,
    env: &BTreeMap<String, String>,
) -> (Result<Arc<RepelloAILifecycle>, String>, Registry) {
    let mut registry = Registry::default();
    let result = initialize_guardrail(configured, env, base_builder(), &mut registry)
        .map_err(|error| error.to_string());
    (result, registry)
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

impl io::Write for CapturedLog {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("the captured log lock is poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
