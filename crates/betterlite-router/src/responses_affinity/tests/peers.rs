use std::slice::from_ref;

use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_models::decisions::Json;

use super::*;
use crate::responses_affinity::config::AFFINITY_CHECK;

fn warning_for(origin: &str) -> String {
    format!(
        "EncryptedContentAffinityCheck: model_id={origin} cannot serve group gpt-group and no deployment on the same encryption boundary is configured; forwarding without its encrypted reasoning"
    )
}

#[test]
fn same_boundary_peers_are_returned_in_pool_order() {
    let origin = spec("A", params(Some("shared-base"), Some("shared-key")));
    let first_peer = spec("B", params(Some("shared-base"), Some("shared-key")));
    let second_peer = spec("E", params(Some("shared-base"), Some("shared-key")));
    let other_base = spec("C", params(Some("other-base"), Some("shared-key")));
    let other_key = spec("D", params(Some("shared-base"), Some("other-key")));
    let fixture = Fixture::new(vec![
        origin,
        first_peer.clone(),
        second_peer.clone(),
        other_base.clone(),
        other_key.clone(),
    ]);
    let healthy = vec![second_peer, other_base, first_peer, other_key];
    let lookup = fixture.lookup();
    let credentials = CredentialSnapshot::default();
    let check = catalog_check(settings(true, &[]), &lookup, &credentials);
    let mut request = Request::with_items(vec![reasoning_item("A")]);
    let (returned, logs) = captured(|| request.filter(&check, "gpt-group", &healthy));
    let returned = returned.unwrap();
    assert_eq!(
        ids(&returned),
        vec![Some("E".to_owned()), Some("B".to_owned())]
    );
    assert!(request.pinned);
    let decoded = "EncryptedContentAffinityCheck: decoded model_id=A from the request's encrypted content markers";
    let peers = "EncryptedContentAffinityCheck: model_id=A not in healthy_deployments; pinning to 2 deployment(s) on same encryption boundary";
    assert_eq!(logs, vec![decoded.to_owned(), peers.to_owned()]);
}

#[test]
fn a_named_credential_supplies_the_origin_boundary() {
    let origin = spec("A", params_with_credential("team-a"));
    let peer = spec("B", params(Some("shared-base"), Some("shared-key")));
    let fixture = Fixture::new(vec![origin, peer.clone()]);
    let credentials = credentials(&[(
        "team-a",
        object(&[
            ("api_base", text("shared-base")),
            ("api_key", text("shared-key")),
        ]),
    )]);
    let lookup = fixture.lookup();
    let check = catalog_check(settings(true, &[]), &lookup, &credentials);
    let mut request = Request::with_items(vec![reasoning_item("A")]);
    let returned = request
        .filter(&check, "gpt-group", from_ref(&peer))
        .unwrap();
    assert_eq!(ids(&returned), vec![Some("B".to_owned())]);
    assert!(request.pinned);
}

#[test]
fn an_absent_named_credential_keeps_the_deployment_boundary() {
    let origin = spec("A", params_with_credential("team-a"));
    let peer = spec("B", params(Some("deployment-base"), Some("deployment-key")));
    let fixture = Fixture::new(vec![origin, peer.clone()]);
    let credentials = CredentialSnapshot::default();
    let lookup = fixture.lookup();
    let check = catalog_check(settings(true, &[]), &lookup, &credentials);
    let mut request = Request::with_items(vec![reasoning_item("A")]);
    let returned = request
        .filter(&check, "gpt-group", from_ref(&peer))
        .unwrap();
    assert_eq!(ids(&returned), vec![Some("B".to_owned())]);
}

#[test]
fn a_null_credential_field_falls_back_to_the_pool() {
    let origin = spec("A", params_with_credential("team-a"));
    let peer = spec("B", params(Some("deployment-base"), Some("deployment-key")));
    let fixture = Fixture::new(vec![origin, peer.clone()]);
    let credentials = credentials(&[("team-a", object(&[("api_base", Json::Null)]))]);
    let lookup = fixture.lookup();
    let check = catalog_check(settings(true, &[]), &lookup, &credentials);
    let mut request = Request::with_items(vec![reasoning_item("A")]);
    let (returned, logs) = captured(|| request.filter(&check, "gpt-group", from_ref(&peer)));
    assert_eq!(ids(&returned.unwrap()), vec![Some("B".to_owned())]);
    assert!(!request.pinned);
    assert_eq!(logs.last(), Some(&warning_for("A")));
}

#[test]
fn a_missing_origin_has_no_peers() {
    let peer = spec("B", params(Some("shared-base"), Some("shared-key")));
    let fixture = Fixture::new(vec![peer.clone()]);
    let lookup = fixture.lookup();
    let credentials = CredentialSnapshot::default();
    let check = catalog_check(settings(true, &[]), &lookup, &credentials);
    let mut request = Request::with_items(vec![reasoning_item("A")]);
    let (returned, logs) = captured(|| request.filter(&check, "gpt-group", from_ref(&peer)));
    assert_eq!(ids(&returned.unwrap()), vec![Some("B".to_owned())]);
    assert!(!request.pinned);
    assert_eq!(logs.last(), Some(&warning_for("A")));
}

#[test]
fn without_a_router_there_are_no_peers() {
    let peer = spec("B", params(Some("shared-base"), Some("shared-key")));
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let mut request = Request::with_items(vec![reasoning_item("A")]);
    let (returned, logs) = captured(|| request.filter(&check, "gpt-group", from_ref(&peer)));
    assert_eq!(ids(&returned.unwrap()), vec![Some("B".to_owned())]);
    assert_eq!(logs.last(), Some(&warning_for("A")));
}

#[test]
fn a_group_listed_for_peers_still_requires_activation() {
    let origin = spec("A", params(Some("shared-base"), Some("shared-key")));
    let peer = spec("B", params(Some("shared-base"), Some("shared-key")));
    let fixture = Fixture::new(vec![origin, peer.clone()]);
    let lookup = fixture.lookup();
    let credentials = CredentialSnapshot::default();
    let check = catalog_check(
        settings(false, &[("other-group", &[AFFINITY_CHECK])]),
        &lookup,
        &credentials,
    );
    let mut request = Request::with_items(vec![reasoning_item("A")]);
    let (returned, logs) = captured(|| request.filter(&check, "gpt-group", from_ref(&peer)));
    assert_eq!(ids(&returned.unwrap()), vec![Some("B".to_owned())]);
    assert!(!request.pinned);
    assert!(logs.is_empty());
}
