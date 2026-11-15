#![cfg(test)]

use axum::http::StatusCode;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_identity::spec::DeploymentSpec;

use super::error::RouterWeightError;
use super::model_info::{RouterWeightModelInfo, RouterWeightModelInfoError};
use super::reference::{RouterWeightDeployment, validate_weight_reference};
use super::settings::validate_settings_weights;
use super::weights::{RouterWeightsError, decode_router_weights};

mod support;

const INVALID_SETTINGS: &str = "Invalid router weights. Replace or clear router_settings.weights.";
const DATABASE_UNAVAILABLE: &str = "Database unavailable while validating router weights";

const TABLE_SCHEMA: &str = r#"
    CREATE TABLE "LiteLLM_ProxyModelTable" (
        model_id text PRIMARY KEY,
        model_name text NOT NULL,
        litellm_params jsonb NOT NULL,
        model_info jsonb
    );
"#;

const SCANNED_SCHEMA: &str = r#"
    CREATE TABLE "WeightRows" (
        model_id text PRIMARY KEY,
        model_name text NOT NULL,
        litellm_params jsonb NOT NULL,
        model_info jsonb
    );
    CREATE TABLE "WeightLookupLog" (at timestamptz NOT NULL DEFAULT now());
    CREATE FUNCTION count_weight_lookups() RETURNS SETOF "WeightRows" LANGUAGE plpgsql AS $count$
    BEGIN
        INSERT INTO "WeightLookupLog" DEFAULT VALUES;
        RETURN QUERY SELECT * FROM "WeightRows";
    END;
    $count$;
    CREATE VIEW "LiteLLM_ProxyModelTable" AS SELECT * FROM count_weight_lookups();
"#;

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn deployment(
    model_name: &str,
    team_id: Option<&str>,
    db_model: Option<bool>,
    public_name: Option<&str>,
) -> RouterWeightDeployment {
    RouterWeightDeployment {
        model_name: model_name.to_owned(),
        model_info: RouterWeightModelInfo {
            team_id: team_id.map(str::to_owned),
            db_model,
            team_public_model_name: public_name.map(str::to_owned),
        },
    }
}

fn configured(model_name: &str, model_info: &str) -> DeploymentSpec {
    let Json::Object(fields) = json(model_info) else {
        panic!("a configured deployment needs a model_info object");
    };
    DeploymentSpec {
        model_name: model_name.to_owned(),
        litellm_params: JsonObject::new(),
        model_info: fields,
        extra: JsonObject::new(),
    }
}

fn assert_unknown(error: &RouterWeightError, deployment_id: &str) {
    assert_eq!(error.status_code(), Some(StatusCode::BAD_REQUEST));
    assert_eq!(
        error.to_string(),
        format!("Unknown deployment ID in router weights: {deployment_id}")
    );
}

fn assert_wrong_group(error: &RouterWeightError, deployment_id: &str, model_group: &str) {
    assert_eq!(error.status_code(), Some(StatusCode::BAD_REQUEST));
    assert_eq!(
        error.to_string(),
        format!("Deployment {deployment_id} does not belong to model group {model_group}")
    );
}

fn assert_invalid_settings(error: &RouterWeightError) {
    assert_eq!(error.status_code(), Some(StatusCode::BAD_REQUEST));
    assert_eq!(error.to_string(), INVALID_SETTINGS);
}

fn assert_falsey_model_info_defaults(values: &[&str]) {
    for value in values {
        let decoded = RouterWeightModelInfo::decode(&json(value)).unwrap();
        assert_eq!(decoded, RouterWeightModelInfo::default(), "{value}");
    }
}

fn assert_model_info_not_an_object(values: &[&str]) {
    for value in values {
        let error = RouterWeightModelInfo::decode(&json(value)).unwrap_err();
        assert!(
            matches!(error, RouterWeightModelInfoError::NotAnObject),
            "{value}"
        );
    }
}

fn assert_model_info_invalid_json(values: &[&str]) {
    for value in values {
        let error = RouterWeightModelInfo::decode(&json(value)).unwrap_err();
        assert!(
            matches!(error, RouterWeightModelInfoError::Json(_)),
            "{value}"
        );
    }
}

fn assert_model_info_field_type(values: &[&str]) {
    for value in values {
        let error = RouterWeightModelInfo::decode(&json(value)).unwrap_err();
        assert!(
            matches!(error, RouterWeightModelInfoError::FieldType { .. }),
            "{value}"
        );
    }
}

#[test]
fn stored_precedes_configured() {
    let stored = deployment("gpt-4", None, Some(true), None);
    let configured_deployment = deployment("other-group", Some("team-b"), Some(true), None);
    let checked = validate_weight_reference(
        "gpt-4",
        "dep-1",
        Some("team-a"),
        Some(&stored),
        Some(&configured_deployment),
    );
    assert!(checked.is_ok());
}

#[test]
fn missing_id() {
    let error = validate_weight_reference("gpt-4", "dep-1", None, None, None).unwrap_err();
    assert_unknown(&error, "dep-1");
}

#[test]
fn stale_db_model() {
    let configured_deployment = deployment("gpt-4", None, Some(true), None);
    let error =
        validate_weight_reference("gpt-4", "dep-1", None, None, Some(&configured_deployment))
            .unwrap_err();
    assert_unknown(&error, "dep-1");
}

#[test]
fn other_team_hidden() {
    let configured_deployment = deployment("gpt-4", Some("team-b"), None, None);
    let error = validate_weight_reference(
        "gpt-4",
        "dep-1",
        Some("team-a"),
        None,
        Some(&configured_deployment),
    )
    .unwrap_err();
    assert_unknown(&error, "dep-1");
}

#[test]
fn same_team_public_name() {
    let stored = deployment("internal", Some("team-a"), None, Some("gpt-4"));
    let checked = validate_weight_reference("gpt-4", "dep-1", Some("team-a"), Some(&stored), None);
    assert!(checked.is_ok());
}

#[test]
fn global_model_in_team() {
    let stored = deployment("gpt-4", None, None, None);
    let checked = validate_weight_reference("gpt-4", "dep-1", Some("team-a"), Some(&stored), None);
    assert!(checked.is_ok());
}

#[test]
fn global_ignores_public_name() {
    let stored = deployment("gpt-4", None, None, Some("public-name"));
    let checked = validate_weight_reference("gpt-4", "dep-1", Some("team-a"), Some(&stored), None);
    assert!(checked.is_ok());
}

#[test]
fn empty_public_name_falls_back() {
    let stored = deployment("gpt-4", Some("team-a"), None, Some(""));
    let checked = validate_weight_reference("gpt-4", "dep-1", Some("team-a"), Some(&stored), None);
    assert!(checked.is_ok());
}

#[test]
fn wrong_group() {
    let stored = deployment("renamed", None, None, None);
    let error = validate_weight_reference("gpt-4", "dep-1", None, Some(&stored), None).unwrap_err();
    assert_wrong_group(&error, "dep-1", "gpt-4");
}

#[test]
fn team_public_name_is_canonical() {
    let stored = deployment("gpt-4", Some("team-a"), None, Some("public-name"));
    let error = validate_weight_reference("gpt-4", "dep-1", Some("team-a"), Some(&stored), None)
        .unwrap_err();
    assert_wrong_group(&error, "dep-1", "gpt-4");
}

#[test]
fn stored_other_team_hidden() {
    let stored = deployment("gpt-4", Some("team-b"), None, None);
    let error = validate_weight_reference("gpt-4", "dep-1", Some("team-a"), Some(&stored), None)
        .unwrap_err();
    assert_unknown(&error, "dep-1");
}

#[test]
fn deployment_fixture_decodes_model_info() {
    let decoded: RouterWeightDeployment = serde_json::from_str(
        r#"{"model_name": "gpt-4", "model_info": "{\"team_id\": \"team-a\"}"}"#,
    )
    .unwrap();
    assert_eq!(
        decoded,
        RouterWeightDeployment {
            model_name: "gpt-4".to_owned(),
            model_info: RouterWeightModelInfo {
                team_id: Some("team-a".to_owned()),
                db_model: None,
                team_public_model_name: None,
            },
        }
    );
    assert!(
        serde_json::from_str::<RouterWeightDeployment>(
            r#"{"model_name": "gpt-4", "model_info": {"team_id": 1}}"#
        )
        .is_err()
    );
}

#[tokio::test]
async fn settings_validation_null_settings() {
    assert!(
        validate_settings_weights(None, None, None, None)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn settings_validation_missing_weights() {
    let settings = json(r#"{"num_retries": 2, "callbacks": []}"#);
    assert!(
        validate_settings_weights(Some(&settings), Some("team-a"), None, None)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn settings_validation_null_weights() {
    let settings = json(r#"{"weights": null}"#);
    assert!(
        validate_settings_weights(Some(&settings), None, None, None)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn settings_validation_empty_weights() {
    let settings = json(r#"{"weights": {}}"#);
    assert!(
        validate_settings_weights(Some(&settings), None, None, None)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn settings_validation_empty_groups() {
    let settings = json(r#"{"weights": {"gpt-4": {}, "gpt-4o": {}}}"#);
    assert!(
        validate_settings_weights(Some(&settings), None, None, None)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn settings_validation_malformed_weights() {
    let settings = json(r#"{"weights": {"gpt-4": {"dep-1": -1}}}"#);
    let error = validate_settings_weights(Some(&settings), None, None, None)
        .await
        .unwrap_err();
    assert_invalid_settings(&error);
}

#[tokio::test]
async fn settings_validation_non_object_settings() {
    let settings = json(r#"[{"weights": {}}]"#);
    let error = validate_settings_weights(Some(&settings), None, None, None)
        .await
        .unwrap_err();
    assert_invalid_settings(&error);
}

#[tokio::test]
async fn settings_validation_non_object_weights() {
    let settings = json(r#"{"weights": []}"#);
    let error = validate_settings_weights(Some(&settings), None, None, None)
        .await
        .unwrap_err();
    assert_invalid_settings(&error);
}

#[tokio::test]
async fn settings_validation_non_object_group() {
    let settings = json(r#"{"weights": {"gpt-4": 1}}"#);
    let error = validate_settings_weights(Some(&settings), None, None, None)
        .await
        .unwrap_err();
    assert_invalid_settings(&error);
}

#[tokio::test]
async fn settings_validation_whitespace_identifier() {
    let settings = json(r#"{"weights": {"gpt-4": {" ": 1}}}"#);
    let error = validate_settings_weights(Some(&settings), None, None, None)
        .await
        .unwrap_err();
    assert_invalid_settings(&error);
}

#[tokio::test]
async fn settings_validation_requires_positive_weight() {
    let settings = json(r#"{"weights": {"gpt-4": {"dep-1": 0}}}"#);
    let error = validate_settings_weights(Some(&settings), None, None, None)
        .await
        .unwrap_err();
    assert_invalid_settings(&error);
}

#[tokio::test]
async fn settings_validation_requires_database_for_references() {
    let settings = json(r#"{"weights": {"gpt-4": {"dep-1": 1}}}"#);
    let error = validate_settings_weights(Some(&settings), None, None, None)
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), Some(StatusCode::SERVICE_UNAVAILABLE));
    assert_eq!(error.to_string(), DATABASE_UNAVAILABLE);
}

#[test]
fn weights_accept_integer_and_zero() {
    let decoded = decode_router_weights(&json(r#"{"weights": {"gpt-4": {"a": 1, "b": 0}}}"#))
        .unwrap()
        .unwrap();
    assert_eq!(decoded["gpt-4"]["a"].to_bits(), 1.0_f64.to_bits());
    assert_eq!(decoded["gpt-4"]["b"].to_bits(), 0.0_f64.to_bits());
}

#[test]
fn weights_reject_negative() {
    let error = decode_router_weights(&json(r#"{"weights": {"gpt-4": {"a": -1}}}"#)).unwrap_err();
    assert!(matches!(error, RouterWeightsError::InvalidWeight));
}

#[test]
fn weights_reject_text() {
    let error = decode_router_weights(&json(r#"{"weights": {"gpt-4": {"a": "1"}}}"#)).unwrap_err();
    assert!(matches!(error, RouterWeightsError::InvalidWeight));
}

#[test]
fn weights_reject_boolean() {
    let error = decode_router_weights(&json(r#"{"weights": {"gpt-4": {"a": true}}}"#)).unwrap_err();
    assert!(matches!(error, RouterWeightsError::InvalidWeight));
}

#[test]
fn weights_reject_huge_number() {
    let error =
        decode_router_weights(&json(r#"{"weights": {"gpt-4": {"a": 1e400}}}"#)).unwrap_err();
    assert!(matches!(error, RouterWeightsError::InvalidWeight));
}

#[test]
fn weights_reject_empty_group_name() {
    let error = decode_router_weights(&json(r#"{"weights": {"": {"a": 1}}}"#)).unwrap_err();
    assert!(matches!(error, RouterWeightsError::InvalidIdentifier));
}

#[test]
fn weights_reject_whitespace_deployment_id() {
    let error = decode_router_weights(&json(r#"{"weights": {"gpt-4": {" ": 1}}}"#)).unwrap_err();
    assert!(matches!(error, RouterWeightsError::InvalidIdentifier));
}

#[test]
fn weights_require_positive_in_nonempty_group() {
    let error = decode_router_weights(&json(r#"{"weights": {"gpt-4": {"a": 0}}}"#)).unwrap_err();
    assert!(matches!(error, RouterWeightsError::NoPositiveWeight));
}

#[test]
fn weights_negative_zero_does_not_count_as_positive() {
    let error = decode_router_weights(&json(r#"{"weights": {"gpt-4": {"a": -0.0}}}"#)).unwrap_err();
    assert!(matches!(error, RouterWeightsError::NoPositiveWeight));
}

#[test]
fn weights_keep_group_and_deployment_order() {
    let decoded = decode_router_weights(&json(
        r#"{"weights": {"z-group": {"b": 1, "a": 2}, "a-group": {"c": 1}}}"#,
    ))
    .unwrap()
    .unwrap();
    let groups: Vec<&str> = decoded.keys().map(String::as_str).collect();
    assert_eq!(groups, ["z-group", "a-group"]);
    let deployments: Vec<&str> = decoded["z-group"].keys().map(String::as_str).collect();
    assert_eq!(deployments, ["b", "a"]);
}

#[test]
fn weights_absent_and_null_are_none() {
    assert!(
        decode_router_weights(&json(r#"{"num_retries": 2}"#))
            .unwrap()
            .is_none()
    );
    assert!(
        decode_router_weights(&json(r#"{"weights": null}"#))
            .unwrap()
            .is_none()
    );
}

#[test]
fn weights_reject_non_object_settings() {
    let error = decode_router_weights(&json(r#""weights""#)).unwrap_err();
    assert!(matches!(error, RouterWeightsError::SettingsNotAnObject));
}

#[test]
fn weights_reject_non_object_weights() {
    let error = decode_router_weights(&json(r#"{"weights": "gpt-4"}"#)).unwrap_err();
    assert!(matches!(error, RouterWeightsError::WeightsNotAnObject));
}

#[test]
fn model_info_accepts_object() {
    let decoded = RouterWeightModelInfo::decode(&json(
        r#"{"team_id": "team-a", "db_model": true, "team_public_model_name": "public"}"#,
    ))
    .unwrap();
    assert_eq!(
        decoded,
        RouterWeightModelInfo {
            team_id: Some("team-a".to_owned()),
            db_model: Some(true),
            team_public_model_name: Some("public".to_owned()),
        }
    );
}

#[test]
fn model_info_accepts_json_string() {
    let decoded = RouterWeightModelInfo::decode(&json(r#""{\"team_id\": \"team-a\"}""#)).unwrap();
    assert_eq!(
        decoded,
        RouterWeightModelInfo {
            team_id: Some("team-a".to_owned()),
            db_model: None,
            team_public_model_name: None,
        }
    );
}

#[test]
fn model_info_falsey_values_default() {
    assert_falsey_model_info_defaults(&["null", "false", "0", "0.0", "[]", "{}"]);
}

#[test]
fn model_info_truthy_scalars_are_not_objects() {
    assert_model_info_not_an_object(&["true", "1", "0.5", "[1]"]);
}

#[test]
fn model_info_json_string_scalars_are_not_objects() {
    assert_model_info_not_an_object(&[r#""x""#, r#""[]""#, r#""1""#]);
}

#[test]
fn model_info_unparsable_json_string_fails() {
    assert_model_info_invalid_json(&[r#""{oops""#, r#""""#]);
}

#[test]
fn model_info_fields_default_to_none() {
    let decoded =
        RouterWeightModelInfo::decode(&json(r#"{"team_id": null, "version": 2}"#)).unwrap();
    assert_eq!(decoded, RouterWeightModelInfo::default());
}

#[test]
fn model_info_rejects_wrong_field_types() {
    assert_model_info_field_type(&[
        r#"{"team_id": 1}"#,
        r#"{"team_id": ["team-a"]}"#,
        r#"{"db_model": "yes"}"#,
        r#"{"db_model": 1}"#,
        r#"{"team_public_model_name": 5}"#,
    ]);
}

#[tokio::test]
async fn postgres_reference_lookup_one_deduplicated_query() {
    let case = support::Case::new(SCANNED_SCHEMA).await;
    case.insert_scanned_row("dep-1", "gpt-4", None).await;
    case.insert_scanned_row("dep-2", "gpt-4", None).await;
    case.insert_scanned_row("dep-3", "gpt-4o", None).await;
    let repository = case.repository();
    let settings = json(
        r#"{"weights": {"gpt-4": {"dep-1": 1, "dep-2": 1}, "gpt-4o": {"dep-3": 1, "dep-1": 1}}}"#,
    );
    let error = validate_settings_weights(Some(&settings), None, Some(&repository), None)
        .await
        .unwrap_err();
    assert_wrong_group(&error, "dep-1", "gpt-4o");
    let lookups = case
        .count(r#"SELECT count(*) FROM "WeightLookupLog""#)
        .await;
    assert_eq!(lookups, 1);
    case.close().await;
}

#[tokio::test]
async fn postgres_reference_lookup_db_required_for_configured_id() {
    let mut catalog = DeploymentCatalog::default();
    catalog.push(configured("gpt-4", r#"{"id": "dep-1"}"#));
    let settings = json(r#"{"weights": {"gpt-4": {"dep-1": 1}}}"#);
    let error = validate_settings_weights(Some(&settings), None, None, Some(&catalog))
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), Some(StatusCode::SERVICE_UNAVAILABLE));
    assert_eq!(error.to_string(), DATABASE_UNAVAILABLE);
}

#[tokio::test]
async fn postgres_reference_lookup_stored_absent_uses_configured() {
    let case = support::Case::new(TABLE_SCHEMA).await;
    case.insert_row("dep-2", "gpt-4o", None).await;
    let repository = case.repository();
    let mut catalog = DeploymentCatalog::default();
    catalog.push(configured("gpt-4", r#"{"id": "dep-1"}"#));
    let settings = json(r#"{"weights": {"gpt-4": {"dep-1": 1}}}"#);
    validate_settings_weights(Some(&settings), None, Some(&repository), Some(&catalog))
        .await
        .unwrap();
    case.close().await;
}

#[tokio::test]
async fn postgres_reference_lookup_stored_model_info_json() {
    let case = support::Case::new(TABLE_SCHEMA).await;
    case.insert_row(
        "dep-1",
        "internal",
        Some(r#""{\"team_id\":\"team-a\",\"team_public_model_name\":\"public\"}""#),
    )
    .await;
    let repository = case.repository();
    let mut catalog = DeploymentCatalog::default();
    catalog.push(configured(
        "internal",
        r#"{"id": "dep-1", "team_id": "team-b", "db_model": true}"#,
    ));
    let settings = json(r#"{"weights": {"public": {"dep-1": 1}}}"#);
    validate_settings_weights(
        Some(&settings),
        Some("team-a"),
        Some(&repository),
        Some(&catalog),
    )
    .await
    .unwrap();
    case.close().await;
}

#[tokio::test]
async fn postgres_reference_lookup_absent_model_info() {
    let case = support::Case::new(TABLE_SCHEMA).await;
    case.insert_row("dep-1", "gpt-4", None).await;
    let repository = case.repository();
    let mut catalog = DeploymentCatalog::default();
    catalog.push(configured(
        "gpt-4",
        r#"{"id": "dep-1", "team_id": "team-a", "db_model": true}"#,
    ));
    let settings = json(r#"{"weights": {"gpt-4": {"dep-1": 1}}}"#);
    validate_settings_weights(
        Some(&settings),
        Some("team-a"),
        Some(&repository),
        Some(&catalog),
    )
    .await
    .unwrap();
    case.close().await;
}

#[tokio::test]
async fn postgres_reference_lookup_bad_stored_json() {
    let case = support::Case::new(TABLE_SCHEMA).await;
    case.insert_row("dep-1", "gpt-4", Some(r#""{oops""#)).await;
    let repository = case.repository();
    let settings = json(r#"{"weights": {"gpt-4": {"dep-1": 1}}}"#);
    let error = validate_settings_weights(Some(&settings), None, Some(&repository), None)
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), None);
    assert!(matches!(&error, RouterWeightError::Stored(_)));
    assert_eq!(error.to_string(), "stored row is invalid JSON");
    assert_ne!(error.to_string(), INVALID_SETTINGS);
    case.close().await;
}

#[tokio::test]
async fn postgres_reference_lookup_scalar_model_info_fails() {
    let case = support::Case::new(TABLE_SCHEMA).await;
    case.insert_row("dep-1", "gpt-4", Some("false")).await;
    let repository = case.repository();
    let settings = json(r#"{"weights": {"gpt-4": {"dep-1": 1}}}"#);
    let error = validate_settings_weights(Some(&settings), None, Some(&repository), None)
        .await
        .unwrap_err();
    assert!(matches!(&error, RouterWeightError::Stored(_)));
    assert_eq!(error.to_string(), "stored row is invalid JSON");
    case.close().await;
}

#[tokio::test]
async fn postgres_reference_lookup_database_failure_no_fallback() {
    let missing = support::MissingDatabase::new();
    let repository = missing.repository();
    let mut catalog = DeploymentCatalog::default();
    catalog.push(configured("gpt-4", r#"{"id": "dep-1"}"#));
    let settings = json(r#"{"weights": {"gpt-4": {"dep-1": 1}}}"#);
    let error = validate_settings_weights(Some(&settings), None, Some(&repository), Some(&catalog))
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), Some(StatusCode::SERVICE_UNAVAILABLE));
    assert_eq!(error.to_string(), DATABASE_UNAVAILABLE);
}

#[tokio::test]
async fn postgres_reference_lookup_no_write_on_failure() {
    let case = support::Case::new(TABLE_SCHEMA).await;
    case.insert_row("dep-1", "gpt-4", Some(r#"{"team_id": "team-a"}"#))
        .await;
    let before = case
        .texts(
            r#"SELECT row_to_json(t)::text FROM "LiteLLM_ProxyModelTable" AS t ORDER BY model_id"#,
        )
        .await;
    let repository = case.repository();
    let settings = json(r#"{"weights": {"gpt-4o": {"dep-1": 1}}}"#);
    let error = validate_settings_weights(Some(&settings), Some("team-a"), Some(&repository), None)
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), Some(StatusCode::BAD_REQUEST));
    let after = case
        .texts(
            r#"SELECT row_to_json(t)::text FROM "LiteLLM_ProxyModelTable" AS t ORDER BY model_id"#,
        )
        .await;
    assert_eq!(before, after);
    let rows = case
        .count(r#"SELECT count(*) FROM "LiteLLM_ProxyModelTable""#)
        .await;
    assert_eq!(rows, 1);
    case.close().await;
}
