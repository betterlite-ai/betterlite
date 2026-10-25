#![cfg(test)]
use super::{
    AgentCard, AgentKillSwitchConfig, AgentKillSwitchResult, AgentObjectPermission, AgentResponse,
    AugmentedAgentCard,
};
use rust_decimal::Decimal;
#[test]
fn agent_records_validate_and_round_trip() {
    let card: AgentCard = serde_json::from_str(r#"{"protocolVersion":"1.0","name":"worker","description":"does work","url":"https://agent.example","version":"1","capabilities":{"streaming":null},"defaultInputModes":["text"],"defaultOutputModes":["text"],"skills":[],"iconUrl":null}"#).unwrap();
    let input = r#"{"protocolVersion":"1.0","name":"worker","description":"does work","url":"https://agent.example","version":"1","capabilities":{"streaming":null},"defaultInputModes":["text"],"defaultOutputModes":["text"],"skills":[],"iconUrl":null}"#;
    assert_eq!(serde_json::to_string(&card).unwrap(), input);
    let permission: AgentObjectPermission =
        serde_json::from_str(r#"{"mcp_servers":["server"],"mcp_access_groups":null}"#).unwrap();
    assert_eq!(permission.mcp_servers, Some(vec![String::from("server")]));
    assert_eq!(permission.mcp_access_groups, None);
    let augmented: AugmentedAgentCard = serde_json::from_str(r#"{"is_public":true}"#).unwrap();
    assert!(augmented.is_public);
    assert_eq!(
        serde_json::to_string(&augmented).unwrap(),
        r#"{"is_public":true}"#
    );
    serde_json::from_str::<AgentCard>(r#"{"unknown":true}"#).unwrap_err();
    let invalid = serde_json::from_str::<AgentKillSwitchConfig>(r#"{"url":"/stop"}"#).unwrap_err();
    assert!(invalid.to_string().contains("absolute http(s)"));
    let config: AgentKillSwitchConfig =
        serde_json::from_str(r#"{"url":"https://agent.example/stop"}"#).unwrap();
    let result = AgentKillSwitchResult {
        agent_id: "agent".to_owned(),
        url: config.url,
        method: config.method,
        status_code: Some(204),
        response_body: None,
        error: None,
    };
    assert!(result.succeeded());
    let response: AgentResponse = serde_json::from_str(r#"{"agent_id":"a","agent_name":"worker","agent_card_params":{},"spend":"1.25","search_score":"0.5"}"#).unwrap();
    assert_eq!(response.spend, Some(Decimal::new(125, 2)));
    assert_eq!(response.search_score, Some(Decimal::new(5, 1)));
    serde_json::from_str::<AgentResponse>(r#"{"agent_name":"worker","agent_card_params":{}}"#)
        .unwrap_err();
}
