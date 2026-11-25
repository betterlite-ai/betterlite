#![cfg(test)]

use rstest::rstest;

use crate::gateway_dcr::models::{
    ConnectFlow, GatewayAuthCode, GatewayDcrClient, is_gateway_dcr_client_id,
};
use crate::gateway_dcr::sealed::Sealable as _;
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
    let one = client(CLIENT_ONE_URI);
    assert_eq!(
        one.redirect_uris,
        vec!["https://client.example/callback".to_owned()]
    );
    assert!(one.acceptable());
    let four = r#"{"redirect_uris":["https://a.example/1","https://a.example/2","https://a.example/3","https://a.example/4"],"iat":7}"#;
    assert_eq!(client(four).redirect_uris.len(), 4);
    assert_eq!(client(four).iat, 7);
    assert!(client(four).acceptable());
}

#[test]
fn gateway_dcr_client_refuses_zero_and_five_redirect_uris_on_open() {
    let none = client(r#"{"redirect_uris":[],"iat":1}"#);
    assert!(!none.acceptable());
    let five = r#"{"redirect_uris":["https://a.example/1","https://a.example/2","https://a.example/3","https://a.example/4","https://a.example/5"],"iat":1}"#;
    assert!(!client(five).acceptable());
}

#[test]
fn gateway_dcr_client_rejects_unknown_fields() {
    serde_json::from_str::<GatewayDcrClient>(
        r#"{"redirect_uris":["https://a.example/cb"],"iat":1,"extra":true}"#,
    )
    .unwrap_err();
    serde_json::from_str::<GatewayDcrClient>(r#"{"redirect_uris":["https://a.example/cb"]}"#)
        .unwrap_err();
}

#[test]
fn gateway_dcr_client_rejects_a_sealed_auth_code() {
    serde_json::from_str::<GatewayDcrClient>(AUTH_CODE_JSON).unwrap_err();
    serde_json::from_str::<GatewayDcrClient>(FLOW_JSON).unwrap_err();
}

#[test]
fn gateway_auth_code_rejects_a_sealed_client_record() {
    serde_json::from_str::<GatewayAuthCode>(CLIENT_ONE_URI).unwrap_err();
}

#[test]
fn connect_flow_keeps_an_empty_state() {
    let text = r#"{"user_id":"user-1","client_id":"llm_dcrc_client","redirect_uri":"https://client.example/callback","state":"","code_challenge":"challenge-1","jti":"jti-1","exp":1700000600}"#;
    assert_eq!(flow(text).state, "");
    assert!(flow(text).acceptable());
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
    assert!(!flow(text).acceptable());
}

#[test]
fn connect_flow_reads_and_writes_the_optional_fields() {
    let opened = flow(FLOW_JSON);
    assert!(opened.resource_server_id.is_none());
    assert!(opened.audience.is_none());
    let plain_text = serde_json::to_string(&opened).unwrap();
    assert!(!plain_text.contains("resource_server_id"));
    assert!(!plain_text.contains("audience"));

    let with_optionals = r#"{"user_id":"user-1","client_id":"llm_dcrc_client","redirect_uri":"https://client.example/callback","state":"s","code_challenge":"c","jti":"j","exp":1,"resource_server_id":"srv-1","audience":"proxy_api"}"#;
    let optional = flow(with_optionals);
    assert_eq!(optional.resource_server_id.as_deref(), Some("srv-1"));
    assert_eq!(optional.audience, Some(SessionAudience::ProxyApi));
    let optional_text = serde_json::to_string(&optional).unwrap();
    assert!(optional_text.contains(r#""resource_server_id":"srv-1""#));
    assert!(optional_text.contains(r#""audience":"proxy_api""#));
}

#[test]
fn gateway_auth_code_reads_and_writes_the_optional_fields() {
    let opened: GatewayAuthCode = serde_json::from_str(AUTH_CODE_JSON).unwrap();
    assert_eq!(opened.jti, "jti-1");
    assert_eq!(opened.iat, 1_700_000_000);
    assert_eq!(opened.exp, 1_700_000_120);
    assert!(opened.team_id.is_none());
    let plain_text = serde_json::to_string(&opened).unwrap();
    assert!(!plain_text.contains("team_id"));
    assert!(!plain_text.contains("resource_server_id"));
    assert!(!plain_text.contains("audience"));

    let with_team = r#"{"user_id":"user-1","client_id":"llm_dcrc_client","redirect_uri":"https://client.example/callback","code_challenge":"challenge-1","jti":"jti-1","iat":1,"exp":2,"team_id":"team-1"}"#;
    let team_code: GatewayAuthCode = serde_json::from_str(with_team).unwrap();
    assert_eq!(team_code.team_id.as_deref(), Some("team-1"));
}

#[test]
fn gateway_auth_code_refuses_empty_required_fields_on_open() {
    let empty_challenge = r#"{"user_id":"user-1","client_id":"llm_dcrc_client","redirect_uri":"https://client.example/callback","code_challenge":"","jti":"jti-1","iat":1,"exp":2}"#;
    let empty_code: GatewayAuthCode = serde_json::from_str(empty_challenge).unwrap();
    assert!(!empty_code.acceptable());
    let complete = r#"{"user_id":"user-1","client_id":"llm_dcrc_client","redirect_uri":"https://client.example/callback","code_challenge":"c","jti":"jti-1","iat":1,"exp":2}"#;
    let complete_code: GatewayAuthCode = serde_json::from_str(complete).unwrap();
    assert!(complete_code.acceptable());
}

#[test]
fn gateway_auth_code_rejects_unknown_keys() {
    serde_json::from_str::<GatewayAuthCode>(r#"{"user_id":"user-1","client_id":"llm_dcrc_client","redirect_uri":"https://client.example/callback","code_challenge":"c","jti":"jti-1","iat":1,"exp":2,"redirect_uris":[]}"#).unwrap_err();
}

#[test]
fn is_gateway_dcr_client_id_matches_only_the_gateway_prefix() {
    assert!(is_gateway_dcr_client_id(Some("llm_dcrc_client")));
    assert!(!is_gateway_dcr_client_id(Some("llm_gcode_code")));
    assert!(!is_gateway_dcr_client_id(Some("llm_dcrc")));
    assert!(!is_gateway_dcr_client_id(Some("")));
    assert!(!is_gateway_dcr_client_id(None));
}
