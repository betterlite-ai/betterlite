use crate::search_tool_access::hook_kwargs::{SearchHookKwargs, caller_from_kwargs};

fn hook_kwargs(json: &str) -> SearchHookKwargs {
    serde_json::from_str(json).unwrap()
}

fn found_team_id(kwargs: &SearchHookKwargs) -> Option<&str> {
    let caller = caller_from_kwargs(Some(kwargs))?;
    caller.team_id.as_deref()
}

#[test]
fn prefers_the_caller_in_the_top_level_metadata() {
    let kwargs = hook_kwargs(
        r#"{"metadata": {"user_api_key_auth": {"team_id": "metadata"}},
        "litellm_metadata": {"user_api_key_auth": {"team_id": "litellm-metadata"}},
        "litellm_params": {"metadata": {"user_api_key_auth": {"team_id": "params"}}}}"#,
    );
    assert_eq!(found_team_id(&kwargs), Some("metadata"));
}

#[test]
fn falls_back_to_the_top_level_litellm_metadata() {
    let kwargs = hook_kwargs(
        r#"{"metadata": {},
        "litellm_metadata": {"user_api_key_auth": {"team_id": "litellm-metadata"}},
        "litellm_params": {"metadata": {"user_api_key_auth": {"team_id": "params"}}}}"#,
    );
    assert_eq!(found_team_id(&kwargs), Some("litellm-metadata"));
}

#[test]
fn skips_a_null_caller_block() {
    let kwargs = hook_kwargs(
        r#"{"metadata": {"user_api_key_auth": null},
        "litellm_metadata": {"user_api_key_auth": {"team_id": "litellm-metadata"}}}"#,
    );
    assert_eq!(found_team_id(&kwargs), Some("litellm-metadata"));
}

#[test]
fn reads_the_caller_from_litellm_params_metadata() {
    let kwargs = hook_kwargs(
        r#"{"metadata": {}, "litellm_params": {"metadata": {"user_api_key_auth": {"team_id": "params"}}}}"#,
    );
    assert_eq!(found_team_id(&kwargs), Some("params"));
}

#[test]
fn reads_the_caller_from_litellm_params_litellm_metadata() {
    let kwargs = hook_kwargs(
        r#"{"litellm_params": {"litellm_metadata": {"user_api_key_auth": {"team_id": "params-litellm"}}}}"#,
    );
    assert_eq!(found_team_id(&kwargs), Some("params-litellm"));
}

#[test]
fn gives_no_caller_for_absent_or_empty_kwargs() {
    assert!(caller_from_kwargs(None).is_none());
    assert!(caller_from_kwargs(Some(&hook_kwargs("{}"))).is_none());
    assert!(
        caller_from_kwargs(Some(&hook_kwargs(r#"{"metadata": {"session_id": "s"}}"#))).is_none()
    );
    assert!(
        caller_from_kwargs(Some(&hook_kwargs(
            r#"{"litellm_params": {"metadata": {}}}"#
        )))
        .is_none()
    );
}

#[test]
fn reads_the_callers_parent_span() {
    let kwargs = hook_kwargs(
        r#"{"metadata": {"user_api_key_auth": {"team_id": "t", "parent_otel_span": "span"}}}"#,
    );
    let caller = caller_from_kwargs(Some(&kwargs)).unwrap();
    assert_eq!(
        serde_json::to_value(caller).unwrap()["parent_otel_span"],
        "span"
    );
}
