use betterlite_a2a::agent_card::{AgentCard, WireValue};
use betterlite_a2a::card_resolver::localhost;

use super::error::A2aCallError;

// The card text crosses the boundary whole, so unknown keys ride along in AgentCard's passthrough map.
pub(crate) fn parse(text: &str) -> Result<AgentCard, A2aCallError> {
    match serde_json::from_str(text) {
        Ok(card) => Ok(card),
        Err(source) => Err(A2aCallError::Card { source }),
    }
}

pub(crate) fn text(card: &AgentCard) -> Result<String, A2aCallError> {
    match serde_json::to_string(card) {
        Ok(json) => Ok(json),
        Err(source) => Err(A2aCallError::Encode { source }),
    }
}

// LiteLLM's fix_agent_card_url: a localhost url field and a localhost first interface are replaced, each on its own.
pub(crate) fn fix(card: &mut AgentCard, base_url: &str) {
    let fixed = format!("{}/", base_url.trim_end_matches('/'));
    if url_field(card.url.as_ref())
        .is_some_and(|url| localhost::is_localhost_or_internal_url(Some(url)))
    {
        card.url = Some(Some(WireValue::Typed(fixed.clone())));
    }
    if let Some(Some(WireValue::Typed(found))) = card.supported_interfaces.as_mut()
        && let Some(first) = found.first_mut()
        && url_field(first.url.as_ref())
            .is_some_and(|url| localhost::is_localhost_or_internal_url(Some(url)))
    {
        first.url = Some(Some(WireValue::Typed(fixed)));
    }
}

fn url_field(value: Option<&Option<WireValue<String>>>) -> Option<&str> {
    match value? {
        Some(WireValue::Typed(url)) if !url.is_empty() => Some(url.as_str()),
        Some(WireValue::Typed(_) | WireValue::Passthrough(_)) | None => None,
    }
}

mod tests;
