use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io;
use std::net::TcpListener as StdTcpListener;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode, Uri};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract as _;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::noma_v2::config::{NomaV2Params, NomaV2Settings};
use betterlite_guardrails::noma_v2::guardrail::NomaV2Guardrail;
use betterlite_guardrails::noma_v2::{CallbackRegistry, guardrail_settings};
use betterlite_http::proxy::base_builder;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tracing::{Level, Subscriber};
use tracing_subscriber::fmt;

pub(crate) const API_KEY: &str = "noma-test-key";
pub(crate) const NONE_REPLY: &str = r#"{"action": "NONE"}"#;
pub(crate) const BLOCKED_REPLY: &str = r#"{"action": "BLOCKED", "violation_reason": "unsafe"}"#;
pub(crate) const INTERVENED_REPLY: &str = r#"{
    "action": "GUARDRAIL_INTERVENED",
    "texts": ["masked"],
    "images": ["aW1hZ2U="],
    "tools": [{"type": "function", "function": {"name": "lookup"}}],
    "tool_calls": [{"id": "call-2"}]
}"#;

pub(crate) struct Seen {
    pub(crate) path: String,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Bytes,
}

pub(crate) struct MockNoma {
    pub(crate) base: String,
    pub(crate) seen: Arc<Mutex<Vec<Seen>>>,
    _server: JoinSet<io::Result<()>>,
}

pub(crate) async fn mock_noma(status: StatusCode, reply: &str) -> MockNoma {
    mock_noma_after(Duration::ZERO, status, reply).await
}

pub(crate) async fn mock_noma_after(delay: Duration, status: StatusCode, reply: &str) -> MockNoma {
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
    MockNoma {
        base,
        seen,
        _server: server,
    }
}

pub(crate) fn captured_body(mock: &MockNoma) -> BTreeMap<String, GuardrailValue> {
    let seen = mock.seen.lock().unwrap();
    serde_json::from_slice(seen.first().unwrap().body.as_ref()).unwrap()
}

pub(crate) fn captured_headers(mock: &MockNoma) -> HeaderMap {
    let seen = mock.seen.lock().unwrap();
    seen.first().unwrap().headers.clone()
}

pub(crate) fn captured_path(mock: &MockNoma) -> String {
    let seen = mock.seen.lock().unwrap();
    seen.first().unwrap().path.clone()
}

pub(crate) fn request_count(mock: &MockNoma) -> usize {
    mock.seen.lock().unwrap().len()
}

pub(crate) fn closed_base() -> String {
    let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{address}")
}

pub(crate) fn data(text: &str) -> BTreeMap<String, GuardrailValue> {
    let parsed = serde_json::from_str(text).unwrap();
    serde_json::from_value(parsed).unwrap()
}

pub(crate) fn value(text: &str) -> GuardrailValue {
    let parsed = serde_json::from_str(text).unwrap();
    serde_json::from_value(parsed).unwrap()
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

pub(crate) fn params() -> NomaV2Params {
    NomaV2Params {
        guardrail_name: Some(String::from("noma-v2-test")),
        api_key: None,
        api_base: None,
        application_id: None,
        gateway_name: None,
        monitor_mode: None,
        block_failures: None,
        timeout: None,
        mode: Some(LitellmParamsMode::One(String::from("pre_call"))),
        default_on: false,
        strict_modes: true,
    }
}

pub(crate) fn settings(base: &str) -> NomaV2Settings {
    NomaV2Settings {
        api_key: Some(SecretString::from(String::from(API_KEY))),
        api_base: base.to_owned(),
        application_id: Some(String::from("app-1")),
        gateway_name: None,
        monitor_mode: false,
        block_failures: true,
        timeout: None,
        request_timeout: Duration::from_secs(600),
    }
}

pub(crate) fn guardrail(settings: NomaV2Settings) -> NomaV2Guardrail {
    guardrail_with(&params(), settings)
}

pub(crate) fn guardrail_with(
    configured: &NomaV2Params,
    settings: NomaV2Settings,
) -> NomaV2Guardrail {
    NomaV2Guardrail {
        base: GuardrailBase::new(guardrail_settings(configured), true).unwrap(),
        settings,
        client: base_builder().build().unwrap(),
        clock: fixed_now,
    }
}

pub(crate) fn guardrail_with_clock(
    settings: NomaV2Settings,
    clock: fn() -> SystemTime,
) -> NomaV2Guardrail {
    let mut guardrail = guardrail_with(&params(), settings);
    guardrail.clock = clock;
    guardrail
}

pub(crate) fn fixed_now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000))
        .unwrap()
}

static TICKS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn ticking_now() -> SystemTime {
    let tick = TICKS.fetch_add(1, Ordering::Relaxed);
    UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000_u64.saturating_add(tick)))
        .unwrap()
}

static BACKWARDS: AtomicU64 = AtomicU64::new(0);

// The first call reads as later than the second, so the scan records a negative duration
pub(crate) fn backwards_now() -> SystemTime {
    let step = BACKWARDS.fetch_add(1, Ordering::Relaxed);
    let seconds = if step == 0 {
        1_700_000_100
    } else {
        1_700_000_000
    };
    UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))
        .unwrap()
}

// A time before the Unix epoch has no epoch-seconds reading, so the record cannot be written
pub(crate) fn before_epoch_now() -> SystemTime {
    UNIX_EPOCH.checked_sub(Duration::from_secs(1)).unwrap()
}

pub(crate) fn inputs() -> GuardrailApiInputs {
    serde_json::from_str(
        r#"{
            "texts": ["hello"],
            "images": ["aW1hZ2U="],
            "tools": [{"type": "function", "function": {"name": "lookup"}}],
            "tool_calls": [{"id": "call-1", "type": "function"}],
            "model": "gpt-4o"
        }"#,
    )
    .unwrap()
}

pub(crate) fn request_data() -> BTreeMap<String, GuardrailValue> {
    data(
        r#"{
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hello"}],
            "input": "hello",
            "litellm_metadata": {"user_api_key_alias": "key-alias"}
        }"#,
    )
}

pub(crate) fn log_target() -> GuardrailLogTarget {
    GuardrailLogTarget {
        litellm_params: Some(BTreeMap::default()),
        model_call_params: Some(BTreeMap::default()),
    }
}

pub(crate) struct Scan {
    pub(crate) result: Result<GuardrailApiInputs, GuardrailFailure>,
    pub(crate) request_data: BTreeMap<String, GuardrailValue>,
    pub(crate) log_target: GuardrailLogTarget,
}

pub(crate) async fn scan(
    guardrail: &NomaV2Guardrail,
    inputs: GuardrailApiInputs,
    mut request_data: BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
) -> Scan {
    let mut log_target = log_target();
    let result = guardrail
        .apply_guardrail(inputs, &mut request_data, input_type, Some(&mut log_target))
        .await;
    Scan {
        result,
        request_data,
        log_target,
    }
}

pub(crate) fn records(request_data: &BTreeMap<String, GuardrailValue>) -> Vec<GuardrailValue> {
    let bucket = request_data
        .get("litellm_metadata")
        .filter(|value| !matches!(value, GuardrailValue::Null))
        .or_else(|| request_data.get("metadata"));
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
    pub(crate) callbacks: Vec<Arc<NomaV2Guardrail>>,
}

impl CallbackRegistry for Registry {
    fn add_litellm_callback(&mut self, guardrail: Arc<NomaV2Guardrail>) {
        self.callbacks.push(guardrail);
    }
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
        let mut log = self
            .0
            .lock()
            .map_err(|_poisoned| io::Error::other("the captured log lock is poisoned"))?;
        log.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
