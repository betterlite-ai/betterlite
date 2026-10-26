use betterlite_core::agents::json::AgentJson;
use std::collections::BTreeMap;

pub type AgentCard = BTreeMap<String, AgentJson>;

const DEFAULT_VERSION: &str = "1.0";
const DEFAULT_AGENT_VERSION: &str = "1.0.0";

#[must_use]
pub fn normalize_protocol_version(raw_version: &AgentJson) -> Option<&'static str> {
    let version_text = raw_version.as_str()?;
    let (major, minor_and_rest) = version_text.split_once('.')?;
    if !is_numeric_component(major) {
        return None;
    }
    let (minor, patch_part) = minor_and_rest
        .split_once('.')
        .map_or((minor_and_rest, None), |(minor, rest)| (minor, Some(rest)));
    if !is_numeric_component(minor) {
        return None;
    }
    if let Some(patch_and_suffix) = patch_part {
        let (patch, suffix) = split_patch_suffix(patch_and_suffix);
        if !is_numeric_component(patch) || !valid_suffix(suffix) {
            return None;
        }
    }
    match (major, minor) {
        ("0", "3") => Some("0.3"),
        ("1", "0") => Some(DEFAULT_VERSION),
        _ => None,
    }
}

pub fn resolve_served_protocol_version(card: Option<&AgentCard>) -> &'static str {
    card.and_then(|agent_card| agent_card.get("protocolVersion"))
        .and_then(normalize_protocol_version)
        .unwrap_or(DEFAULT_VERSION)
}

#[must_use]
pub fn merge_agent_card(
    upstream_card: Option<&AgentCard>,
    proxy_url: &str,
    proxy_base_url: &str,
    caller_name: Option<&str>,
    caller_description: Option<&str>,
) -> AgentCard {
    let mut card = upstream_card.cloned().unwrap_or_default();
    let served_version = resolve_served_protocol_version(upstream_card);
    card.insert(
        "protocolVersion".to_owned(),
        AgentJson::String(served_version.to_owned()),
    );
    apply_overrides(&mut card, caller_name, caller_description);
    apply_defaults(&mut card, proxy_base_url);
    apply_proxy_fields(&mut card, proxy_url, served_version);
    card.retain(|key, _| allowed_top_level(key));
    card
}

fn apply_overrides(
    card: &mut AgentCard,
    caller_name: Option<&str>,
    caller_description: Option<&str>,
) {
    if let Some(name) = caller_name {
        card.insert("name".to_owned(), AgentJson::String(name.to_owned()));
    }
    if let Some(description) = caller_description {
        card.insert(
            "description".to_owned(),
            AgentJson::String(description.to_owned()),
        );
    }
}

fn apply_defaults(card: &mut AgentCard, proxy_base_url: &str) {
    insert_default(
        card,
        "version",
        AgentJson::String(DEFAULT_AGENT_VERSION.to_owned()),
    );
    let capabilities = filtered_capabilities(card.get("capabilities"));
    card.insert("capabilities".to_owned(), capabilities);
    insert_default(card, "skills", default_skills());
    insert_default(card, "defaultInputModes", default_modes());
    insert_default(card, "defaultOutputModes", default_modes());
    insert_default(card, "provider", provider(proxy_base_url));
}

fn apply_proxy_fields(card: &mut AgentCard, proxy_url: &str, served_version: &str) {
    card.insert(
        "supportedInterfaces".to_owned(),
        supported_interface(proxy_url, served_version),
    );
    card.insert("securitySchemes".to_owned(), security_schemes());
    card.insert("security".to_owned(), security_requirements());
}

fn is_numeric_component(component: &str) -> bool {
    !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit())
}

fn split_patch_suffix(rest: &str) -> (&str, Option<&str>) {
    rest.find(['-', '+']).map_or((rest, None), |index| {
        let (patch, suffix) = rest.split_at(index);
        (patch, Some(suffix))
    })
}

fn valid_suffix(raw_suffix: Option<&str>) -> bool {
    let Some(suffix_text) = raw_suffix else {
        return true;
    };
    let (prerelease, build) = suffix_text.strip_prefix('-').map_or_else(
        || (None, suffix_text.strip_prefix('+')),
        |suffix| {
            suffix
                .split_once('+')
                .map_or((Some(suffix), None), |parts| (Some(parts.0), Some(parts.1)))
        },
    );
    valid_identifiers(prerelease) && valid_identifiers(build)
}

fn valid_identifiers(raw_value: Option<&str>) -> bool {
    raw_value.is_none_or(|identifier_text| {
        !identifier_text.is_empty()
            && identifier_text.split('.').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
    })
}

fn insert_default(card: &mut AgentCard, key: &str, value: AgentJson) {
    if card.get(key).is_none_or(is_falsey) {
        card.insert(key.to_owned(), value);
    }
}

fn is_falsey(raw_value: &AgentJson) -> bool {
    match raw_value {
        AgentJson::Null | AgentJson::Bool(false) => true,
        AgentJson::Number(number) => number
            .as_f64()
            .is_some_and(|numeric_value| numeric_value == 0.0_f64),
        AgentJson::String(text) => text.is_empty(),
        AgentJson::Array(items) => items.is_empty(),
        AgentJson::Object(object) => object.is_empty(),
        AgentJson::Bool(true) => false,
    }
}

fn filtered_capabilities(raw_value: Option<&AgentJson>) -> AgentJson {
    let mut capabilities = BTreeMap::new();
    if let Some(AgentJson::Object(upstream)) = raw_value
        && upstream
            .get("streaming")
            .is_some_and(|streaming| !is_falsey(streaming))
    {
        capabilities.insert("streaming".to_owned(), AgentJson::Bool(true));
    }
    AgentJson::Object(capabilities.into_iter().collect())
}

fn default_skills() -> AgentJson {
    let mut skill = BTreeMap::new();
    skill.insert("id".to_owned(), AgentJson::String("chat".to_owned()));
    skill.insert("name".to_owned(), AgentJson::String("Chat".to_owned()));
    skill.insert(
        "description".to_owned(),
        AgentJson::String("Conversational interaction with the agent.".to_owned()),
    );
    skill.insert(
        "tags".to_owned(),
        AgentJson::Array(vec![AgentJson::String("chat".to_owned())]),
    );
    AgentJson::Array(vec![AgentJson::Object(skill.into_iter().collect())])
}

fn default_modes() -> AgentJson {
    AgentJson::Array(vec![AgentJson::String("text".to_owned())])
}

fn provider(proxy_base_url: &str) -> AgentJson {
    let mut provider = BTreeMap::new();
    provider.insert(
        "organization".to_owned(),
        AgentJson::String("LiteLLM Proxy".to_owned()),
    );
    provider.insert(
        "url".to_owned(),
        AgentJson::String(proxy_base_url.to_owned()),
    );
    AgentJson::Object(provider.into_iter().collect())
}

fn supported_interface(proxy_url: &str, served_version: &str) -> AgentJson {
    let mut interface = BTreeMap::new();
    interface.insert("url".to_owned(), AgentJson::String(proxy_url.to_owned()));
    interface.insert(
        "protocolBinding".to_owned(),
        AgentJson::String("JSONRPC".to_owned()),
    );
    interface.insert(
        "protocolVersion".to_owned(),
        AgentJson::String(served_version.to_owned()),
    );
    AgentJson::Array(vec![AgentJson::Object(interface.into_iter().collect())])
}

fn security_schemes() -> AgentJson {
    let mut scheme = BTreeMap::new();
    scheme.insert("type".to_owned(), AgentJson::String("http".to_owned()));
    scheme.insert("scheme".to_owned(), AgentJson::String("bearer".to_owned()));
    scheme.insert(
        "description".to_owned(),
        AgentJson::String("LiteLLM virtual key".to_owned()),
    );
    let mut schemes = BTreeMap::new();
    schemes.insert(
        "LiteLLMKey".to_owned(),
        AgentJson::Object(scheme.into_iter().collect()),
    );
    AgentJson::Object(schemes.into_iter().collect())
}

fn security_requirements() -> AgentJson {
    let mut requirement = BTreeMap::new();
    requirement.insert("LiteLLMKey".to_owned(), AgentJson::Array(Vec::new()));
    AgentJson::Array(vec![AgentJson::Object(requirement.into_iter().collect())])
}

fn allowed_top_level(key: &str) -> bool {
    matches!(
        key,
        "protocolVersion"
            | "name"
            | "description"
            | "version"
            | "capabilities"
            | "defaultInputModes"
            | "defaultOutputModes"
            | "skills"
            | "preferredTransport"
            | "supportedInterfaces"
            | "iconUrl"
            | "provider"
            | "documentationUrl"
            | "securitySchemes"
            | "security"
            | "supportsAuthenticatedExtendedCard"
            | "signatures"
            | "url"
    )
}

#[cfg(test)]
mod tests;
