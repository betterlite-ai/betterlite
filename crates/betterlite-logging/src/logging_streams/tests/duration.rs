use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{json, state, time};
use crate::logging_state::LoggingState;
use crate::logging_streams::duration::{
    CallDurationError, DURATION_KEY, TIMING_WINDOWS_KEY, record_call_duration,
};

fn record(state: &mut LoggingState, start: &str, end: &str) -> Result<(), CallDurationError> {
    record_call_duration(state, time(start), time(end))
}

fn windows(state: &LoggingState) -> Option<Json> {
    let Some(Json::Object(details)) = state.model_call_details.get("litellm_params") else {
        return None;
    };
    let Some(Json::Object(metadata)) = details.get("metadata") else {
        return None;
    };
    metadata.get(TIMING_WINDOWS_KEY).cloned()
}

fn test_records_the_duration_in_milliseconds() {
    let mut state = state(Some(true));
    record(
        &mut state,
        "2024-01-01T00:00:00+00:00",
        "2024-01-01T00:00:02+00:00",
    )
    .unwrap();
    assert_eq!(
        state.model_call_details.get(DURATION_KEY),
        Some(&json("2000"))
    );
}

fn test_records_a_fractional_duration() {
    let mut state = state(Some(true));
    record(
        &mut state,
        "2024-01-01T00:00:00+00:00",
        "2024-01-01T00:00:00.0005+00:00",
    )
    .unwrap();
    assert_eq!(
        state.model_call_details.get(DURATION_KEY),
        Some(&json("0.5"))
    );
}

fn test_records_a_negative_duration() {
    let mut state = state(Some(true));
    record(
        &mut state,
        "2024-01-01T00:00:01+00:00",
        "2024-01-01T00:00:00+00:00",
    )
    .unwrap();
    assert_eq!(
        state.model_call_details.get(DURATION_KEY),
        Some(&json("-1000"))
    );
}

fn test_writes_a_timing_window_into_metadata() {
    let mut state = state(Some(true));
    state.model_call_details.insert(
        "litellm_params".to_owned(),
        json(r#"{"metadata":{"request_retry_count":1}}"#),
    );
    record(
        &mut state,
        "2024-01-01T00:00:00+00:00",
        "2024-01-01T00:00:02+00:00",
    )
    .unwrap();
    assert_eq!(windows(&state), Some(json("[[1704067200,1704067202]]")));
}

fn test_ignores_an_empty_metadata_object() {
    let mut state = state(Some(true));
    state
        .model_call_details
        .insert("litellm_params".to_owned(), json(r#"{"metadata":{}}"#));
    record(
        &mut state,
        "2024-01-01T00:00:00+00:00",
        "2024-01-01T00:00:02+00:00",
    )
    .unwrap();
    assert_eq!(windows(&state), None);
    assert_eq!(
        state.model_call_details.get("litellm_params"),
        Some(&json(r#"{"metadata":{}}"#))
    );
}

fn test_ignores_two_empty_metadata_objects() {
    let mut state = state(Some(true));
    state.model_call_details.insert(
        "litellm_params".to_owned(),
        json(r#"{"metadata":{},"litellm_metadata":{}}"#),
    );
    record(
        &mut state,
        "2024-01-01T00:00:00+00:00",
        "2024-01-01T00:00:02+00:00",
    )
    .unwrap();
    assert_eq!(
        state.model_call_details.get("litellm_params"),
        Some(&json(r#"{"metadata":{},"litellm_metadata":{}}"#))
    );
}

fn test_writes_into_metadata_beside_an_empty_litellm_metadata() {
    let mut state = state(Some(true));
    state.model_call_details.insert(
        "litellm_params".to_owned(),
        json(r#"{"metadata":{"route":"r"},"litellm_metadata":{}}"#),
    );
    record(
        &mut state,
        "2024-01-01T00:00:00+00:00",
        "2024-01-01T00:00:02+00:00",
    )
    .unwrap();
    assert_eq!(windows(&state), Some(json("[[1704067200,1704067202]]")));
    let Some(Json::Object(details)) = state.model_call_details.get("litellm_params") else {
        panic!("litellm_params must stay an object");
    };
    assert_eq!(details.get("litellm_metadata"), Some(&json("{}")));
}

fn test_writes_into_a_lone_litellm_metadata() {
    let mut state = state(Some(true));
    state.model_call_details.insert(
        "litellm_params".to_owned(),
        json(r#"{"litellm_metadata":{"route":"r"}}"#),
    );
    record(
        &mut state,
        "2024-01-01T00:00:00+00:00",
        "2024-01-01T00:00:02+00:00",
    )
    .unwrap();
    let Some(Json::Object(details)) = state.model_call_details.get("litellm_params") else {
        panic!("litellm_params must stay an object");
    };
    let Some(Json::Object(metadata)) = details.get("litellm_metadata") else {
        panic!("litellm_metadata must gain the window");
    };
    assert_eq!(
        metadata.get(TIMING_WINDOWS_KEY),
        Some(&json("[[1704067200,1704067202]]"))
    );
    assert_eq!(metadata.get("route"), Some(&Json::Text("r".to_owned())));
}

fn test_appends_to_earlier_windows() {
    let mut state = state(Some(true));
    state.model_call_details.insert(
        "litellm_params".to_owned(),
        json(r#"{"metadata":{"llm_api_timing_windows":[[1,2],[3,4]]}}"#),
    );
    record(
        &mut state,
        "2024-01-01T00:00:00+00:00",
        "2024-01-01T00:00:02+00:00",
    )
    .unwrap();
    assert_eq!(
        windows(&state),
        Some(json("[[1,2],[3,4],[1704067200,1704067202]]"))
    );
}

fn test_skips_earlier_windows_that_are_not_number_pairs() {
    let mut state = state(Some(true));
    state.model_call_details.insert(
        "litellm_params".to_owned(),
        json(r#"{"metadata":{"llm_api_timing_windows":[[1],[1,2,3],"x",[1,"x"],[1,2]]}}"#),
    );
    record(
        &mut state,
        "2024-01-01T00:00:00+00:00",
        "2024-01-01T00:00:02+00:00",
    )
    .unwrap();
    assert_eq!(
        windows(&state),
        Some(json("[[1,2],[1704067200,1704067202]]"))
    );
}

fn test_ignores_recorded_windows_that_are_not_a_list() {
    let mut state = state(Some(true));
    state.model_call_details.insert(
        "litellm_params".to_owned(),
        json(r#"{"metadata":{"llm_api_timing_windows":"x"}}"#),
    );
    record(
        &mut state,
        "2024-01-01T00:00:00+00:00",
        "2024-01-01T00:00:02+00:00",
    )
    .unwrap();
    assert_eq!(windows(&state), Some(json("[[1704067200,1704067202]]")));
}

fn test_rejects_a_non_finite_earlier_window() {
    let mut state = state(Some(true));
    state.model_call_details.insert(
        "litellm_params".to_owned(),
        json(r#"{"metadata":{"llm_api_timing_windows":[[1,2],[1e400,2]]}}"#),
    );
    let result = record(
        &mut state,
        "2024-01-01T00:00:00+00:00",
        "2024-01-01T00:00:02+00:00",
    );
    assert!(matches!(
        result,
        Err(CallDurationError::WindowValue { index: 1 })
    ));
}

fn test_writes_into_litellm_metadata_and_copies_spend_keys() {
    let mut state = state(Some(true));
    state.model_call_details.insert(
        "litellm_params".to_owned(),
        json(
            r#"{"metadata":{"user_api_key":"key-1","user_api_key_alias":"alias"},"litellm_metadata":{"route":"r"}}"#,
        ),
    );
    record(
        &mut state,
        "2024-01-01T00:00:00+00:00",
        "2024-01-01T00:00:02+00:00",
    )
    .unwrap();
    let Some(Json::Object(details)) = state.model_call_details.get("litellm_params") else {
        panic!("litellm_params must stay an object");
    };
    let Some(Json::Object(metadata)) = details.get("litellm_metadata") else {
        panic!("litellm_metadata must gain the window");
    };
    assert_eq!(
        metadata.get("user_api_key"),
        Some(&Json::Text("key-1".to_owned()))
    );
    assert_eq!(
        metadata.get("user_api_key_alias"),
        Some(&Json::Text("alias".to_owned()))
    );
    assert_eq!(
        metadata.get(TIMING_WINDOWS_KEY),
        Some(&json("[[1704067200,1704067202]]"))
    );
}

fn test_leaves_missing_metadata_alone() {
    let mut state = state(Some(true));
    record(
        &mut state,
        "2024-01-01T00:00:00+00:00",
        "2024-01-01T00:00:02+00:00",
    )
    .unwrap();
    assert_eq!(
        state.model_call_details.get("litellm_params"),
        Some(&json("{}"))
    );
    assert_eq!(
        state.model_call_details.get(DURATION_KEY),
        Some(&json("2000"))
    );
}

fn test_treats_null_litellm_params_as_missing() {
    let mut state = state(Some(true));
    state
        .model_call_details
        .insert("litellm_params".to_owned(), Json::Null);
    record(
        &mut state,
        "2024-01-01T00:00:00+00:00",
        "2024-01-01T00:00:02+00:00",
    )
    .unwrap();
    assert_eq!(
        state.model_call_details.get(DURATION_KEY),
        Some(&json("2000"))
    );
}

fn test_rejects_a_non_object_litellm_params() {
    let mut state = state(Some(true));
    state
        .model_call_details
        .insert("litellm_params".to_owned(), json(r#""x""#));
    let result = record(
        &mut state,
        "2024-01-01T00:00:00+00:00",
        "2024-01-01T00:00:02+00:00",
    );
    assert!(matches!(
        result,
        Err(CallDurationError::FieldShape {
            field: "litellm_params"
        })
    ));
    assert_eq!(
        state.model_call_details.get(DURATION_KEY),
        Some(&json("2000"))
    );
}

fn test_rejects_a_non_object_metadata() {
    let mut state = state(Some(true));
    state
        .model_call_details
        .insert("litellm_params".to_owned(), json(r#"{"metadata":7}"#));
    let result = record(
        &mut state,
        "2024-01-01T00:00:00+00:00",
        "2024-01-01T00:00:02+00:00",
    );
    assert!(matches!(
        result,
        Err(CallDurationError::FieldShape { field: "metadata" })
    ));
}

fn test_rejects_a_non_object_litellm_metadata() {
    let mut state = state(Some(true));
    state.model_call_details.insert(
        "litellm_params".to_owned(),
        json(r#"{"litellm_metadata":[]}"#),
    );
    let result = record(
        &mut state,
        "2024-01-01T00:00:00+00:00",
        "2024-01-01T00:00:02+00:00",
    );
    assert!(matches!(
        result,
        Err(CallDurationError::FieldShape {
            field: "litellm_metadata"
        })
    ));
}

#[rstest]
#[case::duration_records_milliseconds(test_records_the_duration_in_milliseconds)]
#[case::duration_records_a_fractional_duration(test_records_a_fractional_duration)]
#[case::duration_records_a_negative_duration(test_records_a_negative_duration)]
#[case::duration_writes_a_timing_window(test_writes_a_timing_window_into_metadata)]
#[case::duration_ignores_an_empty_metadata(test_ignores_an_empty_metadata_object)]
#[case::duration_ignores_two_empty_metadata_objects(test_ignores_two_empty_metadata_objects)]
#[case::duration_writes_into_metadata_beside_an_empty_litellm_metadata(
    test_writes_into_metadata_beside_an_empty_litellm_metadata
)]
#[case::duration_writes_into_a_lone_litellm_metadata(test_writes_into_a_lone_litellm_metadata)]
#[case::duration_appends_to_earlier_windows(test_appends_to_earlier_windows)]
#[case::duration_skips_invalid_earlier_windows(
    test_skips_earlier_windows_that_are_not_number_pairs
)]
#[case::duration_ignores_a_non_list_window_field(test_ignores_recorded_windows_that_are_not_a_list)]
#[case::duration_rejects_a_non_finite_window(test_rejects_a_non_finite_earlier_window)]
#[case::duration_writes_into_litellm_metadata(
    test_writes_into_litellm_metadata_and_copies_spend_keys
)]
#[case::duration_leaves_missing_metadata_alone(test_leaves_missing_metadata_alone)]
#[case::duration_treats_null_params_as_missing(test_treats_null_litellm_params_as_missing)]
#[case::duration_rejects_a_non_object_litellm_params(test_rejects_a_non_object_litellm_params)]
#[case::duration_rejects_a_non_object_metadata(test_rejects_a_non_object_metadata)]
#[case::duration_rejects_a_non_object_litellm_metadata(test_rejects_a_non_object_litellm_metadata)]
fn logging_streams_behavior_tests_behavior(#[case] behavior: fn()) {
    behavior();
}
