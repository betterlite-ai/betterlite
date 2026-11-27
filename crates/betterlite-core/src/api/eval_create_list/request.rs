use betterlite_models::decisions::Json;
use betterlite_openai::evals::types::{CreateEvalRequest, ListEvalsParams};
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use serde::de::DeserializeOwned;

use super::call::{CreateEvalCall, ListEvalsCall};
use super::error::EvalsCreateListError;

// LiteLLM builds create_request from the required fields and the name, then lets extra_body replace any key
pub fn create_request(
    call: &CreateEvalCall<'_>,
) -> Result<CreateEvalRequest, EvalsCreateListError> {
    let mut request = CreateEvalRequest {
        name: call.name.map(str::to_owned),
        data_source_config: call.data_source_config.clone(),
        testing_criteria: call.testing_criteria.clone(),
        metadata: None,
    };
    for (name, value) in call.extra_body {
        match name.as_str() {
            "name" => request.name = decode(name, value)?,
            "data_source_config" => request.data_source_config = required(name, value)?,
            "testing_criteria" => request.testing_criteria = required(name, value)?,
            "metadata" => request.metadata = decode(name, value)?,
            _ => {}
        }
    }
    Ok(request)
}

// the OpenAI create transform serializes the typed request only, so extra_body's other keys are merged after it
pub fn merge_extra_body(
    body: &mut Object,
    extra_body: &IndexMap<String, Json>,
) -> Result<(), EvalsCreateListError> {
    for (name, value) in extra_body {
        match name.as_str() {
            "name" | "data_source_config" | "testing_criteria" | "metadata" => {}
            _ => {
                if !matches!(value, Json::Null) {
                    body.insert(name.clone(), serde_json::to_value(value)?);
                }
            }
        }
    }
    Ok(())
}

// LiteLLM fills ListEvalsParams from the named arguments, then lets extra_query overlay whatever it carries
pub fn list_params(call: &ListEvalsCall<'_>) -> Result<ListEvalsParams, EvalsCreateListError> {
    let mut params = ListEvalsParams {
        limit: call.limit,
        after: call.after.map(str::to_owned),
        before: call.before.map(str::to_owned),
        order: call.order,
        order_by: call.order_by,
    };
    for (name, value) in call.params.extra_query {
        match name.as_str() {
            "limit" => params.limit = decode(name, value)?,
            "after" => params.after = decode(name, value)?,
            "before" => params.before = decode(name, value)?,
            "order" => params.order = decode(name, value)?,
            "order_by" => params.order_by = decode(name, value)?,
            // the OpenAI transform reads only these five keys, so other extra_query keys never reach the request
            _ => {}
        }
    }
    Ok(params)
}

// a null leaves an optional field out, and docs/compatibility.md reads any other value as its declared type
fn decode<T>(name: &str, value: &Json) -> Result<Option<T>, EvalsCreateListError>
where
    T: DeserializeOwned,
{
    match value {
        Json::Null => Ok(None),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            serde_json::to_value(value)
                .and_then(serde_json::from_value::<T>)
                .map(Some)
                .map_err(|_source| EvalsCreateListError::FieldType {
                    name: name.to_owned(),
                })
        }
    }
}

// a required field has no default, so a null in one is the same type error
fn required<T>(name: &str, value: &Json) -> Result<T, EvalsCreateListError>
where
    T: DeserializeOwned,
{
    decode(name, value)?.ok_or_else(|| EvalsCreateListError::FieldType {
        name: name.to_owned(),
    })
}
