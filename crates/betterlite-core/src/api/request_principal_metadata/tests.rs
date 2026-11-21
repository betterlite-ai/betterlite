#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_auth::agent_policy::{ManagedAgentContext, ManagedAgentMode};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use serde::de::DeserializeOwned;

use super::error::RequestPrincipalMetadataError;
use super::key_controls::{add_key_level_controls, add_management_metadata};
use super::principal::{add_principal_metadata, logged_api_key};
use super::tags::request_tags;
use super::team_callbacks::{DefaultTeamSettings, team_callbacks};
use super::{PrincipalFacts, RequestPrincipalMetadataInput, apply_pre_auth_tags};
use crate::api::api_boundaries::ApiLimits;
use crate::api::request_destination::RequestData;
use crate::api::request_metadata_safety::metadata::MetadataField;

fn parsed<T: DeserializeOwned>(text: &str) -> T {
    serde_json::from_str(text).unwrap()
}

fn tags_header(header: Option<&str>) -> IndexMap<String, String> {
    header
        .map(|value| ("X-LiteLLM-Tags".to_owned(), value.to_owned()))
        .into_iter()
        .collect()
}

#[rstest]
#[case::request_then_header_then_key(
    r#"{"metadata": {"tags": ["own", "own"]}}"#,
    "/v1/chat/completions",
    Some("key, own ,"),
    r#"{"tags": ["key", "team"]}"#,
    r#"{"metadata": {"tags": ["own", "own", "key", "team"]}}"#
)]
#[case::litellm_metadata_route(
    r#"{"metadata": {"tags": ["caller"]}}"#,
    "/v1/messages",
    Some("header"),
    r#"{"tags": ["key"]}"#,
    r#"{"metadata": {"tags": ["caller"]}, "litellm_metadata": {"tags": ["header", "key"]}}"#
)]
#[case::metadata_text_parsed(
    r#"{"metadata": "{\"tags\": [\"old\"]}"}"#,
    "/v1/chat/completions",
    None,
    r#"{"tags": ["key"]}"#,
    r#"{"metadata": {"tags": ["old", "key"]}}"#
)]
#[case::nothing_to_merge(
    r#"{"model": "m"}"#,
    "/v1/chat/completions",
    Some(" , "),
    r#"{"tags": []}"#,
    r#"{"model": "m"}"#
)]
#[case::batches_route_seeds_only(
    r#"{"model": "m"}"#,
    "/v1/batches",
    Some(""),
    r#"{"tags": "solo"}"#,
    r#"{"model": "m", "litellm_metadata": {}}"#
)]
fn principal_tags_precedence(
    #[case] body: &str,
    #[case] route: &str,
    #[case] header: Option<&str>,
    #[case] key_metadata: &str,
    #[case] expected: &str,
) {
    let mut principal = PrincipalFacts::default();
    principal.metadata = parsed(key_metadata);
    let input = RequestPrincipalMetadataInput {
        data: parsed(body),
        headers: tags_header(header),
        route: route.to_owned(),
        principal,
    };
    let result = apply_pre_auth_tags(&input, ApiLimits::default());
    assert_eq!(result.data, parsed::<RequestData>(expected));
    let slot = if route == "/v1/messages" || route == "/v1/batches" {
        MetadataField::LitellmMetadata
    } else {
        MetadataField::Metadata
    };
    assert_eq!(result.metadata_variable_name, slot);
}

#[rstest]
#[case::key_tags_follow_request(r#"["a", "a"]"#, r#"["b", "a"]"#, r#"["a", "a", "b"]"#)]
#[case::key_tags_not_a_list(r#"["a"]"#, r#""b""#, r#"["a"]"#)]
#[case::request_tags_not_a_list(r#""a""#, r#"["b"]"#, r#"["b"]"#)]
#[case::null_key_tags_leave_request_tags(r#""a""#, "null", r#""a""#)]
fn key_level_tags_follow_request_tags(
    #[case] request: &str,
    #[case] key: &str,
    #[case] expected: &str,
) {
    let mut data: RequestData = parsed(&format!(r#"{{"metadata": {{"tags": {request}}}}}"#));
    add_key_level_controls(
        &mut data,
        MetadataField::Metadata,
        &parsed(&format!(r#"{{"tags": {key}}}"#)),
    );
    assert_eq!(
        data.get("metadata"),
        Some(&parsed(&format!(
            r#"{{"tags": {expected}, "user_api_key_auth_metadata": {{}}}}"#
        )))
    );
}

#[rstest]
#[case::header_keeps_empty_entries(Some("a, ,b"), "{}", r#"["a", "", "b"]"#)]
#[case::body_list_replaces_header(Some("a"), r#"{"tags": ["x"]}"#, r#"["x"]"#)]
#[case::body_text_leaves_header(Some("a"), r#"{"tags": "x"}"#, r#"["a"]"#)]
#[case::empty_body_list_kept(None, r#"{"tags": []}"#, "[]")]
#[case::no_caller_tags(None, "{}", "null")]
fn caller_tags_distinct(#[case] header: Option<&str>, #[case] body: &str, #[case] expected: &str) {
    let data: RequestData = parsed(body);
    let tags = request_tags(&tags_header(header), &data);
    assert_eq!(tags, parsed::<Option<Vec<Json>>>(expected));
}

#[test]
fn callback_policy_from_key() {
    let mut principal: PrincipalFacts = parsed(
        r#"{"api_key": "hash-1", "metadata": {"logging": [{"callback_name": "langfuse"}],
        "callback_settings": {"callback_vars": {}}, "secret_manager_settings": {"vault": "x"},
        "owner": "ops", "tags": ["k"], "model_rpm_limit": {"m": 1}},
        "team_metadata": {"logging": [], "team": "t"}}"#,
    );
    let mut data: RequestData = parsed(r#"{"metadata": {}}"#);
    let view = add_principal_metadata(&mut data, MetadataField::Metadata, &principal).unwrap();
    let kept: Json = parsed(r#"{"owner": "ops", "tags": ["k"], "model_rpm_limit": {"m": 1}}"#);
    assert_eq!(Json::Object(view.metadata), kept);
    assert_eq!(view.team_metadata, Some(parsed(r#"{"team": "t"}"#)));
    assert_eq!(view.project_metadata, None);
    add_key_level_controls(&mut data, MetadataField::Metadata, &principal.metadata);
    let Some(Json::Object(bucket)) = data.get("metadata") else {
        panic!("the metadata slot is an object");
    };
    assert_eq!(bucket.get("user_api_key_auth_metadata"), Some(&kept));
    assert_eq!(bucket.get("tags"), Some(&parsed(r#"["k"]"#)));
}

#[rstest]
#[case::reference_resolved(
    r#"[{"team_id": "t1", "success_callback": ["langfuse"], "langfuse_public_key": "os.environ/PK", "langfuse_secret": "plain"}]"#,
    "t1",
    r#"["langfuse"]|[]|[("langfuse_public_key", "pk-1"), ("langfuse_secret", "plain")]"#
)]
#[case::callback_vars_win(
    r#"[{"team_id": "t1", "failure_callback": ["datadog"], "langfuse_host": "top", "callback_vars": {"langfuse_host": "inner", "team_id": "t1"}}]"#,
    "t1",
    r#"[]|["datadog"]|[("langfuse_host", "inner")]"#
)]
#[case::bare_name_resolved(
    r#"[{"team_id": "t1", "langfuse_host": "PK"}]"#,
    "t1",
    r#"[]|[]|[("langfuse_host", "pk-1")]"#
)]
#[case::unset_reference_kept(
    r#"[{"team_id": "t1", "langfuse_host": "os.environ/MISSING"}]"#,
    "t1",
    r#"[]|[]|[("langfuse_host", "os.environ/MISSING")]"#
)]
#[case::empty_secret_kept(
    r#"[{"team_id": "t1", "langfuse_host": "os.environ/EMPTY"}]"#,
    "t1",
    r#"[]|[]|[("langfuse_host", "os.environ/EMPTY")]"#
)]
#[case::first_matching_team(
    r#"[{"team_id": "t0"}, {"team_id": "t1", "dd_site": "a"}, {"team_id": "t1", "dd_site": "b"}]"#,
    "t1",
    r#"[]|[]|[("dd_site", "a")]"#
)]
#[case::no_matching_team(r#"[{"team_id": "t1", "dd_site": "a"}]"#, "t2", "none")]
#[case::unknown_variable(r#"[{"team_id": "t1", "region": "eu"}]"#, "t1", "invalid region")]
fn team_callbacks_from_config(#[case] teams: &str, #[case] team_id: &str, #[case] expected: &str) {
    let teams: Vec<DefaultTeamSettings> = parsed(teams);
    let env = BTreeMap::from([
        ("PK".to_owned(), "pk-1".to_owned()),
        ("EMPTY".to_owned(), String::new()),
    ]);
    let outcome = match team_callbacks(team_id, &teams, &env) {
        Ok(Some(callbacks)) => {
            let vars: Vec<(&str, &str)> = callbacks
                .callback_vars
                .iter()
                .map(|(name, value)| (name.as_str(), value.expose_secret()))
                .collect();
            format!(
                "{:?}|{:?}|{vars:?}",
                callbacks.success_callback, callbacks.failure_callback
            )
        }
        Ok(None) => "none".to_owned(),
        Err(RequestPrincipalMetadataError::CallbackVariable { name }) => format!("invalid {name}"),
        Err(RequestPrincipalMetadataError::Encode { source }) => panic!("{source}"),
    };
    assert_eq!(outcome, expected);
}

#[rstest]
#[case::session_token_logs_alias(true, Some("alias"), Some("alias"))]
#[case::session_token_without_alias(true, None, Some("hash-1"))]
#[case::key_logs_hash(false, Some("alias"), Some("hash-1"))]
fn logged_key_follows_session_alias(
    #[case] is_session_token: bool,
    #[case] key_alias: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let mut principal = PrincipalFacts::default();
    principal.api_key_hash = Some("hash-1".to_owned());
    principal.is_session_token = is_session_token;
    principal.key_alias = key_alias.map(str::to_owned);
    assert_eq!(logged_api_key(&principal), expected);
}

#[rstest]
#[case::invoked_agent_wins(Some("key-agent"), Some("invoked"), "invoked", "key-agent")]
#[case::key_agent_over_header(Some("key-agent"), None, "key-agent", "key-agent")]
#[case::header_agent_kept(None, None, "header-agent", "null")]
#[case::invoked_bills_without_key_agent(None, Some("invoked"), "invoked", "invoked")]
fn principal_facts_reach_metadata(
    #[case] agent_id: Option<&str>,
    #[case] invoked_agent_id: Option<&str>,
    #[case] expected_agent: &str,
    #[case] expected_billing: &str,
) {
    let mut principal: PrincipalFacts = parsed(
        r#"{"api_key": "hash-1", "key_alias": "alias", "spend": 2.5, "max_budget": 10,
        "user_spend": 1, "user_max_budget": 5, "team_spend": 3, "team_max_budget": 9,
        "end_user_max_budget": 4, "budget_reset_at": "2026-12-01T00:00:00Z", "team_id": "team",
        "team_alias": "Team", "project_id": "proj", "project_alias": "Proj", "org_id": "org",
        "organization_alias": "Org", "user_id": "user", "user_email": "u@example.com",
        "end_user_id": "end", "request_route": "/chat/completions",
        "budget_reservation": {"reserved": 1}, "matched_model_access_groups": ["group"]}"#,
    );
    principal.agent_id = agent_id.map(str::to_owned);
    principal.invoked_agent_id = invoked_agent_id.map(str::to_owned);
    principal.managed_agent_context = Some(ManagedAgentContext {
        agent_id: "managed".to_owned(),
        binding_revision: None,
        mode: ManagedAgentMode::Delegated,
        user_id: Some("human".to_owned()),
        subject_oid: None,
    });
    let mut data: RequestData = parsed(r#"{"metadata": {"agent_id": "header-agent", "kept": 1}}"#);
    add_principal_metadata(&mut data, MetadataField::Metadata, &principal).unwrap();
    let expected = format!(
        r#"{{"kept": 1, "user_api_key_hash": "hash-1", "user_api_key_alias": "alias",
        "user_api_key_spend": 2.5, "user_api_key_max_budget": 10, "user_api_key_user_spend": 1,
        "user_api_key_user_max_budget": 5, "user_api_key_team_spend": 3,
        "user_api_key_team_max_budget": 9, "user_api_key_team_id": "team",
        "user_api_key_project_id": "proj", "user_api_key_project_alias": "Proj",
        "user_api_key_user_id": "user", "user_api_key_org_id": "org", "user_api_key_org_alias": "Org",
        "user_api_key_team_alias": "Team", "user_api_key_end_user_id": "end",
        "user_api_key_user_email": "u@example.com", "user_api_key_request_route": "/chat/completions",
        "user_api_key_budget_reset_at": "2026-12-01T00:00:00+00:00", "user_api_key_auth_metadata": {{}},
        "user_api_key": "hash-1", "agent_id": "{expected_agent}", "actor_agent_id": {actor},
        "target_agent_id": {target}, "billing_agent_id": {billing}, "agent_execution_mode": "delegated",
        "verified_human_user_id": "human", "user_api_end_user_max_budget": 4,
        "user_api_key_budget_reservation": {{"reserved": 1}},
        "user_api_key_matched_model_access_groups": ["group"]}}"#,
        actor = serde_json::to_string(&agent_id).unwrap(),
        target = serde_json::to_string(&invoked_agent_id).unwrap(),
        billing = if expected_billing == "null" {
            "null".to_owned()
        } else {
            format!(r#""{expected_billing}""#)
        },
    );
    assert_eq!(data.get("metadata"), Some(&parsed(&expected)));
}

#[test]
fn principal_without_optional_facts_writes_nulls() {
    let mut data: RequestData = parsed(r#"{"litellm_metadata": "not an object"}"#);
    add_principal_metadata(
        &mut data,
        MetadataField::LitellmMetadata,
        &PrincipalFacts::default(),
    )
    .unwrap();
    let Some(Json::Object(bucket)) = data.get("litellm_metadata") else {
        panic!("the slot is replaced by an object");
    };
    assert_eq!(bucket.get("user_api_key_spend"), Some(&parsed("0")));
    assert_eq!(bucket.get("agent_id"), Some(&Json::Null));
    assert_eq!(bucket.get("agent_execution_mode"), Some(&Json::Null));
    assert_eq!(
        bucket.get("user_api_key_budget_reset_at"),
        Some(&Json::Null)
    );
    assert!(!bucket.contains_key("user_api_key_budget_reservation"));
    assert!(!bucket.contains_key("user_api_key_matched_model_access_groups"));
}

#[rstest]
#[case::cache_object_filtered(
    r#"{"cache": {"ttl": 60, "no-store": true, "namespace": "x"}}"#,
    r#"{"metadata": {"user_api_key_auth_metadata": {"cache": {"ttl": 60, "no-store": true, "namespace": "x"}}}, "cache": {"ttl": 60, "no-store": true}}"#
)]
#[case::cache_not_object_resets(
    r#"{"cache": "on"}"#,
    r#"{"metadata": {"user_api_key_auth_metadata": {"cache": "on"}}, "cache": {}}"#
)]
#[case::boolean_flags_copied(
    r#"{"disable_fallbacks": true, "enable_prompt_caching": false, "disable_global_guardrails": true}"#,
    r#"{"metadata": {"disable_global_guardrails": true, "user_api_key_auth_metadata": {"disable_fallbacks": true}}, "disable_fallbacks": true, "enable_prompt_caching": false}"#
)]
#[case::text_flags_ignored(
    r#"{"disable_fallbacks": "true", "enable_prompt_caching": 1, "disable_global_guardrails": "yes"}"#,
    r#"{"metadata": {"user_api_key_auth_metadata": {"disable_fallbacks": "true"}}}"#
)]
#[case::spend_logs_set(
    r#"{"spend_logs_metadata": {"team": "key"}}"#,
    r#"{"metadata": {"spend_logs_metadata": {"team": "key"}, "user_api_key_auth_metadata": {"spend_logs_metadata": {"team": "key"}}}}"#
)]
#[case::management_fields_dropped(
    r#"{"owner": "ops", "guardrails": ["g"], "end_user_budget_id": "b", "secret_manager_settings": {}, "callback_settings": {}}"#,
    r#"{"metadata": {"user_api_key_auth_metadata": {"owner": "ops"}}}"#
)]
fn key_level_controls_reach_body(#[case] key_metadata: &str, #[case] expected: &str) {
    let mut data: RequestData = parsed(r#"{"metadata": {}}"#);
    add_key_level_controls(&mut data, MetadataField::Metadata, &parsed(key_metadata));
    assert_eq!(data, parsed::<RequestData>(expected));
}

#[rstest]
#[case::caller_entries_win(
    r#"{"spend_logs_metadata": {"team": "caller"}}"#,
    r#"{"spend_logs_metadata": {"team": "caller", "env": "prod"}, "user_api_key_auth_metadata": {"spend_logs_metadata": {"team": "key", "env": "prod"}}}"#
)]
#[case::caller_text_replaced(
    r#"{"spend_logs_metadata": "caller"}"#,
    r#"{"spend_logs_metadata": {"team": "key", "env": "prod"}, "user_api_key_auth_metadata": {"spend_logs_metadata": {"team": "key", "env": "prod"}}}"#
)]
fn key_spend_logs_fill_gaps(#[case] bucket: &str, #[case] expected: &str) {
    let mut data: RequestData = parsed(&format!(r#"{{"metadata": {bucket}}}"#));
    let key_metadata = parsed(r#"{"spend_logs_metadata": {"team": "key", "env": "prod"}}"#);
    add_key_level_controls(&mut data, MetadataField::Metadata, &key_metadata);
    assert_eq!(data.get("metadata"), Some(&parsed(expected)));
}

#[rstest]
#[case::missing_slot_untouched(r#"{"model": "m"}"#, r#"{"model": "m"}"#)]
#[case::existing_entries_extended(
    r#"{"metadata": {"user_api_key_auth_metadata": {"key": "k"}}}"#,
    r#"{"metadata": {"user_api_key_auth_metadata": {"key": "k", "team": "t"}}}"#
)]
#[case::text_entry_replaced(
    r#"{"metadata": {"user_api_key_auth_metadata": "forged"}}"#,
    r#"{"metadata": {"user_api_key_auth_metadata": {"team": "t"}}}"#
)]
fn management_metadata_needs_the_slot(#[case] body: &str, #[case] expected: &str) {
    let mut data: RequestData = parsed(body);
    add_management_metadata(
        &mut data,
        MetadataField::Metadata,
        &parsed(r#"{"team": "t"}"#),
    );
    assert_eq!(data, parsed::<RequestData>(expected));
}
