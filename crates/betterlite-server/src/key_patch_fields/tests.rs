#![cfg(test)]

use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::key_access::error::DetailShape;
use crate::management_guards::amounts::Amount;

use super::Row;
use super::accessors::{
    general_settings, handle_update_object_permission, is_different_team,
    requested_end_user_budget_id, require_prisma_client, resolve_token_to_update,
};
use super::drop_request_only_fields;
use super::error::KeyPatchError;
use super::metadata::PatchWarning;
use super::prepare::metadata_request_fields;
use super::soft_budget::{
    apply_soft_budget_update, update_key_soft_budget, validate_max_budget,
    validate_soft_budget_value,
};
use super::transaction::{KeyPatchWrite, update_key_row_with_soft_budget, update_statement};

mod support;

use self::support::{
    FakeSoftBudgetStore, assert_all_catalog_fields, assert_close, assert_created_budget,
    assert_isoformat, assert_metadata, assert_pinned_rows, assert_router_invalid,
    assert_router_unavailable, base_request, callback_secret, cipher, existing, fixture, instant,
    json, json_text, number_f64, object, prepare_error, prepared, prepared_row, run_metadata,
    string_field, update_request,
};
use crate::key_patch_fields::fixtures::database::TestDatabase;

#[tokio::test]
async fn metadata_omitted_copies_explicit_null_retained() {
    let fixture = fixture();
    let stored = existing(r#"{"token": "tok", "metadata": {"team": "t1"}}"#);
    let omitted = prepared_row(&fixture, &stored, &update_request(r#"{"key": "tok"}"#)).await;
    assert_metadata(&omitted, r#"{"team": "t1"}"#);
    let explicit = update_request(r#"{"key": "tok", "metadata": null}"#);
    let values = prepared_row(&fixture, &stored, &explicit).await;
    assert_eq!(values.get("metadata"), Some(&JsonData::Null));
}

#[tokio::test]
async fn reserved_id_immutable_and_preserved() {
    let fixture = fixture();
    let stored = existing(r#"{"token": "tok", "metadata": {"service_account_id": "sa-1"}}"#);
    let omitted = prepared_row(&fixture, &stored, &update_request(r#"{"key": "tok"}"#)).await;
    assert_metadata(&omitted, r#"{"service_account_id": "sa-1"}"#);
    let same = r#"{"key": "tok", "metadata": {"service_account_id": "sa-1"}}"#;
    let kept = prepared_row(&fixture, &stored, &update_request(same)).await;
    assert_metadata(&kept, r#"{"service_account_id": "sa-1"}"#);
    let changed = r#"{"key": "tok", "metadata": {"service_account_id": "sa-2"}}"#;
    let error = prepare_error(&fixture, &stored, &update_request(changed)).await;
    assert_eq!(
        error.to_string(),
        "service_account_id is immutable once set and cannot be changed via update."
    );
    assert_eq!(error.response(), (Some(400_u16), Some(DetailShape::Text)));
    let cleared = update_request(r#"{"key": "tok", "metadata": null}"#);
    assert!(matches!(
        prepare_error(&fixture, &stored, &cleared).await,
        KeyPatchError::ImmutableMetadata { .. }
    ));
    let plain = existing(r#"{"token": "tok", "metadata": {"team": "t1"}}"#);
    let fresh = r#"{"key": "tok", "metadata": {"service_account_id": "sa-1"}}"#;
    let fresh_values = prepared_row(&fixture, &plain, &update_request(fresh)).await;
    assert_metadata(&fresh_values, r#"{"service_account_id": "sa-1"}"#);
}

#[tokio::test]
async fn all_metadata_field_names_and_datetime() {
    let fixture = fixture();
    assert_all_catalog_fields();
    assert_isoformat();
    let stored = existing(r#"{"token": "tok"}"#);
    let dated = r#"{"key": "tok", "temp_budget_increase": 5,
        "temp_budget_expiry": "2026-05-02T10:00:00.500000+02:00"}"#;
    let request = update_request(dated);
    let values = prepared_row(&fixture, &stored, &request).await;
    let Some(JsonData::Object(fields)) = values.get("metadata") else {
        panic!("metadata is an object");
    };
    assert_eq!(
        fields.get("temp_budget_expiry"),
        Some(&json_text("2026-05-02T10:00:00.500000+02:00"))
    );
    let request_fields = metadata_request_fields(&request).expect("the request's metadata fields");
    assert_eq!(
        request_fields.get("temp_budget_expiry"),
        Some(&json_text("2026-05-02T10:00:00.500000+02:00"))
    );
}

#[tokio::test]
async fn metadata_logging_check_first() {
    let fixture = fixture();
    let stored = existing(
        r#"{"token": "tok", "team_id": "t1", "metadata": {"service_account_id": "sa-1"}}"#,
    );
    let request = r#"{"key": "tok", "metadata": {"service_account_id": "sa-2",
        "logging": [{"callback_name": "custom", "callback_vars": {"nested": []}}]}}"#;
    let error = prepare_error(&fixture, &stored, &update_request(request)).await;
    assert!(matches!(error, KeyPatchError::LoggingMetadata(_)));
    assert_eq!(error.response(), (Some(400_u16), Some(DetailShape::Error)));
}

#[tokio::test]
async fn metadata_partial_loop_error_logged() {
    let fixture = fixture();
    let stored = existing(r#"{"token": "tok", "metadata": {}}"#);
    let request = r#"{"key": "tok", "metadata": null, "end_user_budget_id": "budget-1",
        "enable_prompt_caching": true}"#;
    let (result, warnings) = prepared(&fixture, &stored, &update_request(request)).await;
    let values = result.expect("the request is kept");
    assert_eq!(values.get("metadata"), Some(&JsonData::Null));
    assert_eq!(warnings, vec![PatchWarning::MetadataNotAnObject]);
    assert_eq!(
        warnings
            .iter()
            .map(|warning| warning.subject())
            .collect::<Vec<_>>(),
        vec!["metadata"]
    );
    assert!(!values.contains_key("end_user_budget_id"));
    assert!(!values.contains_key("enable_prompt_caching"));
    let premium_only = r#"{"key": "tok", "metadata": null, "tags": ["a"]}"#;
    let (premium_result, premium_warnings) =
        prepared(&fixture, &stored, &update_request(premium_only)).await;
    let premium_values = premium_result.expect("the request is kept");
    assert_eq!(premium_values.get("metadata"), Some(&JsonData::Null));
    assert_eq!(premium_warnings, vec![PatchWarning::MetadataNotAnObject]);
}

// README excludes paid gating: every premium field is projected, where LiteLLM's paid check aborts the loop.
#[test]
fn native_metadata_projection_without_license() {
    assert_all_catalog_fields();
    let request = object(r#"{"guardrails": ["g"], "tags": ["a"], "logging": []}"#);
    let (result, warnings) = run_metadata(&request, &Row::new(), &cipher());
    let JsonData::Object(projected) = result.expect("projected metadata") else {
        panic!("metadata is an object");
    };
    assert_eq!(projected.get("guardrails"), Some(&json(r#"["g"]"#)));
    assert_eq!(projected.get("tags"), Some(&json(r#"["a"]"#)));
    assert_eq!(projected.get("logging"), Some(&json("[]")));
    assert!(warnings.is_empty());
}

#[test]
fn callbacks_encrypted_before_return() {
    let request = r#"{"logging": [{"callback_name": "langfuse",
        "callback_vars": {"langfuse_secret": "plain-value", "host": "https://langfuse"}}]}"#;
    let entry = callback_secret(&object(request), &cipher());
    let JsonData::String(secret) = entry else {
        panic!("the secret is a string");
    };
    assert!(secret.starts_with("litellm_enc::"));
    assert_ne!(secret, "plain-value");
}

#[test]
fn callback_existing_ciphertext_retained() {
    let request = object(
        r#"{"logging": [{"callback_name": "langfuse",
            "callback_vars": {"langfuse_secret": "litellm_enc::sealed"}}]}"#,
    );
    assert_eq!(
        callback_secret(&request, &cipher()),
        json_text("litellm_enc::sealed")
    );
}

#[test]
fn callback_encrypt_failure_original_value() {
    let request = object(
        r#"{"logging": [{"callback_name": "langfuse",
            "callback_vars": {"langfuse_secret": "plain-value"}}]}"#,
    );
    assert_eq!(
        callback_secret(&request, &failing_cipher()),
        json_text("plain-value")
    );
}

#[rstest]
#[case::absent(None, None)]
#[case::zero(Some(Amount::Finite(Decimal::ZERO)), None)]
#[case::positive(Some(Amount::Finite(Decimal::new(25, 1))), None)]
#[case::negative_zero(Some(Amount::Finite(-Decimal::ZERO)), None)]
#[case::nan(Some(Amount::Nan), Some("nan"))]
#[case::positive_infinity(Some(Amount::PositiveInfinity), Some("inf"))]
#[case::negative_infinity(Some(Amount::NegativeInfinity), Some("-inf"))]
#[case::negative(Some(Amount::Finite(Decimal::new(-5, 0))), Some("-5"))]
fn soft_and_max_negative_nonfinite_400(
    #[case] amount: Option<Amount>,
    #[case] rejected: Option<&str>,
) {
    let Some(received) = rejected else {
        validate_soft_budget_value(amount).unwrap();
        validate_max_budget(amount).unwrap();
        return;
    };
    let soft = validate_soft_budget_value(amount).unwrap_err();
    assert_eq!(
        soft.to_string(),
        format!("soft_budget must be a non-negative finite number. Received: {received}")
    );
    assert_eq!(soft.response(), (Some(400_u16), Some(DetailShape::Error)));
    let hard = validate_max_budget(amount).unwrap_err();
    assert_eq!(
        hard.to_string(),
        format!("max_budget must be a non-negative finite number. Received: {received}")
    );
    assert_eq!(hard.response(), (Some(400_u16), Some(DetailShape::Error)));
}

#[tokio::test]
async fn soft_existing_null_clears() {
    let store = FakeSoftBudgetStore::with_id("ignored");
    let budget = update_key_soft_budget(&store, Some("budget-1"), None, "admin")
        .await
        .unwrap();
    assert_eq!(budget, Some(String::from("budget-1")));
    assert_eq!(
        store.updates(),
        vec![(String::from("budget-1"), None, String::from("admin"))]
    );
    let amount = Some(Amount::Finite(Decimal::new(45, 1)));
    update_key_soft_budget(&store, Some("budget-1"), amount, "admin")
        .await
        .unwrap();
    assert_eq!(
        store.updates()[1],
        (
            String::from("budget-1"),
            Some(Decimal::new(45, 1)),
            String::from("admin")
        )
    );
    assert!(store.created().is_empty());
}

#[tokio::test]
async fn soft_missing_null_no_create() {
    let store = FakeSoftBudgetStore::with_id("budget-new");
    let budget = update_key_soft_budget(&store, None, None, "admin")
        .await
        .unwrap();
    assert_eq!(budget, None);
    assert!(store.updates().is_empty());
    assert!(store.created().is_empty());
}

#[tokio::test]
async fn soft_create_links_budget() {
    let store = FakeSoftBudgetStore::with_id("budget-new");
    let amount = Some(Amount::Finite(Decimal::new(5, 0)));
    let values = object(r#"{"key_alias": "a", "soft_budget": 5}"#);
    let applied = apply_soft_budget_update(&store, amount, values, None, "admin")
        .await
        .unwrap();
    assert_eq!(applied.get("budget_id"), Some(&json_text("budget-new")));
    assert!(!applied.contains_key("soft_budget"));
    assert_eq!(
        store.created(),
        vec![(Decimal::new(5, 0), String::from("admin"))]
    );
    let kept = object(r#"{"budget_id": "budget-1", "soft_budget": 5}"#);
    let kept_patch = apply_soft_budget_update(&store, amount, kept, Some("budget-1"), "admin")
        .await
        .unwrap();
    assert_eq!(kept_patch.get("budget_id"), Some(&json_text("budget-1")));
    assert!(!kept_patch.contains_key("soft_budget"));
    assert_eq!(store.created().len(), 1);
}

#[tokio::test]
async fn router_revalidation_presence_matrix() {
    let fixture = fixture();
    let weighted = existing(
        r#"{"token": "tok", "team_id": "t1", "router_settings": "{\"weights\": {\"gpt\": {\"dep-1\": 1.0}}}"}"#,
    );
    let plain = existing(r#"{"token": "tok", "team_id": "t1"}"#);
    let explicit = r#"{"key": "tok", "router_settings": {"weights": {"gpt": {"dep-1": 1.0}}}}"#;
    assert_router_unavailable(&fixture, &plain, &update_request(explicit)).await;
    let team_change = update_request(r#"{"key": "tok", "team_id": "t2"}"#);
    assert_router_unavailable(&fixture, &weighted, &team_change).await;
    let suppressed = update_request(r#"{"key": "tok", "team_id": "t2", "router_settings": null}"#);
    prepared_row(&fixture, &weighted, &suppressed).await;
    prepared_row(&fixture, &plain, &team_change).await;
    let same_team = update_request(r#"{"key": "tok", "team_id": "t1"}"#);
    prepared_row(&fixture, &weighted, &same_team).await;
    let no_weights =
        update_request(r#"{"key": "tok", "router_settings": {"allow_retries": true}}"#);
    let values = prepared_row(&fixture, &plain, &no_weights).await;
    assert_eq!(
        string_field(&values, "router_settings"),
        "{\"allow_retries\":true}"
    );
    let zero = r#"{"key": "tok", "router_settings": {"weights": {"gpt": {"dep-1": 0.0}}}}"#;
    assert_router_invalid(&fixture, &plain, &update_request(zero)).await;
}

#[tokio::test]
async fn service_account_existing_team_fallback() {
    let fixture = fixture();
    let account = r#"{"key": "tok", "metadata": {"service_account_id": "sa-1"}}"#;
    let with_team = existing(r#"{"token": "tok", "team_id": "t1"}"#);
    prepared_row(&fixture, &with_team, &update_request(account)).await;
    let without_team = existing(r#"{"token": "tok"}"#);
    let error = prepare_error(&fixture, &without_team, &update_request(account)).await;
    assert!(matches!(error, KeyPatchError::ServiceAccountTeamRequired));
    assert_eq!(error.response(), (Some(400_u16), Some(DetailShape::Text)));
    let clearing = r#"{"key": "tok", "team_id": null,
        "metadata": {"service_account_id": "sa-1"}}"#;
    prepared_row(&fixture, &with_team, &update_request(clearing)).await;
    assert!(matches!(
        prepare_error(&fixture, &without_team, &update_request(clearing)).await,
        KeyPatchError::ServiceAccountTeamRequired
    ));
    let new_team = r#"{"key": "tok", "team_id": "t2", "metadata": {"service_account_id": "sa-1"}}"#;
    prepared_row(&fixture, &without_team, &update_request(new_team)).await;
    let empty_account = r#"{"key": "tok", "metadata": {"service_account_id": null}}"#;
    prepared_row(&fixture, &without_team, &update_request(empty_account)).await;
}

#[tokio::test]
async fn request_only_fields_removed() {
    let fixture = fixture();
    let request = update_request(r#"{"key": "tok", "key_alias": "a"}"#);
    let values = prepared_row(&fixture, &existing(r#"{"token": "tok"}"#), &request).await;
    assert!(!values.contains_key("key"));
    assert_eq!(values.get("key_alias"), Some(&json_text("a")));
    let mut fields =
        object(r#"{"key": "sk-1", "new_key": "sk-2", "grace_period": 5, "key_alias": "a"}"#);
    drop_request_only_fields(&mut fields);
    assert_eq!(fields, object(r#"{"key_alias": "a"}"#));
}

#[tokio::test]
async fn duration_null_minus_one_empty_and_valid() {
    let fixture = fixture();
    let stored = existing(r#"{"token": "tok"}"#);
    let null = update_request(r#"{"key": "tok", "duration": null}"#);
    let null_values = prepared_row(&fixture, &stored, &null).await;
    assert_eq!(null_values.get("expires"), Some(&JsonData::Null));
    let never = update_request(r#"{"key": "tok", "duration": "-1"}"#);
    let never_values = prepared_row(&fixture, &stored, &never).await;
    assert_eq!(never_values.get("expires"), Some(&JsonData::Null));
    let empty = update_request(r#"{"key": "tok", "duration": ""}"#);
    let empty_values = prepared_row(&fixture, &stored, &empty).await;
    assert!(!empty_values.contains_key("expires"));
    assert!(!empty_values.contains_key("duration"));
    let hour = update_request(r#"{"key": "tok", "duration": "1h"}"#);
    let hour_values = prepared_row(&fixture, &stored, &hour).await;
    assert_eq!(
        string_field(&hour_values, "expires"),
        "2026-05-01T11:00:00.123456Z"
    );
}

#[tokio::test]
async fn budget_duration_null_empty_valid() {
    let fixture = fixture();
    let stored = existing(r#"{"token": "tok"}"#);
    let null = update_request(r#"{"key": "tok", "budget_duration": null}"#);
    let null_values = prepared_row(&fixture, &stored, &null).await;
    assert_eq!(null_values.get("budget_duration"), Some(&JsonData::Null));
    assert_eq!(null_values.get("budget_reset_at"), Some(&JsonData::Null));
    let empty = update_request(r#"{"key": "tok", "budget_duration": ""}"#);
    let empty_values = prepared_row(&fixture, &stored, &empty).await;
    assert!(!empty_values.contains_key("budget_duration"));
    assert!(!empty_values.contains_key("budget_reset_at"));
    let hour = update_request(r#"{"key": "tok", "budget_duration": "1h"}"#);
    let hour_values = prepared_row(&fixture, &stored, &hour).await;
    assert_eq!(string_field(&hour_values, "budget_duration"), "1h");
    assert_eq!(
        string_field(&hour_values, "budget_reset_at"),
        "2026-05-01T11:00:00Z"
    );
}

#[rstest]
#[case::empty_list(r#"{"key": "tok", "budget_limits": []}"#, "null")]
#[case::json_null(r#"{"key": "tok", "budget_limits": null}"#, "null")]
#[tokio::test]
async fn windows_json_null_vs_sql_null(#[case] document: &str, #[case] expected: &str) {
    let fixture = fixture();
    let stored = existing(r#"{"token": "tok"}"#);
    let values = prepared_row(&fixture, &stored, &update_request(document)).await;
    assert_eq!(values.get("budget_limits"), Some(&json_text(expected)));
}

#[tokio::test]
async fn budget_windows_reset_at_serialized() {
    let fixture = fixture();
    let stored = existing(r#"{"token": "tok"}"#);
    let windows = r#"{"key": "tok",
        "budget_limits": [{"budget_duration": "1h", "max_budget": 5}]}"#;
    let values = prepared_row(&fixture, &stored, &update_request(windows)).await;
    let JsonData::Array(items) = json(string_field(&values, "budget_limits")) else {
        panic!("a window list");
    };
    let Some(JsonData::Object(window)) = items.first() else {
        panic!("a window");
    };
    assert_eq!(window.get("budget_duration"), Some(&json_text("1h")));
    assert_eq!(
        window.get("reset_at"),
        Some(&json_text("2026-05-01T11:00:00+00:00"))
    );
    assert_eq!(window.get("max_budget"), Some(&json("5")));
}

#[tokio::test]
async fn model_budget_validation_and_router_json() {
    let fixture = fixture();
    let stored = existing(r#"{"token": "tok"}"#);
    let invalid = r#"{"key": "tok", "model_max_budget": {"gpt-4": {"tpm_limit": "many"}}}"#;
    let error = prepare_error(&fixture, &stored, &update_request(invalid)).await;
    assert!(matches!(error, KeyPatchError::ModelBudget(_)));
    assert!(
        error
            .to_string()
            .contains("gpt-4.tpm_limit must be an integer")
    );
    let valid = r#"{"key": "tok", "model_max_budget": {"gpt-4": {"max_budget": 5}}}"#;
    let valid_values = prepared_row(&fixture, &stored, &update_request(valid)).await;
    assert!(matches!(
        valid_values.get("model_max_budget"),
        Some(JsonData::Object(_))
    ));
    let settings = r#"{"key": "tok", "router_settings": {"allow_retries": true}}"#;
    let settings_values = prepared_row(&fixture, &stored, &update_request(settings)).await;
    assert_eq!(
        string_field(&settings_values, "router_settings"),
        "{\"allow_retries\":true}"
    );
    let null = update_request(r#"{"key": "tok", "router_settings": null}"#);
    let null_values = prepared_row(&fixture, &stored, &null).await;
    assert_eq!(null_values.get("router_settings"), Some(&JsonData::Null));
    let omitted = prepared_row(
        &fixture,
        &stored,
        &update_request(r#"{"key": "tok", "key_alias": "a"}"#),
    )
    .await;
    assert!(!omitted.contains_key("router_settings"));
}

#[test]
fn different_team_null_semantics() {
    assert!(!is_different_team(None, None));
    assert!(!is_different_team(None, Some("t1")));
    assert!(is_different_team(Some("t1"), None));
    assert!(!is_different_team(Some("t1"), Some("t1")));
    assert!(is_different_team(Some("t2"), Some("t1")));
}

#[test]
fn update_token_empty_and_missing_404() {
    assert_eq!(
        resolve_token_to_update(Some(""), Some("stored")).unwrap(),
        ""
    );
    assert_eq!(
        resolve_token_to_update(Some("request"), Some("stored")).unwrap(),
        "request"
    );
    assert_eq!(
        resolve_token_to_update(None, Some("stored")).unwrap(),
        "stored"
    );
    let error = resolve_token_to_update(None, None).unwrap_err();
    assert!(matches!(error, KeyPatchError::KeyNotFound));
    assert_eq!(error.response(), (Some(404_u16), Some(DetailShape::Text)));
    let exception = error.proxy_exception().expect("the ProxyException shape");
    assert_eq!(exception.message, "Key not found.");
    assert_eq!(exception.error_type, "not_found_error");
    assert_eq!(exception.param.as_deref(), Some("key"));
    assert_eq!(exception.code, "404");
}

#[test]
fn no_db_500() {
    let error = require_prisma_client(None).err().expect("the 500");
    assert!(matches!(error, KeyPatchError::DatabaseNotConnected));
    assert_eq!(error.to_string(), "Database not connected");
    assert_eq!(error.response(), (Some(500_u16), Some(DetailShape::Error)));
    assert!(error.proxy_exception().is_none());
}

#[rstest]
#[case::key_only(r#"{"key": "k"}"#, None)]
#[case::metadata_null(r#"{"key": "k", "metadata": null}"#, None)]
#[case::empty_metadata(r#"{"key": "k", "metadata": {}}"#, Some(""))]
#[case::empty_metadata_id(r#"{"key": "k", "metadata": {"end_user_budget_id": ""}}"#, Some(""))]
#[case::metadata_id(
    r#"{"key": "k", "metadata": {"end_user_budget_id": "meta"}}"#,
    Some("meta")
)]
#[case::top_wins(
    r#"{"key": "k", "end_user_budget_id": "top",
        "metadata": {"end_user_budget_id": "meta"}}"#,
    Some("top")
)]
#[case::null_top_falls_back(
    r#"{"key": "k", "end_user_budget_id": null,
        "metadata": {"end_user_budget_id": "meta"}}"#,
    Some("meta")
)]
#[case::empty_top(r#"{"key": "k", "end_user_budget_id": ""}"#, Some(""))]
fn requested_end_user_budget_clear_and_precedence(
    #[case] document: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        requested_end_user_budget_id(&base_request(document)).as_deref(),
        expected
    );
}

#[rstest]
#[case::tree(r#"{"store_model_in_db": true}"#, true)]
#[case::null(r#"null"#, false)]
#[case::array(r#"[]"#, false)]
#[case::text(r#""text""#, false)]
#[case::number(r#"7"#, false)]
fn general_settings_mapping_validation(#[case] document: &str, #[case] mapping: bool) {
    let outcome = general_settings(&json(document));
    if mapping {
        let settings = outcome.expect("a mapping");
        assert_eq!(
            settings.get("store_model_in_db"),
            Some(&JsonData::Bool(true))
        );
        return;
    }
    assert!(matches!(outcome, Err(KeyPatchError::GeneralSettings)));
}

// the statement names only the patch's own keys, and only while they are column names
#[test]
fn statement_column_guard() {
    let statement = update_statement(&object(r#"{"key_alias": "a"}"#)).expect("a statement");
    assert!(statement.contains("\"key_alias\" = r.\"key_alias\""));
    let digits = update_statement(&object(r#"{"col1": "a"}"#)).expect("a statement");
    assert!(digits.contains("\"col1\" = r.\"col1\""));
    let mut empty = Row::new();
    empty.insert(String::new(), JsonData::Null);
    assert!(matches!(
        update_statement(&empty),
        Err(KeyPatchError::UnknownColumn { .. })
    ));
    let mut spaced = Row::new();
    spaced.insert(String::from("key alias"), JsonData::Null);
    assert!(matches!(
        update_statement(&spaced),
        Err(KeyPatchError::UnknownColumn { .. })
    ));
    let mut upper = Row::new();
    upper.insert(String::from("Key"), JsonData::Null);
    assert!(matches!(
        update_statement(&upper),
        Err(KeyPatchError::UnknownColumn { .. })
    ));
    let mut long = Row::new();
    long.insert("x".repeat(65), JsonData::Null);
    assert!(matches!(
        update_statement(&long),
        Err(KeyPatchError::UnknownColumn { .. })
    ));
}

#[tokio::test]
async fn soft_transaction_budget_then_key() {
    let database = TestDatabase::start().await;
    database
        .seed_key("token-2", Some("alias"), "{}", None, None)
        .await;
    let write = KeyPatchWrite {
        key: "token-2",
        soft_budget: Some(Amount::Finite(Decimal::new(3, 0))),
        values: object(r#"{"soft_budget": 3, "key_alias": "linked"}"#),
        existing_budget_id: None,
        changed_by: "admin",
        now: instant("2026-05-01T10:00:00Z"),
    };
    let outcome = update_key_row_with_soft_budget(database.prisma(), &write)
        .await
        .expect("a committed patch");
    assert_created_budget(&database, &outcome).await;
    database.close().await;
}

#[tokio::test]
async fn object_permission_absent_or_updated() {
    let database = TestDatabase::start().await;
    let mut absent = object(r#"{"key_alias": "a"}"#);
    handle_update_object_permission(&mut absent, None, database.prisma(), &[])
        .await
        .unwrap();
    assert_eq!(absent, object(r#"{"key_alias": "a"}"#));
    assert_eq!(database.permission_count().await, 0);
    let mut created = object(r#"{"object_permission": {"models": ["gpt-4"]}}"#);
    handle_update_object_permission(&mut created, None, database.prisma(), &[])
        .await
        .unwrap();
    assert!(!created.contains_key("object_permission"));
    let Some(JsonData::String(permission_id)) = created.get("object_permission_id") else {
        panic!("the created permission id");
    };
    assert_eq!(database.permission_count().await, 1);
    let mut updated = object(r#"{"object_permission": {"models": ["gpt-3.5"]}}"#);
    handle_update_object_permission(
        &mut updated,
        Some(permission_id.as_str()),
        database.prisma(),
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        updated.get("object_permission_id"),
        Some(&json_text(permission_id))
    );
    assert_eq!(database.permission_count().await, 1);
    database.close().await;
}

#[tokio::test]
async fn postgres_pinned_rows() {
    let database = TestDatabase::start().await;
    database.seed_budget("budget-1", Some(1.0)).await;
    database.seed_permission("perm-1").await;
    database
        .seed_key(
            "token-1",
            Some("alias"),
            r#"{"team": "t1"}"#,
            Some("budget-1"),
            Some("t1"),
        )
        .await;
    let write = KeyPatchWrite {
        key: "token-1",
        soft_budget: Some(Amount::Finite(Decimal::new(7, 1))),
        values: object(
            r#"{"key_alias": "renamed", "object_permission_id": "perm-1",
                "metadata": {"team": "t1", "owner": "ops"}}"#,
        ),
        existing_budget_id: Some("budget-1"),
        changed_by: "admin",
        now: instant("2026-05-01T10:00:00.123456Z"),
    };
    let outcome = update_key_row_with_soft_budget(database.prisma(), &write)
        .await
        .expect("a committed patch");
    assert_pinned_rows(&database, &outcome).await;
    database.close().await;
}

#[tokio::test]
async fn postgres_failure_effects() {
    let database = TestDatabase::start().await;
    database.seed_budget("budget-1", Some(1.0)).await;
    database
        .seed_key("token-3", Some("kept"), "{}", Some("budget-1"), None)
        .await;
    let write = KeyPatchWrite {
        key: "token-3",
        soft_budget: Some(Amount::Finite(Decimal::new(9, 0))),
        values: object(r#"{"key_alias": "rejected"}"#),
        existing_budget_id: Some("budget-1"),
        changed_by: "admin",
        now: instant("2026-05-01T10:00:00Z"),
    };
    let error = update_key_row_with_soft_budget(database.prisma(), &write)
        .await
        .err()
        .expect("the key write fails");
    assert!(matches!(error, KeyPatchError::Database(_)));
    let budget = database
        .budget_row("budget-1")
        .await
        .expect("the budget row");
    assert_close(number_f64(&budget, "soft_budget"), 1.0_f64);
    assert_eq!(budget.get("updated_by"), Some(&json_text("seed")));
    let key = database.key_row("token-3").await.expect("the key row");
    assert_eq!(key.get("key_alias"), Some(&json_text("kept")));
    database.close().await;
}
