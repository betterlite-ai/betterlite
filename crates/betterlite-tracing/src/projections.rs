use std::collections::BTreeSet;

use betterlite_logging::trace_types::{AgentNode, Span, SpanStatus, SpanType, Trace, TraceSummary};
use chrono::{DateTime, SecondsFormat, Utc};
use rust_decimal::Decimal;

use crate::agents::agent_nodes;
use crate::spend::{SpendRow, spend_for, trace_spend};
use crate::storage::rows::{SpanQueryRow, TraceListRow};

mod tests;

pub const NANOS_PER_MS: i64 = 1_000_000;

#[must_use]
pub fn status_from_code(code: &str) -> SpanStatus {
    match code {
        "STATUS_CODE_OK" => SpanStatus::Ok,
        "STATUS_CODE_ERROR" => SpanStatus::Error,
        _ => SpanStatus::Unset,
    }
}

fn iso_from_ms(start_ms: i64) -> String {
    let time =
        DateTime::<Utc>::from_timestamp_millis(start_ms).unwrap_or(DateTime::<Utc>::UNIX_EPOCH);
    // Python's isoformat shows microseconds only when nonzero; ms-precision input is always a multiple of 1000us.
    let precision = if time.timestamp_subsec_micros() == 0 {
        SecondsFormat::Secs
    } else {
        SecondsFormat::Micros
    };
    time.to_rfc3339_opts(precision, false)
}

fn span_spend(row: &SpanQueryRow, spend_rows: &[SpendRow]) -> (Option<String>, Option<Decimal>) {
    let litellm_request_id = row.litellm_request_id.clone();
    let spend = litellm_request_id
        .as_deref()
        .and_then(|request_id| spend_for(request_id, &row.team_id, &row.api_key_hash, spend_rows));
    (litellm_request_id, spend)
}

#[must_use]
pub fn span_from_row(row: &SpanQueryRow, trace_start_ns: i64, spend_rows: &[SpendRow]) -> Span {
    let (litellm_request_id, spend) = span_spend(row, spend_rows);
    Span {
        span_id: row.span_id.clone(),
        parent_span_id: row.parent_span_id.clone(),
        name: row.name.clone(),
        span_type: row.span_type,
        agent: row.agent.clone(),
        start_offset_ms: Decimal::from(row.start_ns.saturating_sub(trace_start_ns))
            .checked_div(Decimal::from(NANOS_PER_MS))
            .unwrap_or_default(),
        duration_ms: Decimal::from(row.duration_ns)
            .checked_div(Decimal::from(NANOS_PER_MS))
            .unwrap_or_default(),
        status: status_from_code(&row.status),
        error: row.status_message.clone(),
        error_truncated: row.error_truncated,
        input_preview: row.input_preview.clone(),
        model: row.model.clone(),
        input_tokens: row.input_tokens,
        output_tokens: row.output_tokens,
        litellm_request_id,
        spend,
    }
}

#[must_use]
pub fn trace_summary_from_row(row: &TraceListRow, spend_rows: &[SpendRow]) -> TraceSummary {
    let spend = trace_spend(
        &row.request_ids,
        &row.team_id,
        &row.api_key_hash,
        spend_rows,
    );
    TraceSummary {
        trace_id: row.trace_id.clone(),
        trace_ref: Some(row.trace_ref.clone()),
        name: row.name.clone(),
        service: row.service.clone(),
        input_preview: row.input_preview.clone(),
        start_time: iso_from_ms(row.start_ms),
        duration_ms: Decimal::from(row.duration_ms),
        status: status_from_code(&row.status),
        span_count: row.span_count,
        agent_count: row.agent_count,
        agent_invocations: row
            .agent_invocations
            .filter(|count| *count != 0)
            .unwrap_or(row.agent_count),
        llm_calls: row.llm_calls,
        tool_calls: row.tool_calls,
        error_count: row.error_count.unwrap_or(0),
        input_tokens: row.input_tokens,
        output_tokens: row.output_tokens,
        models: row.models.clone(),
        spend,
    }
}

fn token_totals(spans: &[Span]) -> (i64, i64) {
    let input_tokens = spans
        .iter()
        .fold(0_i64, |total, span| total.saturating_add(span.input_tokens));
    let output_tokens = spans.iter().fold(0_i64, |total, span| {
        total.saturating_add(span.output_tokens)
    });
    (input_tokens, output_tokens)
}

fn distinct_models(spans: &[Span]) -> Vec<String> {
    let models: BTreeSet<String> = spans
        .iter()
        .filter(|span| span.span_type == SpanType::Llm)
        .filter_map(|span| span.model.clone())
        .filter(|model| !model.is_empty())
        .collect();
    models.into_iter().collect()
}

struct TraceRowContext {
    service: String,
    team_id: String,
    api_key_hash: String,
    request_ids: Vec<String>,
}

fn trace_row_context(rows: &[SpanQueryRow]) -> TraceRowContext {
    let first = rows.first();
    TraceRowContext {
        service: first.map(|row| row.service.clone()).unwrap_or_default(),
        team_id: first.map(|row| row.team_id.clone()).unwrap_or_default(),
        api_key_hash: first
            .map(|row| row.api_key_hash.clone())
            .unwrap_or_default(),
        request_ids: rows
            .iter()
            .filter_map(|row| row.litellm_request_id.clone())
            .collect(),
    }
}

struct TraceBounds {
    start_ns: i64,
    end_ns: i64,
}

fn trace_bounds(rows: &[SpanQueryRow]) -> Option<TraceBounds> {
    let start_ns = rows.iter().map(|row| row.start_ns).min()?;
    let end_ns = rows
        .iter()
        .map(|row| row.start_ns.saturating_add(row.duration_ns))
        .max()?;
    Some(TraceBounds { start_ns, end_ns })
}

struct TraceSummaryContext<'ctx> {
    trace_id: &'ctx str,
    trace_ref: &'ctx str,
    rows: &'ctx TraceRowContext,
    bounds: &'ctx TraceBounds,
    spend_rows: &'ctx [SpendRow],
}

fn summary_time_bounds_ms(bounds: &TraceBounds) -> (i64, i64) {
    let start_ms = bounds.start_ns.checked_div(NANOS_PER_MS).unwrap_or(0);
    let duration_ms = bounds.end_ns.saturating_sub(bounds.start_ns);
    (start_ms, duration_ms)
}

struct SummaryCounts {
    span_count: i64,
    agent_count: i64,
    agent_invocations: i64,
    llm_calls: i64,
    tool_calls: i64,
    error_count: i64,
}

fn summary_counts(spans: &[Span], agents: &[AgentNode]) -> SummaryCounts {
    let agent_invocations = agents
        .iter()
        .map(|agent| agent.invocations)
        .fold(0_i64, i64::saturating_add);
    let llm_calls = i64::try_from(
        spans
            .iter()
            .filter(|span| span.span_type == SpanType::Llm)
            .count(),
    )
    .unwrap_or(i64::MAX);
    let tool_calls = i64::try_from(
        spans
            .iter()
            .filter(|span| span.span_type == SpanType::Tool)
            .count(),
    )
    .unwrap_or(i64::MAX);
    let error_count = i64::try_from(
        spans
            .iter()
            .filter(|span| span.status == SpanStatus::Error)
            .count(),
    )
    .unwrap_or(i64::MAX);
    SummaryCounts {
        span_count: i64::try_from(spans.len()).unwrap_or(i64::MAX),
        agent_count: i64::try_from(agents.len()).unwrap_or(i64::MAX),
        agent_invocations,
        llm_calls,
        tool_calls,
        error_count,
    }
}

fn trace_summary(
    spans: &[Span],
    agents: &[AgentNode],
    context: &TraceSummaryContext<'_>,
) -> TraceSummary {
    let (input_tokens, output_tokens) = token_totals(spans);
    let counts = summary_counts(spans, agents);
    let (start_ms, duration_ms) = summary_time_bounds_ms(context.bounds);
    let root = spans
        .iter()
        .find(|span| span.parent_span_id.is_none())
        .or_else(|| spans.first());
    let spend = trace_spend(
        &context.rows.request_ids,
        &context.rows.team_id,
        &context.rows.api_key_hash,
        context.spend_rows,
    );
    TraceSummary {
        trace_id: context.trace_id.to_owned(),
        trace_ref: Some(context.trace_ref.to_owned()),
        name: root.map(|span| span.name.clone()).unwrap_or_default(),
        service: context.rows.service.clone(),
        input_preview: root
            .map(|span| span.input_preview.clone())
            .unwrap_or_default(),
        start_time: iso_from_ms(start_ms),
        duration_ms: Decimal::from(duration_ms)
            .checked_div(Decimal::from(NANOS_PER_MS))
            .unwrap_or_default(),
        status: root.map_or(SpanStatus::Unset, |span| span.status),
        span_count: counts.span_count,
        agent_count: counts.agent_count,
        agent_invocations: counts.agent_invocations,
        llm_calls: counts.llm_calls,
        tool_calls: counts.tool_calls,
        error_count: counts.error_count,
        input_tokens,
        output_tokens,
        models: distinct_models(spans),
        spend,
    }
}

#[must_use]
pub fn trace_from_rows(
    trace_id: &str,
    rows: &[SpanQueryRow],
    trace_ref: &str,
    spend_rows: &[SpendRow],
) -> Option<Trace> {
    if rows.is_empty() {
        return None;
    }
    let bounds = trace_bounds(rows)?;
    let spans: Vec<Span> = rows
        .iter()
        .map(|row| span_from_row(row, bounds.start_ns, spend_rows))
        .collect();
    let agents = agent_nodes(&spans);
    let row_context = trace_row_context(rows);
    let summary_context = TraceSummaryContext {
        trace_id,
        trace_ref,
        rows: &row_context,
        bounds: &bounds,
        spend_rows,
    };
    let summary = trace_summary(&spans, &agents, &summary_context);
    Some(Trace {
        summary,
        agents,
        spans,
    })
}
