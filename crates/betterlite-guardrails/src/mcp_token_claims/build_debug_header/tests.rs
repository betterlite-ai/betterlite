#![cfg(test)]

use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use crate::mcp_token_claims::Claims;
use crate::mcp_token_claims::build_debug_header::render;

fn claims(json: &str) -> Claims {
    serde_json::from_str(json).unwrap()
}

#[test]
fn the_header_names_the_key_and_the_signing_context_in_order() {
    let claims = claims(
        r#"{"sub":"alice","iss":"https://issuer.example","exp":1700000000,"scope":"mcp:tools/call"}"#,
    );
    assert_eq!(
        render(&claims, "kid-1"),
        "v=1; kid=kid-1; sub=alice; iss=https://issuer.example; exp=1700000000; scope=mcp:tools/call"
    );
}

#[test]
fn missing_claims_render_as_empty_values() {
    assert_eq!(
        render(&Claims::new(), "kid-1"),
        "v=1; kid=kid-1; sub=; iss=; exp=0; scope="
    );
}

#[rstest]
#[case::short(12, false)]
#[case::at_the_limit(80, false)]
#[case::one_past_the_limit(81, true)]
#[case::well_past_the_limit(200, true)]
fn long_scopes_are_truncated_after_eighty_characters(
    #[case] length: usize,
    #[case] truncated: bool,
) {
    let scope = "a".repeat(length);
    let mut claims = Claims::new();
    claims.insert("scope".to_owned(), Json::Text(scope.clone()));
    let expected = if truncated {
        format!("{}...", "a".repeat(77))
    } else {
        scope
    };
    assert_eq!(
        render(&claims, "kid-1"),
        format!("v=1; kid=kid-1; sub=; iss=; exp=0; scope={expected}")
    );
}

#[test]
fn the_limit_counts_characters_not_bytes() {
    let scope = "\u{e9}".repeat(80);
    let mut claims = Claims::new();
    claims.insert("scope".to_owned(), Json::Text(scope.clone()));
    assert!(render(&claims, "kid-1").ends_with(&format!("scope={scope}")));
    let long_scope = "\u{e9}".repeat(81);
    claims.insert("scope".to_owned(), Json::Text(long_scope));
    assert!(render(&claims, "kid-1").ends_with(&format!("scope={}...", "\u{e9}".repeat(77))));
}

#[test]
fn a_missing_scope_is_not_truncated() {
    let claims = claims(r#"{"sub":"alice"}"#);
    assert_eq!(
        render(&claims, "kid-1"),
        "v=1; kid=kid-1; sub=alice; iss=; exp=0; scope="
    );
}
