#![cfg(test)]

use crate::cisco_configuration::payload::{
    ChatPayloadInputs, RawRule, RequestMetadata, RuleEntry, WireRule, build_chat_payload,
    normalize_rule,
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
            dst_app: None,
            src_ip: None,
            dst_ip: None,
            dst_host: None,
            sni: None,
            user_agent: None,
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
fn build_chat_payload_assembles_messages_and_metadata() {
    let payload = build_chat_payload(full_inputs());
    assert_eq!(payload.messages.len(), 1);
    assert_eq!(payload.metadata.user, Some("alice".to_owned()));
    assert_eq!(
        payload.metadata.client_transaction_id,
        Some("call-1".to_owned())
    );
    assert_eq!(payload.metadata.src_app, Some("gateway".to_owned()));
    assert_eq!(payload.metadata.dst_app, None);
}

#[test]
fn build_chat_payload_assembles_config() {
    let payload = build_chat_payload(full_inputs());
    assert_eq!(payload.config.enabled_rules.len(), 1);
    assert_eq!(
        payload.config.integration_profile_id,
        Some("profile-1".to_owned())
    );
    assert_eq!(payload.config.integration_profile_version, None);
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
fn normalize_rule_wraps_a_string_rule_in_rule_name() {
    let rule = RawRule::Name("pii-detector".to_owned());
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
fn normalize_rule_passes_through_a_dict_shaped_rule() {
    let rule = RawRule::Entry(RuleEntry {
        rule_name: Some(String::new()),
        entity_types: Some(vec!["EMAIL".to_owned()]),
        rule_id: Some("rule-1".to_owned()),
        classification: Some("SEVERE".to_owned()),
    });
    assert_eq!(
        normalize_rule(&rule),
        WireRule {
            rule_name: Some(String::new()),
            entity_types: Some(vec!["EMAIL".to_owned()]),
            rule_id: Some("rule-1".to_owned()),
            classification: Some("SEVERE".to_owned()),
        }
    );
}

#[test]
fn normalize_rule_rejects_malformed_rule_input() {
    let error = serde_json::from_str::<RawRule>("42").expect_err("not a string or object");
    assert!(!error.to_string().is_empty());
}
