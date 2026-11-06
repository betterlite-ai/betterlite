use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use serde::Serialize;
use thiserror::Error;

mod tests;

#[derive(Debug, Error)]
pub enum ParamsError {
    #[error("the A2A params cannot be serialized")]
    Serialize(#[source] serde_json::Error),
    #[error("the A2A params are not a JSON object")]
    NotObject,
    #[error("the A2A params message is not a JSON object")]
    MessageNotObject,
}

// FastA2A rejects null for optional fields, so they have to be absent
#[must_use]
pub fn remove_none_values(value: AgentJson) -> AgentJson {
    match value {
        AgentJson::Object(fields) => AgentJson::Object(
            fields
                .into_iter()
                .filter(|(_, field)| !field.is_null())
                .map(|(name, field)| (name, remove_none_values(field)))
                .collect(),
        ),
        AgentJson::Array(items) => AgentJson::Array(
            items
                .into_iter()
                .filter(|item| !item.is_null())
                .map(remove_none_values)
                .collect(),
        ),
        scalar @ (AgentJson::Null
        | AgentJson::Bool(_)
        | AgentJson::Number(_)
        | AgentJson::String(_)) => scalar,
    }
}

pub fn build_send_params<P>(params: &P) -> Result<BTreeMap<String, AgentJson>, ParamsError>
where
    P: Serialize,
{
    let value = serde_json::to_value(params).map_err(ParamsError::Serialize)?;
    let AgentJson::Object(fields) = remove_none_values(value) else {
        return Err(ParamsError::NotObject);
    };
    let mut send: BTreeMap<String, AgentJson> = fields.into_iter().collect();
    match send.get_mut("message") {
        Some(AgentJson::Object(message)) => {
            message.insert("kind".to_owned(), AgentJson::String("message".to_owned()));
        }
        Some(_) => return Err(ParamsError::MessageNotObject),
        None => {}
    }
    Ok(send)
}
