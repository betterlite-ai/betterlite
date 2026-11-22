use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_models::decisions::Json;

use super::*;
use crate::responses_affinity::boundary::encryption_boundary;

fn pair(base: &str, key: &str) -> Option<(String, String)> {
    Some((base.to_owned(), key.to_owned()))
}

#[test]
fn the_deployment_fields_form_the_boundary() {
    let credentials = CredentialSnapshot::default();
    assert_eq!(
        encryption_boundary(&params(Some("u"), Some("k")), &credentials),
        pair("u", "k")
    );
}

#[test]
fn named_credential_fields_override_by_presence() {
    let credentials = credentials(&[(
        "team-a",
        object(&[
            ("api_base", text("cred-base")),
            ("api_key", text("cred-key")),
        ]),
    )]);
    assert_eq!(
        encryption_boundary(&params_with_credential("team-a"), &credentials),
        pair("cred-base", "cred-key")
    );
}

#[test]
fn a_null_credential_field_disables_the_boundary() {
    let credentials = credentials(&[("team-a", object(&[("api_base", Json::Null)]))]);
    assert_eq!(
        encryption_boundary(&params_with_credential("team-a"), &credentials),
        None
    );
}

#[test]
fn a_missing_credential_field_falls_back_to_the_deployment() {
    let credentials = credentials(&[("team-a", object(&[("api_base", text("cred-base"))]))]);
    assert_eq!(
        encryption_boundary(&params_with_credential("team-a"), &credentials),
        pair("cred-base", "deployment-key")
    );
}

#[test]
fn an_unknown_credential_name_keeps_both_deployment_values() {
    let credentials = CredentialSnapshot::default();
    assert_eq!(
        encryption_boundary(&params_with_credential("absent"), &credentials),
        pair("deployment-base", "deployment-key")
    );
}

#[test]
fn falsy_values_have_no_boundary() {
    let credentials = CredentialSnapshot::default();
    assert_eq!(
        encryption_boundary(&params(Some(""), Some("k")), &credentials),
        None
    );
    assert_eq!(
        encryption_boundary(&params(Some("u"), Some("")), &credentials),
        None
    );
    assert_eq!(encryption_boundary(&params(None, None), &credentials), None);
    let empty_credential = credentials(&[("team-a", object(&[("api_base", text(""))]))]);
    assert_eq!(
        encryption_boundary(&params_with_credential("team-a"), &empty_credential),
        None
    );
}

#[test]
fn an_empty_credential_name_keeps_the_deployment_values() {
    let credentials = credentials(&[(
        "",
        object(&[
            ("api_base", text("cred-base")),
            ("api_key", text("cred-key")),
        ]),
    )]);
    assert_eq!(
        encryption_boundary(&params_with_credential(""), &credentials),
        pair("deployment-base", "deployment-key")
    );
}

#[test]
fn a_non_text_credential_name_keeps_the_deployment_values() {
    let credentials = credentials(&[(
        "5",
        object(&[
            ("api_base", text("cred-base")),
            ("api_key", text("cred-key")),
        ]),
    )]);
    let mut params = params_with_credential("5");
    params.insert(
        "litellm_credential_name".to_owned(),
        Json::Number(serde_json::Number::from(5_i32)),
    );
    assert_eq!(
        encryption_boundary(&params, &credentials),
        pair("deployment-base", "deployment-key")
    );
}

#[test]
fn non_text_values_have_no_boundary() {
    let credentials = CredentialSnapshot::default();
    let mut params = params(Some("u"), Some("k"));
    params.insert(
        "api_base".to_owned(),
        Json::Number(serde_json::Number::from(1_i32)),
    );
    assert_eq!(encryption_boundary(&params, &credentials), None);
}
