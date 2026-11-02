use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CredentialFieldType {
    #[default]
    Text,
    Password,
    Select,
    Upload,
    Textarea,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProviderCredentialField {
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
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProviderCreateInfo {
    pub provider: String,
    pub provider_display_name: String,
    pub litellm_provider: String,
    pub credential_fields: Vec<ProviderCredentialField>,
    #[serde(default)]
    pub default_model_placeholder: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(super) struct ProviderSupportEntry {
    #[serde(default)]
    pub(super) display_name: Option<String>,
    #[serde(default)]
    pub(super) endpoints: IndexMap<String, bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(super) struct ProvidersFile {
    #[serde(default)]
    pub(super) providers: IndexMap<String, ProviderSupportEntry>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct EndpointProvider {
    pub slug: String,
    pub display_name: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct SupportedEndpoint {
    pub key: String,
    pub label: String,
    pub endpoint: String,
    pub providers: Vec<EndpointProvider>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct SupportedEndpointsResponse {
    pub endpoints: Vec<SupportedEndpoint>,
}
