use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;

#[must_use]
pub fn parse_agent_litellm_params(value: Option<&AgentJson>) -> BTreeMap<String, AgentJson> {
    let Some(params) = value else {
        return BTreeMap::new();
    };
    match params {
        AgentJson::Object(fields) => fields
            .iter()
            .map(|(key, nested)| (key.clone(), nested.clone()))
            .collect(),
        AgentJson::String(text) => match serde_json::from_str::<AgentJson>(text) {
            Ok(AgentJson::Object(fields)) => fields.into_iter().collect(),
            Ok(
                AgentJson::Null
                | AgentJson::Bool(_)
                | AgentJson::Number(_)
                | AgentJson::String(_)
                | AgentJson::Array(_),
            )
            | Err(_) => BTreeMap::new(),
        },
        AgentJson::Null | AgentJson::Bool(_) | AgentJson::Number(_) | AgentJson::Array(_) => {
            BTreeMap::new()
        }
    }
}

mod tests;
