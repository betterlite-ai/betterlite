#![cfg(test)]
use super::{TraceControls, caller_trace_controls};
use crate::otel::model::utils::coercion::OtelModelValue;

fn input(source: &str) -> OtelModelValue {
    serde_json::from_str(source).expect("test JSON is valid")
}

#[test]
fn header_precedence_and_body_fallback() {
    let controls = caller_trace_controls(&input(
        r#"{"litellm_params":{
        "metadata":{"trace_name":"body","trace_user_id":"user"},
        "litellm_metadata":{"trace_name":"later","session_id":"session"},
        "proxy_server_request":{"headers":{"langfuse_trace_name":"header","langfuse_trace_user_id":"header-user","langfuse_session_id":"header-session"}}
    }}"#,
    ));
    assert_eq!(controls.name.as_deref(), Some("header"));
    assert_eq!(controls.user_id.as_deref(), Some("header-user"));
    assert_eq!(controls.session_id.as_deref(), Some("header-session"));
}

#[test]
fn primary_body_scalars_win_without_nonempty_headers() {
    let controls = caller_trace_controls(&input(
        r#"{"litellm_params":{
        "metadata":{"trace_name":"primary-name","trace_user_id":"primary-user","session_id":"primary-session"},
        "litellm_metadata":{"trace_name":"secondary-name","trace_user_id":"secondary-user","session_id":"secondary-session"},
        "proxy_server_request":{"headers":{"langfuse_trace_name":"","langfuse_trace_user_id":"","langfuse_session_id":""}}
    }}"#,
    ));
    assert_eq!(controls.name.as_deref(), Some("primary-name"));
    assert_eq!(controls.user_id.as_deref(), Some("primary-user"));
    assert_eq!(controls.session_id.as_deref(), Some("primary-session"));
}

#[test]
fn empty_primary_body_value_falls_back_to_secondary_metadata() {
    let controls = caller_trace_controls(&input(
        r#"{"litellm_params":{
        "metadata":{"trace_name":""},"litellm_metadata":{"trace_name":"secondary"}
    }}"#,
    ));
    assert_eq!(controls.name.as_deref(), Some("secondary"));
}

#[test]
fn tags_filter_and_mutation_fields_are_excluded() {
    let controls = caller_trace_controls(&input(
        r#"{"litellm_params":{
        "metadata":{"tags":["",1,"one"],"trace_id":"ignored","existing_trace_id":"ignored","update_trace_keys":["x"]},
        "litellm_metadata":{"tags":["later"]}
    }}"#,
    ));
    assert_eq!(controls.tags, vec!["one"]);
    assert_eq!(controls.name, None);
}

#[test]
fn empty_filtered_primary_tags_fall_back_to_secondary_metadata() {
    let controls = caller_trace_controls(&input(
        r#"{"litellm_params":{
        "metadata":{"tags":["",1,false]},
        "litellm_metadata":{"tags":["secondary-tag"]}
    }}"#,
    ));
    assert_eq!(controls.tags, vec!["secondary-tag"]);
}

#[test]
fn malformed_nested_values_produce_empty_controls() {
    assert_eq!(
        caller_trace_controls(&input(r#"{"litellm_params":[]}"#)),
        TraceControls::default()
    );
}
