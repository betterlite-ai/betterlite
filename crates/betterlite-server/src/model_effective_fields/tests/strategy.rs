use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use crate::model_effective_fields::error::ModelFieldError;
use crate::model_effective_fields::stored::StoredDeployment;
use crate::model_effective_fields::strategy::{
    RouterMarker, WriteGrant, check_strategy_router_write, member_router_marker,
};

use super::support::{cipher, object, sealed};

const COMPLEXITY: &str = "auto_router/complexity_router";

fn sealed_model(model: &str) -> String {
    format!(r#"{{"model": "{}"}}"#, sealed(model))
}

#[derive(Debug, PartialEq, Eq)]
enum Refusal {
    Config,
    Placement,
    Model,
}

fn refusal(incoming: Option<&JsonObject>, existing: Option<&JsonObject>) -> Option<Refusal> {
    let error = check_strategy_router_write(&cipher(), incoming, existing).err()?;
    assert_eq!(error.refusal().status, 400);
    assert_eq!(
        error.refusal().error_type,
        Some(ProxyErrorTypes::ValidationError)
    );
    assert_eq!(
        error.refusal().param.as_deref(),
        Some("litellm_params.model")
    );
    match error {
        ModelFieldError::ConfigWrite(_) => Some(Refusal::Config),
        ModelFieldError::Placement(_) => Some(Refusal::Placement),
        ModelFieldError::RouterModel(_) => Some(Refusal::Model),
        other => panic!("not a strategy refusal: {other:?}"),
    }
}

#[rstest]
#[case::config_on_a_plain_model(
    r#"{"complexity_router_config": {"classifier_type": "heuristic"}}"#,
    Some(r#"{"model": "gpt-4o"}"#),
    Some(Refusal::Model)
)]
#[case::unknown_classifier(
    r#"{"complexity_router_config": {"classifier_type": "nope"}}"#,
    None,
    Some(Refusal::Config)
)]
#[case::config_not_an_object(
    r#"{"model": "auto_router/complexity_router", "complexity_router_config": "heuristic"}"#,
    None,
    Some(Refusal::Config)
)]
#[case::setting_beside_the_config(
    r#"{"model": "auto_router/complexity_router", "complexity_router_default_model": "x", "tiers": {}}"#,
    None,
    Some(Refusal::Placement)
)]
#[case::flat_setting_on_a_plain_model(r#"{"model": "gpt-4o", "tiers": {}}"#, None, None)]
#[case::bare_prefix(r#"{"model": "auto_router/"}"#, None, Some(Refusal::Model))]
#[case::stored_router_setting_on_a_rename(
    r#"{"model": "gpt-4o"}"#,
    Some(r#"{"model": "auto_router/complexity_router", "complexity_router_default_model": "x"}"#),
    Some(Refusal::Model)
)]
#[case::router_without_settings(
    r#"{"model": "auto_router/complexity_router"}"#,
    None,
    Some(Refusal::Model)
)]
#[case::unrelated_patch(r#"{"tpm": 5}"#, Some(r#"{"model": "gpt-4o"}"#), None)]
#[case::null_config_is_not_judged(
    r#"{"complexity_router_config": null, "tpm": 1}"#,
    Some(r#"{"model": "auto_router/complexity_router", "complexity_router_config": {"classifier_type": "nope"}}"#),
    None
)]
#[case::null_setting_is_absent(
    r#"{"model": "gpt-4o", "complexity_router_default_model": null}"#,
    None,
    None
)]
fn strategy_router_write(
    #[case] incoming: &str,
    #[case] existing: Option<&str>,
    #[case] expected: Option<Refusal>,
) {
    let existing = existing.map(object);
    assert_eq!(
        refusal(Some(&object(incoming)), existing.as_ref()),
        expected
    );
}

#[test]
fn strategy_router_write_judges_the_decrypted_stored_model() {
    let incoming = object(
        r#"{"complexity_router_config": {"classifier_type": "heuristic"}, "complexity_router_default_model": "x"}"#,
    );
    let existing = object(&sealed_model(COMPLEXITY));
    assert_eq!(refusal(Some(&incoming), Some(&existing)), None);
}

#[test]
fn strategy_router_write_without_params_passes() {
    let existing = object(r#"{"model": "gpt-4o", "complexity_router_default_model": "x"}"#);
    assert_eq!(refusal(None, Some(&existing)), None);
}

// LiteLLM's MIT build caps routers with changed heuristic scoring at one per proxy; betterlite has no license tier
#[test]
fn no_tuning_license_cap() {
    let tuned = r#"{"model": "auto_router/complexity_router", "complexity_router_default_model": "x", "complexity_router_config": {"classifier_type": "heuristic", "code_keywords": ["kernel"]}}"#;
    let retuned = object(
        r#"{"complexity_router_config": {"classifier_type": "heuristic", "code_keywords": ["driver"]}}"#,
    );
    assert_eq!(refusal(Some(&retuned), Some(&object(tuned))), None);
    assert_eq!(refusal(Some(&object(tuned)), None), None);
}

fn deployment(params: &str, model_info: &str) -> StoredDeployment {
    StoredDeployment {
        model_id: "router-1".to_owned(),
        model_name: "team-router".to_owned(),
        litellm_params: object(params),
        model_info: object(model_info),
        blocked: false,
    }
}

#[rstest]
#[case::member_write_marks(
    WriteGrant::MemberAutoRouter,
    r#"{"member_auto_router": false}"#,
    Some("{}"),
    RouterMarker::Set
)]
#[case::unmarked_row(
    WriteGrant::Standard,
    r#"{"member_auto_router": false}"#,
    Some("{}"),
    RouterMarker::Unchanged
)]
#[case::unmarked_by_default(WriteGrant::Standard, "{}", None, RouterMarker::Unchanged)]
#[case::no_params(
    WriteGrant::Standard,
    r#"{"member_auto_router": true}"#,
    None,
    RouterMarker::Set
)]
#[case::strategy_setting(
    WriteGrant::Standard,
    r#"{"member_auto_router": true}"#,
    Some(r#"{"complexity_router_default_model": "x"}"#),
    RouterMarker::Cleared
)]
#[case::null_strategy_setting(
    WriteGrant::Standard,
    r#"{"member_auto_router": true}"#,
    Some(r#"{"complexity_router_config": null}"#),
    RouterMarker::Set
)]
#[case::same_model(
    WriteGrant::Standard,
    r#"{"member_auto_router": true}"#,
    Some(r#"{"model": "auto_router/complexity_router"}"#),
    RouterMarker::Set
)]
#[case::renamed_model(
    WriteGrant::Standard,
    r#"{"member_auto_router": true}"#,
    Some(r#"{"model": "auto_router/complexity_router_b"}"#),
    RouterMarker::Cleared
)]
#[case::unrelated_patch(
    WriteGrant::Standard,
    r#"{"member_auto_router": true}"#,
    Some(r#"{"tpm": 3}"#),
    RouterMarker::Set
)]
fn member_router_marker_follows_the_write(
    #[case] grant: WriteGrant,
    #[case] model_info: &str,
    #[case] incoming: Option<&str>,
    #[case] expected: RouterMarker,
) {
    let existing = deployment(&sealed_model(COMPLEXITY), model_info);
    let incoming = incoming.map(object);
    let marker = member_router_marker(&cipher(), incoming.as_ref(), &existing, grant);
    assert_eq!(marker, expected);
}
