#![cfg(test)]

use betterlite_prompts::arguments::json::{Json, Object};

use crate::mcp_token_claims::Claims;
use crate::mcp_token_claims::apply_claim_operations::apply;

fn claims(json: &str) -> Claims {
    serde_json::from_str(json).unwrap()
}

fn object(json: &str) -> Object {
    serde_json::from_str(json).unwrap()
}

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn add_claims_inserts_only_absent_keys() {
    let mut target = claims(r#"{"iss":"https://issuer.example"}"#);
    apply(
        &mut target,
        &object(r#"{"iss":"https://other.example","deployment_id":"prod-001"}"#),
        &Object::new(),
        &[],
    );
    assert_eq!(
        target.get("iss"),
        Some(&Json::Text("https://issuer.example".to_owned()))
    );
    assert_eq!(
        target.get("deployment_id"),
        Some(&Json::Text("prod-001".to_owned()))
    );
}

#[test]
fn set_claims_override_existing_keys_and_add_new_ones() {
    let mut target = claims(r#"{"scope":"old"}"#);
    apply(
        &mut target,
        &Object::new(),
        &object(r#"{"scope":"new","env":"production"}"#),
        &[],
    );
    assert_eq!(target.get("scope"), Some(&Json::Text("new".to_owned())));
    assert_eq!(
        target.get("env"),
        Some(&Json::Text("production".to_owned()))
    );
}

#[test]
fn remove_claims_deletes_listed_keys_and_ignores_unknown_ones() {
    let mut target = claims(r#"{"nbf":1700000000,"sub":"alice"}"#);
    apply(
        &mut target,
        &Object::new(),
        &Object::new(),
        &names(&["nbf", "missing"]),
    );
    assert_eq!(target.get("nbf"), None);
    assert_eq!(target.get("sub"), Some(&Json::Text("alice".to_owned())));
}

#[test]
fn set_claims_win_over_add_claims_and_remove_claims_delete_both() {
    let mut target = Claims::new();
    apply(
        &mut target,
        &object(r#"{"env":"staging","only_add":"a"}"#),
        &object(r#"{"env":"production"}"#),
        &names(&["only_add"]),
    );
    assert_eq!(
        target.get("env"),
        Some(&Json::Text("production".to_owned()))
    );
    assert_eq!(target.get("only_add"), None);
}

#[test]
fn all_three_operations_apply_in_order_to_one_map() {
    let mut target = claims(r#"{"iss":"https://issuer.example","sub":"alice"}"#);
    apply(
        &mut target,
        &object(r#"{"iss":"ignored","deployment_id":"prod-001","remove_me":"x"}"#),
        &object(r#"{"sub":"override"}"#),
        &names(&["remove_me"]),
    );
    assert_eq!(
        target.get("iss"),
        Some(&Json::Text("https://issuer.example".to_owned()))
    );
    assert_eq!(target.get("sub"), Some(&Json::Text("override".to_owned())));
    assert_eq!(
        target.get("deployment_id"),
        Some(&Json::Text("prod-001".to_owned()))
    );
    assert_eq!(target.get("remove_me"), None);
}

#[test]
fn structured_claim_values_are_inserted_unchanged() {
    let mut target = Claims::new();
    apply(
        &mut target,
        &object(r#"{"roles":["admin","billing"],"limits":{"rpm":10}}"#),
        &Object::new(),
        &[],
    );
    assert_eq!(
        target.get("roles"),
        Some(&Json::List(vec![
            Json::Text("admin".to_owned()),
            Json::Text("billing".to_owned()),
        ]))
    );
    assert_eq!(
        target.get("limits"),
        Some(&Json::Object(object(r#"{"rpm":10}"#)))
    );
}
