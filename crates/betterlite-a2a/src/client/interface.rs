use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;

use crate::agent_card::{AgentCard, Interface, ProtocolVersion, WireValue};
use crate::client::records::A2A_VERSION;

pub(super) struct AgentEndpoint {
    pub(super) url: String,
    pub(super) tenant: Option<String>,
}

// The sdk derives a JSONRPC interface from the legacy url, preferredTransport and protocolVersion fields
#[rustfmt::skip]
pub(super) fn selected_interface(card: &AgentCard) -> Option<AgentEndpoint> {
    let interfaces = card_interfaces(card);
    let chosen = best_jsonrpc(&interfaces)?;
    Some(AgentEndpoint { url: wire_text(chosen.url.as_ref())?.to_owned(), tenant: chosen.passthrough.get("tenant").and_then(AgentJson::as_str).map(str::to_owned) })
}

#[rustfmt::skip]
fn card_interfaces(card: &AgentCard) -> Vec<Interface> {
    if let Some(Some(WireValue::Typed(interfaces))) = card.supported_interfaces.as_ref() {
        return interfaces.clone();
    }
    let Some(url) = wire_text(card.url.as_ref()) else {
        return Vec::new();
    };
    let binding = wire_text(card.preferred_transport.as_ref()).unwrap_or("JSONRPC");
    let mut interfaces = vec![Interface { url: Some(Some(WireValue::Typed(url.to_owned()))), protocol_binding: Some(Some(WireValue::Typed(binding.to_owned()))), protocol_version: card.protocol_version.clone(), passthrough: BTreeMap::new() }];
    let additional = card.passthrough.get("additionalInterfaces").and_then(AgentJson::as_array);
    for extra in additional.into_iter().flatten() {
        let Some(extra_url) = extra.get("url").and_then(AgentJson::as_str) else { continue; };
        let Some(extra_binding) = extra.get("transport").and_then(AgentJson::as_str) else { continue; };
        interfaces.push(Interface { url: Some(Some(WireValue::Typed(extra_url.to_owned()))), protocol_binding: Some(Some(WireValue::Typed(extra_binding.to_owned()))), protocol_version: card.protocol_version.clone(), passthrough: BTreeMap::new() });
    }
    interfaces
}

#[rustfmt::skip]
fn best_jsonrpc(interfaces: &[Interface]) -> Option<&Interface> {
    let candidates: Vec<&Interface> = interfaces.iter().filter(|interface| is_jsonrpc(interface)).collect();
    if let Some(exact) = candidates.iter().find(|interface| interface_version(interface) == Some(A2A_VERSION)) { return Some(exact); }
    let mut above_one = None;
    let mut at_least_three = None;
    for interface in &candidates {
        let Some(version) = interface_version(interface).and_then(version_parts) else { continue; };
        if version > (1, 0, 0) && above_one.is_none() { above_one = Some(*interface); }
        if version >= (0, 3, 0) && at_least_three.is_none() { at_least_three = Some(*interface); }
    }
    above_one.or(at_least_three).or_else(|| candidates.iter().find(|interface| interface_version(interface).is_none()).copied())
}

#[rustfmt::skip]
fn is_jsonrpc(interface: &Interface) -> bool {
    let Some(binding) = wire_text(interface.protocol_binding.as_ref()) else {
        return false;
    };
    binding.eq_ignore_ascii_case("JSONRPC")
}

#[rustfmt::skip]
fn wire_text(value: Option<&Option<WireValue<String>>>) -> Option<&str> {
    match value?.as_ref()? {
        WireValue::Typed(text) if !text.is_empty() => Some(text.as_str()), WireValue::Typed(_) | WireValue::Passthrough(_) => None,
    }
}

#[rustfmt::skip]
fn interface_version(interface: &Interface) -> Option<&str> {
    match interface.protocol_version.as_ref()?.as_ref()? {
        WireValue::Typed(ProtocolVersion::V0_3) => Some("0.3"), WireValue::Typed(ProtocolVersion::V1_0) => Some("1.0"), WireValue::Passthrough(AgentJson::String(text)) if !text.is_empty() => Some(text.as_str()), WireValue::Passthrough(_) => None,
    }
}

fn version_parts(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}
