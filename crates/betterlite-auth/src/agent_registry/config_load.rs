use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Deserializer;

use crate::agent_policy::AgentResponse;

use super::{AgentRegistry, AgentRegistryError, State, ids};

mod tests;

pub(crate) const AGENT_FIELDS: &[&str] = &[
    "identity",
    "identity_managed",
    "enabled",
    "execution_mode",
    "jwt_auth_configured",
    "agent_id",
    "agent_name",
    "litellm_params",
    "agent_card_params",
    "object_permission",
    "spend",
    "tpm_limit",
    "rpm_limit",
    "session_tpm_limit",
    "session_rpm_limit",
    "static_headers",
    "extra_headers",
    "access_group_ids",
    "kill_switch",
    "keys",
    "search_score",
    "created_at",
    "updated_at",
    "created_by",
    "updated_by",
];

pub(super) fn load_agents_from_config(
    registry: &AgentRegistry,
    agent_config: Option<&[Json]>,
) -> Result<(), AgentRegistryError> {
    let Some(entries) = agent_config else {
        return Ok(());
    };
    ensure_entries_are_objects(entries)?;
    let mut state = registry.state_mut();
    state.config_agents = entries.to_vec();
    state.legacy_to_stable = legacy_map(entries)?;
    for entry in entries {
        register_config_entry(&mut state, entry)?;
    }
    Ok(())
}

fn ensure_entries_are_objects(entries: &[Json]) -> Result<(), AgentRegistryError> {
    for entry in entries {
        if !matches!(entry, Json::Object(_)) {
            return Err(AgentRegistryError::NotAListOfDictionaries);
        }
    }
    Ok(())
}

fn legacy_map(entries: &[Json]) -> Result<BTreeMap<String, String>, AgentRegistryError> {
    let mut map = BTreeMap::new();
    for entry in entries {
        let Some(agent_name) = config_agent_name(entry)? else {
            continue;
        };
        map.insert(ids::legacy_id(entry)?, ids::stable_id(agent_name));
    }
    Ok(map)
}

fn config_agent_name(entry: &Json) -> Result<Option<&str>, AgentRegistryError> {
    let Json::Object(fields) = entry else {
        return Err(AgentRegistryError::NotAListOfDictionaries);
    };
    let agent_name = match fields.get("agent_name") {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Text(name)) if name.is_empty() => return Ok(None),
        Some(Json::Text(name)) => name.as_str(),
        Some(_) => return Err(AgentRegistryError::AgentNameType),
    };
    match fields.get("agent_card_params") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(card)) if card.is_empty() => Ok(None),
        Some(Json::Object(_)) => Ok(Some(agent_name)),
        Some(_) => Err(AgentRegistryError::AgentCardType),
    }
}

fn register_config_entry(state: &mut State, entry: &Json) -> Result<(), AgentRegistryError> {
    let Some(agent_name) = config_agent_name(entry)? else {
        return Ok(());
    };
    if state
        .agent_list
        .iter()
        .any(|agent| agent.agent_name == agent_name)
    {
        return Ok(());
    }
    let Json::Object(fields) = entry else {
        return Err(AgentRegistryError::NotAListOfDictionaries);
    };
    for key in fields.keys() {
        if !AGENT_FIELDS.contains(&key.as_str()) {
            return Err(AgentRegistryError::UnknownConfigKey { key: key.clone() });
        }
    }
    if fields.contains_key("agent_id") {
        return Err(AgentRegistryError::ConfigAgentId);
    }
    let mut fields = fields.clone();
    fields.insert(
        String::from("agent_id"),
        Json::Text(ids::stable_id(agent_name)),
    );
    let agent = decode_agent(&fields)?;
    state.agent_list.push(agent);
    Ok(())
}

fn decode_agent(fields: &IndexMap<String, Json>) -> Result<AgentResponse, AgentRegistryError> {
    let text = serde_json::to_string(fields).map_err(|source| AgentRegistryError::Record {
        path: String::from("."),
        source,
    })?;
    let mut deserializer = Deserializer::from_str(&text);
    let agent = serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
        AgentRegistryError::Record {
            path: error.path().to_string(),
            source: error.into_inner(),
        }
    })?;
    Ok(agent)
}
