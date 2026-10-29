#![cfg(test)]

use betterlite_logging::trace_types::{SpanStatus, SpanType};
use rstest::rstest;
use rust_decimal::Decimal;

use super::{span_from_row, status_from_code, trace_from_rows, trace_summary_from_row};
use crate::storage::rows::{SpanQueryRow, TraceListRow};

fn unscoped_ids() -> (String, String) {
    (String::new(), String::new())
}

fn base_span_row() -> SpanQueryRow {
    let (team_id, api_key_hash) = unscoped_ids();
    SpanQueryRow {
        span_id: "span-1".to_owned(),
        parent_span_id: None,
        name: "root".to_owned(),
        span_type: SpanType::Agent,
        agent: "root".to_owned(),
        start_ns: 0,
        duration_ns: 0,
        status: String::new(),
        status_message: None,
        error_truncated: false,
        input_preview: String::new(),
        model: None,
        input_tokens: 0,
        output_tokens: 0,
        litellm_request_id: None,
        team_id,
        api_key_hash,
        service: "svc".to_owned(),
    }
}

fn base_list_row() -> TraceListRow {
    let (team_id, api_key_hash) = unscoped_ids();
    TraceListRow {
        trace_id: "trace-1".to_owned(),
        trace_ref: "ref-1".to_owned(),
        name: "root".to_owned(),
        service: "svc".to_owned(),
        input_preview: "hello".to_owned(),
        start_ms: 1_700_000_000_000,
        duration_ms: 42,
        status: "STATUS_CODE_OK".to_owned(),
        span_count: 3,
        agent_count: 1,
        agent_invocations: None,
        llm_calls: 1,
        tool_calls: 1,
        error_count: None,
        input_tokens: 10,
        output_tokens: 20,
        models: vec!["gpt-4".to_owned()],
        request_ids: Vec::new(),
        team_id,
        api_key_hash,
    }
}

#[rstest]
#[case::ok("STATUS_CODE_OK", SpanStatus::Ok)]
#[case::error("STATUS_CODE_ERROR", SpanStatus::Error)]
#[case::missing("", SpanStatus::Unset)]
#[case::unknown("STATUS_CODE_UNSET", SpanStatus::Unset)]
#[case::unrelated_text("whatever", SpanStatus::Unset)]
fn maps_status_codes(#[case] code: &str, #[case] expected: SpanStatus) {
    assert_eq!(status_from_code(code), expected);
}

#[test]
fn computes_the_span_offset_relative_to_the_trace_start() {
    let mut row = base_span_row();
    row.start_ns = 5_000_000;
    row.duration_ns = 2_000_000;
    let span = span_from_row(&row, 3_000_000, &[]);
    assert_eq!(span.start_offset_ms, Decimal::new(2, 0));
    assert_eq!(span.duration_ms, Decimal::new(2, 0));
}

#[test]
fn projects_a_trace_summary_from_a_list_row() {
    let row = base_list_row();
    let summary = trace_summary_from_row(&row, &[]);
    assert_eq!(summary.trace_id, "trace-1");
    assert_eq!(summary.trace_ref, Some("ref-1".to_owned()));
    assert_eq!(summary.status, SpanStatus::Ok);
    assert_eq!(summary.agent_invocations, row.agent_count);
    assert_eq!(summary.error_count, 0);
    assert_eq!(summary.spend, None);
}

#[test]
fn projects_a_trace_summary_falling_back_from_a_zero_agent_invocations_count() {
    let mut row = base_list_row();
    row.agent_invocations = Some(0);
    row.agent_count = 4;
    let summary = trace_summary_from_row(&row, &[]);
    assert_eq!(summary.agent_invocations, 4);
}

#[test]
fn trace_from_rows_is_none_for_an_empty_trace() {
    assert_eq!(trace_from_rows("trace-1", &[], "ref-1", &[]), None);
}

#[test]
fn trace_from_rows_aggregates_spans_into_a_trace() {
    let mut root = base_span_row();
    root.span_id = "root".to_owned();
    root.start_ns = 0;
    root.duration_ns = 10_000_000;
    root.status = "STATUS_CODE_OK".to_owned();

    let mut llm = base_span_row();
    llm.span_id = "llm".to_owned();
    llm.parent_span_id = Some("root".to_owned());
    llm.span_type = SpanType::Llm;
    llm.agent = "root".to_owned();
    llm.start_ns = 1_000_000;
    llm.duration_ns = 2_000_000;
    llm.model = Some("gpt-4".to_owned());
    llm.input_tokens = 5;
    llm.output_tokens = 7;
    llm.status = "STATUS_CODE_ERROR".to_owned();

    let mut tool = base_span_row();
    tool.span_id = "tool".to_owned();
    tool.parent_span_id = Some("root".to_owned());
    tool.span_type = SpanType::Tool;
    tool.agent = "root".to_owned();
    tool.start_ns = 4_000_000;
    tool.duration_ns = 1_000_000;

    let trace = trace_from_rows("trace-1", &[root, llm, tool], "ref-1", &[]).expect("trace");
    assert_eq!(trace.summary.trace_ref, Some("ref-1".to_owned()));
    assert_eq!(trace.summary.span_count, 3);
    assert_eq!(trace.summary.agent_count, 1);
    assert_eq!(trace.summary.llm_calls, 1);
    assert_eq!(trace.summary.tool_calls, 1);
    assert_eq!(trace.summary.error_count, 1);
    assert_eq!(trace.summary.input_tokens, 5);
    assert_eq!(trace.summary.output_tokens, 7);
    assert_eq!(trace.summary.models, vec!["gpt-4".to_owned()]);
    assert_eq!(trace.summary.duration_ms, Decimal::new(10, 0));
}
