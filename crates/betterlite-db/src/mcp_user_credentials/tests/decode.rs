use rstest::rstest;

use super::super::decode::{decode_user_credential, parse_oauth_payload};
use super::super::types::{CredentialError, CredentialType};
use super::support::{cipher, exposed, secret};
use super::{
    ENCRYPTED_OAUTH, ENCRYPTED_SECRET, ENCRYPTED_TEXT, LEGACY_BYOK, LEGACY_OAUTH, LEGACY_SECRET,
    NOT_JSON, OAUTH_PAYLOAD, UNDECRYPTABLE,
};

#[test]
fn decode_credential_reads_a_lite_llm_ciphertext() {
    let decoded = decode_user_credential(&cipher(), ENCRYPTED_TEXT);
    assert_eq!(exposed(decoded.as_ref()), Some("encrypted-value"));
}

#[test]
fn decode_credential_falls_back_to_url_safe_base64_for_legacy_rows() {
    let decoded = decode_user_credential(&cipher(), LEGACY_SECRET);
    assert_eq!(exposed(decoded.as_ref()), Some("legacy-secret"));
}

#[test]
fn decode_credential_returns_none_when_neither_path_yields_text() {
    assert!(decode_user_credential(&cipher(), UNDECRYPTABLE).is_none());
    assert!(decode_user_credential(&cipher(), "abc").is_none());
}

#[test]
fn parse_oauth_payload_reads_a_stored_payload() {
    let parsed = parse_oauth_payload(Some(&secret(OAUTH_PAYLOAD))).expect("parsing runs");
    let payload = parsed.expect("the value holds an OAuth2 payload");
    assert_eq!(payload.credential_type, CredentialType::Oauth2);
    assert_eq!(exposed(payload.access_token.as_ref()), Some("tok-123"));
    assert_eq!(exposed(payload.refresh_token.as_ref()), Some("ref-456"));
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
    assert_eq!(exposed(decoded.as_ref()), Some("secret-value"));
    let parsed = parse_oauth_payload(decoded.as_ref()).expect("parsing runs");
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
    let decoded = decoded.map(secret);
    let parsed = parse_oauth_payload(decoded.as_ref()).expect("parsing runs");
    assert!(parsed.is_none());
}

#[test]
fn parse_oauth_payload_drops_keys_outside_the_declared_payload() {
    let decoded = secret(r#"{"type":"oauth2","access_token":"tok","client_secret":"hidden"}"#);
    let parsed = parse_oauth_payload(Some(&decoded))
        .expect("parsing runs")
        .expect("the value holds an OAuth2 payload");
    let written = serde_json::to_string(&parsed).expect("the payload serializes");
    assert!(!written.contains("client_secret"));
    assert!(written.contains("\"access_token\":\"tok\""));
}

#[test]
fn parse_oauth_payload_reports_a_payload_field_of_the_wrong_type() {
    let decoded = secret("{\"type\":\"oauth2\",\"scopes\":\"read\"}");
    let failed = parse_oauth_payload(Some(&decoded));
    let Err(CredentialError::Json(source)) = failed else {
        panic!("a mistyped field is a Json error");
    };
    assert!(source.is_data());
}

#[test]
fn decode_oauth_payload_composes_decoding_and_parsing() {
    let stored = decode_user_credential(&cipher(), ENCRYPTED_OAUTH);
    let payload = parse_oauth_payload(stored.as_ref())
        .expect("parsing runs")
        .expect("the row holds a payload");
    assert_eq!(exposed(payload.access_token.as_ref()), Some("tok-123"));
    let byok = parse_oauth_payload(decode_user_credential(&cipher(), ENCRYPTED_SECRET).as_ref())
        .expect("parsing runs");
    assert!(byok.is_none());
    let unreadable = parse_oauth_payload(decode_user_credential(&cipher(), UNDECRYPTABLE).as_ref())
        .expect("parsing runs");
    assert!(unreadable.is_none());
}

#[test]
fn decode_oauth_payload_reads_a_legacy_base64_payload() {
    let stored = decode_user_credential(&cipher(), LEGACY_OAUTH);
    let payload = parse_oauth_payload(stored.as_ref())
        .expect("parsing runs")
        .expect("the legacy row holds a payload");
    assert_eq!(
        payload.scopes,
        Some(vec!["read".to_owned(), "write".to_owned()])
    );
}

#[test]
fn decode_oauth_payload_returns_none_for_a_legacy_row_that_is_not_json() {
    let stored = decode_user_credential(&cipher(), NOT_JSON);
    let decoded = parse_oauth_payload(stored.as_ref()).expect("parsing runs");
    assert!(decoded.is_none());
}
