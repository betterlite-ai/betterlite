use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::embedding::EmbeddingRequest;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::openai_dispatch::call::{
    ChatCall, Credentials, EmbeddingCall, OpenAiOperation, TextCall,
};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Value;

use super::case::{Case, Operation, failure};

fn explicit_credentials(case: &Case) -> Credentials {
    Credentials {
        api_base: case.api_base.clone(),
        api_key: case.api_key.clone().map(SecretString::from),
        organization: case.organization.clone(),
    }
}

pub(super) fn operation(case: &Case) -> Result<OpenAiOperation, Box<HelperError>> {
    match case.case {
        Operation::Chat => Ok(OpenAiOperation::Chat(Box::new(chat_call(case)?))),
        Operation::Embedding => Ok(OpenAiOperation::Embedding(Box::new(embedding_call(case)?))),
        Operation::Text => Ok(OpenAiOperation::Text(Box::new(text_call(case)?))),
    }
}

fn chat_call(case: &Case) -> Result<ChatCall, Box<HelperError>> {
    let mut body = case.optional_params.clone();
    body.insert(
        String::from("messages"),
        Value::Array(case.messages.clone()),
    );
    let encoded =
        serde_json::to_vec(&Value::Object(body)).map_err(|error| failure("ValueError", error))?;
    let request = ChatRequest::parse(&encoded).map_err(|error| failure("ValueError", error))?;
    Ok(ChatCall {
        model: case.model.clone(),
        request,
        credentials: explicit_credentials(case),
        headers: case.headers.clone(),
        extra_headers: case.extra_headers.clone(),
        timeout: None,
        metadata: metadata(case.metadata.as_ref())?,
    })
}

fn embedding_call(case: &Case) -> Result<EmbeddingCall, Box<HelperError>> {
    let encoded = serde_json::to_vec(&Value::Object(case.optional_params.clone()))
        .map_err(|error| failure("ValueError", error))?;
    let request =
        EmbeddingRequest::parse(&encoded).map_err(|error| failure("ValueError", error))?;
    Ok(EmbeddingCall {
        model: case.model.clone(),
        request,
        // the route resolved no format of its own, so the body's encoding_format names it
        encoding_format: None,
        credentials: explicit_credentials(case),
        headers: case.headers.clone(),
        timeout: None,
    })
}

fn text_call(case: &Case) -> Result<TextCall, Box<HelperError>> {
    let messages = case
        .messages
        .iter()
        .cloned()
        .map(|message| {
            serde_json::from_value(message).map_err(|error| failure("ValueError", error))
        })
        .collect::<Result<Vec<JsonValue>, Box<HelperError>>>()?;
    Ok(TextCall {
        model: case.model.clone(),
        messages,
        optional_params: case.optional_params.clone(),
        credentials: explicit_credentials(case),
        headers: merged_headers(case),
        timeout: None,
        acompletion: false,
        text_completion: case.text_completion,
    })
}

fn metadata(
    fields: Option<&Object>,
) -> Result<Option<IndexMap<String, JsonValue>>, Box<HelperError>> {
    fields.map_or_else(
        || Ok(None),
        |fields| {
            serde_json::from_value(Value::Object(fields.clone()))
                .map(Some)
                .map_err(|error| failure("ValueError", error))
        },
    )
}

// completion() takes the supplied map or else the extra one, then writes the extra map over it
fn merged_headers(case: &Case) -> Option<Headers> {
    let supplied = case.headers.as_ref().filter(|map| !map.is_empty());
    let base = supplied.or(case.extra_headers.as_ref().filter(|map| !map.is_empty()));
    let mut merged = base.cloned().unwrap_or_default();
    if let Some(extra) = &case.extra_headers {
        merged.extend(extra.clone());
    }
    (!merged.is_empty()).then_some(merged)
}
