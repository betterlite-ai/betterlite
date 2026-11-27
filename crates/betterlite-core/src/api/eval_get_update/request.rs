use betterlite_models::decisions::Json;
use betterlite_openai::evals::types::UpdateEvalRequest;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use serde::de::DeserializeOwned;

use super::call::UpdateEvalCall;
use super::error::EvalsGetUpdateError;
use super::metadata::filtered_metadata;

// LiteLLM's update_request dict: the optional name, the filtered metadata, then extra_body over the top
pub fn update_request(call: &UpdateEvalCall<'_>) -> Result<UpdateEvalRequest, EvalsGetUpdateError> {
    let name = match call.extra_body.get("name") {
        Some(value) => decode("name", value)?,
        None => call.name.map(str::to_owned),
    };
    let metadata = match call.extra_body.get("metadata") {
        Some(value) => decode("metadata", value)?,
        None => filtered_metadata(call.metadata)?,
    };
    Ok(UpdateEvalRequest { name, metadata })
}

// the OpenAI update transform serializes the typed request only, so extra_body's other keys are merged after it
pub fn merge_extra_body(
    body: &mut Object,
    extra_body: &IndexMap<String, Json>,
) -> Result<(), EvalsGetUpdateError> {
    for (name, value) in extra_body {
        if matches!(name.as_str(), "name" | "metadata") || matches!(value, Json::Null) {
            continue;
        }
        body.insert(name.clone(), serde_json::to_value(value)?);
    }
    Ok(())
}

// a null leaves an optional field out, and docs/compatibility.md reads any other value as its declared type
pub(super) fn decode<T>(name: &str, value: &Json) -> Result<Option<T>, EvalsGetUpdateError>
where
    T: DeserializeOwned,
{
    serde_json::from_value::<Option<T>>(serde_json::to_value(value)?).map_err(|_source| {
        EvalsGetUpdateError::FieldType {
            name: name.to_owned(),
        }
    })
}
