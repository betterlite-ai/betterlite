#![cfg(test)]
use crate::otel::model::metadata::session_id::caller_session_id;
use crate::otel::model::metadata::wire::callback::Callback;
use crate::otel::model::trace_controls::{TraceControls, caller_trace_controls};

fn input(source: &str) -> Callback {
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

#[rstest::rstest]
#[case(r#"{"litellm_params":{"litellm_session_id":"minted","metadata":{"session_id":"minted","litellm_session_id_generated":true}}}"#, None)]
#[case(
    r#"{"litellm_params":{"litellm_session_id":"trace","metadata":{"trace_id":"trace"}}}"#,
    None
)]
#[case(r#"{"litellm_params":{"metadata":{"session_id":"minted","litellm_session_id_generated":true},"litellm_metadata":{"session_id":"caller"}}}"#, Some("caller"))]
#[case(r#"{"litellm_params":{"metadata":{"session_id":"minted","litellm_session_id_generated":true},"proxy_server_request":{"headers":{"langfuse_session_id":"header"}}}}"#, Some("header"))]
fn minted_ids_and_trace_echoes_never_become_caller_sessions(
    #[case] source: &str,
    #[case] expected: Option<&str>,
) {
    let kwargs = input(source);
    let raw = serde_json::from_value(serde_json::to_value(&kwargs).unwrap()).unwrap();
    let trace = caller_trace_controls(&raw);
    assert_eq!(caller_session_id(&kwargs, &trace).as_deref(), expected);
}
