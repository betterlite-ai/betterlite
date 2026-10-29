#![cfg(test)]

use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_logging::trace_types::{SpanType, TraceScope};
use betterlite_logging::ui_content::UiContent;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use thiserror::Error;

use super::{TraceStorage, TraceStore};
use crate::cursor::{ErrorCursor, decode_error_cursor, encode_error_cursor};
use crate::spend::SpendRow;
use crate::storage::query::{
    ListTracesQuery, SpanDetailQuery, SpanErrorQuery, SpendByRequestIdsQuery, TraceSpansQuery,
};
use crate::storage::rows::{SpanDetailRow, SpanErrorRow, SpanQueryRow, TraceListRow};

#[derive(Debug, Error)]
#[error("stub storage failure")]
struct StubError;

#[derive(Default)]
struct StubStorage {
    list_traces_rows: Vec<TraceListRow>,
    trace_spans_rows: Vec<SpanQueryRow>,
    span_detail_rows: Vec<SpanDetailRow>,
    span_error_rows: Vec<SpanErrorRow>,
    spend_rows: Vec<SpendRow>,
    spend_fails: bool,
    seen_scope: Mutex<Option<TraceScope>>,
}

impl TraceStorage for StubStorage {
    type Error = StubError;

    fn list_traces(
        &self,
        query: ListTracesQuery,
    ) -> impl Future<Output = Result<Vec<TraceListRow>, Self::Error>> {
        *self.seen_scope.lock().expect("lock") = Some(query.scope);
        ready(Ok(self.list_traces_rows.clone()))
    }

    fn trace_spans(
        &self,
        query: TraceSpansQuery,
    ) -> impl Future<Output = Result<Vec<SpanQueryRow>, Self::Error>> {
        *self.seen_scope.lock().expect("lock") = Some(query.scope);
        ready(Ok(self.trace_spans_rows.clone()))
    }

    fn span_detail(
        &self,
        query: SpanDetailQuery,
    ) -> impl Future<Output = Result<Vec<SpanDetailRow>, Self::Error>> {
        *self.seen_scope.lock().expect("lock") = Some(query.scope);
        ready(Ok(self.span_detail_rows.clone()))
    }

    fn span_error(
        &self,
        query: SpanErrorQuery,
    ) -> impl Future<Output = Result<Vec<SpanErrorRow>, Self::Error>> {
        *self.seen_scope.lock().expect("lock") = Some(query.scope);
        ready(Ok(self.span_error_rows.clone()))
    }

    fn spend_by_request_ids(
        &self,
        query: SpendByRequestIdsQuery,
    ) -> impl Future<Output = Result<Vec<SpendRow>, Self::Error>> {
        *self.seen_scope.lock().expect("lock") = Some(query.scope);
        if self.spend_fails {
            return ready(Err(StubError));
        }
        ready(Ok(self.spend_rows.clone()))
    }
}

fn scope() -> TraceScope {
    scope_with("team-1", "hash-1")
}

fn scope_with(team_id: &str, api_key_hash: &str) -> TraceScope {
    TraceScope {
        team_ids: vec![team_id.to_owned()],
        api_key_hash: api_key_hash.to_owned(),
    }
}

fn version() -> String {
    "A".repeat(64)
}

fn span_row(span_id: &str, parent: Option<&str>, span_type: SpanType) -> SpanQueryRow {
    SpanQueryRow {
        span_id: span_id.to_owned(),
        parent_span_id: parent.map(str::to_owned),
        name: span_id.to_owned(),
        span_type,
        agent: span_id.to_owned(),
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
    }
}

fn list_row(trace_id: &str, start_ms: i64) -> TraceListRow {
    TraceListRow {
        trace_id: trace_id.to_owned(),
        name: "root".to_owned(),
        service: "svc".to_owned(),
        input_preview: String::new(),
        start_ms,
        duration_ms: 10,
        status: "STATUS_CODE_OK".to_owned(),
        span_count: 1,
        agent_count: 1,
        agent_invocations: None,
        llm_calls: 0,
        tool_calls: 0,
        error_count: None,
        input_tokens: 0,
        output_tokens: 0,
        models: Vec::new(),
        request_ids: Vec::new(),
        team_id: "team-1".to_owned(),
        api_key_hash: "hash-1".to_owned(),
    }
}

#[tokio::test]
async fn get_trace_returns_none_for_an_empty_trace() {
    let store = TraceStore::new(StubStorage::default());
    let trace = store.get_trace("trace-1", &scope()).await.expect("ok");
    assert_eq!(trace, None);
}

#[tokio::test]
async fn get_trace_builds_the_trace_with_offsets_and_agents() {
    let mut root = span_row("root", None, SpanType::Agent);
    root.start_ns = 1_000_000;
    root.duration_ns = 5_000_000;
    let mut llm = span_row("llm", Some("root"), SpanType::Llm);
    llm.agent = "root".to_owned();
    llm.start_ns = 2_000_000;
    llm.duration_ns = 1_000_000;
    let storage = StubStorage {
        trace_spans_rows: vec![root, llm],
        ..StubStorage::default()
    };
    let store = TraceStore::new(storage);
    let trace = store
        .get_trace("trace-1", &scope())
        .await
        .expect("ok")
        .expect("trace");
    assert_eq!(trace.spans.len(), 2);
    assert_eq!(trace.agents.len(), 1);
    let llm_span = trace
        .spans
        .iter()
        .find(|span| span.span_id == "llm")
        .expect("llm span");
    assert_eq!(llm_span.start_offset_ms, Decimal::new(1, 0));
}

#[tokio::test]
async fn get_span_returns_span_detail_with_ui_content_and_attributes() {
    let mut attributes = IndexMap::new();
    attributes.insert("key".to_owned(), "value".to_owned());
    let storage = StubStorage {
        span_detail_rows: vec![SpanDetailRow {
            span_id: "span-1".to_owned(),
            input: "hello".to_owned(),
            output: "world".to_owned(),
            attributes,
        }],
        ..StubStorage::default()
    };
    let store = TraceStore::new(storage);
    let detail = store
        .get_span("trace-1", "span-1", &scope())
        .await
        .expect("ok")
        .expect("detail");
    assert_eq!(
        detail.input_ui,
        UiContent::Text {
            text: "hello".to_owned()
        }
    );
    assert_eq!(
        detail.output_ui,
        UiContent::Text {
            text: "world".to_owned()
        }
    );
    assert_eq!(detail.attributes.get("key"), Some(&"value".to_owned()));
}

#[tokio::test]
async fn get_span_returns_none_when_the_span_is_missing() {
    let store = TraceStore::new(StubStorage::default());
    let detail = store
        .get_span("trace-1", "span-1", &scope())
        .await
        .expect("ok");
    assert_eq!(detail, None);
}

#[tokio::test]
async fn get_span_error_returns_none_when_the_span_has_no_error() {
    let store = TraceStore::new(StubStorage::default());
    let page = store
        .get_span_error("trace-1", "span-1", &scope(), None)
        .await
        .expect("ok");
    assert_eq!(page, None);
}

#[tokio::test]
async fn get_span_error_returns_a_next_cursor_when_more_data_remains() {
    let storage = StubStorage {
        span_error_rows: vec![SpanErrorRow {
            span_id: "span-1".to_owned(),
            message: "partial".to_owned(),
            total_chars: 100,
            version: version(),
        }],
        ..StubStorage::default()
    };
    let store = TraceStore::new(storage);
    let page = store
        .get_span_error("trace-1", "span-1", &scope(), None)
        .await
        .expect("ok")
        .expect("page");
    assert!(page.next_cursor.is_some());
}

#[tokio::test]
async fn get_span_error_omits_the_next_cursor_when_the_chunk_reaches_the_end() {
    let storage = StubStorage {
        span_error_rows: vec![SpanErrorRow {
            span_id: "span-1".to_owned(),
            message: "tail".to_owned(),
            total_chars: 4,
            version: version(),
        }],
        ..StubStorage::default()
    };
    let store = TraceStore::new(storage);
    let page = store
        .get_span_error("trace-1", "span-1", &scope(), None)
        .await
        .expect("ok")
        .expect("page");
    assert_eq!(page.next_cursor, None);
}

#[tokio::test]
async fn get_span_error_omits_the_next_cursor_when_the_stored_version_differs() {
    // The database re-wrote the error under a new version between pages; the chunk is still
    // valid to show, but the cursor is no longer trustworthy for a further page.
    let storage = StubStorage {
        span_error_rows: vec![SpanErrorRow {
            span_id: "span-1".to_owned(),
            message: "partial".to_owned(),
            total_chars: 100,
            version: version(),
        }],
        ..StubStorage::default()
    };
    let store = TraceStore::new(storage);
    let requested = encode_error_cursor(&ErrorCursor {
        offset: 0,
        version: "B".repeat(64),
    });
    let page = store
        .get_span_error("trace-1", "span-1", &scope(), Some(&requested))
        .await
        .expect("ok")
        .expect("page");
    assert_eq!(page.message, "partial");
    assert_eq!(page.next_cursor, None);
}

#[tokio::test]
async fn get_span_error_computes_byte_offsets_for_multi_byte_messages() {
    let message = "h\u{e9}llo".to_owned(); // 5 chars, 6 UTF-8 bytes
    let storage = StubStorage {
        span_error_rows: vec![SpanErrorRow {
            span_id: "span-1".to_owned(),
            message: message.clone(),
            total_chars: 100,
            version: version(),
        }],
        ..StubStorage::default()
    };
    let store = TraceStore::new(storage);
    let page = store
        .get_span_error("trace-1", "span-1", &scope(), None)
        .await
        .expect("ok")
        .expect("page");
    let cursor = page.next_cursor.expect("next cursor");
    let decoded = decode_error_cursor(Some(&cursor))
        .expect("valid")
        .expect("some");
    assert_eq!(decoded.offset, u64::try_from(message.len()).expect("fits"));
}

#[tokio::test]
async fn list_traces_returns_a_next_cursor_when_the_page_is_full() {
    let storage = StubStorage {
        list_traces_rows: vec![list_row("trace-1", 100), list_row("trace-2", 200)],
        ..StubStorage::default()
    };
    let store = TraceStore::new(storage);
    let page = store
        .list_traces(&scope(), (0, 1_000), None, 2)
        .await
        .expect("ok");
    assert_eq!(page.data.len(), 2);
    assert!(page.next_cursor.is_some());
}

#[tokio::test]
async fn list_traces_returns_no_next_cursor_when_the_page_is_short() {
    let storage = StubStorage {
        list_traces_rows: vec![list_row("trace-1", 100)],
        ..StubStorage::default()
    };
    let store = TraceStore::new(storage);
    let page = store
        .list_traces(&scope(), (0, 1_000), None, 5)
        .await
        .expect("ok");
    assert_eq!(page.next_cursor, None);
}

#[tokio::test]
async fn list_traces_rejects_an_invalid_cursor() {
    let store = TraceStore::new(StubStorage::default());
    let error = store
        .list_traces(&scope(), (0, 1_000), Some("not-a-cursor!!"), 5)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Invalid trace cursor");
}

#[tokio::test]
async fn list_traces_forwards_the_scope_to_storage() {
    let scope = scope_with("team-a", "hash-a");
    let store = TraceStore::new(StubStorage::default());
    let _page = store.list_traces(&scope, (0, 1_000), None, 5).await;
    assert_eq!(
        store.storage.seen_scope.lock().expect("lock").clone(),
        Some(scope)
    );
}

#[tokio::test]
async fn get_trace_forwards_the_scope_to_storage() {
    let scope = scope_with("team-b", "hash-b");
    let store = TraceStore::new(StubStorage::default());
    let _trace = store.get_trace("trace-1", &scope).await;
    assert_eq!(
        store.storage.seen_scope.lock().expect("lock").clone(),
        Some(scope)
    );
}

#[tokio::test]
async fn get_span_forwards_the_scope_to_storage() {
    let scope = scope_with("team-c", "hash-c");
    let store = TraceStore::new(StubStorage::default());
    let _detail = store.get_span("trace-1", "span-1", &scope).await;
    assert_eq!(
        store.storage.seen_scope.lock().expect("lock").clone(),
        Some(scope)
    );
}

#[tokio::test]
async fn get_span_error_forwards_the_scope_to_storage() {
    let scope = scope_with("team-d", "hash-d");
    let store = TraceStore::new(StubStorage::default());
    let _page = store
        .get_span_error("trace-1", "span-1", &scope, None)
        .await;
    assert_eq!(
        store.storage.seen_scope.lock().expect("lock").clone(),
        Some(scope)
    );
}

#[tokio::test]
async fn spend_lookup_forwards_the_scope_to_storage() {
    let scope = scope_with("team-e", "hash-e");
    let mut root = span_row("root", None, SpanType::Agent);
    root.litellm_request_id = Some("req-1".to_owned());
    let storage = StubStorage {
        trace_spans_rows: vec![root],
        ..StubStorage::default()
    };
    let store = TraceStore::new(storage);
    let _trace = store.get_trace("trace-1", &scope).await;
    assert_eq!(
        store.storage.seen_scope.lock().expect("lock").clone(),
        Some(scope)
    );
}

#[tokio::test]
async fn spend_log_outage_does_not_fail_get_trace() {
    let mut root = span_row("root", None, SpanType::Agent);
    root.litellm_request_id = Some("req-1".to_owned());
    let storage = StubStorage {
        trace_spans_rows: vec![root],
        spend_fails: true,
        ..StubStorage::default()
    };
    let store = TraceStore::new(storage);
    let trace = store
        .get_trace("trace-1", &scope())
        .await
        .expect("ok")
        .expect("trace");
    assert_eq!(trace.summary.spend, None);
}
