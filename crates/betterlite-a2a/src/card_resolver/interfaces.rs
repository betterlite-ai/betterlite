use crate::agent_card::{AgentCard, Interface, ProtocolVersion, WireValue};

// Lowercase spellings from LiteLLM's canonical binding table, mapped to the spec casing.
const CANONICAL_BINDINGS: [(&str, &str); 3] = [
    ("jsonrpc", "JSONRPC"),
    ("http+json", "HTTP+JSON"),
    ("grpc", "GRPC"),
];

#[must_use]
pub fn normalize_interfaces(mut card: AgentCard) -> AgentCard {
    if let Some(Some(WireValue::Typed(interfaces))) = card.supported_interfaces.as_mut() {
        for interface in interfaces {
            canonicalize_binding(interface);
        }
    }
    card
}

fn canonicalize_binding(interface: &mut Interface) {
    let Some(Some(WireValue::Typed(binding))) = interface.protocol_binding.as_mut() else {
        return;
    };
    let lowered = binding.to_lowercase();
    let Some((_, canonical)) = CANONICAL_BINDINGS
        .iter()
        .find(|(lower, _)| *lower == lowered)
    else {
        return;
    };
    if *canonical == binding.as_str() {
        return;
    }
    (*canonical).clone_into(binding);
    interface.protocol_version = Some(Some(WireValue::Typed(ProtocolVersion::V0_3)));
}
