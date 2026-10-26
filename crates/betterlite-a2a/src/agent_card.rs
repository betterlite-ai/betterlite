use betterlite_core::agents::json::AgentJson;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const DEFAULT_VERSION: &str = "1.0";

pub mod version_parser;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum WireValue<T> {
    Typed(T),
    Passthrough(AgentJson),
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentCard {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<WireValue<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<WireValue<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<WireValue<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<WireValue<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<WireValue<Capabilities>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_input_modes: Option<WireValue<Vec<String>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_output_modes: Option<WireValue<Vec<String>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<WireValue<Vec<Skill>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_transport: Option<WireValue<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supported_interfaces: Option<WireValue<Vec<Interface>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<WireValue<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<WireValue<Provider>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation_url: Option<WireValue<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_schemes: Option<WireValue<BTreeMap<String, SecurityScheme>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security: Option<WireValue<SecurityRequirements>>,
    #[serde(rename = "supportsAuthenticatedExtendedCard")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_auth_extended_card: Option<WireValue<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signatures: Option<WireValue<Vec<Signature>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<WireValue<String>>,
    #[serde(flatten)]
    pub passthrough: BTreeMap<String, AgentJson>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Capabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub streaming: Option<WireValue<bool>>,
    #[serde(flatten)]
    pub passthrough: BTreeMap<String, AgentJson>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Skill {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<WireValue<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<WireValue<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<WireValue<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<WireValue<Vec<String>>>,
    #[serde(flatten)]
    pub passthrough: BTreeMap<String, AgentJson>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Interface {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<WireValue<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_binding: Option<WireValue<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<WireValue<String>>,
    #[serde(flatten)]
    pub passthrough: BTreeMap<String, AgentJson>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Provider {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization: Option<WireValue<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<WireValue<String>>,
    #[serde(flatten)]
    pub passthrough: BTreeMap<String, AgentJson>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SecurityScheme {
    Http { scheme: String, description: String },
    Passthrough(AgentJson),
}

type SecurityRequirements = Vec<BTreeMap<String, Vec<String>>>;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Signature {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protected: Option<WireValue<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<WireValue<String>>,
    #[serde(flatten)]
    pub passthrough: BTreeMap<String, AgentJson>,
}

#[must_use]
pub fn normalize_protocol_version(raw_version: &AgentJson) -> Option<&'static str> {
    let raw_text = raw_version.as_str()?;
    let version = raw_text.strip_suffix('\n').unwrap_or(raw_text);
    let mut components = version.splitn(3, '.');
    let major = components.next()?;
    let minor = components.next()?;
    if let Some(patch_and_suffix) = components.next() {
        let (patch, suffix) = patch_and_suffix
            .find(['-', '+'])
            .map_or((patch_and_suffix, ""), |index| {
                patch_and_suffix.split_at(index)
            });
        if patch.is_empty()
            || !patch.bytes().all(|byte| byte.is_ascii_digit())
            || !valid_suffix(suffix)
        {
            return None;
        }
    }
    match (major, minor) {
        ("0", "3") => Some("0.3"),
        ("1", "0") => Some(DEFAULT_VERSION),
        _ => None,
    }
}

#[must_use]
pub fn resolve_served_version(upstream_card: Option<&AgentCard>) -> &'static str {
    let Some(card) = upstream_card else {
        return DEFAULT_VERSION;
    };
    card.protocol_version
        .as_ref()
        .and_then(|value| match value {
            WireValue::Typed(text) => Some(text.as_str()),
            WireValue::Passthrough(_) => None,
        })
        .and_then(|version| normalize_protocol_version(&AgentJson::String(version.to_owned())))
        .unwrap_or(DEFAULT_VERSION)
}

#[must_use]
pub fn merge_agent_card(
    upstream_card: Option<&AgentCard>,
    proxy_url: &str,
    proxy_base_url: &str,
    name: Option<&str>,
    description: Option<&str>,
) -> AgentCard {
    let mut card = upstream_card.cloned().unwrap_or_default();
    let served_version = resolve_served_version(upstream_card);
    card.protocol_version = Some(WireValue::Typed(served_version.to_owned()));
    if let Some(value) = name.filter(|text| !text.is_empty()) {
        card.name = Some(WireValue::Typed(value.to_owned()));
    }
    if let Some(value) = description.filter(|text| !text.is_empty()) {
        card.description = Some(WireValue::Typed(value.to_owned()));
    }
    card.capabilities = Some(filtered_capabilities(card.capabilities.take()));
    fill_defaults(&mut card, proxy_base_url);
    insert_proxy_fields(&mut card, proxy_url, served_version);
    card.passthrough.clear();
    card
}

fn filtered_capabilities(value: Option<WireValue<Capabilities>>) -> WireValue<Capabilities> {
    let streaming = match value {
        Some(WireValue::Typed(capabilities)) => capabilities.streaming.and_then(truthy_wire),
        _ => None,
    };
    WireValue::Typed(Capabilities {
        streaming,
        passthrough: BTreeMap::new(),
    })
}

fn insert_proxy_fields(card: &mut AgentCard, proxy_url: &str, served_version: &str) {
    let interface = Interface {
        url: Some(WireValue::Typed(proxy_url.to_owned())),
        protocol_binding: Some(WireValue::Typed("JSONRPC".to_owned())),
        protocol_version: Some(WireValue::Typed(served_version.to_owned())),
        passthrough: BTreeMap::new(),
    };
    card.supported_interfaces = Some(WireValue::Typed(vec![interface]));
    let mut schemes = BTreeMap::new();
    schemes.insert(
        "LiteLLMKey".to_owned(),
        SecurityScheme::Http {
            scheme: "bearer".to_owned(),
            description: "LiteLLM virtual key".to_owned(),
        },
    );
    card.security_schemes = Some(WireValue::Typed(schemes));
    let mut requirement = BTreeMap::new();
    requirement.insert("LiteLLMKey".to_owned(), Vec::new());
    card.security = Some(WireValue::Typed(vec![requirement]));
}

fn fill_defaults(card: &mut AgentCard, proxy_base_url: &str) {
    if !is_truthy_option(card.version.as_ref()) {
        card.version = Some(WireValue::Typed("1.0.0".to_owned()));
    }
    if !is_truthy_option(card.skills.as_ref()) {
        card.skills = Some(WireValue::Typed(vec![Skill {
            id: Some(WireValue::Typed("chat".to_owned())),
            name: Some(WireValue::Typed("Chat".to_owned())),
            description: Some(WireValue::Typed(
                "Conversational interaction with the agent.".to_owned(),
            )),
            tags: Some(WireValue::Typed(vec!["chat".to_owned()])),
            passthrough: BTreeMap::new(),
        }]));
    }
    if !is_truthy_option(card.default_input_modes.as_ref()) {
        card.default_input_modes = Some(WireValue::Typed(vec!["text".to_owned()]));
    }
    if !is_truthy_option(card.default_output_modes.as_ref()) {
        card.default_output_modes = Some(WireValue::Typed(vec!["text".to_owned()]));
    }
    if !is_truthy_option(card.provider.as_ref()) {
        card.provider = Some(WireValue::Typed(Provider {
            organization: Some(WireValue::Typed("LiteLLM Proxy".to_owned())),
            url: Some(WireValue::Typed(proxy_base_url.to_owned())),
            passthrough: BTreeMap::new(),
        }));
    }
}

fn is_truthy_option<T>(field: Option<&WireValue<T>>) -> bool
where
    T: Serialize,
{
    if field.is_none() {
        return false;
    }
    field
        .and_then(|value| serde_json::to_value(value).ok())
        .is_some_and(|value| is_truthy(&value))
}
fn truthy_wire(value: WireValue<bool>) -> Option<WireValue<bool>> {
    let json = serde_json::to_value(&value).ok()?;
    is_truthy(&json).then_some(value)
}
fn valid_suffix(suffix: &str) -> bool {
    let (prerelease, build) = suffix
        .split_once('+')
        .map_or((suffix, None), |(head, tail)| (head, Some(tail)));
    let valid = |text: &str| {
        text.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    };
    prerelease.strip_prefix('-').is_none_or(valid) && build.is_none_or(valid)
}
fn is_truthy(value: &AgentJson) -> bool {
    match value {
        AgentJson::Null => false,
        AgentJson::Bool(flag) => *flag,
        AgentJson::Number(number) => {
            number.as_i64() != Some(0) && number.as_u64() != Some(0) && number.as_f64() != Some(0.0)
        }
        AgentJson::String(text) => !text.is_empty(),
        AgentJson::Array(items) => !items.is_empty(),
        AgentJson::Object(fields) => !fields.is_empty(),
    }
}

mod tests;
