use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::Extra;
use betterlite_openai::api::responses_objects::sdk::unions::SdkOutput;
use betterlite_openai::api::responses_objects::{ResponseOutputItem, ResponsesApiResponse};
use indexmap::IndexMap;

use super::error::SyntheticEventsError;

pub trait HasModelDump {
    fn model_dump(&self) -> Result<Extra, SyntheticEventsError>;
}

pub trait HasModelDumpJson {
    fn model_dump_json(&self) -> Result<String, SyntheticEventsError>;
}

#[derive(Clone, Copy)]
pub enum ResponseObject<'object> {
    Model(&'object ResponseOutputItem),
    Value(&'object Json),
}

pub fn dump_response_object(object: ResponseObject<'_>) -> Result<Extra, SyntheticEventsError> {
    match object {
        ResponseObject::Model(item) => item.model_dump(),
        ResponseObject::Value(Json::Object(fields)) => Ok(fields.clone().into_iter().collect()),
        ResponseObject::Value(_) => Ok(Extra::new()),
    }
}

impl HasModelDump for ResponseOutputItem {
    fn model_dump(&self) -> Result<Extra, SyntheticEventsError> {
        if let Self::Unknown(fields) = self {
            return Ok(fields.clone());
        }
        let value = serde_json::to_value(self)?;
        let Json::Object(mut fields) = serde_json::from_value(value)? else {
            return Ok(Extra::new());
        };
        if let Self::Sdk(output) = self
            && let SdkOutput::Reasoning(_) = output.as_ref()
        {
            reasoning_nulls(&mut fields);
        }
        Ok(fields.into_iter().collect())
    }
}

fn reasoning_nulls(fields: &mut IndexMap<String, Json>) {
    for key in ["content", "encrypted_content", "status"] {
        fields.entry(key.to_owned()).or_insert(Json::Null);
    }
}

impl HasModelDumpJson for ResponsesApiResponse {
    fn model_dump_json(&self) -> Result<String, SyntheticEventsError> {
        Ok(serde_json::to_string(self)?)
    }
}

pub fn dump_json_safely<T>(response: &T) -> Option<String>
where
    T: HasModelDumpJson,
{
    match response.model_dump_json() {
        Ok(text) => Some(text),
        Err(error) => {
            tracing::debug!("could not serialize completed response for cache: {error}");
            None
        }
    }
}
