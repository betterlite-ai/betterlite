use super::super::types::{
    CredentialType, McpUserCredentialListItem, OAuthCredentialPayload, OAuthGrantState,
};

#[test]
fn credential_type_uses_the_lite_llm_names() {
    let oauth2 = serde_json::to_string(&CredentialType::Oauth2).expect("the type serializes");
    let byok = serde_json::to_string(&CredentialType::Byok).expect("the type serializes");
    assert_eq!(oauth2, "\"oauth2\"");
    assert_eq!(byok, "\"byok\"");
    let read = serde_json::from_str::<CredentialType>("\"oauth2\"");
    assert_eq!(read.expect("the type reads"), CredentialType::Oauth2);
    let unknown = serde_json::from_str::<CredentialType>("\"anything\"");
    assert!(unknown.is_err());
}

#[test]
fn grant_state_uses_the_lite_llm_literals() {
    assert_eq!(OAuthGrantState::Valid.as_str(), "valid");
    assert_eq!(OAuthGrantState::Refreshable.as_str(), "refreshable");
    assert_eq!(OAuthGrantState::Absent.as_str(), "absent");
    let written =
        serde_json::to_string(&OAuthGrantState::Refreshable).expect("the state serializes");
    assert_eq!(written, "\"refreshable\"");
}

#[test]
fn payload_reads_the_lite_llm_field_names() {
    let text = r#"{"type":"oauth2","access_token":"tok","refresh_token":null,"expires_at":"2030-01-02T03:04:05+00:00","connected_at":"2024-01-02T03:04:05+00:00","scopes":["read"],"identity_binding_proof":"proof","server_id":"s1"}"#;
    let payload: OAuthCredentialPayload = serde_json::from_str(text).expect("the payload reads");
    assert_eq!(payload.credential_type, CredentialType::Oauth2);
    assert_eq!(payload.access_token.as_deref(), Some("tok"));
    assert_eq!(payload.refresh_token, None);
    assert_eq!(
        payload.expires_at.as_deref(),
        Some("2030-01-02T03:04:05+00:00")
    );
    assert_eq!(
        payload.connected_at.as_deref(),
        Some("2024-01-02T03:04:05+00:00")
    );
    assert_eq!(payload.scopes, Some(vec!["read".to_owned()]));
    assert_eq!(payload.identity_binding_proof.as_deref(), Some("proof"));
    assert_eq!(payload.server_id.as_deref(), Some("s1"));
}

#[test]
fn payload_writes_only_the_fields_that_are_set() {
    let text = r#"{"type":"oauth2","access_token":"tok","identity_binding_proof":"proof","server_id":"s1"}"#;
    let payload: OAuthCredentialPayload = serde_json::from_str(text).expect("the payload reads");
    let written = serde_json::to_string(&payload).expect("the payload serializes");
    assert!(written.contains("\"type\":\"oauth2\""));
    assert!(written.contains("\"access_token\":\"tok\""));
    assert!(written.contains("\"identity_binding_proof\":\"proof\""));
    assert!(written.contains("\"server_id\":\"s1\""));
    assert!(!written.contains("refresh_token"));
    assert!(!written.contains("expires_at"));
    assert!(!written.contains("connected_at"));
    assert!(!written.contains("scopes"));
}

#[test]
fn payload_defaults_the_type_when_the_field_is_absent() {
    let payload: OAuthCredentialPayload =
        serde_json::from_str("{\"access_token\":\"tok\"}").expect("the payload reads");
    assert_eq!(payload.credential_type, CredentialType::Oauth2);
}

#[test]
fn item_writes_every_field_including_the_absent_ones() {
    let item = McpUserCredentialListItem {
        user_id: "u1".to_owned(),
        credential_type: CredentialType::Byok,
        expires_at: None,
        connected_at: None,
        updated_at: "2024-01-02T03:04:05+00:00".to_owned(),
    };
    let written = serde_json::to_string(&item).expect("the item serializes");
    assert_eq!(
        written,
        "{\"user_id\":\"u1\",\"credential_type\":\"byok\",\"expires_at\":null,\"connected_at\":null,\"updated_at\":\"2024-01-02T03:04:05+00:00\"}"
    );
}
