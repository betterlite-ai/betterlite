use std::collections::BTreeMap;

use crate::jwt_claims::TypedValue;
use crate::sso_claims::decode::{decode_sso_token_claims, merge_sso_token_claims};

use super::support::{claims, token};

fn merged(
    userinfo: &str,
    id_payload: Option<&str>,
    access_payload: Option<&str>,
) -> BTreeMap<String, TypedValue> {
    let id_token = id_payload.map(token);
    let access_token = access_payload.map(token);
    merge_sso_token_claims(
        &claims(userinfo),
        id_token.as_deref(),
        access_token.as_deref(),
    )
}

#[test]
fn a_missing_token_has_no_claims() {
    assert!(decode_sso_token_claims(None).is_empty());
    assert!(decode_sso_token_claims(Some("")).is_empty());
}

#[test]
fn an_opaque_or_malformed_token_has_no_claims() {
    assert!(decode_sso_token_claims(Some("opaque-token")).is_empty());
    assert!(decode_sso_token_claims(Some("one.two")).is_empty());
    assert!(decode_sso_token_claims(Some("a.b.c")).is_empty());
    assert!(decode_sso_token_claims(Some(&token("[1,2]"))).is_empty());
}

#[test]
fn every_claim_of_a_token_comes_back() {
    let id_token = token(
        r#"{"email":"Id@Example.com","groups":["id-group"],"tenant":{"id":"t-1"},"absent":null}"#,
    );
    assert_eq!(
        decode_sso_token_claims(Some(&id_token)),
        claims(
            r#"{"email":"Id@Example.com","groups":["id-group"],"tenant":{"id":"t-1"},"absent":null}"#
        )
    );
}

#[test]
fn userinfo_wins_over_the_id_token_and_the_access_token() {
    let result = merged(
        r#"{"email":"user@example.com"}"#,
        Some(r#"{"email":"id@example.com","name":"id-name"}"#),
        Some(r#"{"email":"access@example.com","name":"access-name","role":"proxy_admin"}"#),
    );
    assert_eq!(
        result,
        claims(r#"{"email":"user@example.com","name":"id-name","role":"proxy_admin"}"#)
    );
}

#[test]
fn a_null_userinfo_value_falls_through_to_the_id_token() {
    let result = merged(
        r#"{"email":"user@example.com","name":null,"role":null}"#,
        Some(r#"{"name":"id-name","role":null}"#),
        Some(r#"{"role":"proxy_admin"}"#),
    );
    assert_eq!(
        result,
        claims(r#"{"email":"user@example.com","name":"id-name","role":"proxy_admin"}"#)
    );
}

#[test]
fn a_key_that_is_null_in_every_source_stays_null() {
    let result = merged(
        r#"{"x":null}"#,
        Some(r#"{"x":null}"#),
        Some(r#"{"x":null}"#),
    );
    assert_eq!(result, claims(r#"{"x":null}"#));
}

#[test]
fn a_falsy_userinfo_value_still_wins() {
    let result = merged(
        r#"{"enabled":false,"count":0,"label":""}"#,
        Some(r#"{"enabled":true,"count":5,"label":"id"}"#),
        None,
    );
    assert_eq!(result, claims(r#"{"enabled":false,"count":0,"label":""}"#));
}

#[test]
fn an_opaque_id_token_falls_through_to_the_access_token() {
    let access_token = token(r#"{"name":"access-name"}"#);
    let result = merge_sso_token_claims(
        &claims(r#"{"name":null}"#),
        Some("opaque-token"),
        Some(&access_token),
    );
    assert_eq!(result, claims(r#"{"name":"access-name"}"#));
}

#[test]
fn the_access_token_adds_keys_the_earlier_sources_lack() {
    let result = merged(
        r#"{}"#,
        None,
        Some(r#"{"team_ids":["at-1"],"role":"proxy_admin"}"#),
    );
    assert_eq!(
        result,
        claims(r#"{"team_ids":["at-1"],"role":"proxy_admin"}"#)
    );
}
