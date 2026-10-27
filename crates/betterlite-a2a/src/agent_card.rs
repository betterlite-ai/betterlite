use betterlite_core::agents::card::{SecurityHttp, SecurityScheme};
use betterlite_core::agents::json::{AgentJson, SecurityRequirements};
use serde::{Deserialize, Serialize};
use serde_with::rust::double_option;
use std::collections::BTreeMap;

pub mod merge;
pub mod version_parser;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum WireValue<T> {
    Typed(T),
    Passthrough(AgentJson),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProtocolVersion {
    #[serde(rename = "0.3")]
    V0_3,
    #[serde(rename = "1.0")]
    V1_0,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentCard {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub protocol_version: Option<Option<WireValue<ProtocolVersion>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub name: Option<Option<WireValue<String>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub description: Option<Option<WireValue<String>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub version: Option<Option<WireValue<String>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub capabilities: Option<Option<WireValue<Capabilities>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub default_input_modes: Option<Option<WireValue<Vec<String>>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub default_output_modes: Option<Option<WireValue<Vec<String>>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub skills: Option<Option<WireValue<Vec<Skill>>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub preferred_transport: Option<Option<WireValue<String>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub supported_interfaces: Option<Option<WireValue<Vec<Interface>>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub icon_url: Option<Option<WireValue<String>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub provider: Option<Option<WireValue<Provider>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub documentation_url: Option<Option<WireValue<String>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub security_schemes: Option<Option<WireValue<BTreeMap<String, SecurityScheme>>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub security: Option<Option<WireValue<SecurityRequirements>>>,
    #[serde(rename = "supportsAuthenticatedExtendedCard")]
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub supports_auth_extended_card: Option<Option<WireValue<bool>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub signatures: Option<Option<WireValue<Vec<Signature>>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub url: Option<Option<WireValue<String>>>,
    #[serde(flatten)]
    pub passthrough: BTreeMap<String, AgentJson>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Capabilities {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub streaming: Option<Option<WireValue<bool>>>,
    #[serde(flatten)]
    pub passthrough: BTreeMap<String, AgentJson>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Skill {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub id: Option<Option<WireValue<String>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub name: Option<Option<WireValue<String>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub description: Option<Option<WireValue<String>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub tags: Option<Option<WireValue<Vec<String>>>>,
    #[serde(flatten)]
    pub passthrough: BTreeMap<String, AgentJson>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Interface {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub url: Option<Option<WireValue<String>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub protocol_binding: Option<Option<WireValue<String>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub protocol_version: Option<Option<WireValue<ProtocolVersion>>>,
    #[serde(flatten)]
    pub passthrough: BTreeMap<String, AgentJson>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Provider {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub organization: Option<Option<WireValue<String>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub url: Option<Option<WireValue<String>>>,
    #[serde(flatten)]
    pub passthrough: BTreeMap<String, AgentJson>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Signature {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub protected: Option<Option<WireValue<String>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub signature: Option<Option<WireValue<String>>>,
    #[serde(flatten)]
    pub passthrough: BTreeMap<String, AgentJson>,
}

mod tests;
