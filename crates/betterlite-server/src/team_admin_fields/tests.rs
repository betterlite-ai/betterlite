#![cfg(test)]

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use rstest::rstest;

use super::FOLDED_FIELDS;
use super::changes::Columns;
use super::changes::column_changed;
use super::decoder::decode_team_admin_object;
use super::error::Forbidden;
use super::error::TeamAdminError;
use super::key::TeamAdminKeyEditVerdict;
use super::key::changed_fields as changed_key_fields;
use super::key::edit_verdict as key_edit_verdict;
use super::key::request_or_raise as key_request_or_raise;
use super::team::TeamAdminEditVerdict;
use super::team::changed_fields as changed_team_fields;
use super::team::edit_verdict as team_edit_verdict;
use super::team::request_or_raise as team_request_or_raise;
use crate::key_fields::decode::TypedValue;
use crate::key_fields::update::UpdateKeyRequest;
use crate::team_request_contracts::shared::RequestProjection as _;
use crate::team_request_contracts::update::UpdateTeamRequest;

const EDITING_DISABLED: &str = "Team admins on this proxy cannot edit team settings. \
    Ask a proxy admin to enable fields under Settings > UI > Team admin editable fields.";

const KEY_EDITING_DISABLED: &str = "Team admins on this proxy cannot update budgets on other \
    members' keys. Ask a proxy admin to enable 'member_key_budgets' under Settings > UI > Team \
    admin editable fields.";

fn parse(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

fn permitted(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn changed_names(changed: &BTreeSet<String>) -> Vec<&str> {
    changed.iter().map(String::as_str).collect()
}

fn team_request(text: &str) -> UpdateTeamRequest {
    UpdateTeamRequest::from_value(&parse(text)).unwrap()
}

fn team_row(text: &str) -> LiteLLMTeamTable {
    LiteLLMTeamTable::try_from(parse(text)).unwrap()
}

fn key_request(text: &str) -> UpdateKeyRequest {
    UpdateKeyRequest::read(&TypedValue { json: parse(text) }).unwrap()
}

fn key_row(text: &str) -> LiteLLMVerificationToken {
    serde_json::from_str(text).unwrap()
}

fn team_changes(request: &str, existing: &str) -> BTreeSet<String> {
    changed_team_fields(&team_request(request), &team_row(existing)).unwrap()
}

fn key_changes(request: &str, existing: &str) -> BTreeSet<String> {
    changed_key_fields(&key_request(request), &key_row(existing)).unwrap()
}

fn fields(text: &str) -> BTreeMap<String, JsonData> {
    let JsonData::Object(found) = parse(text) else {
        panic!("expected a JSON object");
    };
    found
}

fn allowed_team_edit(request: &str, existing: &str, names: &[&str]) -> BTreeMap<String, JsonData> {
    let verdict = team_edit_verdict(
        &team_request(request),
        &team_row(existing),
        &permitted(names),
    )
    .unwrap();
    let TeamAdminEditVerdict::Allowed { request: filtered } = verdict else {
        panic!("the edit is allowed");
    };
    let JsonData::Object(found) = filtered.supplied().unwrap() else {
        panic!("the request is an object");
    };
    found
}

fn keys(fields: &BTreeMap<String, JsonData>) -> Vec<&str> {
    fields.keys().map(String::as_str).collect()
}

#[rstest]
#[case::model_rpm_limit("model_rpm_limit")]
#[case::model_tpm_limit("model_tpm_limit")]
#[case::model_itpm_limit("model_itpm_limit")]
#[case::model_otpm_limit("model_otpm_limit")]
#[case::default_estimated_output_tokens("default_estimated_output_tokens")]
#[case::per_model_output_tokens("default_estimated_output_tokens_per_model")]
#[case::mcp_rpm_limit("mcp_rpm_limit")]
#[case::tag_rpm_limit("tag_rpm_limit")]
#[case::rpm_limit_type("rpm_limit_type")]
#[case::tpm_limit_type("tpm_limit_type")]
#[case::enforced_params("enforced_params")]
#[case::temp_budget_increase("temp_budget_increase")]
#[case::temp_budget_expiry("temp_budget_expiry")]
#[case::allowed_vector_store_indexes("allowed_vector_store_indexes")]
#[case::enforced_batch_output_expires_after("enforced_batch_output_expires_after")]
#[case::enforced_file_expires_after("enforced_file_expires_after")]
#[case::throttle_on_budget_exceeded("throttle_on_budget_exceeded")]
#[case::enable_prompt_caching("enable_prompt_caching")]
#[case::end_user_budget_id("end_user_budget_id")]
#[case::disable_global_guardrails("disable_global_guardrails")]
#[case::guardrails("guardrails")]
#[case::policies("policies")]
#[case::tags("tags")]
#[case::team_member_key_duration("team_member_key_duration")]
#[case::prompts("prompts")]
#[case::logging("logging")]
#[case::secret_manager_settings("secret_manager_settings")]
#[case::allowed_passthrough_routes("allowed_passthrough_routes")]
fn folded_field_catalog_matrix(#[case] name: &str) {
    let stored = format!(r#"{{"team_id": "t", "metadata": {{"{name}": "stored"}}}}"#);
    let sent = format!(r#"{{"team_id": "t", "metadata": {{"{name}": "sent"}}}}"#);
    assert_eq!(changed_names(&team_changes(&sent, &stored)), vec![name]);
    assert!(team_changes(&stored, &stored).is_empty());
}

#[test]
fn folded_field_tables_cover_both_catalogs() {
    assert_eq!(
        &FOLDED_FIELDS[..19],
        &[
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
        ]
    );
    assert_eq!(
        &FOLDED_FIELDS[19..],
        &[
            "disable_global_guardrails",
            "guardrails",
            "policies",
            "tags",
            "team_member_key_duration",
            "prompts",
            "logging",
            "secret_manager_settings",
            "allowed_passthrough_routes",
        ]
    );
}

#[test]
fn metadata_omitted_reuses_explicit_null_clears() {
    let stored = r#"{"team_id": "t", "metadata": {"a": 1}}"#;
    assert!(team_changes(r#"{"team_id": "t"}"#, stored).is_empty());
    assert!(team_changes(r#"{"team_id": "t", "metadata": {"a": 1}}"#, stored).is_empty());
    assert_eq!(
        changed_names(&team_changes(
            r#"{"team_id": "t", "metadata": null}"#,
            stored
        )),
        vec!["metadata"]
    );
    assert!(
        team_changes(
            r#"{"team_id": "t"}"#,
            r#"{"team_id": "t", "metadata": null}"#
        )
        .is_empty()
    );
}

#[test]
fn folded_top_level_null_overrides() {
    let stored =
        r#"{"team_id": "t", "metadata": {"tags": ["x"], "disable_global_guardrails": true}}"#;
    assert_eq!(
        changed_names(&team_changes(r#"{"team_id": "t", "tags": null}"#, stored)),
        vec!["tags"]
    );
    assert_eq!(
        changed_names(&team_changes(
            r#"{"team_id": "t", "disable_global_guardrails": null}"#,
            stored
        )),
        vec!["disable_global_guardrails"]
    );
    assert!(team_changes(r#"{"team_id": "t", "tags": ["x"]}"#, stored).is_empty());
    assert!(
        team_changes(
            r#"{"team_id": "t", "metadata": {"tags": ["y"], "disable_global_guardrails": true}, "tags": ["x"]}"#,
            stored
        )
        .is_empty()
    );
}

#[test]
fn system_member_budget_ignored() {
    let stored = r#"{"team_id": "t", "metadata": {"team_member_budget_id": "a", "keep": 1}}"#;
    assert!(
        team_changes(
            r#"{"team_id": "t", "metadata": {"team_member_budget_id": "b", "keep": 1}}"#,
            stored
        )
        .is_empty()
    );
    assert_eq!(
        changed_names(&team_changes(
            r#"{"team_id": "t", "metadata": {"team_member_budget_id": "b"}}"#,
            stored
        )),
        vec!["metadata"]
    );
}

#[test]
fn absent_equals_null() {
    assert!(
        team_changes(
            r#"{"team_id": "t", "metadata": {"a": null}}"#,
            r#"{"team_id": "t", "metadata": {}}"#
        )
        .is_empty()
    );
    assert_eq!(
        changed_names(&team_changes(
            r#"{"team_id": "t", "metadata": {"a": null}}"#,
            r#"{"team_id": "t", "metadata": {"a": 1}}"#
        )),
        vec!["metadata"]
    );
    assert!(
        team_changes(
            r#"{"team_id": "t", "tpm_limit": null}"#,
            r#"{"team_id": "t"}"#
        )
        .is_empty()
    );
    assert_eq!(
        changed_names(&team_changes(
            r#"{"team_id": "t", "tpm_limit": 5}"#,
            r#"{"team_id": "t"}"#
        )),
        vec!["tpm_limit"]
    );
}

#[test]
fn unknown_metadata_attributed_to_metadata() {
    let stored = r#"{"team_id": "t", "metadata": {"custom": 1, "nested": {"a": 1}}}"#;
    assert!(
        team_changes(
            r#"{"team_id": "t", "metadata": {"custom": 1, "nested": {"a": 1}}}"#,
            stored
        )
        .is_empty()
    );
    assert_eq!(
        changed_names(&team_changes(
            r#"{"team_id": "t", "metadata": {"custom": 2, "nested": {"a": 1}}}"#,
            stored
        )),
        vec!["metadata"]
    );
    assert_eq!(
        changed_names(&team_changes(
            r#"{"team_id": "t", "metadata": {"custom": 1, "nested": {"a": 1}, "new": true}}"#,
            stored
        )),
        vec!["metadata"]
    );
}

#[test]
fn aliases_from_embedded_table() {
    let table = r#"{"team_id": "t", "litellm_model_table": {"model_aliases": {"gpt": "gpt-4"}, "created_by": "c", "updated_by": "u"}}"#;
    assert!(
        team_changes(
            r#"{"team_id": "t", "model_aliases": {"gpt": "gpt-4"}}"#,
            table
        )
        .is_empty()
    );
    assert_eq!(
        changed_names(&team_changes(
            r#"{"team_id": "t", "model_aliases": {"gpt": "gpt-3"}}"#,
            table
        )),
        vec!["model_aliases"]
    );
    let text = r#"{"team_id": "t", "litellm_model_table": {"model_aliases": "{\"gpt\": \"gpt-4\"}", "created_by": "c", "updated_by": "u"}}"#;
    assert!(
        team_changes(
            r#"{"team_id": "t", "model_aliases": {"gpt": "gpt-4"}}"#,
            text
        )
        .is_empty()
    );
    assert_eq!(
        changed_names(&team_changes(
            r#"{"team_id": "t", "model_aliases": {"gpt": "gpt-3"}}"#,
            text
        )),
        vec!["model_aliases"]
    );
    assert!(
        team_changes(
            r#"{"team_id": "t", "model_aliases": {}}"#,
            r#"{"team_id": "t"}"#
        )
        .is_empty()
    );
    assert_eq!(
        changed_names(&team_changes(
            r#"{"team_id": "t", "model_aliases": {"a": "b"}}"#,
            r#"{"team_id": "t"}"#
        )),
        vec!["model_aliases"]
    );
    let invalid = r#"{"team_id": "t", "litellm_model_table": {"model_aliases": "not json", "created_by": "c", "updated_by": "u"}}"#;
    assert!(team_changes(r#"{"team_id": "t", "model_aliases": {}}"#, invalid).is_empty());
}

#[test]
fn unstored_field_always_changed() {
    let stored = r#"{"team_id": "t"}"#;
    assert_eq!(
        changed_names(&team_changes(
            r#"{"team_id": "t", "team_member_rpm_limit": 5}"#,
            stored
        )),
        vec!["team_member_rpm_limit"]
    );
    assert_eq!(
        changed_names(&team_changes(
            r#"{"team_id": "t", "team_member_budget": null}"#,
            stored
        )),
        vec!["team_member_budget"]
    );
    assert_eq!(
        changed_names(&team_changes(
            r#"{"team_id": "t", "team_member_budget_duration": "1d"}"#,
            stored
        )),
        vec!["team_member_budget_duration"]
    );
}

#[test]
fn disabled_even_for_noop() {
    let request = r#"{"team_id": "t", "tpm_limit": 10}"#;
    let stored = r#"{"team_id": "t", "tpm_limit": 10}"#;
    let verdict = team_edit_verdict(&team_request(request), &team_row(stored), &BTreeSet::new());
    assert_eq!(verdict.unwrap(), TeamAdminEditVerdict::Disabled);
}

#[test]
fn first_sorted_blocked_field() {
    let stored = r#"{"team_id": "t"}"#;
    let request = r#"{"team_id": "t", "tpm_limit": 1, "rpm_limit": 2, "max_budget": 3.0}"#;
    let verdict = team_edit_verdict(
        &team_request(request),
        &team_row(stored),
        &permitted(&["tpm_limit"]),
    );
    assert_eq!(
        verdict.unwrap(),
        TeamAdminEditVerdict::FieldNotPermitted {
            field: "max_budget".to_owned(),
        }
    );
    let request = r#"{"team_id": "t", "tpm_limit": 1, "rpm_limit": 2, "team_member_rpm_limit": 3}"#;
    let verdict = team_edit_verdict(
        &team_request(request),
        &team_row(stored),
        &permitted(&["tpm_limit"]),
    );
    assert_eq!(
        verdict.unwrap(),
        TeamAdminEditVerdict::FieldNotPermitted {
            field: "rpm_limit".to_owned(),
        }
    );
}

#[test]
fn unchanged_team_values_removed() {
    let stored = r#"{"team_id": "t", "team_alias": "a", "tpm_limit": 10, "budget_duration": "1d", "metadata": {"x": 1}}"#;
    let request = r#"{"team_id": "t", "team_alias": "a", "tpm_limit": 10, "budget_duration": "1d", "rpm_limit": 5, "metadata": {"x": 1}}"#;
    let filtered = allowed_team_edit(request, stored, &["rpm_limit"]);
    assert_eq!(keys(&filtered), vec!["rpm_limit", "team_id"]);
    assert_eq!(filtered.get("rpm_limit"), Some(&parse("5")));
    let filtered = allowed_team_edit(
        r#"{"team_id": "t", "rpm_limit": 5, "metadata": {"x": 1, "y": 2}}"#,
        stored,
        &["metadata", "rpm_limit"],
    );
    assert_eq!(keys(&filtered), vec!["metadata", "rpm_limit", "team_id"]);
}

#[test]
fn nested_change_keeps_metadata() {
    let stored = r#"{"team_id": "t", "metadata": {"nested": {"a": 1}}}"#;
    let request = r#"{"team_id": "t", "metadata": {"nested": {"a": 2, "b": 3}}}"#;
    assert_eq!(
        changed_names(&team_changes(request, stored)),
        vec!["metadata"]
    );
    assert!(
        team_changes(
            r#"{"team_id": "t", "metadata": {"nested": {"a": 1.0}}}"#,
            stored
        )
        .is_empty()
    );
    let filtered = allowed_team_edit(request, stored, &["metadata"]);
    assert_eq!(keys(&filtered), vec!["metadata", "team_id"]);
    assert_eq!(
        filtered.get("metadata"),
        Some(&parse(r#"{"nested": {"a": 2, "b": 3}}"#))
    );
}

#[test]
fn key_identity_ignored() {
    let stored = r#"{"token": "sk-1", "key_alias": "a", "metadata": {"x": 1}}"#;
    assert!(
        key_changes(
            r#"{"key": "sk-2", "key_alias": "a", "metadata": {"x": 1}}"#,
            stored
        )
        .is_empty()
    );
    assert_eq!(
        changed_names(&key_changes(r#"{"key": "sk-2", "key_alias": "b"}"#, stored)),
        vec!["key_alias"]
    );
    assert_eq!(
        changed_names(&key_changes(
            r#"{"key": "sk-2", "metadata": {"x": 2}}"#,
            stored
        )),
        vec!["metadata"]
    );
    let verdict = key_edit_verdict(
        &key_request(r#"{"key": "sk-2", "key_alias": "a"}"#),
        &key_row(stored),
        false,
    );
    assert_eq!(verdict.unwrap(), TeamAdminKeyEditVerdict::Disabled);
}

#[rstest]
#[case::max_budget(
    r#"{"token": "sk-1", "max_budget": 100.0}"#,
    r#"{"key": "sk-1", "max_budget": 200.0}"#,
    "max_budget"
)]
#[case::budget_duration(
    r#"{"token": "sk-1", "budget_duration": "1d"}"#,
    r#"{"key": "sk-1", "budget_duration": "2d"}"#,
    "budget_duration"
)]
#[case::soft_budget(
    r#"{"token": "sk-1"}"#,
    r#"{"key": "sk-1", "soft_budget": 5.0}"#,
    "soft_budget"
)]
#[case::budget_limits(
    r#"{"token": "sk-1"}"#,
    r#"{"key": "sk-1", "budget_limits": [{"budget_duration": "1d", "max_budget": 10.0}]}"#,
    "budget_limits"
)]
fn all_four_budget_fields(#[case] stored: &str, #[case] request: &str, #[case] field: &str) {
    assert_eq!(changed_names(&key_changes(request, stored)), vec![field]);
    let verdict = key_edit_verdict(&key_request(request), &key_row(stored), true);
    assert_eq!(
        verdict.unwrap(),
        TeamAdminKeyEditVerdict::Allowed {
            changed: permitted(&[field]),
        }
    );
}

#[test]
fn unchanged_spend_still_forbidden() {
    let stored = r#"{"token": "sk-1", "spend": 12.5}"#;
    assert!(key_changes(r#"{"key": "sk-1", "spend": 12.5}"#, stored).is_empty());
    let verdict = key_edit_verdict(
        &key_request(r#"{"key": "sk-1", "spend": 12.5}"#),
        &key_row(stored),
        true,
    );
    assert_eq!(
        verdict.unwrap(),
        TeamAdminKeyEditVerdict::FieldNotPermitted {
            field: "spend".to_owned(),
        }
    );
    let verdict = key_edit_verdict(&key_request(r#"{"key": "sk-1"}"#), &key_row(stored), true);
    assert_eq!(
        verdict.unwrap(),
        TeamAdminKeyEditVerdict::Allowed {
            changed: BTreeSet::new(),
        }
    );
}

#[test]
fn window_order_and_reset_ignored() {
    let stored = r#"{"token": "sk-1", "budget_limits": [{"budget_duration": "1d", "max_budget": 10.0, "reset_at": "2024-06-01T00:00:00Z"}, {"budget_duration": "7d", "max_budget": 50.0}]}"#;
    assert!(
        key_changes(
            r#"{"key": "sk-1", "budget_limits": [{"budget_duration": "7d", "max_budget": 50.0}, {"budget_duration": "1d", "max_budget": 10.0}]}"#,
            stored
        )
        .is_empty()
    );
    assert!(
        key_changes(
            r#"{"key": "sk-1", "budget_limits": [{"budget_duration": "1d", "max_budget": 10.0, "reset_at": "2030-01-01T00:00:00Z"}, {"budget_duration": "7d", "max_budget": 50.0, "reset_at": "2030-01-01T00:00:00Z"}]}"#,
            stored
        )
        .is_empty()
    );
    assert_eq!(
        changed_names(&key_changes(
            r#"{"key": "sk-1", "budget_limits": [{"budget_duration": "1d", "max_budget": 11.0}, {"budget_duration": "7d", "max_budget": 50.0}]}"#,
            stored
        )),
        vec!["budget_limits"]
    );
}

#[test]
fn null_and_empty_windows_equal() {
    assert!(
        key_changes(
            r#"{"key": "sk-1", "budget_limits": []}"#,
            r#"{"token": "sk-1"}"#
        )
        .is_empty()
    );
    assert!(
        key_changes(
            r#"{"key": "sk-1", "budget_limits": null}"#,
            r#"{"token": "sk-1", "budget_limits": []}"#
        )
        .is_empty()
    );
    assert!(
        key_changes(
            r#"{"key": "sk-1"}"#,
            r#"{"token": "sk-1", "budget_limits": null}"#
        )
        .is_empty()
    );
}

#[test]
fn duplicate_windows_raw_comparison() {
    let window = r#"{"budget_duration": "1d", "max_budget": 10.0}"#;
    let duplicated = format!("[{window}, {window}]");
    let request = format!(r#"{{"key": "sk-1", "budget_limits": {duplicated}}}"#);
    assert_eq!(
        changed_names(&key_changes(&request, r#"{"token": "sk-1"}"#)),
        vec!["budget_limits"]
    );
    let stored = format!(r#"{{"token": "sk-1", "budget_limits": {duplicated}}}"#);
    assert!(key_changes(&request, &stored).is_empty());
}

#[test]
fn invalid_windows_raw_comparison() {
    let sent_empty = fields(r#"{"budget_limits": []}"#);
    let stored_text = fields(r#"{"budget_limits": "not a list"}"#);
    assert!(column_changed(Columns::Key, "budget_limits", &sent_empty, &stored_text).unwrap());
    let sent_window = fields(r#"{"budget_limits": [{"budget_duration": "1d", "max_budget": 1}]}"#);
    let stored_item = fields(r#"{"budget_limits": [5]}"#);
    assert!(column_changed(Columns::Key, "budget_limits", &sent_window, &stored_item).unwrap());
    let text = fields(r#"{"budget_limits": "oops"}"#);
    assert!(!column_changed(Columns::Key, "budget_limits", &text, &text).unwrap());
    let nulls = fields(r#"{"budget_limits": null}"#);
    assert!(!column_changed(Columns::Key, "budget_limits", &nulls, &nulls).unwrap());
}

#[test]
fn unhashable_window_pair_error() {
    let request =
        r#"{"key": "sk-1", "budget_limits": [{"budget_duration": "1d", "max_budget": 10.0}]}"#;
    let stored =
        r#"{"token": "sk-1", "budget_limits": [{"budget_duration": "1d", "max_budget": [1, 2]}]}"#;
    let error = changed_key_fields(&key_request(request), &key_row(stored)).unwrap_err();
    assert!(matches!(error, TeamAdminError::UnhashableWindow));
    let stored = r#"{"token": "sk-1", "budget_limits": [{"budget_duration": {"nested": 1}}]}"#;
    let error = changed_key_fields(&key_request(request), &key_row(stored)).unwrap_err();
    assert!(matches!(error, TeamAdminError::UnhashableWindow));
}

#[test]
fn four_exact_403_details() {
    let noop = r#"{"team_id": "t"}"#;
    let verdict = team_edit_verdict(&team_request(noop), &team_row(noop), &BTreeSet::new());
    let error = team_request_or_raise(verdict.unwrap()).unwrap_err();
    assert_eq!(error, Forbidden::EditingDisabled);
    assert_eq!(error.to_string(), EDITING_DISABLED);
    let request = r#"{"team_id": "t", "rpm_limit": 2}"#;
    let verdict = team_edit_verdict(
        &team_request(request),
        &team_row(noop),
        &permitted(&["tpm_limit"]),
    );
    let error = team_request_or_raise(verdict.unwrap()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Team admins on this proxy do not have permission to update 'rpm_limit'. Ask a proxy \
        admin to add it under Settings > UI > Team admin editable fields."
    );
    let key = r#"{"token": "sk-1"}"#;
    let verdict = key_edit_verdict(&key_request(r#"{"key": "sk-1"}"#), &key_row(key), false);
    let error = key_request_or_raise(verdict.unwrap()).unwrap_err();
    assert_eq!(error.to_string(), KEY_EDITING_DISABLED);
    let request = r#"{"key": "sk-1", "tpm_limit": 3}"#;
    let verdict = key_edit_verdict(&key_request(request), &key_row(key), true);
    let error = key_request_or_raise(verdict.unwrap()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Team admins on this proxy may only update budget fields on other members' keys, not \
        'tpm_limit'. Ask a proxy admin to add it under Settings > UI > Team admin editable fields."
    );
}

#[rstest]
fn replays_the_team_admin_object_corpus(#[files("fuzz/corpus/team_admin_object/*")] path: PathBuf) {
    let text = fs::read_to_string(path).unwrap();
    drop(decode_team_admin_object(&text));
}

#[test]
fn native_object_limits_become_empty() {
    let at_limit = format!(r#"{{"a": "{}"}}"#, "x".repeat(1_048_567));
    assert_eq!(at_limit.len(), 1_048_576);
    assert_eq!(decode_team_admin_object(&at_limit).len(), 1);
    let over = format!(r#"{{"a": "{}"}}"#, "x".repeat(1_048_568));
    assert_eq!(over.len(), 1_048_577);
    assert!(decode_team_admin_object(&over).is_empty());
    let at_depth = format!(r#"{{"a": {}{}}}"#, "[".repeat(63), "]".repeat(63));
    assert_eq!(decode_team_admin_object(&at_depth).len(), 1);
    let over_depth = format!(r#"{{"a": {}{}}}"#, "[".repeat(64), "]".repeat(64));
    assert!(decode_team_admin_object(&over_depth).is_empty());
    assert!(decode_team_admin_object("{").is_empty());
    assert!(decode_team_admin_object(r#""text""#).is_empty());
    assert!(decode_team_admin_object("[1]").is_empty());
    assert!(decode_team_admin_object("null").is_empty());
    assert_eq!(
        decode_team_admin_object(r#"{"a": 1, "a": 2}"#).get("a"),
        Some(&parse("2"))
    );
}
