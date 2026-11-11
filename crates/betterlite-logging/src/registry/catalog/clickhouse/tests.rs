#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io::Read as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use flate2::read::GzDecoder;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use url::Url;

use super::adapter::ClickHouseAdapter;
use super::build;
use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::event::LoggerEvent;
use crate::clickhouse::clickhouse_logging::batch_logger::ClickHouseBatchLogger;
use crate::clickhouse::clickhouse_logging::context::LensAnalysis;
use crate::clickhouse::clickhouse_logging::schema::SPEND_LOGS_TABLE;
use crate::clickhouse::clickhouse_logging::settings::ClickHouseSettings;
use crate::clickhouse::clickhouse_logging::spend_logger::ClickHouseSpendLogger;
use crate::clickhouse::clickhouse_logging::storage::ClickHouseStorage;
use crate::registry::catalog::{CatalogError, CatalogInputs, LoggerCatalog};
use crate::trace_types::SpendLogRecord;

const PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "litellm_call_id": "call-1",
    "call_type": "completion",
    "response_cost": 0.5,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067201.5,
    "completionStartTime": 1704067200.5,
    "response_time": 1.5,
    "model_map_information": {},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {
        "user_api_key_user_id": "user-1",
        "requester_custom_headers": {"traceparent": "00-4bf92f3577b34da6a3ce929d0e0e4736-b7ad6b7169203331-01"}
    },
    "end_user": "end-user-1",
    "saved_cache_cost": 0.0,
    "request_tags": ["team-a"],
    "model_parameters": {},
    "hidden_params": {},
    "messages": [{"role": "user", "content": "Hello"}],
    "response": {"id": "chatcmpl-1", "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}}
}"#;

#[derive(Clone, Debug)]
struct Inserted {
    table: String,
    rows: Vec<SpendLogRecord>,
}

#[derive(Clone, Default)]
struct FakeStorage {
    calls: Arc<Mutex<Vec<Inserted>>>,
}

impl FakeStorage {
    fn calls(&self) -> Vec<Inserted> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake ClickHouse insert failed")]
struct FakeError;

impl ClickHouseStorage for FakeStorage {
    type Error = FakeError;

    fn insert_rows(
        &self,
        table: &str,
        rows: &[SpendLogRecord],
    ) -> impl Future<Output = Result<(), FakeError>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Inserted {
                table: table.to_owned(),
                rows: rows.to_vec(),
            });
        ready(Ok(()))
    }

    fn ensure_schema(
        &self,
        _trace_retention_days: u32,
        _spend_log_retention_days: u32,
    ) -> impl Future<Output = Result<(), FakeError>> + Send {
        ready(Ok(()))
    }
}

fn settings() -> ClickHouseSettings {
    ClickHouseSettings {
        database: "litellm".to_owned(),
        url: SecretString::from(String::new()),
        batch_size: 10,
        flush_interval: Duration::from_secs(3600),
        max_buffered_rows: 200_000,
        max_retries: 3,
    }
}

fn inputs(environment: &[(&str, &str)]) -> CatalogInputs {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("valid proxy policy"),
    );
    CatalogInputs {
        config: Arc::new(
            ConfigLoader {
                lookup: BTreeMap::new(),
            }
            .load("")
            .expect("valid config"),
        ),
        http: ProxiedClient::outbound(&policy).expect("http client"),
        env: Arc::new(
            environment
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect::<BTreeMap<String, String>>(),
        ),
        callback_settings: BTreeMap::new(),
        tracker: TaskTracker::new(),
        shutdown: CancellationToken::new(),
    }
}

fn event_from(payload: &str) -> LoggerEvent {
    LoggerEvent {
        payload: Arc::new(serde_json::from_str(payload).expect("the fixture payload decodes")),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        stream: false,
        guardrail_snapshot: None,
    }
}

fn event() -> LoggerEvent {
    event_from(PAYLOAD)
}

fn adapter(storage: FakeStorage) -> ClickHouseAdapter<FakeStorage> {
    let batch =
        ClickHouseBatchLogger::new(storage, SPEND_LOGS_TABLE, settings(), TaskTracker::new());
    ClickHouseAdapter {
        logger: ClickHouseSpendLogger { batch },
        lens: LensAnalysis::default(),
    }
}

#[test]
fn resolves_by_name_to_a_clickhouse_logger() {
    let catalog = LoggerCatalog::new(inputs(&[("CLICKHOUSE_URL", "http://127.0.0.1:8123")]));
    let logger = catalog
        .resolve_logger("clickhouse")
        .expect("the name is available")
        .expect("the logger builds");
    assert_eq!(logger.callback_name(), "clickhouse");
}

#[test]
fn a_missing_url_gives_init() {
    assert!(matches!(
        build(&inputs(&[])),
        Err(CatalogError::Init { message }) if message == "invalid ClickHouse HTTP URL"
    ));
}

#[test]
fn an_invalid_url_gives_init() {
    assert!(matches!(
        build(&inputs(&[("CLICKHOUSE_URL", "not a url")])),
        Err(CatalogError::Init { message }) if message == "invalid ClickHouse HTTP URL"
    ));
}

#[test]
fn an_invalid_database_gives_init() {
    assert!(matches!(
        build(&inputs(&[
            ("CLICKHOUSE_URL", "http://127.0.0.1:8123"),
            ("CLICKHOUSE_DATABASE", "bad-name"),
        ])),
        Err(CatalogError::Init { message })
            if message == "database must be a nonempty SQL identifier and retention must be positive"
    ));
}

#[tokio::test]
async fn an_async_success_event_queues_the_spend_row() {
    let storage = FakeStorage::default();
    let adapter = adapter(storage.clone());
    adapter
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the event is logged");
    adapter.logger.batch.flush_queue().await;
    let calls = storage.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].table, SPEND_LOGS_TABLE);
    assert_eq!(calls[0].rows.len(), 1);
    let row = &calls[0].rows[0];
    assert_eq!(row.request_id, "chatcmpl-1");
    assert_eq!(row.response_id, "chatcmpl-1");
    assert_eq!(row.call_type, "completion");
    assert_eq!(row.user, "user-1");
    assert_eq!(row.end_user, "end-user-1");
    assert_eq!(row.model, "gpt-4o");
    assert_eq!(row.spend, rust_decimal::Decimal::new(5, 1));
    assert_eq!(row.prompt_tokens, 2);
    assert_eq!(row.completion_tokens, 3);
    assert_eq!(row.total_tokens, 5);
    assert_eq!(row.start_time, 1_704_067_200_000);
    assert_eq!(row.end_time, 1_704_067_201_500);
    assert_eq!(row.completion_start_time, Some(1_704_067_200_500));
    assert_eq!(row.session_id, "session-1");
    assert_eq!(row.trace_id, "4bf92f3577b34da6a3ce929d0e0e4736");
    assert_eq!(row.span_id, "b7ad6b7169203331");
    assert_eq!(row.request_tags, vec!["team-a".to_owned()]);
    assert_eq!(row.status, "success");
    assert!(!row.cache_hit);
    assert!(row.metadata.contains(r#""litellm_lens_internal":false"#));
    assert_eq!(row.messages, r#"[{"role":"user","content":"Hello"}]"#);
}

#[tokio::test]
async fn an_async_failure_event_queues_the_spend_row() {
    let storage = FakeStorage::default();
    let adapter = adapter(storage.clone());
    adapter
        .async_log_failure_event(Arc::new(event_from(
            &PAYLOAD.replace(r#""status": "success""#, r#""status": "failure""#),
        )))
        .await
        .expect("the event is logged");
    adapter.logger.batch.flush_queue().await;
    let calls = storage.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].rows[0].status, "failure");
}

#[tokio::test]
async fn a_trace_ingest_event_queues_nothing() {
    let storage = FakeStorage::default();
    let adapter = adapter(storage.clone());
    adapter
        .async_log_success_event(Arc::new(event_from(&PAYLOAD.replace(
            r#""call_type": "completion""#,
            r#""call_type": "/v1/traces/span""#,
        ))))
        .await
        .expect("the event is logged");
    adapter.logger.batch.flush_queue().await;
    assert!(storage.calls().is_empty());
}

#[tokio::test]
async fn a_resolved_logger_posts_gzip_json_each_row() {
    let (base, served) = serve_once().await;
    let catalog = LoggerCatalog::new(inputs(&[
        ("CLICKHOUSE_URL", &base),
        ("CLICKHOUSE_BATCH_SIZE", "1"),
    ]));
    let logger = catalog
        .resolve_logger("clickhouse")
        .expect("the name is available")
        .expect("the logger builds");
    logger
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the event is logged");
    assert_insert_request(&served.await);
}

fn assert_insert_request(received: &Received) {
    assert_eq!(received.method, "POST");
    assert_eq!(
        received.headers.get("content-encoding").map(String::as_str),
        Some("gzip")
    );
    let url = Url::parse(&format!("http://clickhouse{}", received.path)).expect("a request URL");
    let query: BTreeMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(
        query.get("query").map(String::as_str),
        Some("INSERT INTO `litellm`.spend_logs FORMAT JSONEachRow")
    );
    assert_eq!(query.get("async_insert").map(String::as_str), Some("1"));
    assert!(query.contains_key("insert_deduplication_token"));
    let mut gzip = GzDecoder::new(received.body.as_slice());
    let mut text = String::new();
    gzip.read_to_string(&mut text).expect("the body is gzip");
    let row: Json = serde_json::from_str(&text).expect("the body is one JSON row");
    let Json::Object(fields) = &row else {
        panic!("the row is a JSON object");
    };
    assert_eq!(
        fields.get("request_id"),
        Some(&Json::Text("chatcmpl-1".to_owned()))
    );
    assert_eq!(
        fields.get("start_time"),
        Some(&Json::Text("2024-01-01T00:00:00.000Z".to_owned()))
    );
    assert_eq!(
        fields.get("end_time"),
        Some(&Json::Text("2024-01-01T00:00:01.500Z".to_owned()))
    );
    assert!(fields.contains_key("EngineReceivedMs"));
}

struct Received {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

// The returned future accepts one request and answers it; the test joins it with the call under test.
async fn serve_once() -> (String, impl Future<Output = Received>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let address = listener.local_addr().expect("the bound address");
    let served = async move {
        let (mut stream, _) = listener.accept().await.expect("one request");
        let received = receive(&mut stream).await;
        answer(&mut stream).await;
        received
    };
    (format!("http://{address}"), served)
}

async fn receive(stream: &mut TcpStream) -> Received {
    let mut buffer = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 2048];
        let read = stream.read(&mut chunk).await.expect("a request read");
        assert!(read > 0, "the client closed before sending a request");
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position.saturating_add(4);
        }
    };
    let head = String::from_utf8(buffer[..header_end].to_vec()).expect("a UTF-8 request head");
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next().expect("a request line").split(' ');
    let method = request_line.next().expect("a method").to_owned();
    let path = request_line.next().expect("a path").to_owned();
    let headers: BTreeMap<String, String> = lines
        .filter_map(|line| line.split_once(": "))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.to_owned()))
        .collect();
    let length = headers
        .get("content-length")
        .map_or(0, |value| value.parse::<usize>().expect("a length"));
    let mut body = buffer[header_end..].to_vec();
    while body.len() < length {
        let mut chunk = vec![0_u8; length.saturating_sub(body.len())];
        let read = stream.read(&mut chunk).await.expect("a body read");
        assert!(read > 0, "the client closed before sending the body");
        body.extend_from_slice(&chunk[..read]);
    }
    Received {
        method,
        path,
        headers,
        body,
    }
}

async fn answer(stream: &mut TcpStream) {
    let response = "HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
    stream
        .write_all(response.as_bytes())
        .await
        .expect("the response write");
    stream.flush().await.expect("the response flush");
}
