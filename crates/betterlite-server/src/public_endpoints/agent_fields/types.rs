use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::public_endpoints::providers::types::CredentialFieldType;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AgentCredentialField {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub placeholder: Option<String>,
    #[serde(default)]
    pub tooltip: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub field_type: CredentialFieldType,
    #[serde(default)]
    pub options: Option<Vec<String>>,
    #[serde(default)]
    pub default_value: Option<String>,
    #[serde(default)]
    pub include_in_litellm_params: Option<bool>,
    #[serde(default)]
    pub validation_pattern: Option<String>,
    #[serde(default)]
    pub validation_message: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct AgentCreateInfo {
    pub agent_type: String,
    pub agent_type_display_name: String,
    pub description: Option<String>,
    pub logo_url: Option<String>,
    pub credential_fields: Vec<AgentCredentialField>,
    pub litellm_params_template: Option<BTreeMap<String, String>>,
    pub model_template: Option<String>,
}

// catalog::agent_fields drops inherit_from_provider and use_a2a_form_fields before responding.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RawAgentCreateInfo {
    pub agent_type: String,
    pub agent_type_display_name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub logo_url: Option<String>,
    #[serde(default, rename = "inherit_credentials_from_provider")]
    pub inherit_from_provider: Option<String>,
    #[serde(default)]
    pub credential_fields: Vec<AgentCredentialField>,
    #[serde(default)]
    pub litellm_params_template: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub model_template: Option<String>,
    #[serde(default)]
    pub use_a2a_form_fields: Option<bool>,
}
