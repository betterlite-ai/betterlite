use rstest::rstest;

use super::support::{build_line, built, json, options, stamp};
use crate::spend_event::error::SpendEventBuildError;
use crate::spend_event::event::{DynamicParams, EventVersion};
use crate::spend_event::input::CachePresence;

const SCALARS: &str = r#"{
    "litellm_call_id": "call-1", "call_type": "acompletion", "model": "gpt-4",
    "custom_llm_provider": "openai", "stream": true, "cache_hit": false,
    "completion_start_time": "2026-01-01T00:00:00.5+02:00", "agent_id": "agent-1",
    "litellm_trace_id": "trace-1",
    "standard_callback_dynamic_params": {"turn_off_message_logging": true},
    "realtime_tools": [{"type": "function"}], "realtime_tool_calls": [{"id": "rt-1"}],
    "combined_usage_object": {"prompt_tokens": 3, "completion_tokens": 4, "total_tokens": 7,
        "completion_tokens_details": null, "prompt_tokens_details": null},
    "litellm_params": {"api_base": "https://api", "custom_llm_provider": "openai",
        "litellm_call_id": "call-1", "user_api_key_end_user_id": "end-user"}
}"#;

#[test]
fn emits_one_newline_terminated_line_with_every_required_key() {
    let line = build_line(SCALARS, &options(false, CachePresence::On)).unwrap();
    assert!(line.ends_with(b"\n"));
    assert_eq!(line.split(|byte| *byte == b'\n').count(), 2);
    let text = String::from_utf8(line).unwrap();
    assert!(text.contains(r#""start_time":"2026-01-01T00:00:00Z""#));
    assert!(text.contains(r#""completion_start_time":"2026-01-01T00:00:00.500000+02:00""#));
    assert!(text.starts_with(r#"{"version":1,"#));
    let event = built(SCALARS, &options(false, CachePresence::On));
    assert_eq!(event.version, EventVersion::One);
    assert_eq!(event.start_time, stamp("2026-01-01T00:00:00Z"));
    assert_eq!(event.end_time, stamp("2026-01-01T00:00:01Z"));
    assert_eq!(event.litellm_call_id.as_deref(), Some("call-1"));
    assert_eq!(event.call_type.as_deref(), Some("acompletion"));
    assert_eq!(event.model.as_deref(), Some("gpt-4"));
    assert_eq!(event.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(event.stream, Some(true));
    assert_eq!(event.cache_hit, Some(false));
    assert_eq!(event.agent_id.as_deref(), Some("agent-1"));
    assert_eq!(event.litellm_trace_id.as_deref(), Some("trace-1"));
    assert_eq!(
        event.litellm_params.api_base.as_deref(),
        Some("https://api")
    );
    assert_eq!(
        event.litellm_params.user_api_key_end_user_id.as_deref(),
        Some("end-user")
    );
    assert_eq!(
        event.dynamic_params,
        Some(DynamicParams {
            turn_off_message_logging: Some(true)
        })
    );
    assert_eq!(
        event.realtime_tools,
        Some(vec![json(r#"{"type":"function"}"#)])
    );
    assert_eq!(
        event.realtime_tool_calls,
        Some(vec![json(r#"{"id":"rt-1"}"#)])
    );
    let usage = event.combined_usage.unwrap();
    assert_eq!(usage.get("total_tokens"), Some(&json("7")));
}

#[rstest]
#[case::missing_object_uses_kwargs(r#"{"response_cost": 0.25}"#, Some("0.25"))]
#[case::empty_object_blocks_the_fallback(
    r#"{"response_cost": 0.25, "standard_logging_object": {}}"#,
    None
)]
#[case::object_cost_wins(
    r#"{"response_cost": 0.25, "standard_logging_object": {"response_cost": 0.5}}"#,
    Some("0.5")
)]
#[case::true_counts_as_one(r#"{"standard_logging_object": {"response_cost": true}}"#, Some("1.0"))]
#[case::false_counts_as_zero(
    r#"{"standard_logging_object": {"response_cost": false}}"#,
    Some("0.0")
)]
#[case::whole_number_keeps_a_digit(
    r#"{"standard_logging_object": {"response_cost": 3}}"#,
    Some("3.0")
)]
#[case::exponent_is_a_decimal(
    r#"{"standard_logging_object": {"response_cost": 1e-7}}"#,
    Some("0.0000001")
)]
#[case::text_is_not_numeric(r#"{"standard_logging_object": {"response_cost": "0.5"}}"#, None)]
#[case::absent_cost_in_object(
    r#"{"response_cost": 0.25, "standard_logging_object": {"x": 1}}"#,
    None
)]
#[case::no_cost_anywhere("{}", None)]
fn selects_the_response_cost(#[case] kwargs: &str, #[case] expected: Option<&str>) {
    let event = built(kwargs, &options(false, CachePresence::On));
    let shown = event.response_cost.map(|cost| cost.to_string());
    assert_eq!(shown.as_deref(), expected);
}

#[rstest]
#[case::cache_off_literal(
    CachePresence::Off,
    r#"{"litellm_params": {"preset_cache_key": "key-1"}}"#,
    Some("Cache OFF")
)]
#[case::cache_on_preset(
    CachePresence::On,
    r#"{"litellm_params": {"preset_cache_key": "key-1"}}"#,
    Some("key-1")
)]
#[case::cache_on_without_preset(CachePresence::On, "{}", None)]
fn resolves_the_cache_key(
    #[case] cache: CachePresence,
    #[case] kwargs: &str,
    #[case] expected: Option<&str>,
) {
    let event = built(kwargs, &options(false, cache));
    assert_eq!(event.litellm_params.preset_cache_key.as_deref(), expected);
}

#[rstest]
#[case::absent("{}", false)]
#[case::null(r#"{"complete_streaming_response": null}"#, true)]
#[case::value(r#"{"complete_streaming_response": {"id": "x"}}"#, true)]
fn records_whether_the_stream_key_was_present(#[case] kwargs: &str, #[case] expected: bool) {
    let event = built(kwargs, &options(false, CachePresence::On));
    assert_eq!(event.complete_streaming_response, expected);
}

#[rstest]
#[case::wrong_scalar_type(r#"{"stream": "yes"}"#)]
#[case::null_params(r#"{"litellm_params": null}"#)]
#[case::bad_timestamp(r#"{"completion_start_time": "not a date"}"#)]
fn rejects_kwargs_that_fail_the_projection(#[case] kwargs: &str) {
    let result = build_line(kwargs, &options(false, CachePresence::On));
    assert!(matches!(result, Err(SpendEventBuildError::Kwargs { .. })));
}

#[rstest]
#[case::text_count(r#"{"combined_usage_object": {"prompt_tokens": "x"}}"#)]
#[case::count_past_64_bits(r#"{"combined_usage_object": {"total_tokens": 9223372036854775808}}"#)]
#[case::fractional_count(r#"{"combined_usage_object": {"completion_tokens": 1.5}}"#)]
fn rejects_usage_counts_that_do_not_fit(#[case] kwargs: &str) {
    let result = build_line(kwargs, &options(false, CachePresence::On));
    assert!(matches!(result, Err(SpendEventBuildError::Usage { .. })));
}

#[test]
fn rejects_a_cost_a_decimal_cannot_hold() {
    let kwargs = r#"{"standard_logging_object": {"response_cost": 100000000000000000000000000000000000000000}}"#;
    let result = build_line(kwargs, &options(false, CachePresence::On));
    assert!(matches!(
        result,
        Err(SpendEventBuildError::ResponseCost { .. })
    ));
}
