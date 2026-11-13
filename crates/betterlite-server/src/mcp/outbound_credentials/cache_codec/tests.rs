#![cfg(test)]

use serde::Deserialize;

use super::OAuthTokenCacheCodec;
use crate::mcp::outbound_credentials::{BOUND_PREFIX, OAuthToken};

#[derive(Deserialize)]
struct PlainPayload {
    access_token: String,
    identity_binding_proof: String,
}

fn codec() -> OAuthTokenCacheCodec {
    OAuthTokenCacheCodec {
        encrypt: Box::new(|value: &str| format!("enc:{value}")),
        decrypt: Box::new(|blob: &str| blob.strip_prefix("enc:").map(str::to_owned)),
    }
}

fn token(access_token: &str, proof: Option<&str>) -> OAuthToken {
    OAuthToken {
        access_token: access_token.to_owned(),
        expires_at: None,
        refresh_token: None,
        scopes: Vec::new(),
        identity_binding_proof: proof.map(str::to_owned),
    }
}

fn bound_blob(payload: &str) -> String {
    format!("enc:{BOUND_PREFIX}{payload}")
}

#[test]
fn codec_encode_writes_a_bound_entry_under_the_prefix() {
    let token = token("access-secret", Some("proof-value"));
    let encoded = codec().encode(&token).unwrap();
    assert!(encoded.starts_with(&format!("enc:{BOUND_PREFIX}")));
    let plaintext = encoded.strip_prefix("enc:").unwrap();
    let payload: PlainPayload =
        serde_json::from_str(plaintext.strip_prefix(BOUND_PREFIX).unwrap()).unwrap();
    assert_eq!(payload.access_token, "access-secret");
    assert_eq!(payload.identity_binding_proof, "proof-value");
}

#[test]
fn codec_encode_writes_a_bearer_only_entry_without_proof() {
    let encoded = codec().encode(&token("access-secret", None)).unwrap();
    assert_eq!(encoded, "enc:access-secret");
}

#[test]
fn codec_encode_leaves_the_refresh_token_out() {
    let mut token = token("access-secret", Some("proof-value"));
    token.refresh_token = Some("refresh-secret".to_owned());
    let encoded = codec().encode(&token).unwrap();
    assert!(!encoded.contains("refresh-secret"));
}

#[test]
fn codec_decode_reads_a_bound_entry() {
    let blob = bound_blob(r#"{"access_token":"access-secret","identity_binding_proof":"proof"}"#);
    let decoded = codec().decode(&blob).unwrap();
    assert_eq!(decoded.access_token, "access-secret");
    assert_eq!(decoded.identity_binding_proof.as_deref(), Some("proof"));
    assert_eq!(decoded.refresh_token, None);
    assert_eq!(decoded.expires_at, None);
    assert!(decoded.scopes.is_empty());
}

#[test]
fn codec_decode_reads_a_legacy_bearer_only_entry() {
    let decoded = codec().decode("enc:legacy-access").unwrap();
    assert_eq!(decoded, token("legacy-access", None));
}

#[test]
fn codec_decode_returns_none_for_an_undecryptable_blob() {
    assert_eq!(codec().decode("not-encrypted"), None);
}

#[test]
fn codec_decode_returns_none_for_an_empty_plaintext() {
    assert_eq!(codec().decode("enc:"), None);
}

#[test]
fn codec_decode_returns_none_for_bound_json_that_does_not_parse() {
    assert_eq!(codec().decode(&bound_blob("{not-json")), None);
}

#[test]
fn codec_decode_returns_none_when_the_proof_is_missing() {
    let blob = bound_blob(r#"{"access_token":"access-secret"}"#);
    assert_eq!(codec().decode(&blob), None);
}

#[test]
fn codec_decode_returns_none_when_a_bound_value_is_not_a_string() {
    let blob = bound_blob(r#"{"access_token":"access-secret","identity_binding_proof":7}"#);
    assert_eq!(codec().decode(&blob), None);
}

#[test]
fn codec_decode_returns_none_for_an_empty_bearer_or_proof() {
    let empty_bearer = bound_blob(r#"{"access_token":"","identity_binding_proof":"proof"}"#);
    let empty_proof = bound_blob(r#"{"access_token":"access","identity_binding_proof":""}"#);
    assert_eq!(codec().decode(&empty_bearer), None);
    assert_eq!(codec().decode(&empty_proof), None);
}

#[test]
fn codec_round_trips_a_bound_token() {
    let codec = codec();
    let original = token("access-secret", Some("proof-value"));
    let encoded = codec.encode(&original).unwrap();
    assert_eq!(codec.decode(&encoded), Some(original));
}
