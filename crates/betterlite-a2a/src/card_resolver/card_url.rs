use crate::agent_card::{AgentCard, WireValue};

#[must_use]
pub fn get_agent_card_url(card: &AgentCard) -> Option<&str> {
    if let Some(url) = typed_url(card.url.as_ref()) {
        return Some(url);
    }
    match card.supported_interfaces.as_ref()?.as_ref()? {
        WireValue::Typed(interfaces) => typed_url(
            interfaces
                .first()
                .and_then(|interface| interface.url.as_ref()),
        ),
        WireValue::Passthrough(_) => None,
    }
}

pub fn set_agent_card_url(card: &mut AgentCard, url: &str) {
    let normalized = format!("{}/", url.trim_end_matches('/'));
    if card.url.is_some() {
        card.url = Some(Some(WireValue::Typed(normalized.clone())));
    }
    if let Some(Some(WireValue::Typed(interfaces))) = card.supported_interfaces.as_mut()
        && let Some(interface) = interfaces.first_mut()
    {
        interface.url = Some(Some(WireValue::Typed(normalized)));
    }
}

fn typed_url(value: Option<&Option<WireValue<String>>>) -> Option<&str> {
    match value?.as_ref()? {
        WireValue::Typed(url) if !url.is_empty() => Some(url.as_str()),
        WireValue::Typed(_) | WireValue::Passthrough(_) => None,
    }
}
