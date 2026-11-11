#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io::{Read as _, Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use flate2::read::GzDecoder;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use url::Url;

use super::adapter::ClickHouseAdapter;
use super::build;
use super::rows::{LimitedWriter, compressed_rows, content_token};
use super::storage::{ClickHouseHttpStorage, ClickHouseStorageError};
use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::event::LoggerEvent;
use crate::clickhouse::clickhouse_logging::batch_logger::ClickHouseBatchLogger;
use crate::clickhouse::clickhouse_logging::context::LensAnalysis;
use crate::clickhouse::clickhouse_logging::schema::SPEND_LOGS_TABLE;
use crate::clickhouse::clickhouse_logging::settings::ClickHouseSettings;
use crate::clickhouse::clickhouse_logging::spend_logger::ClickHouseSpendLogger;
use crate::clickhouse::clickhouse_logging::storage::ClickHouseStorage;
use crate::finops::metering::clock::UtcClock;
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
        litellm_metadata: Arc::new(IndexMap::new()),
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
    assert_spend_row(&calls[0].rows[0]);
}

fn assert_spend_row(row: &SpendLogRecord) {
    assert_eq!(row.request_id, "chatcmpl-1");
    assert_eq!(row.response_id, "chatcmpl-1");
    assert_eq!(row.call_type, "completion");
    assert_eq!(row.user, "user-1");
    assert_eq!(row.end_user, "end-user-1");
    assert_eq!(row.model, "gpt-4o");
    assert_eq!(row.status, "success");
    assert_spend_usage(row);
    assert_spend_trace(row);
}

fn assert_spend_usage(row: &SpendLogRecord) {
    assert_eq!(row.spend, rust_decimal::Decimal::new(5, 1));
    assert_eq!(row.prompt_tokens, 2);
    assert_eq!(row.completion_tokens, 3);
    assert_eq!(row.total_tokens, 5);
    assert_eq!(row.start_time, 1_704_067_200_000);
    assert_eq!(row.end_time, 1_704_067_201_500);
    assert_eq!(row.completion_start_time, Some(1_704_067_200_500));
    assert!(!row.cache_hit);
}

fn assert_spend_trace(row: &SpendLogRecord) {
    assert_eq!(row.session_id, "session-1");
    assert_eq!(row.trace_id, "4bf92f3577b34da6a3ce929d0e0e4736");
    assert_eq!(row.span_id, "b7ad6b7169203331");
    assert_eq!(row.request_tags, vec!["team-a".to_owned()]);
    assert!(row.metadata.contains(r#""litellm_lens_internal":false"#));
    assert_eq!(row.messages, r#"[{"role":"user","content":"Hello"}]"#);
}

#[rstest]
#[case::the_payload_session_wins(r#""session_id": "session-1","#, "session-1")]
#[case::the_request_metadata_session_fills_in(r#""session_id": "","#, "request-session")]
#[tokio::test]
async fn the_request_metadata_supplies_the_session_id(
    #[case] payload_session: &str,
    #[case] expected: &str,
) {
    let storage = FakeStorage::default();
    let adapter = adapter(storage.clone());
    let payload = PAYLOAD
        .replace(r#""session_id": "session-1","#, payload_session)
        .replace(
            r#""user_api_key_user_id": "user-1","#,
            r#""user_api_key_user_id": "user-1", "requester_metadata": {"session_id": "request-session"},"#,
        );
    adapter
        .async_log_success_event(Arc::new(event_from(&payload)))
        .await
        .expect("the event is logged");
    adapter.logger.batch.flush_queue().await;
    assert_eq!(storage.calls()[0].rows[0].session_id, expected);
}

#[tokio::test]
async fn a_requester_metadata_that_is_not_an_object_is_ignored() {
    let storage = FakeStorage::default();
    let adapter = adapter(storage.clone());
    let payload = PAYLOAD.replace(
        r#""user_api_key_user_id": "user-1","#,
        r#""user_api_key_user_id": "user-1", "requester_metadata": "plain text","#,
    );
    adapter
        .async_log_success_event(Arc::new(event_from(&payload)))
        .await
        .expect("the event is logged");
    adapter.logger.batch.flush_queue().await;
    assert_eq!(storage.calls()[0].rows[0].session_id, "session-1");
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

#[test]
fn a_database_with_an_underscore_is_accepted() {
    let catalog = LoggerCatalog::new(inputs(&[
        ("CLICKHOUSE_URL", "http://127.0.0.1:8123"),
        ("CLICKHOUSE_DATABASE", "lit_1"),
    ]));
    assert!(
        catalog
            .resolve_logger("clickhouse")
            .expect("the name is available")
            .is_some()
    );
}

#[test]
fn an_empty_database_is_rejected() {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("valid proxy policy"),
    );
    let client = ProxiedClient::outbound(&policy).expect("http client");
    let url = SecretString::from("http://127.0.0.1:8123".to_owned());
    assert!(ClickHouseHttpStorage::new(client, "", &url, FixedClock).is_err());
}

#[test]
fn rows_encode_one_json_row_per_line() {
    let body =
        compressed_rows(&[row("first"), row("second")], 7, usize::MAX).expect("the rows encode");
    let mut text = String::new();
    GzDecoder::new(body.as_slice())
        .read_to_string(&mut text)
        .expect("the body is gzip");
    let lines: Vec<&str> = text.split('\n').collect();
    assert_eq!(lines.len(), 2);
    let first: Json = serde_json::from_str(lines[0]).expect("the first row is JSON");
    let Json::Object(fields) = &first else {
        panic!("the row is a JSON object");
    };
    assert_eq!(
        fields.get("request_id"),
        Some(&Json::Text("first".to_owned()))
    );
    assert_eq!(
        fields.get("EngineReceivedMs"),
        Some(&Json::Number(serde_json::Number::from(7_u64)))
    );
    let _second: Json = serde_json::from_str(lines[1]).expect("the second row is JSON");
}

fn encoded_text(rows: &[SpendLogRecord]) -> String {
    let body = compressed_rows(rows, 7, usize::MAX).expect("the rows encode");
    let mut text = String::new();
    GzDecoder::new(body.as_slice())
        .read_to_string(&mut text)
        .expect("the body is gzip");
    text
}

#[rstest]
#[case::exactly_at_the_limit(0, true)]
#[case::one_byte_over_the_limit(1, false)]
fn the_encoded_rows_are_capped_at_the_insert_limit(#[case] shortfall: usize, #[case] fits: bool) {
    let rows = [row("first"), row("second")];
    let limit = encoded_text(&rows).len().saturating_sub(shortfall);
    let result = compressed_rows(&rows, 7, limit);
    assert_eq!(result.is_ok(), fits);
    if !fits {
        assert!(matches!(
            result,
            Err(ClickHouseStorageError::InsertTooLarge)
        ));
    }
}

#[derive(Default)]
struct FlushCounter {
    flushes: usize,
}

impl Write for FlushCounter {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        self.flushes = self.flushes.saturating_add(1);
        Ok(())
    }
}

#[test]
fn the_limited_writer_flushes_its_inner_writer() {
    let mut writer = LimitedWriter {
        inner: FlushCounter::default(),
        remaining: 8,
        exceeded: false,
    };
    writer.flush().expect("the flush is passed on");
    assert_eq!(writer.inner.flushes, 1);
}

#[test]
fn a_limit_that_ends_with_the_first_row_rejects_the_separator() {
    let rows = [row("first"), row("second")];
    let text = encoded_text(&rows);
    let first_row = text.find('\n').expect("the rows are newline separated");
    assert!(matches!(
        compressed_rows(&rows, 7, first_row),
        Err(ClickHouseStorageError::InsertTooLarge)
    ));
}

#[test]
fn a_batch_keeps_its_deduplication_token() {
    let rows = vec![row("first"), row("second")];
    let token = content_token(&rows).expect("the token is computed");
    assert_eq!(
        token,
        content_token(&rows).expect("the same rows compute the same token")
    );
    assert_ne!(
        token,
        content_token(&rows[..1]).expect("fewer rows compute another token")
    );
    assert_eq!(token.len(), 64);
}

#[tokio::test]
async fn an_insert_posts_gzip_json_each_row() {
    let (base, served) = serve_many(1, 200).await;
    let storage = http_storage(&base);
    let rows = vec![row("first"), row("second")];
    let (outcome, arrived) = tokio::join!(
        storage.insert_rows(SPEND_LOGS_TABLE, &rows),
        timeout(Duration::from_secs(10), served)
    );
    outcome.expect("the insert is accepted");
    let received = arrived.expect("the insert request arrived");
    assert_insert_request(&received[0], &rows);
}

#[tokio::test]
async fn an_insert_of_a_few_megabytes_is_sent() {
    let (base, served) = serve_many(1, 200).await;
    let storage = http_storage(&base);
    let mut large = row("large");
    large.response = "a".repeat(2 * 1024 * 1024);
    let rows = [large];
    let (outcome, arrived) = tokio::join!(
        storage.insert_rows(SPEND_LOGS_TABLE, &rows),
        timeout(Duration::from_secs(10), served)
    );
    outcome.expect("the insert is accepted");
    let received = arrived.expect("the insert request arrived");
    assert_eq!(received.len(), 1);
}

fn assert_insert_request(request: &Received, rows: &[SpendLogRecord]) {
    assert_eq!(request.method, "POST");
    assert_eq!(
        request.headers.get("content-encoding").map(String::as_str),
        Some("gzip")
    );
    let query = request_query(request);
    assert_eq!(
        query.get("query").map(String::as_str),
        Some("INSERT INTO `litellm`.spend_logs FORMAT JSONEachRow")
    );
    assert_eq!(
        query.get("insert_deduplication_token").map(String::as_str),
        Some(content_token(rows).expect("the token is computed").as_str())
    );
    assert_eq!(query.get("async_insert").map(String::as_str), Some("1"));
    let decoded = decoded_rows(request);
    assert_eq!(decoded.len(), rows.len());
    assert_eq!(
        decoded[0].get("request_id"),
        Some(&Json::Text("first".to_owned()))
    );
    assert_eq!(
        decoded[0].get("start_time"),
        Some(&Json::Text("2024-01-01T00:00:00.000Z".to_owned()))
    );
    assert_eq!(
        decoded[0].get("end_time"),
        Some(&Json::Text("2024-01-01T00:00:01.500Z".to_owned()))
    );
    assert_eq!(
        decoded[0].get("EngineReceivedMs"),
        Some(&Json::Number(serde_json::Number::from(
            1_704_067_200_123_i64
        )))
    );
}

#[tokio::test]
async fn a_retried_insert_keeps_its_deduplication_token() {
    let (base, served) = serve_many(3, 200).await;
    let storage = http_storage(&base);
    let rows = vec![row("first")];
    let other = [row("other")];
    let (outcome, arrived) = tokio::join!(
        async {
            storage.insert_rows(SPEND_LOGS_TABLE, &rows).await?;
            storage.insert_rows(SPEND_LOGS_TABLE, &rows).await?;
            storage.insert_rows(SPEND_LOGS_TABLE, &other).await
        },
        timeout(Duration::from_secs(10), served)
    );
    outcome.expect("the inserts are accepted");
    let received = arrived.expect("the insert requests arrived");
    let tokens: Vec<String> = received
        .iter()
        .map(|request| {
            request_query(request)
                .get("insert_deduplication_token")
                .cloned()
                .unwrap_or_default()
        })
        .collect();
    assert_eq!(tokens.len(), 3);
    assert_eq!(tokens[0], tokens[1]);
    assert_ne!(tokens[0], tokens[2]);
}

#[tokio::test]
async fn an_insert_replaces_the_url_insert_parameters() {
    let (base, served) = serve_many(1, 200).await;
    let storage = http_storage(&format!(
        "{base}/?query=stale&async_insert=0&database=other"
    ));
    let rows = [row("first")];
    let (outcome, arrived) = tokio::join!(
        storage.insert_rows(SPEND_LOGS_TABLE, &rows),
        timeout(Duration::from_secs(10), served)
    );
    outcome.expect("the insert is accepted");
    let received = arrived.expect("the insert request arrived");
    let query = request_query(&received[0]);
    assert_eq!(
        query.get("query").map(String::as_str),
        Some("INSERT INTO `litellm`.spend_logs FORMAT JSONEachRow")
    );
    assert_eq!(query.get("async_insert").map(String::as_str), Some("1"));
    assert_eq!(query.get("database").map(String::as_str), Some("other"));
}

#[tokio::test]
async fn an_insert_answered_with_an_error_status_is_reported() {
    let (base, served) = serve_many(1, 500).await;
    let storage = http_storage(&base);
    let rows = [row("first")];
    let (outcome, arrived) = tokio::join!(
        storage.insert_rows(SPEND_LOGS_TABLE, &rows),
        timeout(Duration::from_secs(10), served)
    );
    assert!(matches!(
        outcome,
        Err(ClickHouseStorageError::InsertFailed(500))
    ));
    arrived.expect("the insert request arrived");
}

#[tokio::test]
async fn an_unknown_table_is_rejected() {
    let storage = http_storage("http://127.0.0.1:1");
    assert!(
        storage
            .insert_rows("otel_traces", &[row("first")])
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_zero_retention_schema_is_rejected() {
    let storage = http_storage("http://127.0.0.1:1");
    assert!(storage.ensure_schema(7, 0).await.is_err());
}

#[tokio::test]
async fn schema_setup_posts_the_database_table_and_ttl() {
    let (base, served) = serve_many(3, 200).await;
    let storage = http_storage(&base);
    let (outcome, arrived) = tokio::join!(
        storage.ensure_schema(7, 30),
        timeout(Duration::from_secs(10), served)
    );
    outcome.expect("the schema is accepted");
    let received = arrived.expect("the schema statements arrived");
    let statements: Vec<String> = received
        .iter()
        .map(|request| String::from_utf8(request.body.clone()).expect("a UTF-8 statement"))
        .collect();
    assert_eq!(statements.len(), 3);
    assert!(statements[0].starts_with("CREATE DATABASE IF NOT EXISTS `litellm`"));
    assert!(statements[1].contains("CREATE TABLE IF NOT EXISTS `litellm`.spend_logs"));
    assert!(statements[2].contains("MODIFY TTL toDateTime(start_time) + INTERVAL 30 DAY"));
}

#[tokio::test]
async fn a_resolved_logger_posts_gzip_json_each_row() {
    let (base, served) = serve_many(1, 200).await;
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
    let received = timeout(Duration::from_secs(10), served)
        .await
        .expect("the insert request arrived");
    let decoded = decoded_rows(&received[0]);
    assert_eq!(decoded.len(), 1);
    assert_eq!(
        decoded[0].get("request_id"),
        Some(&Json::Text("chatcmpl-1".to_owned()))
    );
    assert!(matches!(
        decoded[0].get("EngineReceivedMs"),
        Some(Json::Number(_))
    ));
}

#[derive(Clone, Copy, Debug)]
struct FixedClock;

impl UtcClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp_millis(1_704_067_200_123).expect("a valid fixture time")
    }
}

fn http_storage(base: &str) -> ClickHouseHttpStorage<FixedClock> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("valid proxy policy"),
    );
    ClickHouseHttpStorage::new(
        ProxiedClient::outbound(&policy).expect("http client"),
        "litellm",
        &SecretString::from(base.to_owned()),
        FixedClock,
    )
    .expect("the storage builds")
}

fn row(request_id: &str) -> SpendLogRecord {
    SpendLogRecord {
        request_id: request_id.to_owned(),
        response_id: request_id.to_owned(),
        call_type: "completion".to_owned(),
        api_key: String::new(),
        key_alias: String::new(),
        team_id: String::new(),
        team_alias: String::new(),
        organization_id: String::new(),
        user: String::new(),
        end_user: String::new(),
        model: "gpt-4o".to_owned(),
        model_group: String::new(),
        model_id: String::new(),
        custom_llm_provider: String::new(),
        api_base: String::new(),
        spend: rust_decimal::Decimal::ZERO,
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        start_time: 1_704_067_200_000,
        end_time: 1_704_067_201_500,
        completion_start_time: None,
        status: "success".to_owned(),
        error_str: String::new(),
        cache_hit: false,
        session_id: String::new(),
        trace_id: String::new(),
        span_id: String::new(),
        request_tags: Vec::new(),
        metadata: "{}".to_owned(),
        messages: String::new(),
        response: String::new(),
    }
}

fn request_query(request: &Received) -> BTreeMap<String, String> {
    Url::parse(&format!("http://clickhouse{}", request.path))
        .expect("a request URL")
        .query_pairs()
        .into_owned()
        .collect()
}

fn decoded_rows(request: &Received) -> Vec<IndexMap<String, Json>> {
    let mut text = String::new();
    GzDecoder::new(request.body.as_slice())
        .read_to_string(&mut text)
        .expect("the body is gzip");
    text.split('\n')
        .map(
            |line| match serde_json::from_str::<Json>(line).expect("a JSON row") {
                Json::Object(fields) => fields,
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                    panic!("a row object")
                }
            },
        )
        .collect()
}

struct Received {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

// The returned future accepts count requests and answers each with status; the test joins it with the call under test.
async fn serve_many(count: usize, status: u16) -> (String, impl Future<Output = Vec<Received>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let address = listener.local_addr().expect("the bound address");
    let served = async move {
        let mut received = Vec::new();
        for _ in 0..count {
            let (mut stream, _) = listener.accept().await.expect("a request");
            received.push(receive(&mut stream).await);
            answer(&mut stream, status).await;
        }
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

async fn answer(stream: &mut TcpStream, status: u16) {
    let response = format!("HTTP/1.1 {status} X\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
    stream
        .write_all(response.as_bytes())
        .await
        .expect("the response write");
    stream.flush().await.expect("the response flush");
}
