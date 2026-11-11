#![cfg(test)]

use betterlite_models::decisions::Json;

use crate::datadog_llm_metrics::tests::support;

#[test]
fn base_metrics_carry_tokens_cost_and_latency() {
    let payload = support::payload();
    let object = support::metrics(&payload, None);
    support::assert_fields(
        &object,
        r#"{"input_tokens": 4.0, "output_tokens": 6.0, "total_tokens": 10.0, "total_cost": 0.5, "time_to_first_token": 0.5}"#,
    );
    support::assert_all_absent(
        &object,
        &[
            "cache_read_input_tokens",
            "cache_write_input_tokens",
            "non_cached_input_tokens",
            "reasoning_output_tokens",
            "tool_output_tokens",
        ],
    );
}

#[test]
fn cache_read_tokens_subtract_from_the_residual() {
    let mut payload = support::payload();
    payload.metadata.usage_object = Some(support::json(r#"{"cache_read_input_tokens": 2}"#));
    let object = support::metrics(&payload, None);
    support::assert_fields(
        &object,
        r#"{"cache_read_input_tokens": 2.0, "non_cached_input_tokens": 2.0}"#,
    );
    support::assert_absent(&object, "cache_write_input_tokens");
}

#[test]
fn cache_write_tokens_subtract_from_the_residual() {
    let mut payload = support::payload();
    payload.metadata.usage_object = Some(support::json(r#"{"cache_creation_input_tokens": 3}"#));
    let object = support::metrics(&payload, None);
    support::assert_fields(
        &object,
        r#"{"cache_write_input_tokens": 3.0, "non_cached_input_tokens": 1.0}"#,
    );
    support::assert_absent(&object, "cache_read_input_tokens");
}

#[test]
fn both_cache_counts_subtract_from_the_residual() {
    let mut payload = support::payload();
    payload.metadata.usage_object = Some(support::json(
        r#"{"cache_read_input_tokens": 2, "cache_creation_input_tokens": 1}"#,
    ));
    let object = support::metrics(&payload, None);
    support::assert_fields(
        &object,
        r#"{"cache_read_input_tokens": 2.0, "cache_write_input_tokens": 1.0, "non_cached_input_tokens": 1.0}"#,
    );
}

#[test]
fn a_fully_cached_request_keeps_a_zero_residual() {
    let mut payload = support::payload();
    payload.metadata.usage_object = Some(support::json(r#"{"cache_read_input_tokens": 4}"#));
    let object = support::metrics(&payload, None);
    support::assert_fields(
        &object,
        r#"{"cache_read_input_tokens": 4.0, "non_cached_input_tokens": 0.0}"#,
    );
}

#[test]
fn cache_read_beyond_the_prompt_clamps_the_residual_at_zero() {
    let mut payload = support::payload();
    payload.metadata.usage_object = Some(support::json(r#"{"cache_read_input_tokens": 9}"#));
    let object = support::metrics(&payload, None);
    support::assert_fields(
        &object,
        r#"{"cache_read_input_tokens": 9.0, "non_cached_input_tokens": 0.0}"#,
    );
}

#[test]
fn zero_cache_counts_are_absent_from_the_span() {
    let mut payload = support::payload();
    payload.metadata.usage_object = Some(support::json(
        r#"{"cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}"#,
    ));
    let object = support::metrics(&payload, None);
    support::assert_all_absent(
        &object,
        &[
            "cache_read_input_tokens",
            "cache_write_input_tokens",
            "non_cached_input_tokens",
        ],
    );
}

#[test]
fn the_openai_cached_tokens_detail_is_read_as_cache_read() {
    let mut payload = support::payload();
    payload.metadata.usage_object = Some(support::json(
        r#"{"prompt_tokens_details": {"cached_tokens": 2}}"#,
    ));
    let object = support::metrics(&payload, None);
    support::assert_fields(
        &object,
        r#"{"cache_read_input_tokens": 2.0, "non_cached_input_tokens": 2.0}"#,
    );
}

#[test]
fn reasoning_tokens_are_read_from_the_completion_details() {
    let mut payload = support::payload();
    payload.metadata.usage_object = Some(support::json(
        r#"{"completion_tokens_details": {"reasoning_tokens": 3}}"#,
    ));
    let object = support::metrics(&payload, None);
    support::assert_fields(&object, r#"{"reasoning_output_tokens": 3.0}"#);
}

#[test]
fn reasoning_tokens_fall_back_to_the_output_details() {
    let mut payload = support::payload();
    payload.metadata.usage_object = Some(support::json(
        r#"{"output_tokens_details": {"reasoning_tokens": 7}}"#,
    ));
    let object = support::metrics(&payload, None);
    support::assert_fields(&object, r#"{"reasoning_output_tokens": 7.0}"#);
}

#[test]
fn a_zero_completion_reasoning_count_stops_the_output_fallback() {
    let mut payload = support::payload();
    payload.metadata.usage_object = Some(support::json(
        r#"{"completion_tokens_details": {"reasoning_tokens": 0}, "output_tokens_details": {"reasoning_tokens": 7}}"#,
    ));
    let object = support::metrics(&payload, None);
    support::assert_absent(&object, "reasoning_output_tokens");
}

#[test]
fn a_non_numeric_completion_reasoning_count_falls_back_to_the_output_details() {
    let mut payload = support::payload();
    payload.metadata.usage_object = Some(support::json(
        r#"{"completion_tokens_details": {"reasoning_tokens": "3"}, "output_tokens_details": {"reasoning_tokens": 7}}"#,
    ));
    let object = support::metrics(&payload, None);
    support::assert_fields(&object, r#"{"reasoning_output_tokens": 7.0}"#);
}

#[test]
fn a_boolean_reasoning_count_is_not_a_token_count() {
    let mut payload = support::payload();
    payload.metadata.usage_object = Some(support::json(
        r#"{"completion_tokens_details": {"reasoning_tokens": true}}"#,
    ));
    let object = support::metrics(&payload, None);
    support::assert_absent(&object, "reasoning_output_tokens");
}

#[test]
fn a_negative_reasoning_count_is_kept() {
    let mut payload = support::payload();
    payload.metadata.usage_object = Some(support::json(
        r#"{"completion_tokens_details": {"reasoning_tokens": -3}}"#,
    ));
    let object = support::metrics(&payload, None);
    support::assert_fields(&object, r#"{"reasoning_output_tokens": -3.0}"#);
}

#[test]
fn a_zero_tool_output_count_stays_on_the_span() {
    let payload = support::payload();
    let object = support::metrics(&payload, Some(0.0_f64));
    support::assert_fields(&object, r#"{"tool_output_tokens": 0.0}"#);
}

#[test]
fn a_counted_tool_output_total_stays_on_the_span() {
    let payload = support::payload();
    let object = support::metrics(&payload, Some(12.0_f64));
    support::assert_fields(&object, r#"{"tool_output_tokens": 12.0}"#);
}

#[test]
fn an_absent_tool_output_count_is_omitted() {
    let payload = support::payload();
    let object = support::metrics(&payload, None);
    support::assert_absent(&object, "tool_output_tokens");
}

#[test]
fn a_non_object_usage_object_adds_no_cache_or_reasoning_counts() {
    let mut payload = support::payload();
    payload.metadata.usage_object = Some(Json::Text("not an object".to_owned()));
    let object = support::metrics(&payload, None);
    support::assert_all_absent(
        &object,
        &[
            "cache_read_input_tokens",
            "cache_write_input_tokens",
            "non_cached_input_tokens",
            "reasoning_output_tokens",
        ],
    );
}

#[test]
fn a_usage_object_without_cache_or_reasoning_adds_nothing() {
    let mut payload = support::payload();
    payload.metadata.usage_object = Some(support::json(r#"{"prompt_tokens": 4}"#));
    let object = support::metrics(&payload, None);
    support::assert_all_absent(
        &object,
        &[
            "cache_read_input_tokens",
            "cache_write_input_tokens",
            "non_cached_input_tokens",
            "reasoning_output_tokens",
        ],
    );
}
