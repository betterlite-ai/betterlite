use betterlite_core::agents::json::AgentJson;
use std::collections::BTreeMap;

// The merge filters and passes through whatever upstream sent, so the card stays untyped JSON
pub type AgentCard = BTreeMap<String, AgentJson>;

const DEFAULT_VERSION: &str = "1.0";

// v1.0 AgentCard fields plus supportedInterfaces; additionalInterfaces would let callers bypass the proxy
const ALLOWED_TOP_LEVEL: [&str; 18] = [
    "protocolVersion",
    "name",
    "description",
    "version",
    "capabilities",
    "defaultInputModes",
    "defaultOutputModes",
    "skills",
    "preferredTransport",
    "supportedInterfaces",
    "iconUrl",
    "provider",
    "documentationUrl",
    "securitySchemes",
    "security",
    "supportsAuthenticatedExtendedCard",
    "signatures",
    // the invoker reads the upstream url from the stored card; the public route rewrites it
    "url",
];

#[must_use]
pub fn normalize_protocol_version(raw_version: &AgentJson) -> Option<&'static str> {
    let raw_text = raw_version.as_str()?;
    // LiteLLM matches with Python's `$`, which also accepts one trailing newline
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
        let numeric = !patch.is_empty() && patch.bytes().all(|byte| byte.is_ascii_digit());
        if !numeric || !valid_suffix(suffix) {
            return None;
        }
    }
    // only these exact pairs pass, so a non-numeric major or minor is rejected here too
    match (major, minor) {
        ("0", "3") => Some("0.3"),
        ("1", "0") => Some(DEFAULT_VERSION),
        _ => None,
    }
}

#[must_use]
pub fn resolve_served_version(card: Option<&AgentCard>) -> &'static str {
    let pinned = card.and_then(|agent_card| agent_card.get("protocolVersion"));
    pinned
        .and_then(normalize_protocol_version)
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
    card.insert(
        "protocolVersion".to_owned(),
        AgentJson::from(served_version),
    );
    if let Some(caller_name) = name.filter(|text| !text.is_empty()) {
        card.insert("name".to_owned(), AgentJson::from(caller_name));
    }
    if let Some(caller_description) = description.filter(|text| !text.is_empty()) {
        card.insert(
            "description".to_owned(),
            AgentJson::from(caller_description),
        );
    }
    // streaming is the only capability the proxy can deliver today
    let streaming = card
        .get("capabilities")
        .and_then(|capabilities| capabilities.get("streaming"))
        .filter(|value| is_truthy(value))
        .map(|value| ("streaming", value.clone()));
    card.insert("capabilities".to_owned(), AgentJson::from_iter(streaming));
    fill_defaults(&mut card, proxy_base_url);
    insert_proxy_fields(&mut card, proxy_url, served_version);
    card.retain(|key, _| ALLOWED_TOP_LEVEL.contains(&key.as_str()));
    card
}

fn insert_proxy_fields(card: &mut AgentCard, proxy_url: &str, served_version: &str) {
    let interface = AgentJson::from_iter([
        ("url", AgentJson::from(proxy_url)),
        ("protocolBinding", AgentJson::from("JSONRPC")),
        ("protocolVersion", AgentJson::from(served_version)),
    ]);
    card.insert(
        "supportedInterfaces".to_owned(),
        AgentJson::Array(vec![interface]),
    );
    let scheme = AgentJson::from_iter([
        ("type", AgentJson::from("http")),
        ("scheme", AgentJson::from("bearer")),
        ("description", AgentJson::from("LiteLLM virtual key")),
    ]);
    let schemes = AgentJson::from_iter([("LiteLLMKey", scheme)]);
    card.insert("securitySchemes".to_owned(), schemes);
    let requirement = AgentJson::from_iter([("LiteLLMKey", AgentJson::Array(Vec::new()))]);
    card.insert("security".to_owned(), AgentJson::Array(vec![requirement]));
}

fn fill_defaults(card: &mut AgentCard, proxy_base_url: &str) {
    fill_if_falsy(card, "version", AgentJson::from("1.0.0"));
    let chat_skill = AgentJson::from_iter([
        ("id", AgentJson::from("chat")),
        ("name", AgentJson::from("Chat")),
        (
            "description",
            AgentJson::from("Conversational interaction with the agent."),
        ),
        ("tags", AgentJson::from_iter(["chat"])),
    ]);
    fill_if_falsy(card, "skills", AgentJson::Array(vec![chat_skill]));
    fill_if_falsy(card, "defaultInputModes", AgentJson::from_iter(["text"]));
    fill_if_falsy(card, "defaultOutputModes", AgentJson::from_iter(["text"]));
    let provider = AgentJson::from_iter([
        ("organization", AgentJson::from("LiteLLM Proxy")),
        ("url", AgentJson::from(proxy_base_url)),
    ]);
    fill_if_falsy(card, "provider", provider);
}

fn valid_suffix(suffix: &str) -> bool {
    let (prerelease, build) = suffix
        .split_once('+')
        .map_or((suffix, None), |(head, tail)| (head, Some(tail)));
    let valid_identifiers = |dotted: &str| {
        dotted.split('.').all(|identifier| {
            !identifier.is_empty()
                && identifier
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    };
    prerelease.strip_prefix('-').is_none_or(valid_identifiers)
        && build.is_none_or(valid_identifiers)
}

fn fill_if_falsy(card: &mut AgentCard, key: &str, default: AgentJson) {
    if !card.get(key).is_some_and(is_truthy) {
        card.insert(key.to_owned(), default);
    }
}

fn is_truthy(value: &AgentJson) -> bool {
    match value {
        AgentJson::Null => false,
        AgentJson::Bool(flag) => *flag,
        AgentJson::Number(number) => number.as_f64().is_some_and(|numeric| numeric != 0.0_f64),
        AgentJson::String(text) => !text.is_empty(),
        AgentJson::Array(items) => !items.is_empty(),
        AgentJson::Object(fields) => !fields.is_empty(),
    }
}

mod tests;
