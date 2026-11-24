#![cfg(test)]

use rstest::rstest;

use crate::gateway_dcr::models::{
    ConnectFlow, GatewayAuthCode, GatewayDcrClient, is_gateway_dcr_client_id,
};
use crate::types::SessionAudience;

const CLIENT_ONE_URI: &str =
    r#"{"redirect_uris":["https://client.example/callback"],"iat":1700000000}"#;

const AUTH_CODE_JSON: &str = r#"{"user_id":"user-1","client_id":"llm_dcrc_client","redirect_uri":"https://client.example/callback","code_challenge":"challenge-1","jti":"jti-1","iat":1700000000,"exp":1700000120}"#;

const FLOW_JSON: &str = r#"{"user_id":"user-1","client_id":"llm_dcrc_client","redirect_uri":"https://client.example/callback","state":"state-1","code_challenge":"challenge-1","jti":"jti-1","exp":1700000600}"#;

fn client(text: &str) -> GatewayDcrClient {
    serde_json::from_str(text).unwrap()
}

fn flow(text: &str) -> ConnectFlow {
    serde_json::from_str(text).unwrap()
}

#[test]
fn gateway_dcr_client_accepts_one_and_four_redirect_uris() {
    assert_eq!(
        client(CLIENT_ONE_URI).redirect_uris,
        vec!["https://client.example/callback".to_owned()]
    );
    let four = r#"{"redirect_uris":["https://a.example/1","https://a.example/2","https://a.example/3","https://a.example/4"],"iat":7}"#;
    assert_eq!(client(four).redirect_uris.len(), 4);
    assert_eq!(client(four).iat, 7);
}

#[test]
fn gateway_dcr_client_rejects_zero_and_five_redirect_uris() {
    assert!(serde_json::from_str::<GatewayDcrClient>(r#"{"redirect_uris":[],"iat":1}"#).is_err());
    assert!(
        serde_json::from_str::<GatewayDcrClient>(
            r#"{"redirect_uris":["https://a.example/1","https://a.example/2","https://a.example/3","https://a.example/4","https://a.example/5"],"iat":1}"#
        )
        .is_err()
    );
}

#[test]
fn gateway_dcr_client_rejects_unknown_fields() {
    assert!(
        serde_json::from_str::<GatewayDcrClient>(
            r#"{"redirect_uris":["https://a.example/cb"],"iat":1,"extra":true}"#
        )
        .is_err()
    );
    assert!(
        serde_json::from_str::<GatewayDcrClient>(r#"{"redirect_uris":["https://a.example/cb"]}"#)
            .is_err()
    );
}

#[test]
fn gateway_dcr_client_rejects_a_sealed_auth_code() {
    assert!(serde_json::from_str::<GatewayDcrClient>(AUTH_CODE_JSON).is_err());
    assert!(serde_json::from_str::<GatewayDcrClient>(FLOW_JSON).is_err());
}

#[test]
fn gateway_auth_code_rejects_a_sealed_client_record() {
    assert!(serde_json::from_str::<GatewayAuthCode>(CLIENT_ONE_URI).is_err());
}

#[test]
fn connect_flow_keeps_an_empty_state() {
    let text = r#"{"user_id":"user-1","client_id":"llm_dcrc_client","redirect_uri":"https://client.example/callback","state":"","code_challenge":"challenge-1","jti":"jti-1","exp":1700000600}"#;
    assert_eq!(flow(text).state, "");
}

#[rstest]
#[case::empty_user_id(
    r#"{"user_id":"","client_id":"llm_dcrc_client","redirect_uri":"https://client.example/callback","state":"s","code_challenge":"c","jti":"j","exp":1}"#
)]
#[case::empty_client_id(
    r#"{"user_id":"user-1","client_id":"","redirect_uri":"https://client.example/callback","state":"s","code_challenge":"c","jti":"j","exp":1}"#
)]
#[case::empty_redirect_uri(
    r#"{"user_id":"user-1","client_id":"llm_dcrc_client","redirect_uri":"","state":"s","code_challenge":"c","jti":"j","exp":1}"#
)]
#[case::empty_code_challenge(
    r#"{"user_id":"user-1","client_id":"llm_dcrc_client","redirect_uri":"https://client.example/callback","state":"s","code_challenge":"","jti":"j","exp":1}"#
)]
#[case::empty_jti(
    r#"{"user_id":"user-1","client_id":"llm_dcrc_client","redirect_uri":"https://client.example/callback","state":"s","code_challenge":"c","jti":"","exp":1}"#
)]
fn connect_flow_rejects_empty_required_fields(#[case] text: &str) {
    assert!(serde_json::from_str::<ConnectFlow>(text).is_err());
}

#[test]
fn connect_flow_reads_and_writes_the_optional_fields() {
    let opened = flow(FLOW_JSON);
    assert!(opened.resource_server_id.is_none());
    assert!(opened.audience.is_none());
    let text = serde_json::to_string(&opened).unwrap();
    assert!(!text.contains("resource_server_id"));
    assert!(!text.contains("audience"));

    let with_optionals = r#"{"user_id":"user-1","client_id":"llm_dcrc_client","redirect_uri":"https://client.example/callback","state":"s","code_challenge":"c","jti":"j","exp":1,"resource_server_id":"srv-1","audience":"proxy_api"}"#;
    let opened = flow(with_optionals);
    assert_eq!(opened.resource_server_id.as_deref(), Some("srv-1"));
    assert_eq!(opened.audience, Some(SessionAudience::ProxyApi));
    let text = serde_json::to_string(&opened).unwrap();
    assert!(text.contains(r#""resource_server_id":"srv-1""#));
    assert!(text.contains(r#""audience":"proxy_api""#));
}

#[test]
fn gateway_auth_code_reads_and_writes_the_optional_fields() {
    let opened: GatewayAuthCode = serde_json::from_str(AUTH_CODE_JSON).unwrap();
    assert_eq!(opened.jti, "jti-1");
    assert_eq!(opened.iat, 1_700_000_000);
    assert_eq!(opened.exp, 1_700_000_120);
    assert!(opened.team_id.is_none());
    let text = serde_json::to_string(&opened).unwrap();
    assert!(!text.contains("team_id"));
    assert!(!text.contains("resource_server_id"));
    assert!(!text.contains("audience"));

    let with_team = r#"{"user_id":"user-1","client_id":"llm_dcrc_client","redirect_uri":"https://client.example/callback","code_challenge":"challenge-1","jti":"jti-1","iat":1,"exp":2,"team_id":"team-1"}"#;
    let opened: GatewayAuthCode = serde_json::from_str(with_team).unwrap();
    assert_eq!(opened.team_id.as_deref(), Some("team-1"));
}

#[test]
fn gateway_auth_code_rejects_empty_required_fields_and_unknown_keys() {
    assert!(serde_json::from_str::<GatewayAuthCode>(r#"{"user_id":"user-1","client_id":"llm_dcrc_client","redirect_uri":"https://client.example/callback","code_challenge":"","jti":"jti-1","iat":1,"exp":2}"#).is_err());
    assert!(serde_json::from_str::<GatewayAuthCode>(r#"{"user_id":"user-1","client_id":"llm_dcrc_client","redirect_uri":"https://client.example/callback","code_challenge":"c","jti":"jti-1","iat":1,"exp":2,"redirect_uris":[]}"#).is_err());
}

#[test]
fn is_gateway_dcr_client_id_matches_only_the_gateway_prefix() {
    assert!(is_gateway_dcr_client_id(Some("llm_dcrc_client")));
    assert!(!is_gateway_dcr_client_id(Some("llm_gcode_code")));
    assert!(!is_gateway_dcr_client_id(Some("llm_dcrc")));
    assert!(!is_gateway_dcr_client_id(Some("")));
    assert!(!is_gateway_dcr_client_id(None));
}
