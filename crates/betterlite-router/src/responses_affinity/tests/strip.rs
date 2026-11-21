use std::slice::from_ref;

use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_models::decisions::Json;

use super::*;

fn target() -> DeploymentSpec {
    spec("A", params(Some("u-a"), Some("k-a")))
}

#[test]
fn reasoning_from_other_origins_is_stripped_and_the_targets_kept() {
    let deployments = vec![target()];
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let mut request = Request::with_items(vec![
        reasoning_item("A"),
        reasoning_item("B"),
        message_item(),
        titled_item("B"),
    ]);
    let returned = request.filter(&check, "gpt-group", &deployments).unwrap();
    assert_eq!(ids(&returned), vec![Some("A".to_owned())]);
    let expected = Some(Json::List(vec![
        reasoning_item("A"),
        message_item(),
        kept_titled_item(),
    ]));
    assert_eq!(request.input, expected);
}

#[test]
fn a_peer_on_the_targets_boundary_keeps_its_reasoning() {
    let target = target();
    let same_boundary = spec("C", params(Some("u-a"), Some("k-a")));
    let fixture = Fixture::new(vec![target.clone(), same_boundary]);
    let lookup = fixture.lookup();
    let credentials = CredentialSnapshot::default();
    let check = catalog_check(settings(true, &[]), &lookup, &credentials);
    let mut request = Request::with_items(vec![reasoning_item("A"), reasoning_item("C")]);
    let returned = request
        .filter(&check, "gpt-group", from_ref(&target))
        .unwrap();
    assert_eq!(ids(&returned), vec![Some("A".to_owned())]);
    let expected = Some(Json::List(vec![reasoning_item("A"), reasoning_item("C")]));
    assert_eq!(request.input, expected);
}

#[test]
fn a_different_origin_boundary_is_stripped() {
    let target = target();
    let different_key = spec("C", params(Some("u-a"), Some("other-key")));
    let fixture = Fixture::new(vec![target.clone(), different_key]);
    let lookup = fixture.lookup();
    let credentials = CredentialSnapshot::default();
    let check = catalog_check(settings(true, &[]), &lookup, &credentials);
    let mut request = Request::with_items(vec![reasoning_item("A"), reasoning_item("C")]);
    let _returned = request
        .filter(&check, "gpt-group", from_ref(&target))
        .unwrap();
    assert_eq!(request.input, Some(Json::List(vec![reasoning_item("A")])));
}

#[test]
fn anthropic_blocks_are_stripped_in_place() {
    let deployments = vec![target()];
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let mut request = Request::with_items(vec![reasoning_item("A")]);
    request.messages = Some(vec![message(vec![
        thinking_part(Some("A")),
        thinking_part(Some("B")),
        thinking_part(None),
        text_part("keep"),
    ])]);
    let _returned = request.filter(&check, "gpt-group", &deployments).unwrap();
    let expected = Some(vec![message(vec![
        thinking_part(Some("A")),
        thinking_part(None),
        text_part("keep"),
    ])]);
    assert_eq!(request.messages, expected);
}

#[test]
fn request_kwargs_messages_are_stripped_when_the_argument_is_empty() {
    let deployments = vec![target()];
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let mut request = Request::new();
    request.messages = Some(Vec::new());
    request.request_messages = Some(vec![message(vec![
        thinking_part(Some("A")),
        thinking_part(Some("B")),
        text_part("keep"),
    ])]);
    let _returned = request.filter(&check, "gpt-group", &deployments).unwrap();
    assert_eq!(request.messages, Some(Vec::new()));
    let expected = Some(vec![message(vec![
        thinking_part(Some("A")),
        text_part("keep"),
    ])]);
    assert_eq!(request.request_messages, expected);
}

#[test]
fn a_same_id_origin_keeps_its_reasoning_without_a_router() {
    let deployments = vec![target()];
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let mut request = Request::with_items(vec![titled_item("A"), reasoning_item("A")]);
    let _returned = request.filter(&check, "gpt-group", &deployments).unwrap();
    let expected = Some(Json::List(vec![titled_item("A"), reasoning_item("A")]));
    assert_eq!(request.input, expected);
}
