use std::slice::from_ref;

use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_models::decisions::Json;

use super::*;

fn pool() -> Vec<DeploymentSpec> {
    vec![
        spec("B", params(Some("u-b"), Some("k-b"))),
        spec("C", params(Some("u-c"), Some("k-c"))),
    ]
}

fn warning_for(origin: &str) -> String {
    format!(
        "EncryptedContentAffinityCheck: model_id={origin} cannot serve group gpt-group and no deployment on the same encryption boundary is configured; forwarding without its encrypted reasoning"
    )
}

#[test]
fn an_unavailable_origin_returns_the_full_pool_with_a_warning() {
    let deployments = pool();
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let mut request =
        Request::with_items(vec![reasoning_item("B"), titled_item("C"), message_item()]);
    let (returned, logs) = captured(|| request.filter(&check, "gpt-group", &deployments));
    assert_eq!(ids(&returned.unwrap()), ids(&deployments));
    assert!(!request.pinned);
    let expected = Some(Json::List(vec![kept_titled_item(), message_item()]));
    assert_eq!(request.input, expected);
    assert_eq!(logs.last(), Some(&warning_for("A")));
}

#[test]
fn the_warning_shows_only_the_first_64_characters_of_the_origin() {
    let deployments = pool();
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let origin = "z".repeat(70);
    let mut request = Request::with_items(vec![reasoning_item(&origin)]);
    let (returned, logs) = captured(|| request.filter(&check, "gpt-group", &deployments));
    assert_eq!(ids(&returned.unwrap()), ids(&deployments));
    let warning = logs.last().unwrap();
    assert!(warning.contains(&format!("model_id={}", "z".repeat(64))));
    assert!(!warning.contains(&"z".repeat(65)));
}

#[test]
fn a_fallback_strips_anthropic_messages_too() {
    let deployments = pool();
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let mut request = Request::with_items(vec![reasoning_item("A")]);
    request.messages = Some(vec![message(vec![
        thinking_part(Some("A")),
        text_part("keep"),
    ])]);
    let _returned = request.filter(&check, "gpt-group", &deployments).unwrap();
    assert_eq!(
        request.messages,
        Some(vec![message(vec![text_part("keep")])])
    );
}

#[test]
fn an_existing_pin_flag_is_not_reset_by_a_late_fallback() {
    let deployments = pool();
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let mut request = Request::with_items(vec![reasoning_item("A")]);
    request.pinned = true;
    let _returned = request.filter(&check, "gpt-group", &deployments).unwrap();
    assert!(request.pinned);
}

#[test]
fn a_cross_group_origin_strips_and_dispatches() {
    let origin = spec("A", params(Some("group-base"), Some("group-key")));
    let healthy = spec("B", params(Some("pool-base"), Some("pool-key")));
    let fixture = Fixture::new(vec![origin, healthy.clone()]);
    let lookup = fixture.lookup();
    let credentials = CredentialSnapshot::default();
    let check = catalog_check(settings(true, &[]), &lookup, &credentials);
    let mut request = Request::with_items(vec![reasoning_item("A")]);
    let (returned, logs) = captured(|| request.filter(&check, "other-group", from_ref(&healthy)));
    assert_eq!(ids(&returned.unwrap()), vec![Some("B".to_owned())]);
    assert!(!request.pinned);
    assert!(
        logs.last()
            .unwrap()
            .contains("cannot serve group other-group")
    );
}

#[test]
fn a_forged_marker_is_stripped_like_any_other_unavailable_origin() {
    let deployments = pool();
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let mut request = Request::with_items(vec![titled_item("forged"), reasoning_item("forged")]);
    let (returned, _logs) = captured(|| request.filter(&check, "gpt-group", &deployments));
    assert_eq!(ids(&returned.unwrap()), ids(&deployments));
    assert_eq!(request.input, Some(Json::List(vec![kept_titled_item()])));
}
