use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_repository::config::ConfigParam;
use betterlite_db::team_rows::member::BudgetLimitEntry;
use betterlite_logging::private_json::error::PrivateJsonError;
use chrono::{DateTime, Utc};
use rstest::rstest;
use serde::de::DeserializeOwned;

use super::adapters::{KeyListUserRow, LegacyDumpable, env_vars_param_value};
use super::base::{GenerateRequestBase, KeyRequestBase};
use super::broadcast::{BROADCAST_FIELDS, KeyUpdateFields};
use super::bulk::{BulkUpdateKeyRequest, BulkUpdateKeyResponse, BulkUpdateTeamKeysRequest};
use super::decode::{TypedValue, decode_key_request, decode_key_response, to_json};
use super::error::DecodeError;
use super::policy::{CustomKeyPolicyRequest, PolicyRequestBody};
use super::regenerate::RegenerateKeyRequest;
use super::request::GenerateKeyRequest;
use super::response::GenerateKeyResponse;
use super::shared::{hash_token_if_needed, with_settings_updated_at};
use super::update::UpdateKeyRequest;
use super::values::{IndexPermission, InsensitiveContains, LimitType, LiteLLMKeyType, MatchMode};
use super::windows::{stored_budget_limit_windows, typed_budget_limit_windows};

fn value(text: &str) -> TypedValue {
    decode_key_request(text.as_bytes()).expect("a JSON object document")
}

fn json_data(text: &str) -> JsonData {
    serde_json::from_str(text).expect("fixture JSON")
}

fn object(text: &str) -> BTreeMap<String, JsonData> {
    let JsonData::Object(fields) = json_data(text) else {
        panic!("the fixture is an object");
    };
    fields
}

fn decode_value<T: DeserializeOwned>(value: &JsonData) -> Result<T, DecodeError> {
    serde_path_to_error::deserialize(serde_json::to_value(value).map_err(DecodeError::Document)?)
        .map_err(DecodeError::Fields)
}

fn assert_field(fields: &BTreeMap<String, JsonData>, name: &str, text: &str) {
    assert_eq!(fields.get(name), Some(&json_data(text)), "{name}");
}

fn assert_null(fields: &BTreeMap<String, JsonData>, names: &[&str]) {
    let wired: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| fields.get(*name) != Some(&JsonData::Null))
        .collect();
    assert!(wired.is_empty(), "{wired:?}");
}

#[test]
fn base_field_defaults() {
    let base = GenerateRequestBase::read(&value("{}")).expect("the base defaults");
    let resolved = base.project().expect("the projection").resolved;
    assert_eq!(resolved.len(), 29);
    assert_field(&resolved, "models", "[]");
    assert_field(&resolved, "allowed_cache_controls", "[]");
    assert_field(&resolved, "spend", "0.0");
    assert_field(&resolved, "metadata", "{}");
    assert_field(&resolved, "config", "{}");
    assert_field(&resolved, "permissions", "{}");
    assert_field(&resolved, "model_max_budget", "{}");
    assert_field(&resolved, "aliases", "{}");
    assert_null(
        &resolved,
        &[
            "key_alias",
            "duration",
            "max_budget",
            "user_id",
            "team_id",
            "agent_id",
            "max_parallel_requests",
            "tpm_limit",
            "rpm_limit",
            "budget_duration",
            "budget_limits",
            "budget_fallbacks",
            "model_rpm_limit",
            "model_tpm_limit",
            "mcp_rpm_limit",
            "tag_rpm_limit",
            "guardrails",
            "policies",
            "prompts",
            "blocked",
            "object_permission",
        ],
    );
    let explicit = GenerateRequestBase::read(&value(
        r#"{"team_id": null, "models": null, "metadata": null, "spend": null, "object_permission": null}"#,
    ))
    .expect("explicit nulls");
    assert_eq!(
        explicit.project().expect("the projection").supplied,
        object(
            r#"{"team_id": null, "models": null, "metadata": null, "spend": null, "object_permission": null}"#
        )
    );
    let windows = GenerateRequestBase::read(&value(
        r#"{"budget_limits": [{"budget_duration": "1d", "max_budget": 2.5}]}"#,
    ))
    .expect("a budget window");
    let projection = windows.project().expect("the projection");
    assert_field(
        &projection.supplied,
        "budget_limits",
        r#"[{"budget_duration": "1d", "max_budget": 2.5}]"#,
    );
    assert_field(
        &projection.resolved,
        "budget_limits",
        r#"[{"budget_duration": "1d", "max_budget": 2.5, "reset_at": null}]"#,
    );
}

#[test]
fn blank_max_budget() {
    let request =
        GenerateKeyRequest::read(&value(r#"{"max_budget": ""}"#)).expect("a blank budget");
    assert_eq!(request.base.base.max_budget, Some(None));
    assert_eq!(
        request.project().expect("the projection").supplied,
        object(r#"{"max_budget": null}"#)
    );
}

#[test]
fn object_permission_defaults() {
    let base =
        GenerateRequestBase::read(&value(r#"{"object_permission": {}}"#)).expect("a permission");
    let permission = base
        .object_permission
        .flatten()
        .expect("the permission model");
    assert_eq!(permission.mcp_servers, None);
    assert_eq!(permission.mcp_access_groups, None);
    assert_eq!(permission.mcp_tool_permissions, None);
    assert_eq!(permission.mcp_toolsets, None);
    assert_eq!(permission.blocked_tools, None);
    assert_eq!(permission.vector_stores, None);
    assert_eq!(permission.agents, None);
    assert_eq!(permission.agent_access_groups, None);
    assert_eq!(permission.models, None);
    assert_eq!(permission.search_tools, None);
    assert_eq!(permission.mcp_tool_search_enabled, None);
    assert_eq!(permission.skills, None);
    let supplied = GenerateRequestBase::read(&value(
        r#"{"object_permission": {"models": [], "mcp_servers": [], "mcp_tool_permissions": {}}}"#,
    ))
    .expect("supplied empties");
    let projection = supplied.project().expect("the projection");
    assert_field(
        &projection.supplied,
        "object_permission",
        r#"{"mcp_servers": [], "mcp_tool_permissions": {}, "models": []}"#,
    );
    assert_field(
        &projection.resolved,
        "object_permission",
        concat!(
            r#"{"mcp_servers": [], "mcp_access_groups": null, "mcp_tool_permissions": {},"#,
            r#" "mcp_toolsets": null, "blocked_tools": null, "vector_stores": null, "agents": null,"#,
            r#" "agent_access_groups": null, "models": [], "search_tools": null,"#,
            r#" "mcp_tool_search_enabled": null, "skills": null}"#
        ),
    );
}

#[test]
fn key_object_permission() {
    let request =
        GenerateKeyRequest::read(&value(r#"{"object_permission": {"models": ["gpt-4o"]}}"#))
            .expect("a key permission");
    let projection = request.project().expect("the projection");
    assert_field(
        &projection.supplied,
        "object_permission",
        r#"{"models": ["gpt-4o"]}"#,
    );
    assert_field(
        &projection.resolved,
        "object_permission",
        concat!(
            r#"{"mcp_servers": null, "mcp_access_groups": null, "mcp_tool_permissions": null,"#,
            r#" "mcp_toolsets": null, "blocked_tools": null, "vector_stores": null, "agents": null,"#,
            r#" "agent_access_groups": null, "models": ["gpt-4o"], "search_tools": null,"#,
            r#" "mcp_tool_search_enabled": null, "skills": null}"#
        ),
    );
}

#[test]
fn key_field_defaults() {
    let request = KeyRequestBase::read(&value("{}")).expect("the key defaults");
    let fields = &request.key_fields;
    assert_eq!(fields.allowed_routes, Some(Some(Vec::new())));
    let set = [
        fields.key.is_some(),
        fields.tpd_limit.is_some(),
        fields.estimated_output_tokens.is_some(),
        fields.per_model_output_tokens.is_some(),
        fields.budget_id.is_some(),
        fields.end_user_budget_id.is_some(),
        fields.tags.is_some(),
        fields.disable_global_guardrails.is_some(),
        fields.enable_prompt_caching.is_some(),
        fields.throttle_on_budget_exceeded.is_some(),
        fields.enforced_params.is_some(),
        fields.allowed_passthrough_routes.is_some(),
        fields.allowed_vector_store_indexes.is_some(),
        fields.rpm_limit_type.is_some(),
        fields.tpm_limit_type.is_some(),
        fields.router_settings.is_some(),
        fields.access_group_ids.is_some(),
    ];
    assert!(!set.iter().any(|present| *present));
    let resolved = request.project().expect("the projection").resolved;
    assert_field(&resolved, "allowed_routes", "[]");
    assert_null(
        &resolved,
        &[
            "key",
            "tpd_limit",
            "default_estimated_output_tokens",
            "default_estimated_output_tokens_per_model",
            "budget_id",
            "end_user_budget_id",
            "tags",
            "disable_global_guardrails",
            "enable_prompt_caching",
            "throttle_on_budget_exceeded",
            "enforced_params",
            "allowed_passthrough_routes",
            "allowed_vector_store_indexes",
            "rpm_limit_type",
            "tpm_limit_type",
            "router_settings",
            "access_group_ids",
        ],
    );
}

#[test]
fn positive_output_tokens() {
    assert!(matches!(
        KeyRequestBase::read(&value(r#"{"default_estimated_output_tokens": 0}"#)),
        Err(DecodeError::NotPositive { .. })
    ));
    assert!(matches!(
        KeyRequestBase::read(&value(r#"{"default_estimated_output_tokens": -2}"#)),
        Err(DecodeError::NotPositive { .. })
    ));
    let ok = KeyRequestBase::read(&value(r#"{"default_estimated_output_tokens": 5}"#))
        .expect("a positive estimate");
    assert_eq!(ok.key_fields.estimated_output_tokens, Some(Some(5)));
    let Err(DecodeError::NotPositive { field }) = KeyRequestBase::read(&value(
        r#"{"default_estimated_output_tokens_per_model": {"gpt-4": 0}}"#,
    )) else {
        panic!("a zero per-model estimate");
    };
    assert_eq!(field, "default_estimated_output_tokens_per_model.gpt-4");
    let per_model = KeyRequestBase::read(&value(
        r#"{"default_estimated_output_tokens_per_model": {"gpt-4": -1, "gpt-4o": 5}}"#,
    ));
    assert!(matches!(per_model, Err(DecodeError::NotPositive { .. })));
    let positive = KeyRequestBase::read(&value(
        r#"{"default_estimated_output_tokens_per_model": {"gpt-4o": 5}}"#,
    ))
    .expect("a positive per-model estimate");
    assert_eq!(
        positive.key_fields.per_model_output_tokens,
        Some(Some(BTreeMap::from([("gpt-4o".to_owned(), 5)])))
    );
}

#[test]
fn throughput_literals() {
    let request = KeyRequestBase::read(&value(
        r#"{"rpm_limit_type": "guaranteed_throughput", "tpm_limit_type": "best_effort_throughput"}"#,
    ))
    .expect("throughput types");
    assert_eq!(
        request.key_fields.rpm_limit_type,
        Some(Some(LimitType::GuaranteedThroughput))
    );
    assert_eq!(
        request.key_fields.tpm_limit_type,
        Some(Some(LimitType::BestEffortThroughput))
    );
    let dynamic =
        KeyRequestBase::read(&value(r#"{"rpm_limit_type": "dynamic"}"#)).expect("the dynamic type");
    assert_eq!(
        dynamic.key_fields.rpm_limit_type,
        Some(Some(LimitType::Dynamic))
    );
    assert!(KeyRequestBase::read(&value(r#"{"rpm_limit_type": "guaranteed"}"#)).is_err());
    assert!(KeyRequestBase::read(&value(r#"{"tpm_limit_type": "elAstic"}"#)).is_err());
}

#[test]
fn vector_permissions() {
    let request = KeyRequestBase::read(&value(
        r#"{"allowed_vector_store_indexes": [{"index_name": "idx", "index_permissions": ["read", "write"]}]}"#,
    ))
    .expect("a vector index");
    let indexes = request
        .key_fields
        .allowed_vector_store_indexes
        .expect("supplied indexes")
        .expect("a list");
    assert_eq!(indexes[0].index_name, "idx");
    assert_eq!(
        indexes[0].index_permissions,
        vec![IndexPermission::Read, IndexPermission::Write]
    );
    assert!(
        KeyRequestBase::read(&value(
            r#"{"allowed_vector_store_indexes": [{"index_permissions": ["read"]}]}"#
        ))
        .is_err()
    );
    assert!(KeyRequestBase::read(&value(
        r#"{"allowed_vector_store_indexes": [{"index_name": "idx", "index_permissions": ["admin"]}]}"#
    ))
    .is_err());
}

#[test]
fn generate_defaults() {
    let request = GenerateKeyRequest::read(&value("{}")).expect("the generate defaults");
    assert_eq!(request.generate.soft_budget, None);
    assert_eq!(request.generate.send_invite_email, None);
    assert_eq!(request.generate.rotation_interval, None);
    assert_eq!(request.generate.organization_id, None);
    assert_eq!(request.generate.project_id, None);
    assert_eq!(
        request.generate.key_type,
        Some(Some(LiteLLMKeyType::Default))
    );
    assert_eq!(request.generate.auto_rotate, Some(Some(false)));
    let resolved = request.project().expect("the projection").resolved;
    assert_eq!(resolved.len(), 54);
    assert_field(&resolved, "key_type", r#""default""#);
    assert_field(&resolved, "auto_rotate", "false");
    assert_null(
        &resolved,
        &["soft_budget", "send_invite_email", "rotation_interval"],
    );
}

#[test]
fn key_type_literals() {
    let llm = GenerateKeyRequest::read(&value(r#"{"key_type": "llm_api"}"#)).expect("llm_api");
    assert_eq!(llm.generate.key_type, Some(Some(LiteLLMKeyType::LlmApi)));
    let management =
        GenerateKeyRequest::read(&value(r#"{"key_type": "management"}"#)).expect("management");
    assert_eq!(
        management.generate.key_type,
        Some(Some(LiteLLMKeyType::Management))
    );
    let read_only =
        GenerateKeyRequest::read(&value(r#"{"key_type": "read_only"}"#)).expect("read_only");
    assert_eq!(
        read_only.generate.key_type,
        Some(Some(LiteLLMKeyType::ReadOnly))
    );
    let default = GenerateKeyRequest::read(&value(r#"{"key_type": "default"}"#)).expect("default");
    assert_eq!(
        default.generate.key_type,
        Some(Some(LiteLLMKeyType::Default))
    );
    assert!(GenerateKeyRequest::read(&value(r#"{"key_type": "admin"}"#)).is_err());
}

#[test]
fn generate_cleared_ids() {
    let request = GenerateKeyRequest::read(&value(
        r#"{"team_id": "", "organization_id": "", "project_id": ""}"#,
    ))
    .expect("cleared identifiers");
    assert_eq!(
        request.project().expect("the projection").supplied,
        object(r#"{"team_id": null, "organization_id": null, "project_id": null}"#)
    );
}

#[test]
fn update_defaults() {
    let request = UpdateKeyRequest::read(&value(r#"{"key": "k"}"#)).expect("the update defaults");
    assert_eq!(request.update.duration, None);
    assert_eq!(request.update.spend, None);
    assert_eq!(request.update.soft_budget, None);
    assert_eq!(request.update.metadata, None);
    assert_eq!(request.update.temp_budget_increase, None);
    assert_eq!(request.update.temp_budget_expiry, None);
    assert_eq!(request.update.auto_rotate, None);
    assert_eq!(request.update.rotation_interval, None);
    assert_eq!(request.update.organization_id, None);
    assert_eq!(request.update.project_id, None);
    let projection = request.project().expect("the projection");
    assert_eq!(projection.resolved.len(), 54);
    assert_null(
        &projection.resolved,
        &[
            "duration",
            "spend",
            "soft_budget",
            "metadata",
            "temp_budget_increase",
        ],
    );
    assert_null(&projection.resolved, &["temp_budget_expiry", "auto_rotate"]);
}

#[test]
fn update_blank_team() {
    let request =
        UpdateKeyRequest::read(&value(r#"{"key": "k", "team_id": ""}"#)).expect("a blank team");
    assert_eq!(
        request.project().expect("the projection").supplied,
        object(r#"{"key": "k"}"#)
    );
    let explicit = UpdateKeyRequest::read(&value(r#"{"key": "k", "team_id": null}"#))
        .expect("an explicit null team");
    assert_eq!(
        explicit.project().expect("the projection").supplied,
        object(r#"{"key": "k", "team_id": null}"#)
    );
}

#[test]
fn update_blank_org() {
    let request = UpdateKeyRequest::read(&value(r#"{"key": "k", "organization_id": ""}"#))
        .expect("a blank organization");
    assert_eq!(
        request.project().expect("the projection").supplied,
        object(r#"{"key": "k", "organization_id": null}"#)
    );
}

#[test]
fn update_project_null() {
    let request = UpdateKeyRequest::read(&value(r#"{"key": "k", "project_id": null}"#))
        .expect("an explicit null project");
    assert_eq!(
        request.project().expect("the projection").supplied,
        object(r#"{"key": "k", "project_id": null}"#)
    );
    let blank = UpdateKeyRequest::read(&value(r#"{"key": "k", "project_id": ""}"#))
        .expect("a blank project");
    assert_eq!(
        blank.project().expect("the projection").supplied,
        object(r#"{"key": "k", "project_id": ""}"#)
    );
}

#[test]
fn update_identifier() {
    assert!(matches!(
        UpdateKeyRequest::read(&value("{}")),
        Err(DecodeError::MissingKeyIdentifier)
    ));
    assert!(matches!(
        UpdateKeyRequest::read(&value(r#"{"key": null, "key_alias": null}"#)),
        Err(DecodeError::MissingKeyIdentifier)
    ));
    assert!(UpdateKeyRequest::read(&value(r#"{"key": ""}"#)).is_ok());
    assert!(UpdateKeyRequest::read(&value(r#"{"key_alias": ""}"#)).is_ok());
}

#[test]
fn temporary_budget_pair() {
    assert!(matches!(
        UpdateKeyRequest::read(&value(r#"{"key": "k", "temp_budget_increase": 1.5}"#)),
        Err(DecodeError::TemporaryBudgetPair)
    ));
    assert!(matches!(
        UpdateKeyRequest::read(&value(
            r#"{"key": "k", "temp_budget_expiry": "2030-01-01T00:00:00"}"#
        )),
        Err(DecodeError::TemporaryBudgetPair)
    ));
    assert!(
        UpdateKeyRequest::read(&value(r#"{"key": "k", "temp_budget_increase": null}"#)).is_ok()
    );
    assert!(matches!(
        KeyUpdateFields::read(&value(r#"{"temp_budget_increase": 1.5}"#)),
        Err(DecodeError::TemporaryBudgetPair)
    ));
    assert!(UpdateKeyRequest::read(&value(
        r#"{"key": "k", "temp_budget_increase": -1.5, "temp_budget_expiry": "2020-01-01T00:00:00"}"#
    ))
    .is_ok());
}

#[test]
fn temporary_budget_expiry() {
    let request = UpdateKeyRequest::read(&value(
        r#"{"key": "k", "temp_budget_increase": 2.5, "temp_budget_expiry": "2030-01-01T00:00:00"}"#,
    ))
    .expect("a temporary grant");
    let projection = request.project().expect("the projection");
    assert_field(
        &projection.supplied,
        "temp_budget_expiry",
        r#""2030-01-01T00:00:00""#,
    );
    assert_field(&projection.resolved, "temp_budget_increase", "2.5");
    assert_field(
        &projection.resolved,
        "temp_budget_expiry",
        r#""2030-01-01T00:00:00""#,
    );
}

#[test]
fn regenerate_defaults() {
    let request = RegenerateKeyRequest::read(&value("{}")).expect("the regenerate defaults");
    assert_eq!(request.regenerate.key, None);
    assert_eq!(request.regenerate.new_key, None);
    assert_eq!(request.regenerate.duration, None);
    assert_eq!(request.regenerate.spend, None);
    assert_eq!(request.regenerate.metadata, None);
    assert_eq!(request.regenerate.new_master_key, None);
    assert_eq!(request.regenerate.grace_period, None);
    let resolved = request.project().expect("the projection").resolved;
    assert_eq!(resolved.len(), 57);
    assert_null(
        &resolved,
        &["spend", "new_key", "new_master_key", "grace_period"],
    );
    assert_field(&resolved, "key_type", r#""default""#);
    assert_field(&resolved, "soft_budget", "null");
}

#[test]
fn response_token_wins() {
    let response = GenerateKeyResponse::read(&value(r#"{"key": "k", "token": "t"}"#))
        .expect("a response with a token");
    assert_eq!(response.response.token, Some(Some("t".to_owned())));
    assert_eq!(response.response.key, Some(Some("t".to_owned())));
    let emptied =
        GenerateKeyResponse::read(&value(r#"{"key": "k", "token": ""}"#)).expect("an empty token");
    assert_eq!(emptied.response.key, Some(Some(String::new())));
    assert!(matches!(
        GenerateKeyResponse::read(&value(r#"{"token": null}"#)),
        Err(DecodeError::MissingResponseKey)
    ));
}

#[test]
fn response_json_fields() {
    let response = GenerateKeyResponse::read(&value(concat!(
        r#"{"key": "k", "metadata": "{\"a\": 1}", "aliases": "{}", "config": "{}","#,
        r#" "permissions": "{}", "model_max_budget": "{}","#,
        r#" "budget_fallbacks": "{\"gpt-4\": [\"gpt-4o\"]}","#,
        r#" "router_settings": "{\"num_retries\": 2}","#,
        r#" "budget_limits": "[{\"budget_duration\": \"1d\", \"max_budget\": 2.5}]"}"#
    )))
    .expect("stored JSON strings");
    let projection = response.project().expect("the projection");
    assert_field(
        &projection.supplied,
        "budget_limits",
        r#"[{"budget_duration": "1d", "max_budget": 2.5}]"#,
    );
    let resolved = projection.resolved;
    assert_eq!(resolved.len(), 59);
    assert_field(&resolved, "metadata", r#"{"a": 1}"#);
    assert_field(&resolved, "aliases", "{}");
    assert_field(&resolved, "config", "{}");
    assert_field(&resolved, "permissions", "{}");
    assert_field(&resolved, "model_max_budget", "{}");
    assert_field(&resolved, "budget_fallbacks", r#"{"gpt-4": ["gpt-4o"]}"#);
    assert_field(&resolved, "router_settings", r#"{"num_retries": 2}"#);
    assert_field(
        &resolved,
        "budget_limits",
        r#"[{"budget_duration": "1d", "max_budget": 2.5, "reset_at": null}]"#,
    );
    assert_null(
        &resolved,
        &[
            "key_name",
            "key_type",
            "expires",
            "token_id",
            "litellm_budget_table",
            "created_by",
        ],
    );
    assert_null(
        &resolved,
        &[
            "updated_by",
            "created_at",
            "updated_at",
            "organization_id",
            "project_id",
            "user_id",
        ],
    );
    let Err(DecodeError::InvalidDictionary { field, .. }) =
        GenerateKeyResponse::read(&value(r#"{"key": "k", "metadata": "{bad"}"#))
    else {
        panic!("malformed metadata");
    };
    assert_eq!(field, "metadata");
    let Err(DecodeError::InvalidDictionary { field, .. }) =
        GenerateKeyResponse::read(&value(r#"{"key": "k", "budget_limits": "{bad"}"#))
    else {
        panic!("malformed budget_limits");
    };
    assert_eq!(field, "budget_limits");
    let nulled = GenerateKeyResponse::read(&value(
        r#"{"key": "k", "metadata": "null", "budget_limits": "null"}"#,
    ))
    .expect("valid JSON nulls");
    let nulled_projection = nulled.project().expect("the projection");
    assert_null(&nulled_projection.supplied, &["metadata", "budget_limits"]);
    assert_null(&nulled_projection.resolved, &["metadata", "budget_limits"]);
    assert!(matches!(
        GenerateKeyResponse::read(&value(r#"{"key": "k", "metadata": "[]"}"#)),
        Err(DecodeError::Fields(_))
    ));
}

#[test]
fn bulk_item_fields() {
    let request: BulkUpdateKeyRequest =
        serde_json::from_str(r#"{"keys": [{"key": "k", "max_budget": 5.0, "extra": 1}]}"#)
            .expect("a bulk request");
    let item = &request.keys[0];
    assert_eq!(item.key, "k");
    assert_eq!(item.budget_id, None);
    assert_eq!(item.team_id, None);
    assert_eq!(
        to_json(item).expect("the item dump"),
        json_data(
            r#"{"key": "k", "budget_id": null, "max_budget": 5.0, "team_id": null, "tags": null, "object_permission": null}"#
        )
    );
    let missing: Result<BulkUpdateKeyRequest, _> =
        serde_json::from_str(r#"{"keys": [{"budget_id": "b"}]}"#);
    assert!(missing.is_err());
}

#[test]
fn bulk_result_fields() {
    let response: BulkUpdateKeyResponse = serde_json::from_str(concat!(
        r#"{"total_requested": 2, "successful_updates": [{"key": "a", "key_info": {}}],"#,
        r#" "failed_updates": [{"key": "b", "failed_reason": "no"}]}"#
    ))
    .expect("a bulk response");
    assert_eq!(response.total_requested, 2);
    assert_eq!(response.failed_updates[0].key_info, None);
    assert_eq!(
        to_json(&response).expect("the response dump"),
        json_data(concat!(
            r#"{"total_requested": 2, "successful_updates": [{"key": "a", "key_info": {}}],"#,
            r#" "failed_updates": [{"key": "b", "key_info": null, "failed_reason": "no"}]}"#
        ))
    );
    let failed: Result<BulkUpdateKeyResponse, _> = serde_json::from_str(
        r#"{"total_requested": 1, "successful_updates": [], "failed_updates": [{"key": "b"}]}"#,
    );
    assert!(failed.is_err());
}

#[test]
fn broadcast_field_allowlist() {
    let fields = KeyUpdateFields::read(&value(r#"{"tags": [], "metadata": {}}"#))
        .expect("two broadcast fields");
    let resolved = fields.project().expect("the projection").resolved;
    let names: Vec<&str> = resolved.keys().map(String::as_str).collect();
    assert_eq!(names, BROADCAST_FIELDS);
    assert_field(&resolved, "tags", "[]");
    assert_field(&resolved, "metadata", "{}");
    assert!(KeyUpdateFields::read(&value(r#"{"spend": 1.0}"#)).is_err());
    assert!(KeyUpdateFields::read(&value(r#"{"team_id": "t"}"#)).is_err());
    assert!(KeyUpdateFields::read(&value(r#"{"user_id": "u"}"#)).is_err());
    assert!(KeyUpdateFields::read(&value(r#"{"models": []}"#)).is_err());
    assert!(KeyUpdateFields::read(&value(r#"{"permissions": {}}"#)).is_err());
}

#[test]
fn empty_patch_rejected() {
    assert!(matches!(
        KeyUpdateFields::read(&value("{}")),
        Err(DecodeError::EmptyPatch)
    ));
}

#[test]
fn explicit_null_patch() {
    let fields =
        KeyUpdateFields::read(&value(r#"{"metadata": null}"#)).expect("an explicit null patch");
    assert_eq!(
        fields.project().expect("the projection").supplied,
        object(r#"{"metadata": null}"#)
    );
}

#[test]
fn selection_exclusive() {
    assert!(matches!(
        BulkUpdateTeamKeysRequest::read(&value(
            r#"{"team_id": "t", "key_ids": ["k"], "all_keys_in_team": true, "update_fields": {"tags": []}}"#
        )),
        Err(DecodeError::SelectionConflict)
    ));
    assert!(matches!(
        BulkUpdateTeamKeysRequest::read(&value(
            r#"{"team_id": "t", "update_fields": {"tags": []}}"#
        )),
        Err(DecodeError::SelectionMissing)
    ));
    let request = BulkUpdateTeamKeysRequest::read(&value(
        r#"{"team_id": "t", "key_ids": [], "all_keys_in_team": true, "update_fields": {"tags": []}}"#,
    ))
    .expect("all keys with an empty key list");
    let resolved = request.resolved_object().expect("the request dump");
    assert_eq!(resolved.len(), 4);
    assert_field(&resolved, "all_keys_in_team", "true");
    assert_field(&resolved, "key_ids", "[]");
    let Some(JsonData::Object(updates)) = resolved.get("update_fields") else {
        panic!("update_fields is an object");
    };
    assert_eq!(updates.len(), 17);
    assert_field(updates, "tags", "[]");
    let missing_team = BulkUpdateTeamKeysRequest::read(&value(
        r#"{"key_ids": ["k"], "update_fields": {"tags": []}}"#,
    ));
    assert!(missing_team.is_err());
}

#[test]
fn custom_policy_record() {
    let generate = CustomKeyPolicyRequest::read(&value(
        r#"{"operation": "generate", "existing_key": null, "effective_key": {}, "request": {"team_id": "t"}}"#,
    ))
    .expect("a generate policy record");
    let projection = generate.project().expect("the policy projection");
    assert_field(&projection, "operation", r#""generate""#);
    assert_field(&projection, "request_kind", r#""GenerateKeyRequest""#);
    assert!(matches!(generate.request, PolicyRequestBody::Generate(_)));
    let update = CustomKeyPolicyRequest::read(&value(concat!(
        r#"{"operation": "update", "existing_key": {"token": "k"}, "effective_key": {},"#,
        r#" "request": {"key": "k", "temp_budget_increase": null, "temp_budget_expiry": null}}"#
    )))
    .expect("an update policy record");
    assert!(matches!(update.request, PolicyRequestBody::Update(_)));
    let rotated = CustomKeyPolicyRequest::read(&value(
        r#"{"operation": "regenerate", "existing_key": null, "effective_key": {}, "request": {"new_key": "n"}}"#,
    ))
    .expect("a regenerate policy record");
    assert!(matches!(rotated.request, PolicyRequestBody::Regenerate(_)));
    let tied = CustomKeyPolicyRequest::read(&value(
        r#"{"operation": "generate", "existing_key": null, "effective_key": {}, "request": {"key": "k"}}"#,
    ))
    .expect("a tied union");
    assert!(matches!(tied.request, PolicyRequestBody::Generate(_)));
}

#[test]
fn custom_policy_rejects() {
    assert!(matches!(
        CustomKeyPolicyRequest::read(&value(
            r#"{"operation": "generate", "effective_key": {}, "request": {}}"#
        )),
        Err(DecodeError::MissingField {
            field: "existing_key"
        })
    ));
    assert!(
        CustomKeyPolicyRequest::read(&value(
            r#"{"operation": "rotate", "existing_key": null, "effective_key": {}, "request": {}}"#
        ))
        .is_err()
    );
    assert!(matches!(
        CustomKeyPolicyRequest::read(&value(concat!(
            r#"{"operation": "generate", "existing_key": null, "effective_key": {},"#,
            r#" "request": {"default_estimated_output_tokens": 0}}"#
        ))),
        Err(DecodeError::RequestUnion)
    ));
}

#[test]
fn settings_stamp_overwrites() {
    let body = object(r#"{"a": 1, "settings_updated_at": "old"}"#);
    let now: DateTime<Utc> = "2030-01-01T00:00:00Z".parse().expect("a fixed clock");
    let stamp = with_settings_updated_at(&body, now);
    assert_field(&stamp, "a", "1");
    assert_field(
        &stamp,
        "settings_updated_at",
        r#""2030-01-01T00:00:00+00:00""#,
    );
    assert_eq!(
        body.get("settings_updated_at"),
        Some(&JsonData::String("old".to_owned()))
    );
}

#[test]
fn prefix_hash() {
    assert_eq!(
        hash_token_if_needed("sk-abc"),
        "1460db1b6902f8b1fc2a40d9381a24d0fd22c3bc1b2c6f999c521da73776fbe0"
    );
    assert_eq!(
        hash_token_if_needed("sk-"),
        "449b11820b0f65268bca74c8c9153fa9bebd2a9ee8b3e6ec479d768b46242c2a"
    );
}

#[test]
fn bare_hash_passthrough() {
    assert_eq!(hash_token_if_needed("SK-abc"), "SK-abc");
    assert_eq!(hash_token_if_needed("Bearer sk-abc"), "Bearer sk-abc");
    assert_eq!(
        hash_token_if_needed("jwt.header.payload"),
        "jwt.header.payload"
    );
    assert_eq!(hash_token_if_needed(""), "");
}

#[test]
fn stored_windows_empty() {
    assert_eq!(
        stored_budget_limit_windows(None).expect("no stored value"),
        Vec::new()
    );
    assert_eq!(
        stored_budget_limit_windows(Some(&json_data("null"))).expect("null"),
        Vec::new()
    );
    assert_eq!(
        stored_budget_limit_windows(Some(&json_data("false"))).expect("false"),
        Vec::new()
    );
    assert_eq!(
        stored_budget_limit_windows(Some(&json_data("0"))).expect("zero"),
        Vec::new()
    );
    assert_eq!(
        stored_budget_limit_windows(Some(&json_data(r#""""#))).expect("empty text"),
        Vec::new()
    );
    assert_eq!(
        stored_budget_limit_windows(Some(&json_data("[]"))).expect("empty list"),
        Vec::new()
    );
    assert_eq!(
        stored_budget_limit_windows(Some(&json_data("{}"))).expect("empty object"),
        Vec::new()
    );
}

#[test]
fn stored_windows_json() {
    let stored = JsonData::String(r#"[{"budget_duration": "1d", "max_budget": 2.5}]"#.to_owned());
    let windows = stored_budget_limit_windows(Some(&stored)).expect("JSON text");
    assert_eq!(
        windows,
        vec![object(r#"{"budget_duration": "1d", "max_budget": 2.5}"#)]
    );
    let listed = json_data(r#"[{"budget_duration": "1d", "max_budget": 2.5}]"#);
    let same = stored_budget_limit_windows(Some(&listed)).expect("a parsed list");
    assert_eq!(same, windows);
}

#[test]
fn stored_windows_typed() {
    let entry: BudgetLimitEntry = decode_value(&json_data(
        r#"{"budget_duration": "1d", "max_budget": 2.5}"#,
    ))
    .expect("a typed window");
    let windows = typed_budget_limit_windows(Some(&[entry])).expect("typed entries");
    assert_eq!(
        windows,
        vec![object(
            r#"{"budget_duration": "1d", "max_budget": 2.5, "reset_at": null}"#
        )]
    );
    assert_eq!(
        typed_budget_limit_windows(Some(&[])).expect("no entries"),
        Vec::new()
    );
    assert_eq!(
        typed_budget_limit_windows(None).expect("no list"),
        Vec::new()
    );
}

#[test]
fn stored_windows_invalid() {
    let broken = JsonData::String("{bad".to_owned());
    assert!(matches!(
        stored_budget_limit_windows(Some(&broken)),
        Err(DecodeError::StoredWindowsJson(_))
    ));
    let not_a_list = JsonData::String("null".to_owned());
    assert!(matches!(
        stored_budget_limit_windows(Some(&not_a_list)),
        Err(DecodeError::StoredWindowsNotAList)
    ));
    assert!(matches!(
        stored_budget_limit_windows(Some(&json_data("true"))),
        Err(DecodeError::StoredWindowsNotAList)
    ));
    assert!(matches!(
        stored_budget_limit_windows(Some(&json_data("5"))),
        Err(DecodeError::StoredWindowsNotAList)
    ));
    assert!(matches!(
        stored_budget_limit_windows(Some(&json_data(r#"{"a": 1}"#))),
        Err(DecodeError::StoredWindowsNotAList)
    ));
    assert!(matches!(
        stored_budget_limit_windows(Some(&json_data(r#"["x"]"#))),
        Err(DecodeError::StoredWindowsUnsupported { index: 0 })
    ));
    assert!(matches!(
        stored_budget_limit_windows(Some(&json_data(r#"[{}, "x"]"#))),
        Err(DecodeError::StoredWindowsUnsupported { index: 1 })
    ));
}

#[test]
fn key_body_limits() {
    let oversized = vec![b' '; 1_048_577];
    assert!(matches!(
        decode_key_request(&oversized),
        Err(DecodeError::Bounded(PrivateJsonError::TooLarge(..)))
    ));
    assert!(matches!(
        decode_key_response(&oversized),
        Err(DecodeError::Bounded(PrivateJsonError::TooLarge(..)))
    ));
    let deep = format!("{}{}", "[".repeat(65), "]".repeat(65));
    assert!(matches!(
        decode_key_request(deep.as_bytes()),
        Err(DecodeError::Bounded(PrivateJsonError::TooDeep(_)))
    ));
    assert!(decode_key_request(b"{").is_err());
    assert!(matches!(
        decode_key_request(b"[]"),
        Err(DecodeError::NotAnObject)
    ));
    assert!(decode_key_request(b"{}").is_ok());
}

#[test]
fn user_row_view() {
    let row: LiteLLMUserTable = serde_json::from_str(
        r#"{"user_id": "u", "user_alias": "al", "user_email": "e@example.com", "spend": 1.5}"#,
    )
    .expect("a user row");
    assert_eq!(row.user_id(), "u");
    assert_eq!(row.user_alias(), Some("al"));
    assert_eq!(row.user_email(), Some("e@example.com"));
    let dumped = row.dumped().expect("the row dump");
    assert_field(&dumped, "user_id", r#""u""#);
    assert_field(&dumped, "user_alias", r#""al""#);
}

#[derive(serde::Serialize)]
struct LegacyRow {
    id: String,
}

#[test]
fn legacy_dump() {
    let row = LegacyRow { id: "r".to_owned() };
    assert_eq!(
        row.legacy_dict().expect("the legacy dump"),
        object(r#"{"id": "r"}"#)
    );
}

#[test]
fn env_vars_param() {
    let param = ConfigParam {
        param_name: "environment_variables".to_owned(),
        param_value: json_data(r#"{"A": "1", "B": "2"}"#),
    };
    assert_eq!(
        env_vars_param_value(&param),
        Some(BTreeMap::from([
            ("A".to_owned(), "1".to_owned()),
            ("B".to_owned(), "2".to_owned()),
        ]))
    );
    let not_text = ConfigParam {
        param_name: "environment_variables".to_owned(),
        param_value: json_data(r#"{"A": 1}"#),
    };
    assert_eq!(env_vars_param_value(&not_text), None);
}

#[test]
fn insensitive_contains() {
    let filter = decode_value::<InsensitiveContains>(&json_data(
        r#"{"contains": "abc", "mode": "insensitive"}"#,
    ))
    .expect("the search filter");
    assert_eq!(filter.contains, "abc");
    assert_eq!(filter.mode, MatchMode::Insensitive);
    assert!(decode_value::<InsensitiveContains>(&json_data(r#"{"contains": "abc"}"#)).is_err());
    assert!(
        decode_value::<InsensitiveContains>(&json_data(
            r#"{"contains": "abc", "mode": "sensitive"}"#
        ))
        .is_err()
    );
    assert_eq!(
        to_json(&filter).expect("the filter dump"),
        json_data(r#"{"contains": "abc", "mode": "insensitive"}"#)
    );
}

#[rstest]
fn replays_the_key_fields_corpus(#[files("fuzz/corpus/key_fields/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("a corpus seed");
    drop(decode_key_request(&bytes));
    drop(decode_key_response(&bytes));
}
