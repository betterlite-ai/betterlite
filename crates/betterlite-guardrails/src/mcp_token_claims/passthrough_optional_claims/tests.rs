#![cfg(test)]

use betterlite_prompts::arguments::json::Json;

use crate::mcp_token_claims::Claims;
use crate::mcp_token_claims::passthrough_optional_claims::apply;

fn claims(json: &str) -> Claims {
    serde_json::from_str(json).unwrap()
}

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn optional_claims_absent_from_the_outbound_token_are_copied() {
    let incoming = claims(r#"{"groups":["eng"],"roles":["admin"],"sub":"alice"}"#);
    let mut outbound = claims(r#"{"sub":"alice"}"#);
    apply(&mut outbound, &names(&["groups", "roles"]), Some(&incoming));
    assert_eq!(
        outbound.get("groups"),
        Some(&Json::List(vec![Json::Text("eng".to_owned())]))
    );
    assert_eq!(
        outbound.get("roles"),
        Some(&Json::List(vec![Json::Text("admin".to_owned())]))
    );
}

#[test]
fn optional_claims_never_overwrite_an_outbound_claim() {
    let incoming = claims(r#"{"sub":"mallory","groups":["eng"]}"#);
    let mut outbound = claims(r#"{"sub":"alice"}"#);
    apply(&mut outbound, &names(&["sub", "groups"]), Some(&incoming));
    assert_eq!(outbound.get("sub"), Some(&Json::Text("alice".to_owned())));
}

#[test]
fn optional_claims_absent_from_the_incoming_token_are_skipped() {
    let incoming = claims(r#"{"groups":["eng"]}"#);
    let mut outbound = Claims::new();
    apply(&mut outbound, &names(&["groups", "roles"]), Some(&incoming));
    assert!(outbound.get("groups").is_some());
    assert_eq!(outbound.get("roles"), None);
}

#[test]
fn without_incoming_claims_nothing_is_copied() {
    let mut outbound = claims(r#"{"sub":"alice"}"#);
    apply(&mut outbound, &names(&["groups"]), None);
    assert_eq!(outbound.len(), 1);
}

#[test]
fn an_empty_incoming_claim_map_copies_nothing() {
    let mut outbound = claims(r#"{"sub":"alice"}"#);
    apply(&mut outbound, &names(&["groups"]), Some(&Claims::new()));
    assert_eq!(outbound.len(), 1);
}

#[test]
fn an_empty_optional_list_changes_nothing() {
    let incoming = claims(r#"{"groups":["eng"]}"#);
    let mut outbound = claims(r#"{"sub":"alice"}"#);
    apply(&mut outbound, &[], Some(&incoming));
    assert_eq!(outbound.len(), 1);
}

#[test]
fn every_claim_in_the_optional_list_is_forwarded_when_present() {
    let incoming = claims(r#"{"a":1,"b":"two","c":null,"d":[true]}"#);
    let mut outbound = Claims::new();
    apply(
        &mut outbound,
        &names(&["a", "b", "c", "d", "e"]),
        Some(&incoming),
    );
    assert_eq!(
        outbound.get("a"),
        Some(&Json::Number(serde_json::Number::from(1_i32)))
    );
    assert_eq!(outbound.get("b"), Some(&Json::Text("two".to_owned())));
    assert_eq!(outbound.get("c"), Some(&Json::Null));
    assert_eq!(outbound.get("d"), Some(&Json::List(vec![Json::Bool(true)])));
    assert_eq!(outbound.get("e"), None);
}
