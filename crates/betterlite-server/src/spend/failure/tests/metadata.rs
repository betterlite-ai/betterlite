use betterlite_config::spend_storage::SpendUpdates;
use betterlite_models::decisions::Json;
use rstest::rstest;
use serde_json::Number;

use super::super::fields::{
    TrackingIdentity, metadata_keys, request_tags_for_cost_tracking, should_track_cost_callback,
    write_spend_metadata_to_kwargs,
};
use super::super::metadata::{
    as_object, merge_failure_metadata, trusted_failure_metadata, used_client_oauth_token,
};
use super::support::{nested, object, principal};

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

#[test]
fn trusted_metadata_carries_the_key_facts_and_the_failure_stamp() {
    let metadata = trusted_failure_metadata(&principal()).expect("the key metadata encodes");
    assert_eq!(metadata["user_api_key"], text("hashed-key"));
    assert_eq!(metadata["user_api_key_hash"], text("hashed-key"));
    assert_eq!(metadata["user_api_key_alias"], text("test-key"));
    assert_eq!(metadata["user_api_key_team_id"], text("team-1"));
    assert_eq!(metadata["user_api_key_org_id"], text("org-1"));
    assert_eq!(metadata["user_api_key_project_id"], text("project-1"));
    assert_eq!(metadata["status"], text("failure"));
}

#[test]
fn a_session_token_logs_under_its_alias() {
    let mut facts = principal();
    facts.is_session_token = true;
    let metadata = trusted_failure_metadata(&facts).expect("the key metadata encodes");
    assert_eq!(metadata["user_api_key"], text("test-key"));
}

#[rstest]
#[case::no_bucket("{}", None, None)]
#[case::metadata_true(r#"{"metadata": {"used_client_oauth_token": true}}"#, None, Some(true))]
#[case::metadata_false(
    r#"{"metadata": {"used_client_oauth_token": false}}"#,
    None,
    Some(false)
)]
#[case::text_is_not_a_bool(r#"{"metadata": {"used_client_oauth_token": "true"}}"#, None, None)]
#[case::number_is_not_a_bool(r#"{"metadata": {"used_client_oauth_token": 1}}"#, None, None)]
#[case::null_is_not_a_bool(r#"{"metadata": {"used_client_oauth_token": null}}"#, None, None)]
#[case::the_present_internal_bucket_wins(
    r#"{"metadata": {"used_client_oauth_token": true}, "litellm_metadata": {}}"#,
    None,
    None
)]
#[case::a_route_selects_litellm_metadata(
    r#"{"litellm_metadata": {"used_client_oauth_token": false}, "metadata": {"used_client_oauth_token": true}}"#,
    Some("/v1/messages"),
    Some(false)
)]
#[case::a_route_ignores_the_other_bucket(
    r#"{"metadata": {"used_client_oauth_token": true}}"#,
    Some("/v1/messages"),
    None
)]
#[case::a_chat_route_reads_the_caller_bucket(
    r#"{"metadata": {"used_client_oauth_token": true}}"#,
    Some("/v1/chat/completions"),
    Some(true)
)]
fn oauth_provenance_is_a_boolean_from_the_route_selected_bucket(
    #[case] request: &str,
    #[case] route: Option<&str>,
    #[case] expected: Option<bool>,
) {
    assert_eq!(used_client_oauth_token(&object(request), route), expected);
}

#[test]
fn merge_lets_trusted_fields_win_and_updates_a_non_empty_caller_bucket() {
    let mut request_data = object(r#"{"metadata": {"keep": 1, "user_api_key": "stale"}}"#);
    let mut trusted = object(r#"{"user_api_key": "fresh", "status": "failure"}"#);
    trusted.insert(TAGS_TRUSTED.to_owned(), Json::List(vec![text("t")]));
    merge_failure_metadata(&mut request_data, trusted);
    let merged = as_object(&request_data["metadata"]).expect("the caller bucket survives");
    assert_eq!(merged["keep"], Json::Number(Number::from(1_u8)));
    assert_eq!(merged["user_api_key"], text("fresh"));
    assert_eq!(merged["tags"], Json::List(vec![text("t")]));
    assert_eq!(
        *nested(&request_data, &["litellm_params", "metadata", "status"]),
        text("failure")
    );
}

const TAGS_TRUSTED: &str = "tags";

#[test]
fn merge_does_not_create_a_top_level_metadata_bucket() {
    let mut request_data = object(r#"{"model": "gpt-4o"}"#);
    merge_failure_metadata(&mut request_data, object(r#"{"status": "failure"}"#));
    assert!(!request_data.contains_key("metadata"));
    assert_eq!(
        *nested(&request_data, &["litellm_params", "metadata", "status"]),
        text("failure")
    );
}

#[rstest]
#[case::copied_when_missing(
    r#"{"litellm_metadata": {"standard_logging_guardrail_information": [{"guardrail_cost": 0.5}]}}"#,
    true
)]
#[case::a_null_source_is_not_copied(
    r#"{"litellm_metadata": {"standard_logging_guardrail_information": null}}"#,
    false
)]
fn merge_copies_guardrail_information(#[case] request: &str, #[case] copied: bool) {
    let mut request_data = object(request);
    merge_failure_metadata(&mut request_data, object("{}"));
    let metadata = as_object(nested(&request_data, &["litellm_params", "metadata"]));
    assert_eq!(
        metadata
            .is_some_and(|fields| fields.contains_key("standard_logging_guardrail_information")),
        copied
    );
}

#[test]
fn merge_keeps_an_existing_guardrail_key_even_when_null() {
    let mut request_data = object(
        r#"{"metadata": {"standard_logging_guardrail_information": null},
            "litellm_metadata": {"standard_logging_guardrail_information": [{"guardrail_cost": 1}]}}"#,
    );
    merge_failure_metadata(&mut request_data, object("{}"));
    assert_eq!(
        *nested(
            &request_data,
            &[
                "litellm_params",
                "metadata",
                "standard_logging_guardrail_information"
            ]
        ),
        Json::Null
    );
}

#[rstest]
#[case::truthy_tags_win(r#"{"metadata": {"tags": ["internal"]}}"#, true)]
#[case::empty_tags_are_falsey(r#"{"metadata": {"tags": []}}"#, false)]
fn merge_preserves_truthy_nested_tags(#[case] params: &str, #[case] preserved: bool) {
    let mut request_data = object(&format!(r#"{{"litellm_params": {params}}}"#));
    merge_failure_metadata(&mut request_data, object(r#"{"tags": ["header"]}"#));
    let tags = nested(&request_data, &["litellm_params", "metadata", "tags"]);
    let expected = if preserved { "internal" } else { "header" };
    assert_eq!(tags, &Json::List(vec![text(expected)]));
}

#[rstest]
#[case::top_level_wins(
    r#"{"proxy_server_request": {"url": "top"}}"#,
    r#"{"proxy_server_request": {"url": "params"}}"#,
    "top"
)]
#[case::params_are_the_fallback(
    r#"{"litellm_params": {"proxy_server_request": {"url": "params"}}}"#,
    "{}",
    "params"
)]
#[case::an_empty_top_object_is_falsey(
    r#"{"proxy_server_request": {}}"#,
    r#"{"litellm_params": {"proxy_server_request": {"url": "params"}}}"#,
    "params"
)]
#[case::defaults_to_an_empty_object("{}", "{}", "")]
fn merge_preserves_the_proxy_server_request(
    #[case] request: &str,
    #[case] extra: &str,
    #[case] url: &str,
) {
    let mut request_data = object(request);
    let extra_request = object(extra);
    if let Some(params) = extra_request.get("litellm_params") {
        request_data.insert("litellm_params".to_owned(), params.clone());
    }
    merge_failure_metadata(&mut request_data, object("{}"));
    assert_eq!(
        *nested(&request_data, &["litellm_params", "proxy_server_request"]),
        Json::Object(object(&format!(r#"{{"url": "{url}"}}"#)))
    );
}

#[rstest]
#[case::a_present_model_stays(r#"{"model": "gpt-4o"}"#, "gpt-4o", "")]
#[case::params_supply_the_value(
    r#"{"litellm_params": {"model": "gpt-4o", "custom_llm_provider": "openai"}}"#,
    "gpt-4o",
    "openai"
)]
#[case::present_values_stay(r#"{"model": "", "custom_llm_provider": ""}"#, "", "")]
fn merge_fills_model_and_provider_only_when_absent(
    #[case] request: &str,
    #[case] model: &str,
    #[case] provider: &str,
) {
    let mut request_data = object(request);
    merge_failure_metadata(&mut request_data, object("{}"));
    assert_eq!(request_data["model"], text(model));
    assert_eq!(request_data["custom_llm_provider"], text(provider));
}

#[test]
fn the_patch_fills_only_nulls_in_existing_buckets() {
    let mut kwargs = object(
        r#"{"litellm_params": {
            "litellm_metadata": {"user_api_key_alias": null, "tags": null},
            "metadata": {"user_api_key_alias": "old", "tags": ["caller"]}
        }}"#,
    );
    let metadata = object(
        r#"{"user_api_key_alias": "new", "user_api_key": null, "tags": ["trusted"], "score": 4}"#,
    );
    write_spend_metadata_to_kwargs(&mut kwargs, &metadata);
    assert_eq!(
        *nested(
            &kwargs,
            &["litellm_params", "litellm_metadata", "user_api_key_alias"]
        ),
        text("new")
    );
    assert_eq!(
        *nested(&kwargs, &["litellm_params", "litellm_metadata", "tags"]),
        Json::List(vec![text("trusted")])
    );
    assert_eq!(
        *nested(
            &kwargs,
            &["litellm_params", "metadata", "user_api_key_alias"]
        ),
        text("old")
    );
    assert_eq!(
        *nested(&kwargs, &["litellm_params", "metadata", "tags"]),
        Json::List(vec![text("caller")])
    );
    let litellm_metadata = as_object(nested(&kwargs, &["litellm_params", "litellm_metadata"]));
    let caller_bucket = as_object(nested(&kwargs, &["litellm_params", "metadata"]));
    assert!(!caller_bucket.is_some_and(|fields| fields.contains_key("score")));
    assert!(!litellm_metadata.is_some_and(|fields| fields.contains_key("user_api_key")));
}

#[rstest]
#[case::an_empty_patch_changes_nothing(r#"{"user_api_key": null}"#)]
#[case::a_non_key_field_changes_nothing(r#"{"other": "value"}"#)]
fn the_patch_returns_early(#[case] metadata: &str) {
    let mut kwargs = object(r#"{"litellm_params": {}}"#);
    write_spend_metadata_to_kwargs(&mut kwargs, &object(metadata));
    assert_eq!(kwargs["litellm_params"], Json::Object(object("{}")));
}

#[test]
fn the_patch_creates_litellm_params_when_it_is_absent() {
    let mut kwargs = object("{}");
    write_spend_metadata_to_kwargs(&mut kwargs, &object(r#"{"user_api_key": "hash"}"#));
    assert_eq!(kwargs["litellm_params"], Json::Object(object("{}")));
}

#[rstest]
#[case::an_empty_list_wins(r#"{"request_tags": []}"#, r#"{"tags": ["metadata"]}"#, Some("[]"))]
#[case::a_list_wins(
    r#"{"request_tags": ["payload"]}"#,
    r#"{"tags": ["metadata"]}"#,
    Some(r#"["payload"]"#)
)]
#[case::a_non_list_falls_through(
    r#"{"request_tags": "payload"}"#,
    r#"{"tags": ["metadata"]}"#,
    Some(r#"["metadata"]"#)
)]
#[case::metadata_only("{}", r#"{"tags": ["metadata"]}"#, Some(r#"["metadata"]"#))]
#[case::nothing_is_none("{}", "{}", None)]
fn request_tags_prefer_the_standard_payload(
    #[case] standard: &str,
    #[case] metadata: &str,
    #[case] expected: Option<&str>,
) {
    let standard_payload = object(standard);
    let standard_value = if standard_payload.is_empty() {
        None
    } else {
        Some(Json::Object(standard_payload))
    };
    let metadata_payload = Json::Object(object(metadata));
    let tags = request_tags_for_cost_tracking(standard_value.as_ref(), Some(&metadata_payload));
    let rendered = tags.map(|value| serde_json::to_string(value).expect("the tags serialize"));
    assert_eq!(rendered, expected.map(str::to_owned));
}

#[test]
fn metadata_keys_are_sorted_and_other_values_have_none() {
    let keys = metadata_keys(Some(&Json::Object(object(r#"{"b": 1, "a": 2}"#))));
    assert_eq!(keys, vec!["a".to_owned(), "b".to_owned()]);
    assert!(metadata_keys(Some(&text("x"))).is_empty());
    assert!(metadata_keys(None).is_empty());
}

#[rstest]
#[case::disabled_wins(
    SpendUpdates::Disabled,
    TrackingIdentity {
        hashed_user_api_key: Some("hash"),
        ..TrackingIdentity::default()
    },
    Some("pass_through_endpoint"),
    false
)]
#[case::a_key_qualifies(
    SpendUpdates::Allowed,
    TrackingIdentity {
        hashed_user_api_key: Some(""),
        ..TrackingIdentity::default()
    },
    None,
    true
)]
#[case::a_user_qualifies(
    SpendUpdates::Allowed,
    TrackingIdentity {
        user_id: Some("u"),
        ..TrackingIdentity::default()
    },
    None,
    true
)]
#[case::a_team_qualifies(
    SpendUpdates::Allowed,
    TrackingIdentity {
        team_id: Some("t"),
        ..TrackingIdentity::default()
    },
    None,
    true
)]
#[case::an_end_user_qualifies(
    SpendUpdates::Allowed,
    TrackingIdentity {
        end_user_id: Some("e"),
        ..TrackingIdentity::default()
    },
    None,
    true
)]
#[case::an_agent_qualifies(
    SpendUpdates::Allowed,
    TrackingIdentity {
        agent_id: Some("a"),
        ..TrackingIdentity::default()
    },
    None,
    true
)]
#[case::pass_through_is_trackable(
    SpendUpdates::Allowed,
    TrackingIdentity::default(),
    Some("pass_through_endpoint"),
    true
)]
#[case::llm_passthrough_is_trackable(
    SpendUpdates::Allowed,
    TrackingIdentity::default(),
    Some("llm_passthrough_route"),
    true
)]
#[case::allm_passthrough_is_trackable(
    SpendUpdates::Allowed,
    TrackingIdentity::default(),
    Some("allm_passthrough_route"),
    true
)]
#[case::mcp_tools_are_trackable(
    SpendUpdates::Allowed,
    TrackingIdentity::default(),
    Some("call_mcp_tool"),
    true
)]
#[case::batch_retrieve_is_trackable(
    SpendUpdates::Allowed,
    TrackingIdentity::default(),
    Some("aretrieve_batch"),
    true
)]
#[case::another_call_type_is_not(
    SpendUpdates::Allowed,
    TrackingIdentity::default(),
    Some("completion"),
    false
)]
#[case::no_call_type_is_not(SpendUpdates::Allowed, TrackingIdentity::default(), None, false)]
fn eligibility_honors_the_spend_switch_and_any_non_null_identity(
    #[case] spend_updates: SpendUpdates,
    #[case] identity: TrackingIdentity<'_>,
    #[case] call_type: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(
        should_track_cost_callback(spend_updates, &identity, call_type),
        expected
    );
}
