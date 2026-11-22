use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_models::decisions::Json;
use secrecy::ExposeSecret as _;

use super::*;
use crate::responses_affinity::boundary::{Boundary, encryption_boundary, same_boundary};

// The boundary keeps its key secret; only these assertions read the key text back out.
fn assert_boundary(found: Option<Boundary>, base: &str, key: &str) {
    let (actual_base, actual_key) = found.unwrap();
    assert_eq!(actual_base, base);
    assert_eq!(actual_key.expose_secret(), key);
}

fn assert_no_boundary(found: Option<Boundary>) {
    if let Some(boundary) = found {
        let base = boundary.0;
        panic!("a boundary was found at {base}");
    }
}

#[test]
fn the_deployment_fields_form_the_boundary() {
    let credentials = CredentialSnapshot::default();
    assert_boundary(
        encryption_boundary(&params(Some("u"), Some("k")), &credentials),
        "u",
        "k",
    );
}

#[test]
fn named_credential_fields_override_by_presence() {
    let credentials = named_credentials(&[(
        "team-a",
        object(&[
            ("api_base", text("cred-base")),
            ("api_key", text("cred-key")),
        ]),
    )]);
    assert_boundary(
        encryption_boundary(&params_with_credential("team-a"), &credentials),
        "cred-base",
        "cred-key",
    );
}

#[test]
fn a_null_credential_field_disables_the_boundary() {
    let credentials = named_credentials(&[("team-a", object(&[("api_base", Json::Null)]))]);
    assert_no_boundary(encryption_boundary(
        &params_with_credential("team-a"),
        &credentials,
    ));
}

#[test]
fn a_missing_credential_field_falls_back_to_the_deployment() {
    let credentials = named_credentials(&[("team-a", object(&[("api_base", text("cred-base"))]))]);
    assert_boundary(
        encryption_boundary(&params_with_credential("team-a"), &credentials),
        "cred-base",
        "deployment-key",
    );
}

#[test]
fn an_unknown_credential_name_keeps_both_deployment_values() {
    let credentials = CredentialSnapshot::default();
    assert_boundary(
        encryption_boundary(&params_with_credential("absent"), &credentials),
        "deployment-base",
        "deployment-key",
    );
}

#[test]
fn falsy_values_have_no_boundary() {
    let credentials = CredentialSnapshot::default();
    assert_no_boundary(encryption_boundary(
        &params(Some(""), Some("k")),
        &credentials,
    ));
    assert_no_boundary(encryption_boundary(
        &params(Some("u"), Some("")),
        &credentials,
    ));
    assert_no_boundary(encryption_boundary(&params(None, None), &credentials));
    let empty_credential = named_credentials(&[("team-a", object(&[("api_base", text(""))]))]);
    assert_no_boundary(encryption_boundary(
        &params_with_credential("team-a"),
        &empty_credential,
    ));
}

#[test]
fn an_empty_credential_name_keeps_the_deployment_values() {
    let credentials = named_credentials(&[(
        "",
        object(&[
            ("api_base", text("cred-base")),
            ("api_key", text("cred-key")),
        ]),
    )]);
    assert_boundary(
        encryption_boundary(&params_with_credential(""), &credentials),
        "deployment-base",
        "deployment-key",
    );
}

#[test]
fn a_non_text_credential_name_keeps_the_deployment_values() {
    let credentials = named_credentials(&[(
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
    assert_boundary(
        encryption_boundary(&params, &credentials),
        "deployment-base",
        "deployment-key",
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
    assert_no_boundary(encryption_boundary(&params, &credentials));
}

#[test]
fn boundaries_match_only_when_base_and_key_are_equal() {
    let credentials = CredentialSnapshot::default();
    let boundary = encryption_boundary(&params(Some("u"), Some("k")), &credentials).unwrap();
    assert!(same_boundary(&boundary, &boundary));
    let other_base = encryption_boundary(&params(Some("v"), Some("k")), &credentials).unwrap();
    assert!(!same_boundary(&boundary, &other_base));
    let other_key = encryption_boundary(&params(Some("u"), Some("z")), &credentials).unwrap();
    assert!(!same_boundary(&boundary, &other_key));
}
