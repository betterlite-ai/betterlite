#![cfg(test)]

use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

use crate::mcp_token_claims::build_channel_token_claims::build;
use crate::mcp_token_claims::{ClaimError, Claims};

const NOW: i64 = 1_700_000_000;
const CHANNEL_AUDIENCE: &str = "bedrock-gateway";

fn claims(json: &str) -> Claims {
    serde_json::from_str(json).unwrap()
}

#[test]
fn the_channel_token_keeps_identity_and_replaces_audience_and_timing() {
    let base = claims(
        r#"{"iss":"https://issuer.example","aud":"mcp","iat":1699999999,"exp":1700000300,"nbf":1699999999,"sub":"alice","act":{"sub":"team-1"},"scope":"mcp:tools/call","email":"alice@example.com","deployment_id":"prod-001"}"#,
    );
    let channel = build(&base, CHANNEL_AUDIENCE, 60, NOW).unwrap();
    assert_eq!(channel.get("sub"), Some(&Json::Text("alice".to_owned())));
    assert_eq!(
        channel.get("act"),
        Some(&Json::Object(Object::from_iter([(
            "sub".to_owned(),
            Json::Text("team-1".to_owned()),
        )])))
    );
    assert_eq!(
        channel.get("scope"),
        Some(&Json::Text("mcp:tools/call".to_owned()))
    );
    assert_eq!(
        channel.get("email"),
        Some(&Json::Text("alice@example.com".to_owned()))
    );
    assert_eq!(
        channel.get("deployment_id"),
        Some(&Json::Text("prod-001".to_owned()))
    );
    assert_eq!(
        channel.get("iss"),
        Some(&Json::Text("https://issuer.example".to_owned()))
    );
    assert_eq!(
        channel.get("aud"),
        Some(&Json::Text(CHANNEL_AUDIENCE.to_owned()))
    );
    assert_eq!(channel.get("iat"), Some(&Json::Number(Number::from(NOW))));
    assert_eq!(
        channel.get("exp"),
        Some(&Json::Number(Number::from(1_700_000_060_i64)))
    );
    assert_eq!(channel.get("nbf"), Some(&Json::Number(Number::from(NOW))));
}

#[test]
fn the_access_token_claims_are_left_untouched() {
    let base = claims(r#"{"aud":"mcp","iat":1,"exp":2,"nbf":1,"sub":"alice"}"#);
    let before = base.clone();
    let channel = build(&base, CHANNEL_AUDIENCE, 60, NOW).unwrap();
    assert_eq!(base, before);
    assert_ne!(channel, base);
}

#[test]
fn the_channel_expiry_uses_the_channel_ttl() {
    let base = claims(r#"{"sub":"alice"}"#);
    let channel = build(&base, CHANNEL_AUDIENCE, 900, NOW).unwrap();
    assert_eq!(
        channel.get("exp"),
        Some(&Json::Number(Number::from(1_700_000_900_i64)))
    );
}

#[test]
fn a_timestamp_overflow_is_a_typed_error() {
    let error = build(&Claims::new(), CHANNEL_AUDIENCE, 1, i64::MAX).unwrap_err();
    assert_eq!(error, ClaimError::TimestampOverflow);
}
