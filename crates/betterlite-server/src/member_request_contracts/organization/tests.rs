#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::json;

use super::{
    MANAGEMENT_METADATA_FIELDS, OrganizationTableUpdate, OrganizationTableWithMembers,
    OrganizationUpdateRequestV2, PresentFields,
};
use crate::member_request_contracts::MemberContractError;

fn parse(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn raw<T: Serialize>(body: T) -> Json {
    serde_json::from_str(&serde_json::to_string(&body).unwrap()).unwrap()
}

fn data(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

fn table_update<T: Serialize>(body: T) -> Result<OrganizationTableUpdate, MemberContractError> {
    OrganizationTableUpdate::from_value(&raw(body))
}

fn v2_request<T: Serialize>(body: T) -> Result<OrganizationUpdateRequestV2, MemberContractError> {
    OrganizationUpdateRequestV2::from_value(&raw(body))
}

fn with_members<T: Serialize>(
    body: T,
) -> Result<OrganizationTableWithMembers, MemberContractError> {
    OrganizationTableWithMembers::from_value(&raw(body))
}

fn message(error: MemberContractError) -> String {
    match error {
        MemberContractError::Validation { message } => message,
        MemberContractError::Decode { source } => {
            panic!("expected a validation error, got {source}")
        }
    }
}

fn organization_body() -> Json {
    raw(json!({
        "budget_id": "budget-1",
        "created_by": "creator",
        "updated_by": "updater",
        "spend": 12.5,
        "created_at": "2026-03-01T00:00:00",
        "updated_at": "2026-03-02T00:00:00",
    }))
}

fn metadata_keys<T: Serialize>(body: T) -> Option<BTreeSet<String>> {
    table_update(body)
        .unwrap()
        .metadata
        .map(|metadata| metadata.keys().cloned().collect())
}

#[test]
fn exports_the_management_metadata_fields() {
    let expected = [
        "model_rpm_limit",
        "model_tpm_limit",
        "model_itpm_limit",
        "model_otpm_limit",
        "default_estimated_output_tokens",
        "default_estimated_output_tokens_per_model",
        "mcp_rpm_limit",
        "tag_rpm_limit",
        "rpm_limit_type",
        "tpm_limit_type",
        "enforced_params",
        "temp_budget_increase",
        "temp_budget_expiry",
        "allowed_vector_store_indexes",
        "enforced_batch_output_expires_after",
        "enforced_file_expires_after",
        "throttle_on_budget_exceeded",
        "enable_prompt_caching",
        "end_user_budget_id",
    ];
    assert_eq!(MANAGEMENT_METADATA_FIELDS, expected);
}

#[test]
fn moves_every_management_field_into_metadata() {
    let fields: IndexMap<String, Json> = MANAGEMENT_METADATA_FIELDS
        .into_iter()
        .map(|name| (name.to_owned(), parse("\"v\"")))
        .collect();
    let metadata = metadata_keys(Json::Object(fields)).unwrap();
    let expected: BTreeSet<String> = MANAGEMENT_METADATA_FIELDS
        .into_iter()
        .map(str::to_owned)
        .collect();
    assert_eq!(metadata, expected);
}

#[rstest]
#[case("model_tpm_limit", "{\"gpt-4\": 100}")]
#[case("temp_budget_increase", "12.5")]
#[case("throttle_on_budget_exceeded", "true")]
#[case("enforced_params", "[\"temperature\"]")]
fn moves_a_management_field_value_unchanged(#[case] name: &str, #[case] value: &str) {
    let request = table_update(parse(&format!(r#"{{"{name}": {value}}}"#))).unwrap();
    let metadata = request.metadata.unwrap();
    assert_eq!(metadata.len(), 1);
    assert_eq!(metadata[name], data(value));
}

#[test]
fn merges_into_a_caller_metadata_object() {
    let request = table_update(json!({
        "metadata": { "model_rpm_limit": "old", "keep": 1 },
        "model_rpm_limit": { "gpt-4": 10 },
    }))
    .unwrap();
    let metadata = request.metadata.unwrap();
    assert_eq!(metadata.len(), 2);
    assert_eq!(metadata["keep"], data("1"));
    assert_eq!(metadata["model_rpm_limit"], data("{\"gpt-4\": 10}"));
}

#[test]
fn creates_metadata_when_it_is_absent() {
    let metadata = metadata_keys(json!({ "temp_budget_increase": 12.5 })).unwrap();
    assert_eq!(
        metadata,
        BTreeSet::from(["temp_budget_increase".to_owned()])
    );
}

#[test]
fn creates_metadata_when_it_is_null() {
    let metadata =
        metadata_keys(json!({ "metadata": null, "temp_budget_increase": 12.5 })).unwrap();
    assert_eq!(
        metadata,
        BTreeSet::from(["temp_budget_increase".to_owned()])
    );
}

#[test]
fn does_not_create_metadata_for_a_null_field() {
    let request = table_update(json!({ "model_tpm_limit": null })).unwrap();
    assert_eq!(request.metadata, None);
}

#[test]
fn does_not_move_a_null_field_into_metadata() {
    let metadata =
        metadata_keys(json!({ "metadata": { "keep": 1 }, "model_tpm_limit": null })).unwrap();
    assert_eq!(metadata, BTreeSet::from(["keep".to_owned()]));
}

#[test]
fn keeps_a_null_metadata_as_none() {
    let request = table_update(json!({ "metadata": null })).unwrap();
    assert_eq!(request.metadata, None);
}

#[test]
fn rejects_a_non_object_metadata_with_a_move() {
    let error = table_update(json!({
        "metadata": [1],
        "model_tpm_limit": { "gpt-4": 100 },
    }))
    .unwrap_err();
    assert_eq!(message(error), "Input should be a valid dictionary");
}

#[test]
fn rejects_a_non_object_metadata_without_a_move() {
    let error = table_update(json!({ "metadata": [1] })).unwrap_err();
    assert!(matches!(error, MemberContractError::Decode { .. }));
}

#[test]
fn decodes_the_budget_fields() {
    let request = table_update(json!({
        "budget_id": "budget-1",
        "soft_budget": 1.5,
        "max_budget": 12.5,
        "max_parallel_requests": 4,
        "tpm_limit": 100,
        "rpm_limit": 10,
        "tpd_limit": 1000,
        "budget_duration": "1d",
        "allowed_models": ["gpt-4"],
    }))
    .unwrap();
    assert_eq!(request.budget.budget_id, Some("budget-1".to_owned()));
    assert_eq!(request.budget.soft_budget, Some(Decimal::new(15, 1)));
    assert_eq!(request.budget.max_budget, Some(Decimal::new(125, 1)));
    assert_eq!(request.budget.max_parallel_requests, Some(4));
    assert_eq!(request.budget.tpm_limit, Some(100));
    assert_eq!(request.budget.rpm_limit, Some(10));
    assert_eq!(request.budget.tpd_limit, Some(1000));
    assert_eq!(request.budget.budget_duration, Some("1d".to_owned()));
    assert_eq!(
        request.budget.allowed_models,
        Some(vec!["gpt-4".to_owned()])
    );
}

#[test]
fn moves_the_temp_budget_pair_into_metadata() {
    let request = table_update(json!({
        "temp_budget_increase": 2.5,
        "temp_budget_expiry": "2026-03-01T00:00:00",
    }))
    .unwrap();
    assert_eq!(request.budget.temp_budget_increase, None);
    assert_eq!(request.budget.temp_budget_expiry, None);
    let metadata = request.metadata.unwrap();
    assert_eq!(metadata.len(), 2);
    assert_eq!(metadata["temp_budget_increase"], data("2.5"));
    assert_eq!(
        metadata["temp_budget_expiry"],
        data("\"2026-03-01T00:00:00\"")
    );
}

#[test]
fn decodes_the_organization_fields() {
    let request = table_update(json!({
        "organization_id": "org-1",
        "organization_alias": "alias",
        "spend": 3.5,
        "models": ["gpt-4"],
        "updated_by": "updater",
    }))
    .unwrap();
    assert_eq!(request.organization_id, Some("org-1".to_owned()));
    assert_eq!(request.organization_alias, Some("alias".to_owned()));
    assert_eq!(request.spend, Some(Decimal::new(35, 1)));
    assert_eq!(request.models, Some(vec!["gpt-4".to_owned()]));
    assert_eq!(request.updated_by, Some("updater".to_owned()));
}

#[test]
fn leaves_moved_management_fields_null() {
    let request = table_update(json!({
        "model_tpm_limit": { "gpt-4": 100 },
        "model_rpm_limit": { "gpt-4": 10 },
    }))
    .unwrap();
    assert_eq!(request.model_tpm_limit, None);
    assert_eq!(request.model_rpm_limit, None);
    let metadata = request.metadata.unwrap();
    assert_eq!(metadata["model_tpm_limit"], data("{\"gpt-4\": 100}"));
    assert_eq!(metadata["model_rpm_limit"], data("{\"gpt-4\": 10}"));
}

#[test]
fn rejects_a_body_that_is_not_an_object() {
    let error = table_update(parse("[]")).unwrap_err();
    assert_eq!(
        message(error),
        "Input should be a valid dictionary or object to extract fields from"
    );
}

#[test]
fn serializes_the_budget_fields_flat() {
    let dumped = serde_json::to_value(table_update(json!({ "budget_id": "budget-1" })).unwrap())
        .expect("the organization update serializes");
    assert_eq!(dumped["budget_id"], "budget-1");
    assert!(dumped.get("budget").is_none());
}

#[test]
fn records_the_present_keys_of_a_v2_patch() {
    let request = v2_request(json!({ "organization_alias": "alias", "metadata": null })).unwrap();
    let expected = BTreeSet::from(["metadata".to_owned(), "organization_alias".to_owned()]);
    assert_eq!(request.fields_set(), &expected);
}

#[test]
fn records_no_present_keys_for_an_empty_v2_patch() {
    let request = v2_request(json!({})).unwrap();
    assert!(request.fields_set().is_empty());
}

#[test]
fn rejects_an_unknown_key_in_a_v2_patch() {
    let error =
        v2_request(json!({ "organization_alias": "alias", "budget_id": "budget-1" })).unwrap_err();
    assert_eq!(message(error), "Extra inputs are not permitted");
}

#[test]
fn decodes_every_v2_field() {
    let request = v2_request(json!({
        "organization_alias": "alias",
        "models": ["gpt-4"],
        "metadata": { "team": "core" },
        "tpm_limit": 100,
        "rpm_limit": 10,
        "max_budget": 12.5,
        "soft_budget": 1.5,
        "max_parallel_requests": 4,
        "model_max_budget": { "gpt-4": 2.5 },
        "budget_duration": "1d",
    }))
    .unwrap();
    assert_eq!(request.organization_alias, Some("alias".to_owned()));
    assert_eq!(request.models, Some(vec!["gpt-4".to_owned()]));
    assert!(request.metadata.is_some());
    assert_eq!(request.tpm_limit, Some(100));
    assert_eq!(request.rpm_limit, Some(10));
    assert_eq!(request.max_budget, Some(Decimal::new(125, 1)));
    assert_eq!(request.soft_budget, Some(Decimal::new(15, 1)));
    assert_eq!(request.max_parallel_requests, Some(4));
    assert!(request.model_max_budget.is_some());
    assert_eq!(request.budget_duration, Some("1d".to_owned()));
    assert_eq!(request.fields_set().len(), 10);
}

#[test]
fn keeps_a_null_v2_value_as_none() {
    let request = v2_request(json!({ "max_budget": null })).unwrap();
    assert_eq!(request.max_budget, None);
    assert_eq!(
        request.fields_set(),
        &BTreeSet::from(["max_budget".to_owned()])
    );
}

#[test]
fn does_not_serialize_the_v2_fields_set() {
    let dumped = serde_json::to_value(v2_request(json!({ "tpm_limit": 100 })).unwrap())
        .expect("the v2 patch serializes");
    assert_eq!(dumped["tpm_limit"], 100);
    assert!(dumped.get("fields_set").is_none());
}

#[test]
fn rejects_a_v2_body_that_is_not_an_object() {
    let error = v2_request(parse("1")).unwrap_err();
    assert_eq!(
        message(error),
        "Input should be a valid dictionary or object to extract fields from"
    );
}

#[test]
fn defaults_the_member_and_team_lists() {
    let request = with_members(organization_body()).unwrap();
    assert!(request.members.is_empty());
    assert!(request.teams.is_empty());
    assert_eq!(request.table.budget_id, "budget-1");
    assert_eq!(request.table.spend, Decimal::new(125, 1));
    assert_eq!(request.table.created_by, "creator");
    assert_eq!(request.table.updated_by, "updater");
}

#[test]
fn decodes_the_members_of_an_organization_table() {
    let request = with_members(json!({
        "budget_id": "budget-1",
        "created_by": "creator",
        "updated_by": "updater",
        "spend": 12.5,
        "created_at": "2026-03-01T00:00:00",
        "updated_at": "2026-03-02T00:00:00",
        "members": [{
            "user_id": "user-1",
            "organization_id": "org-1",
            "user_role": "org_admin",
            "spend": 2.5,
            "created_at": "2026-03-01T00:00:00",
            "updated_at": "2026-03-02T00:00:00",
            "user_email": "a@b.c",
        }],
    }))
    .unwrap();
    assert_eq!(request.members.len(), 1);
    assert_eq!(request.members[0].user_id, "user-1");
    assert_eq!(request.members[0].user_role, Some("org_admin".to_owned()));
    assert_eq!(request.members[0].spend, Decimal::new(25, 1));
}

#[test]
fn decodes_the_teams_of_an_organization_table() {
    let request = with_members(json!({
        "budget_id": "budget-1",
        "created_by": "creator",
        "updated_by": "updater",
        "spend": 12.5,
        "created_at": "2026-03-01T00:00:00",
        "updated_at": "2026-03-02T00:00:00",
        "teams": [{ "team_id": "team-1" }],
    }))
    .unwrap();
    assert_eq!(request.teams.len(), 1);
}

#[test]
fn rejects_a_team_without_a_team_id() {
    let Json::Object(mut body) = organization_body() else {
        panic!("the organization body is an object");
    };
    body.insert("teams".to_owned(), parse(r#"[{ "team_alias": "alias" }]"#));
    let error = with_members(Json::Object(body)).unwrap_err();
    assert!(matches!(error, MemberContractError::Decode { .. }));
}

#[rstest]
#[case("created_at")]
#[case("updated_at")]
fn requires_the_organization_timestamps(#[case] field: &str) {
    let Json::Object(mut body) = organization_body() else {
        panic!("the organization body is an object");
    };
    body.shift_remove(field);
    let error = with_members(Json::Object(body)).unwrap_err();
    assert!(matches!(error, MemberContractError::Decode { .. }));
}

#[test]
fn requires_the_organization_row_fields() {
    let error = with_members(json!({
        "created_at": "2026-03-01T00:00:00",
        "updated_at": "2026-03-02T00:00:00",
    }))
    .unwrap_err();
    assert!(matches!(error, MemberContractError::Decode { .. }));
}

#[test]
fn rejects_an_organization_table_that_is_not_an_object() {
    let error = with_members(parse("null")).unwrap_err();
    assert_eq!(
        message(error),
        "Input should be a valid dictionary or object to extract fields from"
    );
}

#[test]
fn serializes_the_timestamps_and_the_lists() {
    let dumped = serde_json::to_value(with_members(organization_body()).unwrap())
        .expect("the organization table serializes");
    assert_eq!(dumped["created_at"], "2026-03-01T00:00:00");
    assert_eq!(dumped["updated_at"], "2026-03-02T00:00:00");
    assert_eq!(dumped["members"], json!([]));
    assert_eq!(dumped["teams"], json!([]));
    assert_eq!(dumped["budget_id"], "budget-1");
}
