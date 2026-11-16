use betterlite_models::decisions::Json as DecisionJson;
use indexmap::IndexMap;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::databricks_chat::error::DatabricksChatError;

// Anthropic and base OpenAI helpers take decisions::Json maps, the provider surface takes prompts::Object
pub(crate) type Params = IndexMap<String, DecisionJson>;

// The target type names the JSON model a value moves into, so the caller spells it.
pub(crate) fn convert<Source, Target>(value: &Source) -> Result<Target, DatabricksChatError>
where
    Source: Serialize + ?Sized,
    Target: DeserializeOwned,
{
    match serde_json::to_value(value) {
        Ok(encoded) => serde_json::from_value(encoded).map_err(DatabricksChatError::Decode),
        Err(error) => Err(DatabricksChatError::Encode(error)),
    }
}
