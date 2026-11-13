#![cfg(test)]
use crate::otel::model::metadata::request_identity::parse::from_payload;
use crate::otel::model::metadata::wire::{decode, payload::Payload};

#[test]
fn parse_prefers_canonical_team_id_and_deployment_model() {
    let payload: Payload = serde_json::from_str(r#"{"id":"id","model":"openai/deploy","metadata":{"user_api_key_team_id":"canonical","team_id":"alias","deployment":"azure/deploy","user_api_key_team_metadata":{"region":"west"},"nested":{"value":2}}}"#).expect("test JSON is valid");
    let identity = from_payload(&payload).unwrap();
    assert_eq!(identity.call_id.as_deref(), Some("id"));
    assert_eq!(identity.team_id.as_deref(), Some("canonical"));
    assert_eq!(identity.provider_model.as_deref(), Some("azure/deploy"));
    assert_eq!(
        identity.metadata.get("nested.value").map(String::as_str),
        Some("2")
    );
}

#[test]
fn parse_uses_payload_identity_fallbacks() {
    let payload: Payload = serde_json::from_str(r#"{"litellm_call_id":"call","model":"model","end_user":"caller","metadata":{"team_id":"team","team_alias":"alias","user_api_key_hash":"hash","user_api_key_request_route":"/chat","user_api_key_end_user_id":"fallback"}}"#).expect("test JSON is valid");
    let identity = from_payload(&payload).unwrap();
    assert_eq!(identity.call_id.as_deref(), Some("call"));
    assert_eq!(identity.team_alias.as_deref(), Some("alias"));
    assert_eq!(identity.key_hash.as_deref(), Some("hash"));
    assert_eq!(identity.end_user.as_deref(), Some("caller"));
    assert_eq!(identity.request_route.as_deref(), Some("/chat"));
}

#[rstest::rstest]
#[case::user(
    r#"{"metadata":{"user_api_key_user_id":false}}"#,
    "metadata.user_api_key_user_id"
)]
#[case::model(r#"{"model":17}"#, "model")]
#[case::team(
    r#"{"metadata":{"user_api_key_team_id":false}}"#,
    "metadata.user_api_key_team_id"
)]
#[case::hidden(
    r#"{"hidden_params":{"litellm_model_name":[]}}"#,
    "hidden_params.litellm_model_name"
)]
fn rejects_malformed_model(#[case] source: &str, #[case] field: &str) {
    assert!(
        decode::<Payload>(source)
            .unwrap_err()
            .to_string()
            .contains(field)
    );
}

#[test]
fn null_payload_sections_default_and_empty_values_are_preserved() {
    let payload: Payload =
        decode(r#"{"metadata":null,"hidden_params":null,"response":null,"model":""}"#).unwrap();
    let identity = from_payload(&payload).unwrap();
    assert_eq!(identity.provider_model.as_deref(), Some(""));
    assert!(identity.metadata.is_empty());
}
