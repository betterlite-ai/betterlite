use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use sha2::{Digest as _, Sha256};

use super::AgentRegistry;
use super::python_json::{self, JsonEncodingError};

mod tests;

pub(crate) fn stable_id(agent_name: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(agent_name.as_bytes());
    hex::encode(hasher.finalize())
}

pub(crate) fn legacy_id(entry: &Json) -> Result<String, JsonEncodingError> {
    let text = python_json::dumps(entry)?;
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    Ok(hex::encode(hasher.finalize()))
}

pub(super) fn ids_for_agent(registry: &AgentRegistry, agent_id: &str) -> BTreeSet<String> {
    let state = registry.state();
    let mut ids = BTreeSet::new();
    ids.insert(agent_id.to_owned());
    for (legacy, stable) in &state.legacy_to_stable {
        if stable == agent_id {
            ids.insert(legacy.clone());
        }
    }
    ids
}

pub(super) fn stable_agent_id(registry: &AgentRegistry, agent_id: &str) -> String {
    let state = registry.state();
    match state.legacy_to_stable.get(agent_id) {
        Some(stable) => stable.clone(),
        None => agent_id.to_owned(),
    }
}
