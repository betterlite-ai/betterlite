#![cfg(test)]

use betterlite_config::sections::cisco_verdict_contracts::Rule;

use crate::cisco_configuration::payload::{
    ChatPayloadInputs, RequestMetadata, WireRule, build_chat_payload, normalize_rule,
};
use crate::cisco_conversation::{InspectMessage, Role};

fn full_inputs() -> ChatPayloadInputs {
    ChatPayloadInputs {
        messages: vec![InspectMessage {
            role: Role::User,
            content: "hello".to_owned(),
        }],
        request_user: Some("alice".to_owned()),
        key_user_id: Some("key-user".to_owned()),
        litellm_call_id: Some("call-1".to_owned()),
        request_metadata: RequestMetadata {
            src_app: Some("gateway".to_owned()),
            dst_app: Some("dst-app".to_owned()),
            src_ip: Some("src-ip".to_owned()),
            dst_ip: Some("dst-ip".to_owned()),
            dst_host: Some("dst-host".to_owned()),
            sni: Some("sni".to_owned()),
            user_agent: Some("user-agent".to_owned()),
        },
        enabled_rules: vec![WireRule {
            rule_name: Some("pii".to_owned()),
            entity_types: None,
            rule_id: None,
            classification: None,
        }],
        integration_profile_id: Some("profile-1".to_owned()),
        integration_profile_version: None,
        integration_tenant_id: None,
        integration_type: None,
    }
}

#[test]
fn build_chat_payload_serializes_every_metadata_key() {
    let payload = build_chat_payload(full_inputs());
    let serialized = serde_json::to_value(&payload).expect("payload serializes");
    assert_eq!(
        serialized["metadata"],
        serde_json::json!({
            "user":"alice", "client_transaction_id":"call-1", "src_app":"gateway",
            "dst_app":"dst-app", "src_ip":"src-ip", "dst_ip":"dst-ip", "dst_host":"dst-host",
            "sni":"sni", "user_agent":"user-agent"
        })
    );
}

#[test]
fn build_chat_payload_serializes_the_full_config() {
    let mut inputs = full_inputs();
    inputs.enabled_rules[0].rule_id = Some("rule-id".to_owned());
    inputs.integration_profile_version = Some("v2".to_owned());
    inputs.integration_tenant_id = Some("tenant".to_owned());
    inputs.integration_type = Some("type".to_owned());
    let payload = build_chat_payload(inputs);
    let serialized = serde_json::to_value(&payload).expect("payload serializes");
    assert_eq!(
        serialized["config"],
        serde_json::json!({
            "enabled_rules":[{"rule_name":"pii","rule_id":"rule-id"}],
            "integration_profile_id":"profile-1", "integration_profile_version":"v2",
            "integration_tenant_id":"tenant", "integration_type":"type"
        })
    );
}

#[test]
fn build_chat_payload_serializes_empty_inputs() {
    let payload = build_chat_payload(ChatPayloadInputs::default());
    assert_eq!(
        serde_json::to_value(payload).expect("payload serializes"),
        serde_json::json!({"messages":[],"metadata":{},"config":{}})
    );
}

#[test]
fn build_chat_payload_user_falls_back_to_the_api_key_user_id() {
    let inputs = ChatPayloadInputs {
        messages: Vec::new(),
        request_user: None,
        key_user_id: Some("key-user".to_owned()),
        litellm_call_id: None,
        request_metadata: RequestMetadata::default(),
        enabled_rules: Vec::new(),
        integration_profile_id: None,
        integration_profile_version: None,
        integration_tenant_id: None,
        integration_type: None,
    };
    let payload = build_chat_payload(inputs);
    assert_eq!(payload.metadata.user, Some("key-user".to_owned()));
}

#[test]
fn build_chat_payload_omits_metadata_fields_that_are_absent() {
    let payload = build_chat_payload(ChatPayloadInputs::default());
    assert_eq!(payload.metadata.user, None);
    assert_eq!(payload.metadata.client_transaction_id, None);
    assert_eq!(payload.metadata.src_app, None);
    assert!(payload.config.enabled_rules.is_empty());
    assert_eq!(payload.config.integration_profile_id, None);
    let serialized = serde_json::to_string(&payload.metadata).expect("metadata serializes");
    assert_eq!(serialized, "{}");
}

#[test]
fn normalize_rule_wraps_a_bare_rule_name_with_no_entity_types() {
    let rule = Rule {
        rule_name: Some("pii-detector".to_owned()),
        entity_types: Vec::new(),
        rule_id: None,
        classification: None,
    };
    assert_eq!(
        normalize_rule(&rule),
        WireRule {
            rule_name: Some("pii-detector".to_owned()),
            entity_types: None,
            rule_id: None,
            classification: None,
        }
    );
}

#[test]
fn normalize_rule_passes_through_a_fully_populated_rule() {
    let rule = Rule {
        rule_name: Some("PII".to_owned()),
        entity_types: vec!["EMAIL".to_owned()],
        rule_id: Some("rule-1".to_owned()),
        classification: Some("SEVERE".to_owned()),
    };
    assert_eq!(
        normalize_rule(&rule),
        WireRule {
            rule_name: Some("PII".to_owned()),
            entity_types: Some(vec!["EMAIL".to_owned()]),
            rule_id: Some("rule-1".to_owned()),
            classification: Some("SEVERE".to_owned()),
        }
    );
}

#[test]
fn normalize_rule_omits_an_absent_rule_name() {
    let rule = Rule {
        rule_name: None,
        entity_types: Vec::new(),
        rule_id: Some("r1".to_owned()),
        classification: None,
    };
    let serialized = serde_json::to_value(normalize_rule(&rule)).expect("rule serializes");
    assert_eq!(serialized, serde_json::json!({"rule_id":"r1"}));
}
