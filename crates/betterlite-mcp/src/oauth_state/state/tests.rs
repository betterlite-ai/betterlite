#![cfg(test)]

use betterlite_db::mcp_stored_contracts::MCPTokenEndpointAuthMethod;
use betterlite_db::mcp_stored_contracts::secret::StoredSecret;
use secrecy::{ExposeSecret as _, SecretString};

use crate::oauth_state::test_support::{FieldBoundCipher, PlainCipher, RefusingCipher};

use super::{OauthState, StateError, decode_state, encode_state};

const FULL_STATE_JSON: &str = r#"{"base_url":"https://gateway.example/mcp","client_redirect_uri":"https://client.example/callback","code_challenge":"challenge","code_challenge_method":"S256","dcr_client_id":"dcr-client","dcr_client_secret":"dcr-secret","dcr_token_endpoint_auth_method":"client_secret_post","litellm_user_id":"user-1","mcp_server_id":"server-1","oauth_nonce":"nonce-1","original_state":"client-state"}"#;

const MINIMAL_STATE_JSON: &str = r#"{"base_url":"https://gateway.example/mcp","client_redirect_uri":null,"code_challenge":null,"code_challenge_method":null,"dcr_client_id":null,"dcr_client_secret":null,"dcr_token_endpoint_auth_method":null,"litellm_user_id":null,"mcp_server_id":null,"oauth_nonce":null,"original_state":"client-state"}"#;

fn secret(text: &str) -> StoredSecret {
    StoredSecret(SecretString::from(text.to_owned()))
}

fn full_state() -> OauthState {
    OauthState {
        base_url: "https://gateway.example/mcp".to_owned(),
        client_redirect_uri: Some("https://client.example/callback".to_owned()),
        code_challenge: Some("challenge".to_owned()),
        code_challenge_method: Some("S256".to_owned()),
        dcr_client_id: Some("dcr-client".to_owned()),
        dcr_client_secret: Some(secret("dcr-secret")),
        dcr_token_endpoint_auth_method: Some(MCPTokenEndpointAuthMethod::ClientSecretPost),
        litellm_user_id: Some("user-1".to_owned()),
        mcp_server_id: Some("server-1".to_owned()),
        oauth_nonce: Some("nonce-1".to_owned()),
        original_state: "client-state".to_owned(),
    }
}

fn minimal_state() -> OauthState {
    OauthState {
        base_url: "https://gateway.example/mcp".to_owned(),
        client_redirect_uri: None,
        code_challenge: None,
        code_challenge_method: None,
        dcr_client_id: None,
        dcr_client_secret: None,
        dcr_token_endpoint_auth_method: None,
        litellm_user_id: None,
        mcp_server_id: None,
        oauth_nonce: None,
        original_state: "client-state".to_owned(),
    }
}

#[test]
fn encode_state_writes_every_key_in_sorted_order() {
    let encoded = encode_state(&PlainCipher, &full_state());
    assert_eq!(encoded, FULL_STATE_JSON);
}

#[test]
fn encode_state_keeps_null_values_for_absent_options() {
    let encoded = encode_state(&PlainCipher, &minimal_state());
    assert_eq!(encoded, MINIMAL_STATE_JSON);
}

#[test]
fn encode_state_writes_the_client_secret_value() {
    let encoded = encode_state(&PlainCipher, &full_state());
    assert!(encoded.contains(r#""dcr_client_secret":"dcr-secret""#));
}

#[test]
fn decode_state_returns_every_field() {
    let state = decode_state(&PlainCipher, FULL_STATE_JSON).unwrap();
    assert_eq!(state.base_url, "https://gateway.example/mcp");
    assert_eq!(
        state.client_redirect_uri.as_deref(),
        Some("https://client.example/callback")
    );
    assert_eq!(state.code_challenge.as_deref(), Some("challenge"));
    assert_eq!(state.code_challenge_method.as_deref(), Some("S256"));
    assert_eq!(state.dcr_client_id.as_deref(), Some("dcr-client"));
    assert_eq!(
        state
            .dcr_client_secret
            .as_ref()
            .map(|stored| stored.0.expose_secret().to_owned()),
        Some("dcr-secret".to_owned())
    );
    assert_eq!(
        state.dcr_token_endpoint_auth_method,
        Some(MCPTokenEndpointAuthMethod::ClientSecretPost)
    );
    assert_eq!(state.litellm_user_id.as_deref(), Some("user-1"));
    assert_eq!(state.mcp_server_id.as_deref(), Some("server-1"));
    assert_eq!(state.oauth_nonce.as_deref(), Some("nonce-1"));
    assert_eq!(state.original_state, "client-state");
}

#[test]
fn decode_state_reads_absent_options_as_null() {
    let state = decode_state(&PlainCipher, MINIMAL_STATE_JSON).unwrap();
    assert_eq!(state.base_url, "https://gateway.example/mcp");
    assert!(state.client_redirect_uri.is_none());
    assert!(state.code_challenge.is_none());
    assert!(state.code_challenge_method.is_none());
    assert!(state.dcr_client_id.is_none());
    assert!(state.dcr_client_secret.is_none());
    assert!(state.dcr_token_endpoint_auth_method.is_none());
    assert!(state.litellm_user_id.is_none());
    assert!(state.mcp_server_id.is_none());
    assert!(state.oauth_nonce.is_none());
    assert_eq!(state.original_state, "client-state");
}

#[test]
fn encode_then_decode_round_trips() {
    let encoded = encode_state(&PlainCipher, &full_state());
    let state = decode_state(&PlainCipher, &encoded).unwrap();
    assert_eq!(state.litellm_user_id.as_deref(), Some("user-1"));
    assert_eq!(state.oauth_nonce.as_deref(), Some("nonce-1"));
}

#[test]
fn decode_state_reports_an_undecryptable_state() {
    let failure = decode_state(&RefusingCipher, "ciphertext").unwrap_err();
    assert_eq!(failure, StateError::Undecryptable);
    assert_eq!(failure.to_string(), "Failed to decrypt state parameter");
}

#[test]
fn decode_state_reports_malformed_json() {
    let failure = decode_state(&PlainCipher, "not json").unwrap_err();
    assert_eq!(failure, StateError::Malformed);
}

#[test]
fn decode_state_reports_a_json_array() {
    let failure = decode_state(&PlainCipher, "[]").unwrap_err();
    assert_eq!(failure, StateError::Malformed);
}

#[test]
fn decode_state_reports_a_wrong_field_type() {
    let failure = decode_state(&PlainCipher, r#"{"base_url":5,"original_state":"s"}"#).unwrap_err();
    assert_eq!(failure, StateError::Malformed);
}

#[test]
fn decode_state_reports_a_missing_required_field() {
    let failure = decode_state(&PlainCipher, r#"{"base_url":"https://gw"}"#).unwrap_err();
    assert_eq!(failure, StateError::Malformed);
}

#[test]
fn decode_state_ignores_an_unknown_key() {
    let text = FULL_STATE_JSON.replace('}', r#","unknown":"kept"}"#);
    let state = decode_state(&PlainCipher, &text).unwrap();
    assert_eq!(state.original_state, "client-state");
}

#[test]
fn decode_state_passes_the_oauth_state_label() {
    let state = decode_state(
        &FieldBoundCipher {
            field: "oauth_state",
        },
        FULL_STATE_JSON,
    );
    assert_eq!(state.unwrap().mcp_server_id.as_deref(), Some("server-1"));
    let refused = decode_state(
        &FieldBoundCipher {
            field: "bridge_authorization_code",
        },
        FULL_STATE_JSON,
    );
    assert_eq!(refused.unwrap_err(), StateError::Undecryptable);
}
