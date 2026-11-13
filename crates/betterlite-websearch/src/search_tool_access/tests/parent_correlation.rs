use crate::search_tool_access::hook_kwargs::{SearchCaller, SearchHookKwargs};
use crate::search_tool_access::parent_correlation::parent_correlation;
use crate::search_tool_access::search_metadata::SearchMetadataValue;

fn hook_kwargs(json: &str) -> SearchHookKwargs {
    serde_json::from_str(json).unwrap()
}

fn search_caller(json: &str) -> SearchCaller {
    serde_json::from_str(json).unwrap()
}

#[test]
fn top_level_ids_win_over_the_metadata_blocks() {
    let kwargs = hook_kwargs(
        r#"{"litellm_session_id": "top-session", "litellm_trace_id": "top-trace", "litellm_call_id": "top-call",
        "metadata": {"session_id": "meta-session", "trace_id": "meta-trace", "parent_request_id": "meta-request"}}"#,
    );
    let correlation = parent_correlation(Some(&kwargs), None);
    assert_eq!(correlation.session_id.as_deref(), Some("top-session"));
    assert_eq!(correlation.trace_id.as_deref(), Some("top-trace"));
    assert_eq!(correlation.parent_request_id.as_deref(), Some("top-call"));
}

#[test]
fn reads_ids_from_the_metadata_blocks_in_search_order() {
    let kwargs = hook_kwargs(
        r#"{"metadata": {"session_id": "first"}, "litellm_metadata": {"session_id": "second"},
        "litellm_params": {"metadata": {"session_id": "third"}, "litellm_metadata": {"session_id": "fourth"}}}"#,
    );
    assert_eq!(
        parent_correlation(Some(&kwargs), None)
            .session_id
            .as_deref(),
        Some("first")
    );
    let params_only =
        hook_kwargs(r#"{"litellm_params": {"metadata": {"trace_id": "params-trace"}}}"#);
    assert_eq!(
        parent_correlation(Some(&params_only), None)
            .trace_id
            .as_deref(),
        Some("params-trace")
    );
}

#[test]
fn reads_ids_from_the_litellm_params_scope() {
    let kwargs = hook_kwargs(
        r#"{"litellm_params": {"litellm_session_id": "params-session",
        "litellm_trace_id": "params-trace", "litellm_call_id": "params-call"}}"#,
    );
    let correlation = parent_correlation(Some(&kwargs), None);
    assert_eq!(correlation.session_id.as_deref(), Some("params-session"));
    assert_eq!(correlation.trace_id.as_deref(), Some("params-trace"));
    assert_eq!(
        correlation.parent_request_id.as_deref(),
        Some("params-call")
    );
}

#[test]
fn skips_empty_and_non_string_values() {
    let kwargs = hook_kwargs(
        r#"{"litellm_session_id": 7, "litellm_trace_id": "", "litellm_call_id": false,
        "metadata": {"session_id": "", "trace_id": 5},
        "litellm_metadata": {"session_id": "session", "trace_id": "trace"}}"#,
    );
    let correlation = parent_correlation(Some(&kwargs), None);
    assert_eq!(correlation.session_id.as_deref(), Some("session"));
    assert_eq!(correlation.trace_id.as_deref(), Some("trace"));
    assert!(correlation.parent_request_id.is_none());
}

#[test]
fn reads_the_parent_request_id_only_from_litellm_call_id() {
    let kwargs = hook_kwargs(r#"{"metadata": {"parent_request_id": "metadata"}}"#);
    assert!(
        parent_correlation(Some(&kwargs), None)
            .parent_request_id
            .is_none()
    );
}

#[test]
fn absent_kwargs_give_no_correlation_even_with_a_caller() {
    let auth = search_caller(r#"{"team_id": "t", "parent_otel_span": "caller-span"}"#);
    let correlation = parent_correlation(None, Some(&auth));
    assert!(correlation.session_id.is_none());
    assert!(correlation.trace_id.is_none());
    assert!(correlation.parent_request_id.is_none());
    assert!(correlation.parent_otel_span.is_none());
}

#[test]
fn empty_kwargs_give_no_correlation_even_with_a_caller() {
    let auth = search_caller(r#"{"team_id": "t", "parent_otel_span": "caller-span"}"#);
    let values = hook_kwargs("{}");
    let correlation = parent_correlation(Some(&values), Some(&auth));
    assert!(correlation.session_id.is_none());
    assert!(correlation.trace_id.is_none());
    assert!(correlation.parent_request_id.is_none());
    assert!(correlation.parent_otel_span.is_none());
}

#[test]
fn the_parent_span_comes_from_the_first_metadata_block() {
    let kwargs = hook_kwargs(
        r#"{"metadata": {"litellm_parent_otel_span": null},
        "litellm_metadata": {"litellm_parent_otel_span": "meta-span"}}"#,
    );
    let auth = search_caller(r#"{"team_id": "t", "parent_otel_span": "caller-span"}"#);
    let correlation = parent_correlation(Some(&kwargs), Some(&auth));
    assert_eq!(
        serde_json::to_value(&correlation).unwrap()["parent_otel_span"],
        "meta-span"
    );
}

#[test]
fn the_parent_span_falls_back_to_the_caller() {
    let kwargs = hook_kwargs(r#"{"metadata": {"session_id": "s"}}"#);
    let auth = search_caller(r#"{"team_id": "t", "parent_otel_span": "caller-span"}"#);
    let correlation = parent_correlation(Some(&kwargs), Some(&auth));
    assert_eq!(
        serde_json::to_value(&correlation).unwrap()["parent_otel_span"],
        "caller-span"
    );
}

#[test]
fn search_metadata_lists_the_set_values_in_order() {
    let kwargs = hook_kwargs(
        r#"{"litellm_session_id": "s", "litellm_trace_id": "t", "litellm_call_id": "c",
        "metadata": {"litellm_parent_otel_span": "span"}}"#,
    );
    let correlation = parent_correlation(Some(&kwargs), None);
    let metadata = correlation.as_search_metadata();
    assert_eq!(
        metadata.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "session_id",
            "trace_id",
            "parent_request_id",
            "litellm_parent_otel_span"
        ]
    );
    assert!(matches!(
        metadata["litellm_parent_otel_span"],
        SearchMetadataValue::Span(_)
    ));
    let search_kwargs = correlation.as_search_kwargs();
    assert_eq!(
        search_kwargs.keys().map(String::as_str).collect::<Vec<_>>(),
        ["litellm_session_id", "litellm_trace_id"]
    );
    assert_eq!(search_kwargs["litellm_session_id"], "s");
    assert_eq!(search_kwargs["litellm_trace_id"], "t");
}

#[test]
fn search_metadata_omits_unset_values() {
    let kwargs = hook_kwargs(r#"{"litellm_trace_id": "t", "litellm_call_id": "c"}"#);
    let correlation = parent_correlation(Some(&kwargs), None);
    let metadata = correlation.as_search_metadata();
    assert_eq!(
        metadata.keys().map(String::as_str).collect::<Vec<_>>(),
        ["trace_id", "parent_request_id"]
    );
    assert_eq!(
        correlation
            .as_search_kwargs()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["litellm_trace_id"]
    );
}
