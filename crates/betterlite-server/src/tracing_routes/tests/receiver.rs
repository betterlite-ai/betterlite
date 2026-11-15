use std::future::Future;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{Body, Bytes};
use betterlite_logging::otlp_errors::CodecError;
use betterlite_logging::otlp_normalize::{DecodedSpan, OTLP_MAX_BODY_BYTES, SpanCodec, SpanRow};
use betterlite_logging::trace_types::{SpanType, TraceScope};
use betterlite_tracing::spend::SpendRow;
use betterlite_tracing::storage::query::{
    ListTracesQuery, SpanDetailQuery, SpanErrorQuery, SpendByRequestIdsQuery, TraceSpansQuery,
};
use betterlite_tracing::storage::rows::{SpanDetailRow, SpanErrorRow, SpanQueryRow, TraceListRow};
use betterlite_tracing::storage::trace_store::{SpanErrorTarget, TraceStorage, TraceStore};
use futures_util::stream;
use indexmap::IndexMap;
use thiserror::Error;

use super::support::{FakeQueryStore, QueryAnswer, decoded_span, span_row};
use crate::tracing_routes::access::Tenant;
use crate::tracing_routes::query::QueryScope;
use crate::tracing_routes::receiver::{
    IngestError, ReceiverConfigError, StoreTraceReceiver, TraceReadError, TraceRuntime as _,
    TraceSpanSink,
};

#[derive(Debug, Error)]
#[error("stub storage failure")]
struct StubError;

#[derive(Clone, Debug)]
enum SinkMode {
    Ok,
    Overflow(String),
    Unavailable,
}

#[derive(Clone, Copy, Debug)]
enum ReadMode {
    Ok,
    Rows,
    Storage,
}

#[derive(Debug, Error)]
enum SinkError {
    #[error("{0}")]
    Overflow(String),
    #[error("trace storage is unavailable")]
    Unavailable,
}

struct FakeStorage {
    inserted: Arc<Mutex<Vec<SpanRow>>>,
    sink: SinkMode,
    read: ReadMode,
}

impl FakeStorage {
    fn new(sink: SinkMode, read: ReadMode) -> Self {
        Self {
            inserted: Arc::new(Mutex::new(Vec::new())),
            sink,
            read,
        }
    }

    fn read_rows<Row>(&self) -> Result<Vec<Row>, StubError> {
        match self.read {
            ReadMode::Ok | ReadMode::Rows => Ok(Vec::new()),
            ReadMode::Storage => Err(StubError),
        }
    }
}

impl TraceSpanSink for FakeStorage {
    type Error = SinkError;

    fn insert_spans(
        &self,
        rows: &[SpanRow],
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        std::future::ready(match &self.sink {
            SinkMode::Ok => {
                self.inserted.lock().expect("lock").extend_from_slice(rows);
                Ok(())
            }
            SinkMode::Overflow(message) => Err(SinkError::Overflow(message.clone())),
            SinkMode::Unavailable => Err(SinkError::Unavailable),
        })
    }

    fn overflow_message(error: &Self::Error) -> Option<String> {
        match error {
            SinkError::Overflow(message) => Some(message.clone()),
            SinkError::Unavailable => None,
        }
    }
}

impl TraceStorage for FakeStorage {
    type Error = StubError;

    fn list_traces(
        &self,
        _query: ListTracesQuery,
    ) -> impl Future<Output = Result<Vec<TraceListRow>, StubError>> + Send {
        std::future::ready(self.read_rows())
    }

    fn trace_spans(
        &self,
        _query: TraceSpansQuery,
    ) -> impl Future<Output = Result<Vec<SpanQueryRow>, StubError>> + Send {
        std::future::ready(if matches!(self.read, ReadMode::Rows) {
            let row = SpanQueryRow {
                span_id: "span-1".to_owned(),
                parent_span_id: None,
                name: "root".to_owned(),
                span_type: SpanType::Agent,
                agent: "root".to_owned(),
                start_ns: 0,
                duration_ns: 1_000_000,
                status: "STATUS_CODE_OK".to_owned(),
                status_message: None,
                error_truncated: false,
                input_preview: String::new(),
                model: None,
                input_tokens: 0,
                output_tokens: 0,
                litellm_request_id: None,
                team_id: "team-1".to_owned(),
                api_key_hash: "hash-1".to_owned(),
                service: "svc".to_owned(),
            };
            Ok(vec![row])
        } else {
            self.read_rows()
        })
    }

    fn span_detail(
        &self,
        _query: SpanDetailQuery,
    ) -> impl Future<Output = Result<Vec<SpanDetailRow>, StubError>> + Send {
        std::future::ready(if matches!(self.read, ReadMode::Rows) {
            Ok(vec![SpanDetailRow {
                span_id: "span-1".to_owned(),
                input: "in".to_owned(),
                output: "out".to_owned(),
                attributes: IndexMap::new(),
            }])
        } else {
            self.read_rows()
        })
    }

    fn span_error(
        &self,
        _query: SpanErrorQuery,
    ) -> impl Future<Output = Result<Vec<SpanErrorRow>, StubError>> + Send {
        std::future::ready(if matches!(self.read, ReadMode::Rows) {
            Ok(vec![SpanErrorRow {
                span_id: "span-1".to_owned(),
                message: "boom".to_owned(),
                total_chars: 4,
                version: "0".repeat(64),
            }])
        } else {
            self.read_rows()
        })
    }

    fn spend_by_request_ids(
        &self,
        _query: SpendByRequestIdsQuery,
    ) -> impl Future<Output = Result<Vec<SpendRow>, StubError>> + Send {
        std::future::ready(self.read_rows())
    }
}

enum CodecAnswer {
    Ok(Vec<DecodedSpan>),
    Overflow(String),
    Malformed(String),
}

struct FakeCodec(CodecAnswer);

impl SpanCodec for FakeCodec {
    fn decode(
        &self,
        _payload: &[u8],
        _content_type: Option<&str>,
    ) -> Result<Vec<DecodedSpan>, CodecError> {
        match &self.0 {
            CodecAnswer::Ok(spans) => Ok(spans.clone()),
            CodecAnswer::Overflow(message) => Err(CodecError::Overflow(message.clone())),
            CodecAnswer::Malformed(message) => Err(CodecError::Malformed(message.clone())),
        }
    }
}

type Receiver = StoreTraceReceiver<FakeStorage, FakeCodec, FakeQueryStore>;

fn receiver(storage: FakeStorage, codec: CodecAnswer, timeout: Duration) -> Receiver {
    let store = TraceStore {
        storage,
        page_size: 50,
    };
    let queries: Arc<FakeQueryStore> = Arc::new(FakeQueryStore::new(
        QueryAnswer::Ok("{}".to_owned()),
        QueryAnswer::Ok("{}".to_owned()),
    ));
    StoreTraceReceiver::new(store, FakeCodec(codec), queries, 2, timeout).expect("receiver")
}

fn tenant() -> Tenant {
    Tenant {
        team_id: "team-1".to_owned(),
        api_key_hash: "hash-1".to_owned(),
        org_id: "org-1".to_owned(),
    }
}

async fn ingest(receiver: &Receiver, body: Vec<u8>) -> Result<usize, IngestError> {
    receiver
        .ingest(Body::from(body), Some("application/json"), None, &tenant())
        .await
}

#[tokio::test]
async fn ingest_decodes_stamps_and_inserts() {
    let storage = FakeStorage::new(SinkMode::Ok, ReadMode::Ok);
    let inserted_rows = Arc::clone(&storage.inserted);
    let receiver = receiver(
        storage,
        CodecAnswer::Ok(vec![decoded_span()]),
        Duration::from_secs(30),
    );
    let count = ingest(&receiver, b"{}".to_vec()).await.expect("ingest");
    assert_eq!(count, 1);
    let row = inserted_rows
        .lock()
        .expect("lock")
        .first()
        .cloned()
        .expect("inserted row");
    assert_eq!(row.team_id, "team-1");
    assert_eq!(row.api_key_hash, "hash-1");
    assert_eq!(
        row.resource_attributes.get("litellm.org_id"),
        Some(&"org-1".to_owned())
    );
}

#[tokio::test]
async fn ingest_rejects_a_body_over_the_limit() {
    let receiver = receiver(
        FakeStorage::new(SinkMode::Ok, ReadMode::Ok),
        CodecAnswer::Ok(Vec::new()),
        Duration::from_secs(30),
    );
    let body = vec![0_u8; OTLP_MAX_BODY_BYTES.saturating_add(1)];
    match ingest(&receiver, body).await {
        Err(IngestError::TooLarge(message)) => {
            assert_eq!(
                message,
                format!("OTLP body exceeds {OTLP_MAX_BODY_BYTES} bytes")
            );
        }
        _ => panic!("an oversized body must be rejected"),
    }
}

#[tokio::test]
async fn ingest_accepts_a_body_of_exactly_the_limit() {
    let receiver = receiver(
        FakeStorage::new(SinkMode::Ok, ReadMode::Ok),
        CodecAnswer::Ok(Vec::new()),
        Duration::from_secs(30),
    );
    let body = vec![0_u8; OTLP_MAX_BODY_BYTES];
    assert_eq!(ingest(&receiver, body).await.expect("ingest"), 0);
}

#[tokio::test]
async fn ingest_maps_a_decoder_overflow_to_the_too_large_message() {
    let receiver = receiver(
        FakeStorage::new(SinkMode::Ok, ReadMode::Ok),
        CodecAnswer::Overflow("payload too large".to_owned()),
        Duration::from_secs(30),
    );
    match ingest(&receiver, b"{}".to_vec()).await {
        Err(IngestError::TooLarge(message)) => assert_eq!(message, "payload too large"),
        _ => panic!("a decoder overflow must be too large"),
    }
}

#[tokio::test]
async fn ingest_maps_a_malformed_payload_to_the_invalid_message() {
    let receiver = receiver(
        FakeStorage::new(SinkMode::Ok, ReadMode::Ok),
        CodecAnswer::Malformed("bad payload".to_owned()),
        Duration::from_secs(30),
    );
    match ingest(&receiver, b"{}".to_vec()).await {
        Err(IngestError::Invalid(message)) => assert_eq!(message, "bad payload"),
        _ => panic!("a malformed payload must be invalid"),
    }
}

#[tokio::test]
async fn ingest_maps_a_sink_overflow_to_the_too_large_message() {
    let receiver = receiver(
        FakeStorage::new(
            SinkMode::Overflow("rows too large".to_owned()),
            ReadMode::Ok,
        ),
        CodecAnswer::Ok(vec![decoded_span()]),
        Duration::from_secs(30),
    );
    match ingest(&receiver, b"{}".to_vec()).await {
        Err(IngestError::TooLarge(message)) => assert_eq!(message, "rows too large"),
        _ => panic!("a sink overflow must be too large"),
    }
}

#[tokio::test]
async fn ingest_maps_a_sink_failure_to_storage() {
    let receiver = receiver(
        FakeStorage::new(SinkMode::Unavailable, ReadMode::Ok),
        CodecAnswer::Ok(vec![decoded_span()]),
        Duration::from_secs(30),
    );
    assert!(matches!(
        ingest(&receiver, b"{}".to_vec()).await,
        Err(IngestError::Storage)
    ));
}

#[tokio::test]
async fn ingest_times_out_to_an_overload() {
    let receiver = receiver(
        FakeStorage::new(SinkMode::Ok, ReadMode::Ok),
        CodecAnswer::Ok(Vec::new()),
        Duration::ZERO,
    );
    let body: Body = Body::from_stream(stream::pending::<Result<Bytes, io::Error>>());
    let result = receiver
        .ingest(body, Some("application/json"), None, &tenant())
        .await;
    assert!(matches!(result, Err(IngestError::Overloaded)));
}

#[test]
fn the_constructor_rejects_zero_concurrency() {
    let store = TraceStore {
        storage: FakeStorage::new(SinkMode::Ok, ReadMode::Ok),
        page_size: 50,
    };
    let queries: Arc<FakeQueryStore> = Arc::new(FakeQueryStore::new(
        QueryAnswer::Ok("{}".to_owned()),
        QueryAnswer::Ok("{}".to_owned()),
    ));
    let result = StoreTraceReceiver::new(
        store,
        FakeCodec(CodecAnswer::Ok(Vec::new())),
        queries,
        0,
        Duration::from_secs(30),
    );
    assert!(matches!(result, Err(ReceiverConfigError::Concurrency)));
}

#[tokio::test]
async fn a_cursor_failure_is_an_invalid_cursor() {
    let receiver = receiver(
        FakeStorage::new(SinkMode::Ok, ReadMode::Ok),
        CodecAnswer::Ok(Vec::new()),
        Duration::from_secs(30),
    );
    let scope = TraceScope {
        team_ids: Vec::new(),
        api_key_hash: String::new(),
    };
    let result = receiver.list_traces(&scope, (0, 1), Some("!!!")).await;
    assert!(matches!(
        result,
        Err(TraceReadError::InvalidCursor(_message))
    ));
}

#[tokio::test]
async fn a_storage_failure_is_storage() {
    let receiver = receiver(
        FakeStorage::new(SinkMode::Ok, ReadMode::Storage),
        CodecAnswer::Ok(Vec::new()),
        Duration::from_secs(30),
    );
    let scope = TraceScope {
        team_ids: Vec::new(),
        api_key_hash: String::new(),
    };
    let result = receiver.list_traces(&scope, (0, 1), None).await;
    assert!(matches!(result, Err(TraceReadError::Storage)));
}

#[tokio::test]
async fn the_query_store_is_the_injected_one() {
    use secrecy::SecretString;
    let receiver = receiver(
        FakeStorage::new(SinkMode::Ok, ReadMode::Ok),
        CodecAnswer::Ok(Vec::new()),
        Duration::from_secs(30),
    );
    let store = receiver.query_store();
    let secret = SecretString::from("sk-master".to_owned());
    let scope = QueryScope::Admin;
    let result = store.query_sql("SELECT 1", &scope, &secret).await;
    assert_eq!(result.expect("query"), "{}");
}

#[tokio::test]
async fn a_span_row_can_be_stamped_twice_without_duplication() {
    let tenant = tenant();
    let mut rows = vec![span_row()];
    tenant.stamp_rows(&mut rows);
    tenant.stamp_rows(&mut rows);
    let row = rows.first().expect("row");
    assert_eq!(row.resource_attributes.len(), 4);
}

#[tokio::test]
async fn reads_return_what_the_store_found() {
    let receiver = receiver(
        FakeStorage::new(SinkMode::Ok, ReadMode::Rows),
        CodecAnswer::Ok(Vec::new()),
        Duration::from_secs(30),
    );
    let scope = TraceScope {
        team_ids: Vec::new(),
        api_key_hash: String::new(),
    };
    let trace = receiver
        .get_trace("trace-1", &scope, "")
        .await
        .expect("trace");
    assert_eq!(trace.expect("a trace").spans.len(), 1);
    let span = receiver
        .get_span("trace-1", "span-1", &scope, "")
        .await
        .expect("span");
    assert_eq!(span.expect("a span").output, "out");
    let target = SpanErrorTarget {
        trace_id: "trace-1",
        span_id: "span-1",
        trace_ref: "",
    };
    let page = receiver
        .get_span_error(target, &scope, None)
        .await
        .expect("error page");
    assert_eq!(page.expect("an error page").message, "boom");
}

#[tokio::test]
async fn reads_find_nothing_when_the_store_is_empty() {
    let receiver = receiver(
        FakeStorage::new(SinkMode::Ok, ReadMode::Ok),
        CodecAnswer::Ok(Vec::new()),
        Duration::from_secs(30),
    );
    let scope = TraceScope {
        team_ids: Vec::new(),
        api_key_hash: String::new(),
    };
    let trace = receiver.get_trace("trace-1", &scope, "").await;
    assert!(matches!(trace, Ok(None)));
    let span = receiver.get_span("trace-1", "span-1", &scope, "").await;
    assert!(matches!(span, Ok(None)));
    let target = SpanErrorTarget {
        trace_id: "trace-1",
        span_id: "span-1",
        trace_ref: "",
    };
    let page = receiver.get_span_error(target, &scope, None).await;
    assert!(matches!(page, Ok(None)));
}
