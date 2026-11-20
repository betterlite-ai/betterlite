#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_core::agents::kill_switch::AgentKillSwitchConfig;
use betterlite_core::agents::{AgentKeySummary, AgentResponse};
use rstest::rstest;

use super::{JwtAgentSettings, redact_sensitive_agent_fields};
use crate::agent_credentials::REDACTED_BY_LITELM;
use crate::agent_credentials::kill_switch::serialize_agent_kill_switch;
use crate::agent_credentials::support::{
    agent, agent_with_identity, api_key_auth, basic_auth, bearer, json, masker, switch,
};

const fn jwt_off() -> JwtAgentSettings {
    JwtAgentSettings {
        enabled: false,
        agent_id_field_set: false,
    }
}

fn with_managed_fields() -> AgentResponse {
    let mut source = agent_with_identity("agent-1");
    source.static_headers = Some(BTreeMap::from([(
        String::from("x-static"),
        String::from("on"),
    )]));
    source.extra_headers = Some(vec![String::from("x-extra")]);
    source.keys = Some(vec![AgentKeySummary {
        token: String::from("hashed"),
        key_alias: None,
        key_name: None,
    }]);
    source.kill_switch = Some(switch(Some(bearer("sk-live"))));
    source
}

#[test]
fn litellm_params_are_redacted_for_an_admin() {
    let mut source = agent("agent-1");
    source.litellm_params = Some(BTreeMap::from([
        (String::from("api_key"), json(r#""sk-live""#)),
        (String::from("model"), json(r#""gpt""#)),
    ]));
    let sources = [source];
    let responses = redact_sensitive_agent_fields(&sources, true, jwt_off(), &masker()).unwrap();
    assert_eq!(
        responses[0]["litellm_params"],
        json(r#"{"api_key":"REDACTED_BY_LITELM","model":"gpt"}"#)
    );
}

#[test]
fn litellm_params_are_redacted_for_a_non_admin() {
    let mut source = agent("agent-1");
    source.litellm_params = Some(BTreeMap::from([(
        String::from("access_token"),
        json(r#""token-live""#),
    )]));
    let sources = [source];
    let responses = redact_sensitive_agent_fields(&sources, false, jwt_off(), &masker()).unwrap();
    assert_eq!(
        responses[0]["litellm_params"],
        json(r#"{"access_token":"REDACTED_BY_LITELM"}"#)
    );
}

#[test]
fn an_empty_or_absent_params_map_is_left_alone() {
    let mut empty = agent("agent-1");
    empty.litellm_params = Some(BTreeMap::new());
    let sources = [empty, agent("agent-2")];
    let responses = redact_sensitive_agent_fields(&sources, true, jwt_off(), &masker()).unwrap();
    assert_eq!(responses[0]["litellm_params"], json("{}"));
    assert!(responses[1]["litellm_params"].is_null());
}

#[test]
fn a_non_admin_loses_the_managed_fields() {
    let sources = [with_managed_fields()];
    let responses = redact_sensitive_agent_fields(&sources, false, jwt_off(), &masker()).unwrap();
    assert!(responses[0]["static_headers"].is_null());
    assert!(responses[0]["extra_headers"].is_null());
    assert!(responses[0]["keys"].is_null());
    assert!(responses[0]["kill_switch"].is_null());
    assert!(responses[0]["identity"].is_null());
}

#[test]
fn an_admin_keeps_the_managed_fields() {
    let sources = [with_managed_fields()];
    let responses = redact_sensitive_agent_fields(&sources, true, jwt_off(), &masker()).unwrap();
    assert_eq!(responses[0]["static_headers"], json(r#"{"x-static":"on"}"#));
    assert_eq!(responses[0]["extra_headers"], json(r#"["x-extra"]"#));
    assert_eq!(responses[0]["keys"][0]["token"], json(r#""hashed""#));
    assert!(!responses[0]["identity"].is_null());
    assert_eq!(
        responses[0]["kill_switch"]["auth"]["token"].as_str(),
        Some(REDACTED_BY_LITELM)
    );
}

#[rstest]
#[case::bearer(switch(Some(bearer("sk-live"))), "token")]
#[case::api_key(switch(Some(api_key_auth("x-custom-key", "key-live"))), "api_key")]
#[case::basic(switch(Some(basic_auth("operator", "hunter2"))), "password")]
fn the_response_marks_the_kill_switch_secret(
    #[case] config: AgentKillSwitchConfig,
    #[case] field: &str,
) {
    let mut source = agent("agent-1");
    source.kill_switch = Some(config);
    let sources = [source];
    let responses = redact_sensitive_agent_fields(&sources, true, jwt_off(), &masker()).unwrap();
    assert_eq!(
        responses[0]["kill_switch"]["auth"][field].as_str(),
        Some(REDACTED_BY_LITELM)
    );
    assert_eq!(
        responses[0]["kill_switch"]["url"].as_str(),
        Some("https://agent.example/stop")
    );
}

#[test]
fn jwt_is_configured_for_a_stored_identity() {
    let sources = [agent_with_identity("agent-1")];
    let settings = JwtAgentSettings {
        enabled: true,
        agent_id_field_set: false,
    };
    let responses = redact_sensitive_agent_fields(&sources, true, settings, &masker()).unwrap();
    assert_eq!(responses[0]["jwt_auth_configured"], json("true"));
}

#[test]
fn jwt_is_configured_for_the_agent_id_field() {
    let sources = [agent("agent-1")];
    let settings = JwtAgentSettings {
        enabled: true,
        agent_id_field_set: true,
    };
    let responses = redact_sensitive_agent_fields(&sources, true, settings, &masker()).unwrap();
    assert_eq!(responses[0]["jwt_auth_configured"], json("true"));
}

#[test]
fn jwt_is_not_configured_without_an_identity_or_the_field() {
    let sources = [agent("agent-1")];
    let settings = JwtAgentSettings {
        enabled: true,
        agent_id_field_set: false,
    };
    let responses = redact_sensitive_agent_fields(&sources, true, settings, &masker()).unwrap();
    assert_eq!(responses[0]["jwt_auth_configured"], json("false"));
}

#[test]
fn jwt_is_not_configured_with_the_auth_switched_off() {
    let sources = [agent_with_identity("agent-1")];
    let settings = JwtAgentSettings {
        enabled: false,
        agent_id_field_set: true,
    };
    let responses = redact_sensitive_agent_fields(&sources, true, settings, &masker()).unwrap();
    assert_eq!(responses[0]["jwt_auth_configured"], json("false"));
}

#[test]
fn a_non_admin_copy_keeps_the_jwt_flag_after_its_identity_is_stripped() {
    let sources = [agent_with_identity("agent-1")];
    let settings = JwtAgentSettings {
        enabled: true,
        agent_id_field_set: false,
    };
    let responses = redact_sensitive_agent_fields(&sources, false, settings, &masker()).unwrap();
    assert_eq!(responses[0]["jwt_auth_configured"], json("true"));
    assert!(responses[0]["identity"].is_null());
}

#[test]
fn responses_are_copies_in_order_and_leave_the_input_unchanged() {
    let mut first = agent("agent-1");
    first.litellm_params = Some(BTreeMap::from([(
        String::from("api_key"),
        json(r#""sk-live""#),
    )]));
    let second = agent("agent-2");
    let before = serde_json::to_value(&first).unwrap();
    let sources = [first, second];
    let responses = redact_sensitive_agent_fields(&sources, true, jwt_off(), &masker()).unwrap();
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["agent_id"].as_str(), Some("agent-1"));
    assert_eq!(responses[1]["agent_id"].as_str(), Some("agent-2"));
    assert_eq!(serde_json::to_value(&sources[0]).unwrap(), before);
}

#[test]
fn response_marker_round_trips_to_stored_secret() {
    let config = switch(Some(bearer("sk-live")));
    let mut source = agent("agent-1");
    source.kill_switch = Some(config.clone());
    let sources = [source];
    let responses = redact_sensitive_agent_fields(&sources, true, jwt_off(), &masker()).unwrap();
    let echoed = &responses[0]["kill_switch"];
    assert_eq!(echoed["auth"]["token"].as_str(), Some(REDACTED_BY_LITELM));
    let stored = json(&serialize_agent_kill_switch(Some(&config), None).unwrap());
    let incoming: AgentKillSwitchConfig = serde_json::from_value(echoed.clone()).unwrap();
    let resaved = serialize_agent_kill_switch(Some(&incoming), Some(&stored)).unwrap();
    assert_eq!(json(&resaved)["auth"]["token"].as_str(), Some("sk-live"));
}
