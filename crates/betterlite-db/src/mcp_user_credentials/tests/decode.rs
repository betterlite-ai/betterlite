use rstest::rstest;

use super::super::decode::{decode_oauth_payload, decode_user_credential, parse_oauth_payload};
use super::super::types::CredentialType;
use super::support::cipher;
use super::{
    ENCRYPTED_OAUTH, ENCRYPTED_SECRET, ENCRYPTED_TEXT, LEGACY_BYOK, LEGACY_OAUTH, LEGACY_SECRET,
    NOT_JSON, OAUTH_PAYLOAD, UNDECRYPTABLE,
};

#[test]
fn decode_credential_reads_a_lite_llm_ciphertext() {
    let decoded = decode_user_credential(&cipher(), ENCRYPTED_TEXT);
    assert_eq!(decoded.as_deref(), Some("encrypted-value"));
}

#[test]
fn decode_credential_falls_back_to_url_safe_base64_for_legacy_rows() {
    let decoded = decode_user_credential(&cipher(), LEGACY_SECRET);
    assert_eq!(decoded.as_deref(), Some("legacy-secret"));
}

#[test]
fn decode_credential_returns_none_when_neither_path_yields_text() {
    assert_eq!(decode_user_credential(&cipher(), UNDECRYPTABLE), None);
    assert_eq!(decode_user_credential(&cipher(), "abc"), None);
}

#[test]
fn parse_oauth_payload_reads_a_stored_payload() {
    let parsed = parse_oauth_payload(Some(OAUTH_PAYLOAD)).expect("parsing runs");
    let payload = parsed.expect("the value holds an OAuth2 payload");
    assert_eq!(payload.credential_type, CredentialType::Oauth2);
    assert_eq!(payload.access_token.as_deref(), Some("tok-123"));
    assert_eq!(payload.refresh_token.as_deref(), Some("ref-456"));
    assert_eq!(
        payload.connected_at.as_deref(),
        Some("2024-01-02T03:04:05+00:00")
    );
    assert_eq!(
        payload.expires_at.as_deref(),
        Some("2030-01-02T03:04:05+00:00")
    );
    assert_eq!(
        payload.scopes,
        Some(vec!["read".to_owned(), "write".to_owned()])
    );
    assert_eq!(payload.identity_binding_proof, None);
    assert_eq!(payload.server_id, None);
}

#[test]
fn parse_oauth_payload_returns_none_for_a_byok_key() {
    let decoded = decode_user_credential(&cipher(), LEGACY_BYOK);
    assert_eq!(decoded.as_deref(), Some("secret-value"));
    let parsed = parse_oauth_payload(decoded.as_deref()).expect("parsing runs");
    assert!(parsed.is_none());
}

#[rstest]
#[case::absent(None)]
#[case::not_json(Some("not-json{"))]
#[case::number(Some("12345"))]
#[case::text(Some("\"oauth2\""))]
#[case::array(Some("[\"oauth2\"]"))]
#[case::another_type(Some("{\"type\":\"byok\",\"access_token\":\"tok\"}"))]
#[case::no_type(Some("{\"access_token\":\"tok\"}"))]
#[case::type_not_text(Some("{\"type\":7}"))]
fn parse_oauth_payload_returns_none_for_values_that_are_not_a_payload_object(
    #[case] decoded: Option<&str>,
) {
    let parsed = parse_oauth_payload(decoded).expect("parsing runs");
    assert!(parsed.is_none());
}

#[test]
fn parse_oauth_payload_drops_keys_outside_the_declared_payload() {
    let parsed = parse_oauth_payload(Some(
        r#"{"type":"oauth2","access_token":"tok","client_secret":"hidden"}"#,
    ))
    .expect("parsing runs")
    .expect("the value holds an OAuth2 payload");
    let written = serde_json::to_string(&parsed).expect("the payload serializes");
    assert!(!written.contains("client_secret"));
}

#[test]
fn parse_oauth_payload_reports_a_payload_field_of_the_wrong_type() {
    let failed = parse_oauth_payload(Some("{\"type\":\"oauth2\",\"scopes\":\"read\"}"));
    assert!(failed.is_err());
}

#[test]
fn decode_oauth_payload_composes_decoding_and_parsing() {
    let decoded =
        decode_oauth_payload(&cipher(), ENCRYPTED_OAUTH).expect("the row holds a payload");
    assert_eq!(decoded.access_token.as_deref(), Some("tok-123"));
    assert!(decode_oauth_payload(&cipher(), ENCRYPTED_SECRET).is_none());
    assert!(decode_oauth_payload(&cipher(), UNDECRYPTABLE).is_none());
}

#[test]
fn decode_oauth_payload_reads_a_legacy_base64_payload() {
    let decoded =
        decode_oauth_payload(&cipher(), LEGACY_OAUTH).expect("the legacy row holds a payload");
    assert_eq!(
        decoded.scopes,
        Some(vec!["read".to_owned(), "write".to_owned()])
    );
}

#[test]
fn decode_oauth_payload_returns_none_for_a_legacy_row_that_is_not_json() {
    assert!(decode_oauth_payload(&cipher(), NOT_JSON).is_none());
}
