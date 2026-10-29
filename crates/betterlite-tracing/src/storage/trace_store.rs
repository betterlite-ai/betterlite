use std::collections::BTreeSet;
use std::error::Error;
use std::future::Future;

use betterlite_logging::trace_types::{SpanDetail, SpanErrorPage, Trace, TracePage, TraceScope};
use betterlite_logging::ui_content::to_ui_content;

use crate::cursor::{
    ErrorCursor, decode_error_cursor, decode_trace_cursor, encode_error_cursor, encode_trace_cursor,
};
use crate::projections::{NANOS_PER_MS, trace_from_rows, trace_summary_from_row};
use crate::spend::{SPEND_WINDOW_MS, SpendRow};
use crate::storage::error::StorageError;
use crate::storage::query::{
    ListTracesQuery, SpanDetailQuery, SpanErrorQuery, SpendByRequestIdsQuery, TraceSpansQuery,
};
use crate::storage::rows::{SpanDetailRow, SpanErrorRow, SpanQueryRow, TraceListRow};

mod tests;

pub trait TraceStorage {
    type Error: Error + Send + Sync + 'static;

    fn list_traces(
        &self,
        query: ListTracesQuery,
    ) -> impl Future<Output = Result<Vec<TraceListRow>, Self::Error>> + Send;
    fn trace_spans(
        &self,
        query: TraceSpansQuery,
    ) -> impl Future<Output = Result<Vec<SpanQueryRow>, Self::Error>> + Send;
    fn span_detail(
        &self,
        query: SpanDetailQuery,
    ) -> impl Future<Output = Result<Vec<SpanDetailRow>, Self::Error>> + Send;
    fn span_error(
        &self,
        query: SpanErrorQuery,
    ) -> impl Future<Output = Result<Vec<SpanErrorRow>, Self::Error>> + Send;
    fn spend_by_request_ids(
        &self,
        query: SpendByRequestIdsQuery,
    ) -> impl Future<Output = Result<Vec<SpendRow>, Self::Error>> + Send;
}

pub struct TraceStore<S> {
    storage: S,
}

fn unique_nonempty(ids: &[String]) -> Vec<String> {
    let set: BTreeSet<&str> = ids
        .iter()
        .map(String::as_str)
        .filter(|id| !id.is_empty())
        .collect();
    set.into_iter().map(str::to_owned).collect()
}

fn u64_from_usize(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn list_window(rows: &[TraceListRow], start_ms: i64, end_ms: i64) -> (i64, i64) {
    let window_start = rows
        .iter()
        .map(|row| row.start_ms)
        .min()
        .unwrap_or(start_ms);
    let window_end = rows
        .iter()
        .map(|row| row.start_ms.saturating_add(row.duration_ms))
        .max()
        .unwrap_or(end_ms);
    (window_start, window_end)
}

fn first_of<T, E>(rows: Result<Vec<T>, E>) -> Result<Option<T>, StorageError<E>>
where
    E: Error,
{
    Ok(rows.map_err(StorageError::Query)?.into_iter().next())
}

fn span_error_query(
    trace_id: &str,
    span_id: &str,
    scope: &TraceScope,
    position: Option<&ErrorCursor>,
) -> SpanErrorQuery {
    SpanErrorQuery {
        scope: scope.clone(),
        trace_id: trace_id.to_owned(),
        span_id: span_id.to_owned(),
        error_offset: position.map_or(0, |requested| requested.offset),
        error_version: position.map_or_else(String::new, |requested| requested.version.clone()),
    }
}

fn span_detail_from_row(row: SpanDetailRow) -> SpanDetail {
    SpanDetail {
        span_id: row.span_id,
        input_ui: to_ui_content(&row.input),
        output_ui: to_ui_content(&row.output),
        input: row.input,
        output: row.output,
        attributes: row.attributes,
    }
}

fn error_page(row: SpanErrorRow, position: Option<&ErrorCursor>) -> SpanErrorPage {
    let requested_offset = position.map_or(0, |cursor| cursor.offset);
    let version_matches = position.is_none_or(|cursor| cursor.version == row.version);
    let next_offset = requested_offset.saturating_add(u64_from_usize(row.message.len()));
    let next_cursor =
        (version_matches && next_offset < u64_from_usize(row.total_chars)).then(|| {
            encode_error_cursor(&ErrorCursor {
                offset: next_offset,
                version: row.version.clone(),
            })
        });
    SpanErrorPage {
        span_id: row.span_id,
        message: row.message,
        total_chars: row.total_chars,
        next_cursor,
    }
}

impl<S> TraceStore<S>
where
    S: TraceStorage + Sync,
{
    pub const fn new(storage: S) -> Self {
        Self { storage }
    }

    async fn spend_rows(
        &self,
        scope: &TraceScope,
        request_ids: &[String],
        start_ms: i64,
        end_ms: i64,
    ) -> Vec<SpendRow> {
        let ids = unique_nonempty(request_ids);
        if ids.is_empty() {
            return Vec::new();
        }
        let query = SpendByRequestIdsQuery {
            scope: scope.clone(),
            request_ids: ids,
            start_ms: start_ms.saturating_sub(SPEND_WINDOW_MS),
            end_ms: end_ms.saturating_add(SPEND_WINDOW_MS),
        };
        match self.storage.spend_by_request_ids(query).await {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(error = %error, "trace spend lookup unavailable");
                Vec::new()
            }
        }
    }

    async fn list_spend_rows(
        &self,
        scope: &TraceScope,
        rows: &[TraceListRow],
        start_ms: i64,
        end_ms: i64,
    ) -> Vec<SpendRow> {
        let request_ids: Vec<String> = rows
            .iter()
            .flat_map(|row| row.request_ids.iter().cloned())
            .collect();
        let (window_start, window_end) = list_window(rows, start_ms, end_ms);
        self.spend_rows(scope, &request_ids, window_start, window_end)
            .await
    }

    pub async fn list_traces(
        &self,
        scope: &TraceScope,
        time_range: (i64, i64),
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<TracePage, StorageError<S::Error>> {
        let (start_ms, end_ms) = time_range;
        let (cursor_ms, cursor_trace_id) = decode_trace_cursor(cursor)?;
        let query = ListTracesQuery {
            scope: scope.clone(),
            start_ms,
            end_ms,
            cursor_ms,
            cursor_trace_id,
            limit,
        };
        let rows = self
            .storage
            .list_traces(query)
            .await
            .map_err(StorageError::Query)?;
        let spend_rows = self.list_spend_rows(scope, &rows, start_ms, end_ms).await;
        let data = rows
            .iter()
            .map(|row| trace_summary_from_row(row, &spend_rows))
            .collect();
        let next_cursor = (rows.len() == limit)
            .then(|| rows.last())
            .flatten()
            .map(|row| encode_trace_cursor(row.start_ms, &row.trace_id));
        Ok(TracePage { data, next_cursor })
    }

    pub async fn get_trace(
        &self,
        trace_id: &str,
        scope: &TraceScope,
    ) -> Result<Option<Trace>, StorageError<S::Error>> {
        let query = TraceSpansQuery {
            scope: scope.clone(),
            trace_id: trace_id.to_owned(),
        };
        let rows = self
            .storage
            .trace_spans(query)
            .await
            .map_err(StorageError::Query)?;
        if rows.is_empty() {
            return Ok(None);
        }
        let trace_start_ns = rows.iter().map(|row| row.start_ns).min().unwrap_or(0);
        let trace_end_ns = rows
            .iter()
            .map(|row| row.start_ns.saturating_add(row.duration_ns))
            .max()
            .unwrap_or(0);
        let request_ids: Vec<String> = rows
            .iter()
            .filter_map(|row| row.litellm_request_id.clone())
            .collect();
        let start_ms = trace_start_ns.checked_div(NANOS_PER_MS).unwrap_or(0);
        let end_ms = trace_end_ns.checked_div(NANOS_PER_MS).unwrap_or(0);
        let spend_rows = self.spend_rows(scope, &request_ids, start_ms, end_ms).await;
        Ok(trace_from_rows(trace_id, &rows, &spend_rows))
    }

    pub async fn get_span(
        &self,
        trace_id: &str,
        span_id: &str,
        scope: &TraceScope,
    ) -> Result<Option<SpanDetail>, StorageError<S::Error>> {
        let query = SpanDetailQuery {
            scope: scope.clone(),
            trace_id: trace_id.to_owned(),
            span_id: span_id.to_owned(),
        };
        Ok(first_of(self.storage.span_detail(query).await)?.map(span_detail_from_row))
    }

    pub async fn get_span_error(
        &self,
        trace_id: &str,
        span_id: &str,
        scope: &TraceScope,
        cursor: Option<&str>,
    ) -> Result<Option<SpanErrorPage>, StorageError<S::Error>> {
        let position = decode_error_cursor(cursor)?;
        let query = span_error_query(trace_id, span_id, scope, position.as_ref());
        let Some(row) = first_of(self.storage.span_error(query).await)? else {
            return Ok(None);
        };
        Ok(Some(error_page(row, position.as_ref())))
    }
}
