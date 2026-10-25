use super::{
    json::{AgentJson, SecurityRequirements},
    validation,
};
use serde::{Deserialize, Serialize};
use serde_with::rust::double_option;
use std::collections::BTreeMap;

macro_rules! card_record {
    ($name:ident { required: { $($required:ident: $required_ty:ty),* $(,)? } nullable: { $($(#[$attribute:meta])* $nullable:ident: $nullable_ty:ty),* $(,)? } }) => {
        #[derive(Clone, Debug, Deserialize, Serialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub struct $name {
            $(#[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "validation::non_null_option")] pub $required: Option<$required_ty>,)*
            $($(#[$attribute])* #[serde(default, skip_serializing_if = "Option::is_none", with = "double_option")] pub $nullable: Option<Option<$nullable_ty>>,)*
        }
    };
}
card_record! { AgentProvider { required: { organization: String, url: String } nullable: {} } }
card_record! { AgentExtension { required: { uri: String } nullable: { description: String, required: bool, params: BTreeMap<String, AgentJson> } } }
card_record! { AgentCapabilities { required: {} nullable: { streaming: bool, push_notifications: bool, state_transition_history: bool, extensions: Vec<AgentExtension> } } }
card_record! { AgentSkill { required: { id: String, name: String, description: String, tags: Vec<String> } nullable: { examples: Vec<String>, input_modes: Vec<String>, output_modes: Vec<String>, security: SecurityRequirements } } }
card_record! { AgentInterface { required: { url: String, transport: String } nullable: {} } }
card_record! { AgentCardSignature { required: { protected: String, signature: String } nullable: { header: BTreeMap<String, AgentJson> } } }

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum SecurityScheme {
    #[serde(rename = "apiKey")]
    ApiKey(SecurityApiKey),
    #[serde(rename = "http")]
    Http(SecurityHttp),
    #[serde(rename = "mutualTLS")]
    MutualTls(SecurityMutualTls),
    #[serde(rename = "oauth2")]
    Oauth2(SecurityOauth2),
    #[serde(rename = "openIdConnect")]
    OpenIdConnect(SecurityOpenIdConnect),
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecurityApiKey {
    #[serde(
        rename = "in",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "validation::non_null_option"
    )]
    pub location: Option<SecurityLocation>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "validation::non_null_option"
    )]
    pub name: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub description: Option<Option<String>>,
}
card_record! { SecurityHttp { required: { scheme: String } nullable: { bearer_format: String, description: String } } }
card_record! { SecurityMutualTls { required: {} nullable: { description: String } } }
card_record! { OAuthFlows { required: {} nullable: { authorization_code: BTreeMap<String, AgentJson>, client_credentials: BTreeMap<String, AgentJson>, implicit: BTreeMap<String, AgentJson>, password: BTreeMap<String, AgentJson> } } }
card_record! { SecurityOauth2 { required: { flows: OAuthFlows } nullable: { oauth2_metadata_url: String, description: String } } }
card_record! { SecurityOpenIdConnect { required: { open_id_connect_url: String } nullable: { description: String } } }

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SecurityLocation {
    Query,
    Header,
    Cookie,
}

card_record! { AgentCard {
    required: {
        protocol_version: String,
        name: String,
        description: String,
        url: String,
        version: String,
        capabilities: AgentCapabilities,
        default_input_modes: Vec<String>,
        default_output_modes: Vec<String>,
        skills: Vec<AgentSkill>,
    }
    nullable: {
        preferred_transport: String,
        additional_interfaces: Vec<AgentInterface>,
        icon_url: String,
        provider: AgentProvider,
        documentation_url: String,
        security_schemes: BTreeMap<String, SecurityScheme>,
        security: SecurityRequirements,
        #[serde(rename = "supportsAuthenticatedExtendedCard")]
        supports_authenticated_card: bool,
        signatures: Vec<AgentCardSignature>,
    }
} }
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AugmentedAgentCard {
    #[serde(flatten)]
    pub card: AgentCard,
    pub is_public: bool,
}
