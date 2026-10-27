use super::{
    AgentCard, Capabilities, Interface, ProtocolVersion, Provider, SecurityHttp, SecurityScheme,
    Skill, WireValue,
};
use betterlite_core::agents::json::AgentJson;
use serde::Serialize;
use std::collections::BTreeMap;

#[must_use]
pub fn normalize_protocol_version(raw_version: &AgentJson) -> Option<ProtocolVersion> {
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
        ("0", "3") => Some(ProtocolVersion::V0_3),
        ("1", "0") => Some(ProtocolVersion::V1_0),
        _ => None,
    }
}

#[must_use]
pub fn resolve_served_version(upstream_card: Option<&AgentCard>) -> ProtocolVersion {
    let Some(card) = upstream_card else {
        return ProtocolVersion::V1_0;
    };
    card.protocol_version
        .as_ref()
        .and_then(Option::as_ref)
        .and_then(|wire_value| match wire_value {
            WireValue::Typed(version) => Some(*version),
            WireValue::Passthrough(raw_value) => normalize_protocol_version(raw_value),
        })
        .unwrap_or(ProtocolVersion::V1_0)
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
    card.protocol_version = Some(Some(WireValue::Typed(served_version)));
    if let Some(value) = name.filter(|text| !text.is_empty()) {
        card.name = Some(Some(WireValue::Typed(value.to_owned())));
    }
    if let Some(value) = description.filter(|text| !text.is_empty()) {
        card.description = Some(Some(WireValue::Typed(value.to_owned())));
    }
    card.capabilities = Some(Some(filtered_capabilities(
        card.capabilities.take().flatten(),
    )));
    fill_defaults(&mut card, proxy_base_url);
    insert_proxy_fields(&mut card, proxy_url, served_version);
    card.passthrough.clear();
    card
}

fn filtered_capabilities(value: Option<WireValue<Capabilities>>) -> WireValue<Capabilities> {
    let streaming = match value {
        Some(WireValue::Typed(capabilities)) => {
            capabilities.streaming.flatten().and_then(truthy_wire)
        }
        _ => None,
    };
    WireValue::Typed(Capabilities {
        streaming: streaming.map(Some),
        passthrough: BTreeMap::new(),
    })
}

fn insert_proxy_fields(card: &mut AgentCard, proxy_url: &str, served_version: ProtocolVersion) {
    let interface = Interface {
        url: Some(Some(WireValue::Typed(proxy_url.to_owned()))),
        protocol_binding: Some(Some(WireValue::Typed("JSONRPC".to_owned()))),
        protocol_version: Some(Some(WireValue::Typed(served_version))),
        passthrough: BTreeMap::new(),
    };
    card.supported_interfaces = Some(Some(WireValue::Typed(vec![interface])));
    let mut schemes = BTreeMap::new();
    schemes.insert(
        "LiteLLMKey".to_owned(),
        SecurityScheme::Http(SecurityHttp {
            scheme: Some("bearer".to_owned()),
            bearer_format: None,
            description: Some(Some("LiteLLM virtual key".to_owned())),
        }),
    );
    card.security_schemes = Some(Some(WireValue::Typed(schemes)));
    let mut requirement = BTreeMap::new();
    requirement.insert("LiteLLMKey".to_owned(), Vec::new());
    card.security = Some(Some(WireValue::Typed(vec![requirement])));
}

fn fill_defaults(card: &mut AgentCard, proxy_base_url: &str) {
    if !is_truthy_option(card.version.as_ref()) {
        card.version = Some(Some(WireValue::Typed("1.0.0".to_owned())));
    }
    if !is_truthy_option(card.skills.as_ref()) {
        card.skills = Some(Some(WireValue::Typed(vec![Skill {
            id: Some(Some(WireValue::Typed("chat".to_owned()))),
            name: Some(Some(WireValue::Typed("Chat".to_owned()))),
            description: Some(Some(WireValue::Typed(
                "Conversational interaction with the agent.".to_owned(),
            ))),
            tags: Some(Some(WireValue::Typed(vec!["chat".to_owned()]))),
            passthrough: BTreeMap::new(),
        }])));
    }
    if !is_truthy_option(card.default_input_modes.as_ref()) {
        card.default_input_modes = Some(Some(WireValue::Typed(vec!["text".to_owned()])));
    }
    if !is_truthy_option(card.default_output_modes.as_ref()) {
        card.default_output_modes = Some(Some(WireValue::Typed(vec!["text".to_owned()])));
    }
    if !is_truthy_option(card.provider.as_ref()) {
        card.provider = Some(Some(WireValue::Typed(Provider {
            organization: Some(Some(WireValue::Typed("LiteLLM Proxy".to_owned()))),
            url: Some(Some(WireValue::Typed(proxy_base_url.to_owned()))),
            passthrough: BTreeMap::new(),
        })));
    }
}

fn is_truthy_option<T>(field: Option<&Option<WireValue<T>>>) -> bool
where
    T: Serialize,
{
    if field.is_none() {
        return false;
    }
    field
        .and_then(Option::as_ref)
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
