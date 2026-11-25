#![cfg(test)]

use axum::http::StatusCode;

use crate::lens_access::error::failure_text;
use crate::lens_access::model::validate_model;
use crate::lens_access::tests::support::{
    Harness, auth_key, execution_id, settings, settings_with_ids,
};

const UNKNOWN: &str = "Choose a model configured on this LiteLLM instance";
const DENIED: &str = "This key does not have access to the analysis model";

#[test]
fn a_configured_model_passes() {
    let harness = Harness::new();
    let admin = auth_key(Some("proxy_admin"), None, &[]);
    validate_model(&settings("gpt-4o"), &admin, &harness.state()).expect("the model is configured");
}

#[test]
fn without_a_catalog_the_model_is_unknown() {
    let harness = Harness::new().without_catalog();
    let admin = auth_key(Some("proxy_admin"), None, &[]);
    let error =
        validate_model(&settings("gpt-4o"), &admin, &harness.state()).expect_err("no catalog");
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert_eq!(failure_text(&error), UNKNOWN);
}

#[test]
fn an_unconfigured_model_is_unknown() {
    let harness = Harness::new();
    let admin = auth_key(Some("proxy_admin"), None, &[]);
    let error = validate_model(&settings("not-deployed"), &admin, &harness.state())
        .expect_err("the model is unknown");
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert_eq!(failure_text(&error), UNKNOWN);
}

#[test]
fn a_non_admin_key_without_the_model_is_denied() {
    let harness = Harness::new();
    let key = auth_key(Some("team"), Some("team-1"), &["another-model"]);
    let error = validate_model(&settings("gpt-4o"), &key, &harness.state())
        .expect_err("the key lacks the model");
    assert_eq!(error.status(), StatusCode::FORBIDDEN);
    assert_eq!(failure_text(&error), DENIED);
}

#[test]
fn a_non_admin_key_naming_the_model_passes() {
    let harness = Harness::new();
    let key = auth_key(Some("team"), Some("team-1"), &["gpt-4o"]);
    validate_model(&settings("gpt-4o"), &key, &harness.state()).expect("the key names the model");
}

#[test]
fn all_proxy_models_covers_every_model() {
    let harness = Harness::new();
    let key = auth_key(Some("team"), None, &["all-proxy-models"]);
    validate_model(&settings("gpt-4o"), &key, &harness.state())
        .expect("the wildcard allows the model");
}

#[test]
fn a_key_without_a_model_list_passes_the_last_check() {
    let harness = Harness::new();
    let key = auth_key(Some("team"), None, &[]);
    validate_model(&settings("gpt-4o"), &key, &harness.state())
        .expect("an empty list skips the key check");
}

#[test]
fn the_selection_is_validated_before_the_model() {
    let harness = Harness::new().without_catalog();
    let admin = auth_key(Some("proxy_admin"), None, &[]);
    let broken = execution_id(&["logs", "team-1", "trace-1"]);
    let chosen = settings_with_ids(&[broken.as_str()]);
    let error =
        validate_model(&chosen, &admin, &harness.state()).expect_err("the selection fails first");
    assert_eq!(error.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[test]
fn the_callers_team_reaches_the_catalog() {
    let harness = Harness::new();
    let key = auth_key(Some("team"), Some("team-9"), &[]);
    validate_model(&settings("gpt-4o"), &key, &harness.state()).expect("the model is configured");
    let teams = harness
        .catalog
        .as_ref()
        .expect("the catalog exists")
        .teams
        .lock()
        .expect("the catalog team lock")
        .clone();
    assert_eq!(teams, [Some("team-9".to_owned())]);
}
