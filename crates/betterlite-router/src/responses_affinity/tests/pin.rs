use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_models::decisions::Json;

use super::*;

fn pool() -> Vec<DeploymentSpec> {
    vec![
        spec("A", params(Some("u-a"), Some("k-a"))),
        spec("B", params(Some("u-b"), Some("k-b"))),
    ]
}

#[test]
fn a_marker_pins_the_healthy_deployment_with_that_id() {
    let deployments = pool();
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let mut request = Request::with_items(vec![reasoning_item("A")]);
    let (returned, logs) = captured(|| request.filter(&check, "gpt-group", &deployments));
    let returned = returned.unwrap();
    assert_eq!(ids(&returned), vec![Some("A".to_owned())]);
    assert!(request.pinned);
    let decoded = "EncryptedContentAffinityCheck: decoded model_id=A from the request's encrypted content markers";
    assert_eq!(
        logs,
        vec![
            decoded.to_owned(),
            "EncryptedContentAffinityCheck: pinning -> deployment=A".to_owned()
        ]
    );
}

#[test]
fn only_the_first_matching_healthy_row_is_returned() {
    let first = spec("A", params(Some("first-base"), Some("first-key")));
    let second = spec("A", params(Some("second-base"), Some("second-key")));
    let deployments = vec![spec("B", params(Some("u-b"), Some("k-b"))), first, second];
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let mut request = Request::with_items(vec![reasoning_item("A")]);
    let returned = request.filter(&check, "gpt-group", &deployments).unwrap();
    assert_eq!(ids(&returned), vec![Some("A".to_owned())]);
    let expected = Some(text("first-base"));
    assert_eq!(
        returned[0].litellm_params.get("api_base"),
        expected.as_ref()
    );
}

#[test]
fn a_numeric_healthy_id_matches_its_string_marker() {
    let numeric = spec_with_model_info(object(&[(
        "id",
        Json::Number(serde_json::Number::from(7_i32)),
    )]));
    let deployments = vec![numeric];
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let mut request = Request::with_items(vec![reasoning_item("7")]);
    let returned = request.filter(&check, "gpt-group", &deployments).unwrap();
    assert_eq!(ids(&returned), vec![Some("7".to_owned())]);
    assert!(request.pinned);
}

#[test]
fn a_row_with_a_malformed_or_missing_id_is_skipped() {
    let null_id = spec_with_model_info(object(&[("id", Json::Null)]));
    let no_info = spec_with_model_info(JsonObject::new());
    let target = spec("A", params(Some("u-a"), Some("k-a")));
    let deployments = vec![null_id, no_info, target];
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let mut request = Request::with_items(vec![reasoning_item("A")]);
    let returned = request.filter(&check, "gpt-group", &deployments).unwrap();
    assert_eq!(ids(&returned), vec![Some("A".to_owned())]);
}

#[test]
fn no_marker_returns_the_original_pool_without_the_pin_flag() {
    let deployments = pool();
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let mut request = Request::with_items(vec![message_item()]);
    let (returned, logs) = captured(|| request.filter(&check, "gpt-group", &deployments));
    assert_eq!(ids(&returned.unwrap()), ids(&deployments));
    assert!(!request.pinned);
    assert!(logs.is_empty());
}

#[test]
fn an_existing_pin_flag_is_kept_when_no_marker_is_found() {
    let deployments = pool();
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let mut request = Request::with_items(vec![message_item()]);
    request.pinned = true;
    let _returned = request.filter(&check, "gpt-group", &deployments).unwrap();
    assert!(request.pinned);
}

#[test]
fn pinning_does_not_rewrite_unmarked_input() {
    let deployments = pool();
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let mut request = Request::with_items(vec![reasoning_item("A"), message_item()]);
    let _returned = request.filter(&check, "gpt-group", &deployments).unwrap();
    assert_eq!(
        request.input,
        Some(Json::List(vec![reasoning_item("A"), message_item()]))
    );
}
