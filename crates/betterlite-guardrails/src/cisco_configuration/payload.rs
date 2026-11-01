use betterlite_config::sections::cisco_verdict_contracts::Rule;
use serde::Serialize;

use crate::cisco_conversation::InspectMessage;

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct Metadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_transaction_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub src_app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dst_app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub src_ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dst_ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dst_host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sni: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestMetadata {
    pub src_app: Option<String>,
    pub dst_app: Option<String>,
    pub src_ip: Option<String>,
    pub dst_ip: Option<String>,
    pub dst_host: Option<String>,
    pub sni: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct WireRule {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_types: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct ScanConfig {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub enabled_rules: Vec<WireRule>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_profile_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_tenant_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_type: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatPayload {
    pub messages: Vec<InspectMessage>,
    pub metadata: Metadata,
    pub config: ScanConfig,
}

#[derive(Debug, Clone, Default)]
pub struct ChatPayloadInputs {
    pub messages: Vec<InspectMessage>,
    pub request_user: Option<String>,
    pub key_user_id: Option<String>,
    pub litellm_call_id: Option<String>,
    pub request_metadata: RequestMetadata,
    pub enabled_rules: Vec<WireRule>,
    pub integration_profile_id: Option<String>,
    pub integration_profile_version: Option<String>,
    pub integration_tenant_id: Option<String>,
    pub integration_type: Option<String>,
}

#[must_use]
pub fn build_chat_payload(inputs: ChatPayloadInputs) -> ChatPayload {
    let metadata = Metadata {
        user: inputs.request_user.or(inputs.key_user_id),
        client_transaction_id: inputs.litellm_call_id,
        src_app: inputs.request_metadata.src_app,
        dst_app: inputs.request_metadata.dst_app,
        src_ip: inputs.request_metadata.src_ip,
        dst_ip: inputs.request_metadata.dst_ip,
        dst_host: inputs.request_metadata.dst_host,
        sni: inputs.request_metadata.sni,
        user_agent: inputs.request_metadata.user_agent,
    };
    let config = ScanConfig {
        enabled_rules: inputs.enabled_rules,
        integration_profile_id: inputs.integration_profile_id,
        integration_profile_version: inputs.integration_profile_version,
        integration_tenant_id: inputs.integration_tenant_id,
        integration_type: inputs.integration_type,
    };
    ChatPayload {
        messages: inputs.messages,
        metadata,
        config,
    }
}

#[must_use]
pub fn normalize_rule(rule: &Rule) -> WireRule {
    let entity_types = (!rule.entity_types.is_empty()).then(|| rule.entity_types.clone());
    WireRule {
        rule_name: Some(rule.rule_name.clone()),
        entity_types,
        rule_id: rule.rule_id.clone(),
        classification: rule.classification.clone(),
    }
}

mod tests;
