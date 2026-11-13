use std::collections::BTreeMap;

use super::{RESERVED_EXACT_HEADERS, RESERVED_PREFIX_HEADERS};

pub(super) fn filter_reserved_headers(
    agent_extra_headers: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut filtered = BTreeMap::new();
    let mut dropped = Vec::new();
    for (name, value) in agent_extra_headers {
        let lower = name.to_lowercase();
        let reserved = RESERVED_EXACT_HEADERS.contains(&lower.as_str())
            || RESERVED_PREFIX_HEADERS
                .iter()
                .any(|prefix| lower.starts_with(prefix));
        if reserved {
            dropped.push(name.clone());
            continue;
        }
        filtered.insert(name.clone(), value.clone());
    }
    if !dropped.is_empty() {
        dropped.sort();
        tracing::warn!(
            headers = ?dropped,
            "BedrockAgentCore A2A: dropping reserved header(s) from agent_extra_headers (not forwarded to AgentCore)"
        );
    }
    filtered
}
