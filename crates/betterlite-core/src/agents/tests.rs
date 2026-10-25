#![cfg(test)]
use super::{AgentCard, AgentKillSwitchConfig, AgentKillSwitchResult, AgentResponse};
#[test]
fn agent_records_validate_and_round_trip() {
    let card: AgentCard = serde_json::from_str(r#"{"protocolVersion":"1.0","name":"worker","description":"does work","url":"https://agent.example","version":"1","capabilities":{"streaming":null},"defaultInputModes":["text"],"defaultOutputModes":["text"],"skills":[],"iconUrl":null}"#).unwrap();
    serde_json::to_string(&card).unwrap();
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
    drop(serde_json::from_str::<AgentResponse>(r#"{"agent_id":"a","agent_name":"worker","agent_card_params":{},"spend":"1.25","search_score":"0.5"}"#).unwrap());
}
