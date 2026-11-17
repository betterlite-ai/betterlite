use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::openai_dispatch::call::{Credentials, OpenAiOperation};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Value;

use super::case::{Case, Operation, failure};

mod chat;
mod embedding;
mod text;

fn explicit_credentials(case: &Case) -> Credentials {
    Credentials {
        api_base: case.api_base.clone(),
        api_key: case.api_key.clone().map(SecretString::from),
        organization: case.organization.clone(),
    }
}

pub(super) fn operation(case: &Case) -> Result<OpenAiOperation, Box<HelperError>> {
    match case.operation {
        Operation::Chat => Ok(OpenAiOperation::Chat(Box::new(chat::chat_call(case)?))),
        Operation::Embedding => Ok(OpenAiOperation::Embedding(Box::new(
            embedding::embedding_call(case)?,
        ))),
        Operation::Text => Ok(OpenAiOperation::Text(Box::new(text::text_call(case)?))),
    }
}

fn encoded(body: Object) -> Result<Vec<u8>, Box<HelperError>> {
    serde_json::to_vec(&Value::Object(body)).map_err(|error| failure("ValueError", error))
}

fn metadata(
    fields: Option<&Object>,
) -> Result<Option<IndexMap<String, JsonValue>>, Box<HelperError>> {
    fields.map_or_else(
        || Ok(None),
        |object| {
            serde_json::from_value(Value::Object(object.clone()))
                .map(Some)
                .map_err(|error| failure("ValueError", error))
        },
    )
}

// completion() takes the supplied map or else the extra one, then writes the extra map over it
fn merged_headers(case: &Case) -> Option<Headers> {
    let supplied = case.headers.as_ref().filter(|map| !map.is_empty());
    let base = supplied.or_else(|| case.extra_headers.as_ref().filter(|map| !map.is_empty()));
    let mut merged = base.cloned().unwrap_or_default();
    if let Some(extra) = &case.extra_headers {
        merged.extend(extra.clone());
    }
    (!merged.is_empty()).then_some(merged)
}
