#![cfg(test)]

use chrono::{DateTime, Utc};
use rstest::rstest;
use serde::de::DeserializeOwned;

use crate::context::ResolvedPolicy;
use crate::pipeline::json_value::JsonValue;
use crate::registry::PolicySource;
use crate::values::{Policy, PolicyGuardrails, PolicyScope};

use super::JsonObject;
use super::attachment::{
    PolicyAttachmentCreateRequest, PolicyAttachmentDBResponse, PolicyAttachmentListResponse,
};
use super::crud::{PolicyConditionRequest, PolicyCreateRequest, PolicyUpdateRequest};
use super::db::{PolicyDBResponse, PolicyListDBResponse};
use super::inspection::{
    PolicyGuardrailsResponse, PolicyInfoResponse, PolicyListResponse, PolicyScopeResponse,
    PolicySummaryItem, PolicyTestResponse,
};
use super::resolution::{
    AttachmentImpactResponse, PipelineTestRequest, PolicyMatchDetail, PolicyResolveRequest,
    PolicyResolveResponse,
};
use super::version::{
    PolicyVersionCompareResponse, PolicyVersionCreateRequest, PolicyVersionListResponse,
    PolicyVersionStatusUpdateRequest,
};

fn parse<T>(text: &str) -> T
where
    T: DeserializeOwned,
{
    serde_json::from_str(text).unwrap()
}

fn json_object(text: &str) -> JsonObject {
    parse(text)
}

fn condition_request() -> PolicyConditionRequest {
    PolicyConditionRequest {
        model: Some("gpt-4.*".to_owned()),
    }
}

#[test]
fn condition_request_defaults_to_no_model() {
    assert_eq!(parse::<PolicyConditionRequest>("{}").model, None);
    assert_eq!(
        parse::<PolicyConditionRequest>(r#"{"model":null}"#).model,
        None
    );
    assert_eq!(
        parse::<PolicyConditionRequest>(r#"{"model":"gpt-4"}"#).model,
        Some("gpt-4".to_owned())
    );
}

#[test]
fn condition_request_ignores_unknown_fields() {
    let request: PolicyConditionRequest = parse(r#"{"model":"m","extra":1}"#);
    assert_eq!(request.model.as_deref(), Some("m"));
}

#[test]
fn create_request_reads_every_field() {
    let request: PolicyCreateRequest = parse(
        r#"{
            "policy_name": "healthcare",
            "inherit": "baseline",
            "description": "HIPAA",
            "guardrails_add": ["hipaa"],
            "guardrails_remove": ["pii"],
            "condition": {"model": "gpt-4.*"},
            "pipeline": {"mode": "pre_call"}
        }"#,
    );
    assert_eq!(request.policy_name, "healthcare");
    assert_eq!(request.inherit.as_deref(), Some("baseline"));
    assert_eq!(request.description.as_deref(), Some("HIPAA"));
    assert_eq!(request.guardrails_add, Some(vec!["hipaa".to_owned()]));
    assert_eq!(request.guardrails_remove, Some(vec!["pii".to_owned()]));
    assert_eq!(request.condition, Some(condition_request()));
    assert_eq!(
        request.pipeline,
        Some(json_object(r#"{"mode":"pre_call"}"#))
    );
}

#[test]
fn create_request_defaults_and_keeps_empty_lists() {
    let minimal: PolicyCreateRequest = parse(r#"{"policy_name":"p"}"#);
    assert_eq!(minimal.inherit, None);
    assert_eq!(minimal.description, None);
    assert_eq!(minimal.guardrails_add, None);
    assert_eq!(minimal.guardrails_remove, None);
    assert_eq!(minimal.condition, None);
    assert_eq!(minimal.pipeline, None);
    let empty: PolicyCreateRequest =
        parse(r#"{"policy_name":"p","guardrails_add":[],"guardrails_remove":[]}"#);
    assert_eq!(empty.guardrails_add, Some(Vec::new()));
    assert_eq!(empty.guardrails_remove, Some(Vec::new()));
}

#[test]
fn create_request_requires_a_policy_name_and_ignores_unknown_fields() {
    serde_json::from_str::<PolicyCreateRequest>("{}").unwrap_err();
    let request: PolicyCreateRequest = parse(r#"{"policy_name":"p","unknown":true}"#);
    assert_eq!(request.policy_name, "p");
}

#[test]
fn update_request_is_partial() {
    let empty: PolicyUpdateRequest = parse("{}");
    assert_eq!(empty.policy_name, None);
    assert_eq!(empty.inherit, None);
    assert_eq!(empty.description, None);
    assert_eq!(empty.guardrails_add, None);
    assert_eq!(empty.guardrails_remove, None);
    assert_eq!(empty.condition, None);
    assert_eq!(empty.pipeline, None);
    let partial: PolicyUpdateRequest = parse(
        r#"{
            "policy_name": "renamed",
            "inherit": "baseline",
            "description": "new",
            "guardrails_add": ["phi"],
            "guardrails_remove": ["pii"],
            "condition": {"model": "gpt-4.*"},
            "pipeline": {"mode": "post_call"}
        }"#,
    );
    assert_eq!(partial.policy_name.as_deref(), Some("renamed"));
    assert_eq!(partial.inherit.as_deref(), Some("baseline"));
    assert_eq!(partial.description.as_deref(), Some("new"));
    assert_eq!(partial.guardrails_add, Some(vec!["phi".to_owned()]));
    assert_eq!(partial.guardrails_remove, Some(vec!["pii".to_owned()]));
    assert_eq!(partial.condition, Some(condition_request()));
    assert_eq!(
        partial.pipeline,
        Some(json_object(r#"{"mode":"post_call"}"#))
    );
}

#[test]
fn update_request_reads_condition_and_pipeline() {
    let request: PolicyUpdateRequest =
        parse(r#"{"condition":{"model":"m"},"pipeline":{"mode":"post_call"}}"#);
    assert_eq!(
        request.condition,
        Some(PolicyConditionRequest {
            model: Some("m".to_owned()),
        })
    );
    assert_eq!(
        request.pipeline,
        Some(json_object(r#"{"mode":"post_call"}"#))
    );
}

#[test]
fn db_response_defaults_every_version_column() {
    let response: PolicyDBResponse = parse(r#"{"policy_id":"p1","policy_name":"baseline"}"#);
    assert_eq!(response.version_number, 1_i32);
    assert_eq!(response.version_status, "production");
    assert_eq!(response.parent_version_id, None);
    assert!(response.is_latest);
    assert_eq!(response.published_at, None);
    assert_eq!(response.production_at, None);
    assert_eq!(response.guardrails_add, Vec::<String>::new());
    assert_eq!(response.guardrails_remove, Vec::<String>::new());
    assert_eq!(response.created_at, None);
    assert_eq!(response.updated_at, None);
    assert_eq!(response.definition_location, PolicySource::Db);
}

#[test]
fn db_response_reads_every_stored_column() {
    let response: PolicyDBResponse = parse(
        r#"{
            "policy_id": "p1",
            "policy_name": "healthcare",
            "version_number": 4,
            "version_status": "draft",
            "parent_version_id": "p0",
            "is_latest": false,
            "published_at": "2024-02-03T04:05:06Z", "production_at": "2024-03-04T05:06:07Z",
            "inherit": "baseline",
            "description": "HIPAA",
            "guardrails_add": ["hipaa"],
            "guardrails_remove": ["pii"],
            "condition": {"model": "gpt-4"},
            "pipeline": {"mode": "pre_call"},
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-02T00:00:00Z",
            "created_by": "alice",
            "updated_by": "bob",
            "definition_location": "config"
        }"#,
    );
    assert_eq!(response.version_number, 4_i32);
    assert_eq!(response.version_status, "draft");
    assert_eq!(response.parent_version_id.as_deref(), Some("p0"));
    assert!(!response.is_latest);
    assert_eq!(
        response.published_at,
        Some("2024-02-03T04:05:06Z".parse::<DateTime<Utc>>().unwrap())
    );
    assert_eq!(response.guardrails_add, ["hipaa"]);
    assert_eq!(
        response.condition,
        Some(json_object(r#"{"model":"gpt-4"}"#))
    );
    assert_eq!(response.created_by.as_deref(), Some("alice"));
    assert_eq!(response.definition_location, PolicySource::Config);
}

#[test]
fn db_response_refuses_a_null_list() {
    let error = serde_json::from_str::<PolicyDBResponse>(
        r#"{"policy_id":"p","policy_name":"n","guardrails_add":null}"#,
    );
    error.unwrap_err();
}

#[test]
fn list_db_response_defaults_to_no_policies() {
    let response: PolicyListDBResponse = parse("{}");
    assert!(response.policies.is_empty());
    assert_eq!(response.total_count, 0);
    let filled: PolicyListDBResponse =
        parse(r#"{"policies":[{"policy_id":"p","policy_name":"n"}],"total_count":1}"#);
    assert_eq!(filled.policies.len(), 1);
    assert_eq!(filled.total_count, 1);
}

#[test]
fn version_create_request_defaults_to_the_production_source() {
    assert_eq!(
        parse::<PolicyVersionCreateRequest>("{}").source_policy_id,
        None
    );
    assert_eq!(
        parse::<PolicyVersionCreateRequest>(r#"{"source_policy_id":"p9"}"#).source_policy_id,
        Some("p9".to_owned())
    );
}

#[test]
fn version_status_update_requires_the_new_status() {
    serde_json::from_str::<PolicyVersionStatusUpdateRequest>("{}").unwrap_err();
    let update: PolicyVersionStatusUpdateRequest = parse(r#"{"version_status":"published"}"#);
    assert_eq!(update.version_status, "published");
}

#[test]
fn version_list_response_defaults_to_no_versions() {
    let response: PolicyVersionListResponse = parse(r#"{"policy_name":"baseline"}"#);
    assert_eq!(response.policy_name, "baseline");
    assert!(response.versions.is_empty());
    assert_eq!(response.total_count, 0);
}

#[test]
fn version_compare_response_reads_field_diffs() {
    let response: PolicyVersionCompareResponse = parse(
        r#"{
            "version_a": {"policy_id": "a", "policy_name": "n"},
            "version_b": {"policy_id": "b", "policy_name": "n"},
            "field_diffs": {"description": {"version_a": "old", "version_b": "new"}}
        }"#,
    );
    assert_eq!(response.version_a.policy_id, "a");
    assert_eq!(response.version_b.policy_id, "b");
    assert_eq!(
        response.field_diffs.get("description"),
        Some(&json_object(r#"{"version_a":"old","version_b":"new"}"#))
    );
    let equal: PolicyVersionCompareResponse = parse(
        r#"{"version_a":{"policy_id":"a","policy_name":"n"},"version_b":{"policy_id":"a","policy_name":"n"}}"#,
    );
    assert!(equal.field_diffs.is_empty());
}

#[test]
fn attachment_create_request_defaults_scope_lists_and_priority() {
    let request: PolicyAttachmentCreateRequest = parse(r#"{"policy_name":"baseline"}"#);
    assert_eq!(request.scope, None);
    assert_eq!(request.teams, None);
    assert_eq!(request.keys, None);
    assert_eq!(request.models, None);
    assert_eq!(request.tags, None);
    assert_eq!(request.priority, None);
    assert!(!request.default);
}

#[test]
fn attachment_create_request_reads_the_scope_wildcard_and_lists() {
    let request: PolicyAttachmentCreateRequest = parse(
        r#"{
            "policy_name": "baseline",
            "scope": "*",
            "teams": ["health-*"],
            "keys": ["key-a"],
            "models": ["gpt-4"],
            "tags": ["health-*"],
            "priority": 3,
            "default": true
        }"#,
    );
    assert_eq!(request.scope.as_deref(), Some("*"));
    assert_eq!(request.teams, Some(vec!["health-*".to_owned()]));
    assert_eq!(request.keys, Some(vec!["key-a".to_owned()]));
    assert_eq!(request.models, Some(vec!["gpt-4".to_owned()]));
    assert_eq!(request.tags, Some(vec!["health-*".to_owned()]));
    assert_eq!(request.priority, Some(3_i32));
    assert!(request.default);
}

#[rstest]
#[case::above_the_maximum("2147483648")]
#[case::below_the_minimum("-2147483649")]
fn attachment_create_request_refuses_a_priority_over_the_i32_range(#[case] priority: &str) {
    let text = format!(r#"{{"policy_name":"p","priority":{priority}}}"#);
    serde_json::from_str::<PolicyAttachmentCreateRequest>(&text).unwrap_err();
}

#[rstest]
#[case::at_the_maximum("2147483647")]
#[case::at_the_minimum("-2147483648")]
fn attachment_create_request_accepts_a_priority_inside_the_i32_range(#[case] priority: &str) {
    let text = format!(r#"{{"policy_name":"p","priority":{priority}}}"#);
    let request: PolicyAttachmentCreateRequest = parse(&text);
    assert_eq!(request.priority, Some(priority.parse::<i32>().unwrap()));
}

#[test]
fn attachment_db_response_defaults_lists_and_location() {
    let response: PolicyAttachmentDBResponse =
        parse(r#"{"attachment_id":"a1","policy_name":"baseline"}"#);
    assert_eq!(response.scope, None);
    assert_eq!(response.teams, Vec::<String>::new());
    assert_eq!(response.priority, None);
    assert!(!response.default);
    assert_eq!(response.created_at, None);
    assert_eq!(response.definition_location, PolicySource::Db);
}

#[test]
fn attachment_db_response_reads_stored_columns() {
    let response: PolicyAttachmentDBResponse = parse(
        r#"{
            "attachment_id": "a1",
            "policy_name": "baseline",
            "scope": "*",
            "teams": ["t"],
            "keys": [],
            "models": ["m"],
            "tags": ["g"],
            "priority": -1,
            "default": true,
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-02T00:00:00Z",
            "created_by": "alice",
            "updated_by": "bob",
            "definition_location": "config"
        }"#,
    );
    assert_eq!(response.teams, ["t"]);
    assert!(response.keys.is_empty());
    assert_eq!(response.priority, Some(-1_i32));
    assert!(response.default);
    assert_eq!(
        response.created_at,
        Some("2024-01-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap())
    );
    assert_eq!(response.definition_location, PolicySource::Config);
}

#[test]
fn attachment_list_response_defaults_to_no_attachments() {
    let response: PolicyAttachmentListResponse = parse("{}");
    assert!(response.attachments.is_empty());
    assert_eq!(response.total_count, 0);
}

#[test]
fn scope_response_defaults_to_four_empty_lists() {
    let response: PolicyScopeResponse = parse("{}");
    assert_eq!(response.teams, Vec::<String>::new());
    assert_eq!(response.keys, Vec::<String>::new());
    assert_eq!(response.models, Vec::<String>::new());
    assert_eq!(response.tags, Vec::<String>::new());
    assert_eq!(
        serde_json::to_string(&response).unwrap(),
        r#"{"teams":[],"keys":[],"models":[],"tags":[]}"#
    );
}

#[test]
fn scope_response_projects_the_accessor_defaults() {
    let empty = PolicyScope {
        teams: None,
        keys: Some(Vec::new()),
        models: None,
        tags: Some(Vec::new()),
    };
    let projected = PolicyScopeResponse::from_scope(&empty);
    assert_eq!(projected.teams, ["*"]);
    assert_eq!(projected.keys, ["*"]);
    assert_eq!(projected.models, ["*"]);
    assert!(projected.tags.is_empty());
    let filled = PolicyScope {
        teams: Some(vec!["team-a".to_owned()]),
        keys: Some(vec!["key-a".to_owned()]),
        models: Some(vec!["gpt-4".to_owned()]),
        tags: Some(vec!["health".to_owned()]),
    };
    assert_eq!(
        PolicyScopeResponse::from_scope(&filled),
        PolicyScopeResponse {
            teams: vec!["team-a".to_owned()],
            keys: vec!["key-a".to_owned()],
            models: vec!["gpt-4".to_owned()],
            tags: vec!["health".to_owned()],
        }
    );
}

#[test]
fn guardrails_response_projects_add_and_remove() {
    let empty = PolicyGuardrails::default();
    let projected = PolicyGuardrailsResponse::from_guardrails(&empty);
    assert!(projected.add.is_empty());
    assert!(projected.remove.is_empty());
    let filled = PolicyGuardrails {
        add: Some(vec!["hipaa".to_owned()]),
        remove: Some(Vec::new()),
    };
    assert_eq!(
        PolicyGuardrailsResponse::from_guardrails(&filled),
        PolicyGuardrailsResponse {
            add: vec!["hipaa".to_owned()],
            remove: Vec::new(),
        }
    );
}

fn policy_with_guardrails() -> Policy {
    Policy {
        inherit: Some("baseline".to_owned()),
        description: Some("HIPAA".to_owned()),
        guardrails: PolicyGuardrails {
            add: Some(vec!["hipaa".to_owned()]),
            remove: Some(vec!["pii".to_owned()]),
        },
        condition: None,
        pipeline: None,
    }
}

fn resolved() -> ResolvedPolicy {
    ResolvedPolicy {
        policy_name: "healthcare".to_owned(),
        guardrails: vec!["hipaa".to_owned(), "pii".to_owned()],
        inheritance_chain: vec!["baseline".to_owned(), "healthcare".to_owned()],
    }
}

#[test]
fn info_response_projects_the_policy_with_an_empty_scope() {
    let response =
        PolicyInfoResponse::from_policy("healthcare", &policy_with_guardrails(), &resolved());
    assert_eq!(response.policy_name, "healthcare");
    assert_eq!(response.inherit.as_deref(), Some("baseline"));
    assert_eq!(response.scope, PolicyScopeResponse::default());
    assert_eq!(response.guardrails.add, ["hipaa"]);
    assert_eq!(response.guardrails.remove, ["pii"]);
    assert_eq!(response.resolved_guardrails, ["hipaa", "pii"]);
    assert_eq!(response.inheritance_chain, ["baseline", "healthcare"]);
}

#[test]
fn info_response_keeps_a_missing_inherit_null() {
    let mut policy = policy_with_guardrails();
    policy.inherit = None;
    let response = PolicyInfoResponse::from_policy("healthcare", &policy, &resolved());
    assert_eq!(response.inherit, None);
}

#[test]
fn summary_item_requires_scope_and_guardrails() {
    let minimal = r#"{
        "scope": {},
        "guardrails": {},
        "resolved_guardrails": [],
        "inheritance_chain": []
    }"#;
    let item: PolicySummaryItem = parse(minimal);
    assert_eq!(item.inherit, None);
    assert_eq!(item.scope, PolicyScopeResponse::default());
    let missing_scope = r#"{"guardrails":{},"resolved_guardrails":[],"inheritance_chain":[]}"#;
    serde_json::from_str::<PolicySummaryItem>(missing_scope).unwrap_err();
}

#[test]
fn list_response_keeps_policy_names_as_keys() {
    let response: PolicyListResponse = parse(
        r#"{
            "policies": {
                "healthcare": {
                    "inherit": "baseline",
                    "scope": {},
                    "guardrails": {"add": ["hipaa"]},
                    "resolved_guardrails": ["hipaa"],
                    "inheritance_chain": ["baseline", "healthcare"]
                }
            },
            "total_count": 1
        }"#,
    );
    assert_eq!(response.policies.keys().collect::<Vec<_>>(), ["healthcare"]);
    assert_eq!(response.total_count, 1);
    serde_json::from_str::<PolicyListResponse>(r#"{"policies":{}}"#).unwrap_err();
}

#[test]
fn response_echoes_the_context_and_defaults_message() {
    let response: PolicyTestResponse = parse(
        r#"{
            "context": {"team_alias": "t", "key_alias": null, "model": "gpt-4", "tags": ["a"]},
            "matching_policies": ["healthcare"],
            "resolved_guardrails": ["hipaa"]
        }"#,
    );
    assert_eq!(response.context.model.as_deref(), Some("gpt-4"));
    assert_eq!(response.matching_policies, ["healthcare"]);
    assert_eq!(response.message, None);
    let message: PolicyTestResponse = parse(
        r#"{"context":{},"matching_policies":[],"resolved_guardrails":[],"message":"none loaded"}"#,
    );
    assert_eq!(message.message.as_deref(), Some("none loaded"));
    let extra_context =
        r#"{"context":{"unknown":1},"matching_policies":[],"resolved_guardrails":[]}"#;
    serde_json::from_str::<PolicyTestResponse>(extra_context).unwrap_err();
}

#[test]
fn pipeline_test_request_requires_pipeline_and_messages() {
    serde_json::from_str::<PipelineTestRequest>("{}").unwrap_err();
    serde_json::from_str::<PipelineTestRequest>(r#"{"pipeline":{}}"#).unwrap_err();
    let request: PipelineTestRequest = parse(
        r#"{
            "pipeline": {"mode": "pre_call", "steps": [{"guardrail": "hipaa"}]},
            "test_messages": [{"role": "user", "content": "hello"}]
        }"#,
    );
    assert_eq!(
        request.pipeline.get("mode"),
        Some(&JsonValue::String("pre_call".to_owned()))
    );
    assert_eq!(
        request.test_messages[0].get("role").map(String::as_str),
        Some("user")
    );
}

#[test]
fn resolve_request_defaults_and_ignores_unknown_fields() {
    let empty: PolicyResolveRequest = parse("{}");
    assert_eq!(empty.team_alias, None);
    assert_eq!(empty.key_alias, None);
    assert_eq!(empty.model, None);
    assert_eq!(empty.tags, None);
    let filled: PolicyResolveRequest =
        parse(r#"{"team_alias":"t","key_alias":"k","model":"m","tags":["a"],"extra":1}"#);
    assert_eq!(filled.team_alias.as_deref(), Some("t"));
    assert_eq!(filled.key_alias.as_deref(), Some("k"));
    assert_eq!(filled.model.as_deref(), Some("m"));
    assert_eq!(filled.tags, Some(vec!["a".to_owned()]));
}

#[test]
fn match_detail_defaults_guardrails_added() {
    let detail: PolicyMatchDetail = parse(r#"{"policy_name":"p","matched_via":"tag:health"}"#);
    assert_eq!(detail.matched_via, "tag:health");
    assert!(detail.guardrails_added.is_empty());
}

#[test]
fn resolve_response_defaults_to_no_matches() {
    let response: PolicyResolveResponse = parse("{}");
    assert!(response.effective_guardrails.is_empty());
    assert!(response.matched_policies.is_empty());
    let filled: PolicyResolveResponse = parse(
        r#"{"effective_guardrails":["hipaa"],"matched_policies":[{"policy_name":"p","matched_via":"scope:*"}]}"#,
    );
    assert_eq!(filled.effective_guardrails, ["hipaa"]);
    assert_eq!(filled.matched_policies[0].policy_name, "p");
}

#[test]
fn impact_response_defaults_counts_and_keeps_negative_ones() {
    let empty: AttachmentImpactResponse = parse("{}");
    assert_eq!(empty.affected_keys_count, 0);
    assert_eq!(empty.affected_teams_count, 0);
    assert_eq!(empty.unnamed_keys_count, 0);
    assert_eq!(empty.unnamed_teams_count, 0);
    assert!(empty.sample_keys.is_empty());
    assert!(empty.sample_teams.is_empty());
    let global: AttachmentImpactResponse = parse(
        r#"{"affected_keys_count":-1,"affected_teams_count":-1,"sample_keys":["*"],"sample_teams":["*"]}"#,
    );
    assert_eq!(global.affected_keys_count, -1);
    assert_eq!(global.sample_keys, ["*"]);
}
