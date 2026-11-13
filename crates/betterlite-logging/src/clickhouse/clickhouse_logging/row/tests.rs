#![cfg(test)]

use rust_decimal::Decimal;

use super::{Projection, spend_log_row_from_payload};
use crate::clickhouse::clickhouse_logging::context::LensAnalysis;
use crate::clickhouse::clickhouse_logging::payload::CallbackKwargs;
use crate::clickhouse::clickhouse_logging::tests::support::kwargs;
use crate::trace_types::SpendLogRecord;

const TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
const SPAN_ID: &str = "00f067aa0be902b7";

fn full_payload() -> CallbackKwargs {
    kwargs(
        r#"{"standard_logging_object": {
            "id": "chatcmpl-1_cache_hit1712345678.5",
            "trace_id": "trace-from-payload",
            "session_id": "session-1",
            "call_type": "acompletion",
            "response_cost": 0.0123,
            "status": "success",
            "custom_llm_provider": "openai",
            "prompt_tokens": 10,
            "completion_tokens": 4,
            "total_tokens": 14,
            "startTime": 1700000000.5,
            "endTime": 1700000001.25,
            "completionStartTime": 1700000000.75,
            "model": "gpt-4o",
            "model_id": "model-1",
            "model_group": "gpt-4o",
            "api_base": "https://api.openai.com",
            "cache_hit": true,
            "request_tags": ["team-a", 2],
            "end_user": "user-1",
            "error_str": "",
            "metadata": {
                "user_api_key_hash": "hash",
                "user_api_key_alias": "alias",
                "user_api_key_team_id": "team",
                "user_api_key_team_alias": "team alias",
                "user_api_key_org_id": "org",
                "user_api_key_user_id": "user",
                "user_api_key_end_user_id": "end",
                "requester_custom_headers": {"TraceParent": "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0be902b7-01"},
                "usage_object": {"prompt_tokens_details": {"cached_tokens": 7, "cache_creation_tokens": 3}}
            },
            "hidden_params": {},
            "messages": [{"role": "user", "content": "hi"}],
            "response": {"choices": []}
        }}"#,
    )
}

fn row_of(event: &CallbackKwargs, message_logging_disabled: bool) -> SpendLogRecord {
    let lens = LensAnalysis::default();
    spend_log_row_from_payload(Projection {
        payload: event
            .standard_logging_object
            .as_ref()
            .expect("the fixture carries a payload"),
        kwargs: event,
        message_logging_disabled,
        lens: &lens,
    })
}

#[test]
fn projects_the_request_and_model_identity_fields() {
    let event = full_payload();
    let row = row_of(&event, false);
    assert_eq!(row.request_id, "chatcmpl-1_cache_hit1712345678.5");
    assert_eq!(row.response_id, "chatcmpl-1");
    assert_eq!(row.call_type, "acompletion");
    assert_eq!(row.model, "gpt-4o");
    assert_eq!(row.model_group, "gpt-4o");
    assert_eq!(row.model_id, "model-1");
    assert_eq!(row.custom_llm_provider, "openai");
    assert_eq!(row.api_base, "https://api.openai.com");
    assert_eq!(row.status, "success");
    assert_eq!(row.error_str, "");
    assert!(row.cache_hit);
}

#[test]
fn projects_the_key_team_and_user_fields() {
    let event = full_payload();
    let row = row_of(&event, false);
    assert_eq!(row.api_key, "hash");
    assert_eq!(row.key_alias, "alias");
    assert_eq!(row.team_id, "team");
    assert_eq!(row.team_alias, "team alias");
    assert_eq!(row.organization_id, "org");
    assert_eq!(row.user, "user");
    assert_eq!(row.end_user, "user-1");
    assert_eq!(row.session_id, "session-1");
}

#[test]
fn projects_the_usage_and_spend_counters() {
    let event = full_payload();
    let row = row_of(&event, false);
    assert_eq!(row.spend, Decimal::new(123, 4));
    assert_eq!(row.prompt_tokens, 10);
    assert_eq!(row.completion_tokens, 4);
    assert_eq!(row.total_tokens, 14);
    assert_eq!(row.cache_read_tokens, 7);
    assert_eq!(row.cache_write_tokens, 3);
}

#[test]
fn projects_the_timing_trace_and_tag_fields() {
    let event = full_payload();
    let row = row_of(&event, false);
    assert_eq!(row.start_time, 1_700_000_000_500);
    assert_eq!(row.end_time, 1_700_000_001_250);
    assert_eq!(row.completion_start_time, Some(1_700_000_000_750));
    assert_eq!(row.trace_id, TRACE_ID);
    assert_eq!(row.span_id, SPAN_ID);
    assert_eq!(row.request_tags, vec!["team-a".to_owned(), "2".to_owned()]);
}

#[test]
fn the_row_metadata_carries_the_lens_flag() {
    let event = full_payload();
    let row = row_of(&event, false);
    assert!(
        row.metadata.contains(r#""litellm_lens_internal":false"#),
        "{}",
        row.metadata
    );
    assert!(row.metadata.contains(r#""user_api_key_hash":"hash""#));
}

#[test]
fn the_lens_analysis_scope_marks_the_row_metadata() {
    let event = full_payload();
    let lens = LensAnalysis::default();
    let scope = lens.lens_analysis();
    let projection = Projection {
        payload: event
            .standard_logging_object
            .as_ref()
            .expect("the fixture carries a payload"),
        kwargs: &event,
        message_logging_disabled: false,
        lens: &lens,
    };
    let inside = spend_log_row_from_payload(projection);
    drop(scope);
    assert!(
        inside.metadata.contains(r#""litellm_lens_internal":true"#),
        "{}",
        inside.metadata
    );
    assert!(
        row_of(&event, false)
            .metadata
            .contains(r#""litellm_lens_internal":false"#)
    );
}

#[test]
fn messages_and_response_are_blank_when_message_logging_is_disabled() {
    let event = full_payload();
    let row = row_of(&event, true);
    assert_eq!(row.messages, "");
    assert_eq!(row.response, "");
    let kept = row_of(&event, false);
    assert_eq!(kept.messages, r#"[{"role":"user","content":"hi"}]"#);
    assert_eq!(kept.response, r#"{"choices":[]}"#);
}

#[test]
fn a_zero_completion_start_time_is_none() {
    let event = kwargs(
        r#"{"standard_logging_object": {"completionStartTime": 0.0004, "startTime": 0.0004}}"#,
    );
    let row = row_of(&event, false);
    assert_eq!(row.completion_start_time, None);
    assert_eq!(row.start_time, 0);
}

#[test]
fn hidden_params_usage_is_the_fallback() {
    let event = kwargs(
        r#"{"standard_logging_object": {"hidden_params": {"usage_object": {"cache_read_input_tokens": 5}}}}"#,
    );
    let row = row_of(&event, false);
    assert_eq!(row.cache_read_tokens, 5);
}

#[test]
fn team_and_end_user_fall_back_to_the_metadata_keys() {
    let event = kwargs(
        r#"{"standard_logging_object": {
            "end_user": "",
            "metadata": {"team_id": "team-b", "team_alias": "alias-b", "user_api_key_end_user_id": "end-b"}
        }}"#,
    );
    let row = row_of(&event, false);
    assert_eq!(row.team_id, "team-b");
    assert_eq!(row.team_alias, "alias-b");
    assert_eq!(row.end_user, "end-b");
}

#[test]
fn an_empty_payload_projects_a_row_of_defaults() {
    let event = kwargs(r#"{"standard_logging_object": {}}"#);
    let row = row_of(&event, false);
    assert_eq!(row.request_id, "");
    assert_eq!(row.response_id, "");
    assert_eq!(row.spend, Decimal::ZERO);
    assert_eq!(row.start_time, 0);
    assert_eq!(row.completion_start_time, None);
    assert!(!row.cache_hit);
    assert_eq!(row.session_id, "");
    assert_eq!(row.trace_id, "");
    assert_eq!(row.metadata, r#"{"litellm_lens_internal":false}"#);
}

#[test]
fn spend_serializes_as_a_json_number() {
    let event = full_payload();
    let row = row_of(&event, false);
    let serialized = serde_json::to_string(&row).expect("the row serializes");
    assert!(serialized.contains(r#""spend":0.0123"#), "{serialized}");
}
