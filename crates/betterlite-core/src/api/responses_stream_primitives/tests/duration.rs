use indexmap::IndexMap;

use crate::api::responses_stream_primitives::duration::STREAM_TIMEOUT_STATUS;
use crate::api::responses_stream_primitives::state::{
    BaseStreamStateInput, ResponsesStreamBaseState, base_stream_state,
};

use super::support::{NoApiBase, snapshot, times};

fn state_with_limit(
    model: &str,
    provider: Option<&str>,
    limit: Option<f64>,
) -> ResponsesStreamBaseState {
    let headers = IndexMap::new();
    base_stream_state(&BaseStreamStateInput {
        model,
        custom_llm_provider: provider,
        litellm_metadata: None,
        request_data: None,
        call_type: None,
        logging: snapshot(IndexMap::new()),
        headers: &headers,
        api_base: &NoApiBase,
        times: times(),
        max_streaming_duration_seconds: limit,
    })
}

#[test]
fn an_unset_limit_never_times_out() {
    let state = state_with_limit("gpt-4o", None, None);
    assert!(state.check_max_streaming_duration(1e12).is_ok());
}

#[test]
fn a_limit_equal_to_the_elapsed_time_still_streams() {
    let state = state_with_limit("gpt-4o", None, Some(2.5));
    assert!(state.check_max_streaming_duration(2.5).is_ok());
}

#[test]
fn a_limit_over_the_elapsed_time_times_out() {
    let state = state_with_limit("gpt-4o", None, Some(2.5));
    let timeout = state
        .check_max_streaming_duration(2.6)
        .expect_err("the stream ran past the limit");
    assert_eq!(timeout.status_code, STREAM_TIMEOUT_STATUS);
    assert_eq!(timeout.status_code, 408);
    assert_eq!(
        timeout.message,
        "litellm.Timeout: Stream exceeded max streaming duration of 2.5s (elapsed 2.6s)"
    );
    assert_eq!(timeout.model, "gpt-4o");
    assert_eq!(timeout.custom_llm_provider, "");
}

#[test]
fn a_timeout_keeps_the_model_and_provider() {
    let state = state_with_limit("claude-3", Some("anthropic"), Some(1.0));
    let timeout = state
        .check_max_streaming_duration(1.5)
        .expect_err("the stream ran past the limit");
    assert_eq!(timeout.model, "claude-3");
    assert_eq!(timeout.custom_llm_provider, "anthropic");
    assert_eq!(
        timeout.message,
        "litellm.Timeout: Stream exceeded max streaming duration of 1.0s (elapsed 1.5s)"
    );
}

#[test]
fn a_zero_limit_allows_a_zero_elapsed_time() {
    let state = state_with_limit("gpt-4o", None, Some(0.0));
    assert!(state.check_max_streaming_duration(0.0).is_ok());
    let timeout = state
        .check_max_streaming_duration(0.1)
        .expect_err("any elapsed time exceeds zero");
    assert_eq!(timeout.status_code, 408);
}

#[test]
fn a_negative_limit_fails_at_a_zero_elapsed_time() {
    let state = state_with_limit("gpt-4o", None, Some(-1.0));
    let timeout = state
        .check_max_streaming_duration(0.0)
        .expect_err("zero is greater than a negative limit");
    assert_eq!(
        timeout.message,
        "litellm.Timeout: Stream exceeded max streaming duration of -1.0s (elapsed 0.0s)"
    );
}

#[test]
fn a_nan_limit_never_times_out() {
    let state = state_with_limit("gpt-4o", None, Some(f64::NAN));
    assert!(state.check_max_streaming_duration(1e12).is_ok());
}

#[rstest::rstest]
#[case::positive_infinity(f64::INFINITY)]
#[case::negative_infinity(f64::NEG_INFINITY)]
fn an_infinite_limit_compares_like_python(#[case] limit: f64) {
    let state = state_with_limit("gpt-4o", None, Some(limit));
    let checked = state.check_max_streaming_duration(1.0);
    assert_eq!(checked.is_err(), limit.is_sign_negative());
}
