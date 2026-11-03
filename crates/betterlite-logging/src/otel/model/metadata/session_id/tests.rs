#![cfg(test)]
use crate::otel::model::metadata::callback::caller_session_id;
use crate::otel::model::trace_controls::TraceControls;
use crate::otel::model::utils::coercion::OtelModelValue;

fn input(source: &str) -> OtelModelValue {
    serde_json::from_str(source).expect("test JSON is valid")
}

#[test]
fn minted_session_is_ignored_and_trace_fallback_used() {
    let trace = TraceControls {
        name: None,
        user_id: None,
        session_id: Some("trace-session".to_owned()),
        tags: Vec::new(),
    };
    let kwargs = input(
        r#"{"litellm_params":{"metadata":{"session_id":"minted","litellm_session_id_generated":true}}}"#,
    );
    assert_eq!(
        caller_session_id(&kwargs, &trace).as_deref(),
        Some("trace-session")
    );
}

#[test]
fn explicit_header_parameter_wins_over_body() {
    let trace = TraceControls {
        name: None,
        user_id: None,
        session_id: Some("header".to_owned()),
        tags: Vec::new(),
    };
    let kwargs = input(
        r#"{"litellm_params":{"litellm_session_id":"header","metadata":{"session_id":"body"}}}"#,
    );
    assert_eq!(
        caller_session_id(&kwargs, &trace).as_deref(),
        Some("header")
    );
}
