use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs;
use std::future::IntoFuture as _;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode, Uri};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_http::proxy::base_builder;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::value::RawValue;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::sleep;

use crate::noma_scan_policy::wire::ScanResponse;
use crate::noma_scan_policy::{NomaScanPolicy, NomaSettings, ScanCallContext};

pub(crate) const SAFE_REPLY: &str =
    r#"{"aggregatedScanResult": false, "scanResult": [{"role": "user", "results": {}}]}"#;

pub(crate) struct Seen {
    pub(crate) path: String,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Vec<u8>,
}

pub(crate) struct MockNoma {
    pub(crate) base: String,
    pub(crate) seen: Arc<Mutex<Vec<Seen>>>,
    _server: JoinSet<std::io::Result<()>>,
}

pub(crate) async fn mock_noma(status: StatusCode, reply: &str) -> MockNoma {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_by_handler = Arc::clone(&seen);
    let reply = reply.to_owned();
    let answer = move |uri: Uri, headers: HeaderMap, bytes: Bytes| {
        let seen = Arc::clone(&seen_by_handler);
        let reply = reply.clone();
        async move {
            seen.lock().unwrap().push(Seen {
                path: uri.path().to_owned(),
                headers,
                body: bytes.to_vec(),
            });
            (status, [("content-type", "application/json")], reply)
        }
    };
    serve(Router::new().fallback(answer), seen).await
}

pub(crate) async fn slow_noma(delay: Duration) -> MockNoma {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_by_handler = Arc::clone(&seen);
    let answer = move |uri: Uri, headers: HeaderMap, bytes: Bytes| {
        let seen = Arc::clone(&seen_by_handler);
        async move {
            seen.lock().unwrap().push(Seen {
                path: uri.path().to_owned(),
                headers,
                body: bytes.to_vec(),
            });
            sleep(delay).await;
            (
                StatusCode::OK,
                [("content-type", "application/json")],
                String::from(SAFE_REPLY),
            )
        }
    };
    serve(Router::new().fallback(answer), seen).await
}

async fn serve(router: Router, seen: Arc<Mutex<Vec<Seen>>>) -> MockNoma {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    MockNoma {
        base,
        seen,
        _server: server,
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct SeenRequest {
    pub(crate) input: Vec<SeenItem>,
    #[serde(rename = "x-noma-context")]
    pub(crate) context: SeenContext,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SeenItem {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) role: Option<String>,
    pub(crate) content: Option<Box<RawValue>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct SeenContext {
    #[serde(rename = "applicationId")]
    pub(crate) application_id: Option<String>,
    #[serde(rename = "ipAddress")]
    pub(crate) ip_address: Option<String>,
    #[serde(rename = "userId")]
    pub(crate) user_id: Option<String>,
    #[serde(rename = "sessionId")]
    pub(crate) session_id: Option<String>,
    #[serde(rename = "requestId")]
    pub(crate) request_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SeenPart {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) text: String,
}

pub(crate) fn parts(item: &SeenItem) -> Vec<SeenPart> {
    let Some(content) = item.content.as_ref() else {
        return Vec::new();
    };
    serde_json::from_str(content.get()).unwrap()
}

pub(crate) fn captured(mock: &MockNoma) -> SeenRequest {
    let seen = mock.seen.lock().unwrap();
    serde_json::from_slice(&seen.first().unwrap().body).unwrap()
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

pub(crate) fn data(text: &str) -> BTreeMap<String, GuardrailValue> {
    let parsed = serde_json::from_str(text).unwrap();
    serde_json::from_value(parsed).unwrap()
}

pub(crate) fn value(text: &str) -> GuardrailValue {
    let parsed = serde_json::from_str(text).unwrap();
    serde_json::from_value(parsed).unwrap()
}

pub(crate) fn response(text: &str) -> ScanResponse {
    let parsed = serde_json::from_str(text).unwrap();
    serde_json::from_value(parsed).unwrap()
}

pub(crate) fn corpus_answer(name: &str) -> ScanResponse {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz/corpus/noma_scan_response")
        .join(name);
    ScanResponse::from_body(&fs::read(path).unwrap()).unwrap()
}

pub(crate) fn settings() -> NomaSettings {
    NomaSettings {
        guardrail_name: Some(String::from("noma-test")),
        event_hook: None,
        api_key: Some(SecretString::from(String::from("test-api-key"))),
        api_base: None,
        application_id: Some(String::from("test-app")),
        monitor_mode: false,
        anonymize_input: false,
        timeout: None,
    }
}

pub(crate) fn policy(base: &str) -> NomaScanPolicy {
    let mut settings = settings();
    settings.api_base = Some(base.to_owned());
    NomaScanPolicy {
        settings,
        client: base_builder().build().unwrap(),
        now: fixed_now,
    }
}

pub(crate) fn policy_with(settings: NomaSettings) -> NomaScanPolicy {
    NomaScanPolicy {
        settings,
        client: base_builder().build().unwrap(),
        now: fixed_now,
    }
}

pub(crate) fn fixed_now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

thread_local! {
    static TICKS: Cell<u64> = const { Cell::new(0) };
}

pub(crate) fn ticking_now() -> SystemTime {
    let tick = TICKS.with(|ticks| {
        let current = ticks.get();
        ticks.set(current.saturating_add(1));
        current
    });
    UNIX_EPOCH + Duration::from_secs(1_700_000_000_u64.saturating_add(tick))
}

pub(crate) fn context<'data>(
    user_auth: &'data BTreeMap<String, GuardrailValue>,
    extra_data: &'data BTreeMap<String, GuardrailValue>,
) -> ScanCallContext<'data> {
    ScanCallContext {
        user_auth,
        extra_data,
        event_type: Some(GuardrailEventHooks::PreCall),
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

pub(crate) const USER_MESSAGES: &str = r#"{"messages": [{"role": "user", "content": "hello"}]}"#;

pub(crate) fn anonymizing_settings() -> NomaSettings {
    let mut settings = settings();
    settings.anonymize_input = true;
    settings
}

pub(crate) fn monitoring_settings() -> NomaSettings {
    let mut settings = settings();
    settings.monitor_mode = true;
    settings.anonymize_input = true;
    settings
}

pub(crate) fn response_of(record: &GuardrailValue) -> &GuardrailValue {
    let GuardrailValue::Object(fields) = record else {
        panic!("the record is not an object");
    };
    let Some(scan_response) = fields.get("guardrail_response") else {
        panic!("the record has no guardrail_response");
    };
    scan_response
}

#[derive(Clone, Default)]
pub(crate) struct CapturedLog(pub(crate) Arc<Mutex<Vec<u8>>>);

impl std::io::Write for CapturedLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
