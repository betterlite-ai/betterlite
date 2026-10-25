#![cfg(test)]
use super::{
    AgentCard, AgentKillSwitchConfig, AgentKillSwitchResult, AgentObjectPermission, AgentResponse,
    AugmentedAgentCard,
};
use rust_decimal::Decimal;
use serde_json::{from_str, to_string};
#[test]
fn agent_records_validate_and_round_trip() {
    let card: AgentCard = from_str(r#"{"protocolVersion":"1.0","name":"worker","description":"does work","url":"https://agent.example","version":"1","capabilities":{"streaming":null},"defaultInputModes":["text"],"defaultOutputModes":["text"],"skills":[],"iconUrl":null}"#).unwrap();
    let input = r#"{"protocolVersion":"1.0","name":"worker","description":"does work","url":"https://agent.example","version":"1","capabilities":{"streaming":null},"defaultInputModes":["text"],"defaultOutputModes":["text"],"skills":[],"iconUrl":null}"#;
    assert_eq!(to_string(&card).unwrap(), input);
    let permission: AgentObjectPermission =
        from_str(r#"{"mcp_servers":["server"],"mcp_access_groups":null}"#).unwrap();
    assert_eq!(
        to_string(&permission).unwrap(),
        r#"{"mcp_servers":["server"],"mcp_access_groups":null}"#
    );
    let augmented: AugmentedAgentCard = from_str(r#"{"is_public":true}"#).unwrap();
    assert!(augmented.is_public);
    assert_eq!(to_string(&augmented).unwrap(), r#"{"is_public":true}"#);
    from_str::<AgentCard>(r#"{"unknown":true}"#).unwrap_err();
    let invalid = from_str::<AgentKillSwitchConfig>(r#"{"url":"/stop"}"#).unwrap_err();
    assert!(invalid.to_string().contains("absolute http(s)"));
    let config: AgentKillSwitchConfig =
        from_str(r#"{"url":"https://agent.example/stop"}"#).unwrap();
    let result: AgentKillSwitchResult = from_str(r#"{"agent_id":"agent","url":"https://agent.example/stop","method":"POST","status_code":204}"#).unwrap();
    assert_eq!(config.method, result.method);
    assert_eq!(config.url, result.url);
    assert!(result.succeeded());
    let failed: AgentKillSwitchResult = from_str(
        r#"{"agent_id":"agent","url":"https://agent.example","method":"POST","status_code":300}"#,
    )
    .unwrap();
    assert!(!failed.succeeded());
    let public = r#"{"message":"updated","public_agent_groups":["a"],"updated_by":"admin"}"#;
    let published: super::AgentMakePublicResponse = from_str(public).unwrap();
    assert_eq!(to_string(&published).unwrap(), public);
    let response: AgentResponse = from_str(r#"{"agent_id":"a","agent_name":"worker","agent_card_params":{},"spend":"1.25","search_score":"0.5"}"#).unwrap();
    assert_eq!(response.spend, Some(Decimal::new(125, 2)));
    assert_eq!(response.search_score, Some(Decimal::new(5, 1)));
    from_str::<AgentResponse>(r#"{"agent_name":"worker","agent_card_params":{}}"#).unwrap_err();
}

#[test]
fn nested_and_augmented_cards_preserve_nulls() {
    let input = r#"{"is_public":true,"capabilities":{"pushNotifications":null,"extensions":[{"description":null}]},"skills":[{"examples":null}]}"#;
    let card: AugmentedAgentCard = from_str(input).unwrap();
    assert_eq!(to_string(&card).unwrap(), input);
    let patch: super::PatchAgentRequest = from_str(r#"{"kill_switch":null}"#).unwrap();
    assert_eq!(to_string(&patch).unwrap(), r#"{"kill_switch":null}"#);
}

#[test]
fn webhook_defaults_and_passthrough_values_are_wire_compatible() {
    let config: AgentKillSwitchConfig = from_str(
        r#"{"url":"http://agent.example/stop","auth":{"type":"api_key","api_key":"secret"}}"#,
    )
    .unwrap();
    assert_eq!(config.method, super::AgentKillSwitchMethod::Post);
    assert!(config.headers.is_empty());
    let serialized = to_string(&config).unwrap();
    assert!(serialized.contains(r#""header_name":"x-api-key""#));
    assert!(!serialized.contains("secret"));

    let response: AgentResponse = from_str(
        r#"{"agent_id":"a","agent_name":"worker","agent_card_params":{"custom":{"enabled":true}}}"#,
    )
    .unwrap();
    assert_eq!(response.agent_card_params["custom"]["enabled"], true);
}
