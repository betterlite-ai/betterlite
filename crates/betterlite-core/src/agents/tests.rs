#![cfg(test)]
use super::{
    AgentCard, AgentKillSwitchConfig, AgentKillSwitchResult, AgentObjectPermission, AgentResponse,
    AugmentedAgentCard, PatchAgentRequest,
};
use rust_decimal::Decimal;
use serde_json::{from_str, to_string};

const CARD: &str = r#"{"protocolVersion":"1.0","name":"worker","description":"does work","url":"https://agent.example","version":"1","capabilities":{"streaming":null},"defaultInputModes":["text"],"defaultOutputModes":["text"],"skills":[],"iconUrl":null}"#;

#[test]
fn agent_cards_preserve_nulls_and_security_variants() {
    let card: AgentCard = from_str(CARD).unwrap();
    assert_eq!(to_string(&card).unwrap(), CARD);
    let schemes = r#"{"protocolVersion":"1.0","name":"worker","description":"does work","url":"https://agent.example","version":"1","capabilities":{},"defaultInputModes":[],"defaultOutputModes":[],"skills":[],"additionalInterfaces":[{"url":"https://agent.example","transport":"HTTP"}],"provider":{"organization":"acme","url":"https://acme.example"},"securitySchemes":{"http":{"type":"http","scheme":"bearer"},"key":{"type":"apiKey","in_":"header","name":"x-key","description":null},"mtls":{"type":"mutualTLS"},"oauth":{"type":"oauth2","flows":{"authorizationCode":null}},"oidc":{"type":"openIdConnect","openIdConnectUrl":"https://issuer.example"}},"signatures":[{"protected":"p","signature":"s"}]}"#;
    let parsed: AgentCard = from_str(schemes).unwrap();
    assert_eq!(to_string(&parsed).unwrap(), schemes);
    from_str::<AgentCard>(&schemes.replace("\"type\":\"apiKey\"", "\"type\":\"unknown\""))
        .unwrap_err();
    from_str::<AgentCard>(&schemes.replace("\"name\":\"x-key\"", "\"unknown\":\"x-key\""))
        .unwrap_err();
    from_str::<AgentCard>(&schemes.replace("\"in_\"", "\"in\"")).unwrap_err();
    from_str::<AgentCard>(r#"{"protocolVersion":null}"#).unwrap_err();
    from_str::<AgentCard>(r#"{"provider":{"organization":null}}"#).unwrap_err();
}

#[test]
fn registry_non_nullable_fields_reject_null() {
    from_str::<PatchAgentRequest>(r#"{"agent_name":null}"#).unwrap_err();
    from_str::<PatchAgentRequest>(r#"{"enabled":null}"#).unwrap_err();
    from_str::<AgentCard>(r#"{"skills":null}"#).unwrap_err();
}

#[test]
fn response_decimal_and_typed_card_round_trip() {
    let input = r#"{"agent_id":"a","agent_name":"worker","agent_card_params":{},"spend":1.25,"search_score":0.5}"#;
    let response: AgentResponse = from_str(input).unwrap();
    assert_eq!(response.spend, Some(Decimal::new(125, 2)));
    assert_eq!(response.search_score, Some(Decimal::new(5, 1)));
    assert!(to_string(&response).unwrap().contains(r#""spend":1.25"#));
    from_str::<AgentResponse>(
        r#"{"agent_id":"a","agent_name":"worker","agent_card_params":{"custom":true}}"#,
    )
    .unwrap_err();
}

#[test]
fn webhook_defaults_and_url_validation() {
    let config: AgentKillSwitchConfig = from_str(r#"{"url":"http://agent.example/stop"}"#).unwrap();
    assert_eq!(config.method, super::AgentKillSwitchMethod::Post);
    from_str::<AgentKillSwitchConfig>(r#"{"url":"HTTPS://agent.example"}"#).unwrap();
    let result: AgentKillSwitchResult = from_str(
        r#"{"agent_id":"agent","url":"https://agent.example","method":"POST","status_code":204}"#,
    )
    .unwrap();
    assert!(result.succeeded());
    let failed: AgentKillSwitchResult = from_str(
        r#"{"agent_id":"agent","url":"https://agent.example","method":"POST","status_code":300}"#,
    )
    .unwrap();
    assert!(!failed.succeeded());
}

#[test]
fn public_card_and_permission_round_trip() {
    let augmented: AugmentedAgentCard = from_str(r#"{"is_public":true}"#).unwrap();
    assert!(augmented.is_public);
    assert_eq!(to_string(&augmented).unwrap(), r#"{"is_public":true}"#);
    let permission: AgentObjectPermission =
        from_str(r#"{"mcp_servers":["server"],"mcp_access_groups":null}"#).unwrap();
    assert_eq!(
        to_string(&permission).unwrap(),
        r#"{"mcp_servers":["server"],"mcp_access_groups":null}"#
    );
}

#[rstest::rstest]
#[case::relative("/stop")]
#[case::ftp("ftp://agent.example/stop")]
#[case::mailto("mailto:ops@example.com")]
fn webhook_rejects_invalid_urls(#[case] url: &str) {
    let error = from_str::<AgentKillSwitchConfig>(&format!(r#"{{"url":"{url}"}}"#)).unwrap_err();
    assert!(error.to_string().contains("absolute http(s)"));
}

#[test]
fn extended_card_flag_preserves_wire_name_and_null() {
    let input = r#"{"supportsAuthenticatedExtendedCard":null}"#;
    let card: AgentCard = from_str(input).unwrap();
    assert_eq!(card.supports_authenticated_card, Some(None));
    assert_eq!(to_string(&card).unwrap(), input);
}

#[test]
fn api_key_auth_defaults_header_and_hides_secret() {
    let auth: super::AgentKillSwitchAuth =
        from_str(r#"{"type":"api_key","api_key":"secret"}"#).unwrap();
    assert_eq!(
        to_string(&auth).unwrap(),
        r#"{"type":"api_key","header_name":"x-api-key"}"#
    );
    assert!(!format!("{auth:?}").contains("secret"));
}

#[test]
fn config_requires_name_but_patch_can_omit_it() {
    from_str::<super::AgentConfig>("{}").unwrap_err();
    from_str::<super::AgentConfig>(r#"{"agent_name":null}"#).unwrap_err();
    let input = r#"{"agent_name":"worker"}"#;
    let config: super::AgentConfig = from_str(input).unwrap();
    assert_eq!(to_string(&config).unwrap(), input);
    let patch: PatchAgentRequest = from_str("{}").unwrap();
    assert_eq!(to_string(&patch).unwrap(), "{}");
}
