use betterlite_openai::embedding::EmbeddingRequest;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use betterlite_provider::azure_dispatch::call::{
    ChatCall, Credentials, EmbeddingCall, Request, TextCall,
};
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::case::{Case, Operation, failure};

pub(crate) fn decode<T>(value: &Value) -> Result<T, Box<HelperError>>
where
    T: DeserializeOwned,
{
    serde_json::from_value(value.clone()).map_err(|error| failure("ValueError", error))
}

fn decode_optional<T>(value: Option<&Value>) -> Result<Option<T>, Box<HelperError>>
where
    T: DeserializeOwned,
{
    let Some(named) = value else {
        return Ok(None);
    };
    Ok(Some(decode(named)?))
}

fn messages(case: &Case) -> Result<Vec<Message>, Box<HelperError>> {
    serde_json::from_value(Value::Array(case.messages.clone()))
        .map_err(|error| failure("ValueError", error))
}

fn optional(case: &Case) -> Result<Object, Box<HelperError>> {
    let Some(value) = case.optional_params.as_ref() else {
        return Ok(Object::new());
    };
    decode(value)
}

// One provider call built the way main.py reads its own arguments; every credential comes from the case.
pub(crate) fn request(case: &Case) -> Result<Request, Box<HelperError>> {
    let credentials = Credentials {
        api_base: case.api_base.clone(),
        api_version: case.api_version.clone(),
        api_key: case.api_key.clone().map(SecretString::from),
        headers: case.headers.clone(),
        extra_headers: case.extra_headers.clone(),
        max_retries: decode_optional(case.max_retries.as_ref())?,
        extra_body: decode_optional(case.extra_body.as_ref())?,
        azure_ad_token: case.azure_ad_token.clone().map(SecretString::from),
        base_model: None,
        timeout: None,
    };
    match case.operation {
        Operation::Chat => Ok(Request::Chat(Box::new(ChatCall {
            model: case.model.clone(),
            client_model: None,
            messages: messages(case)?,
            optional: optional(case)?,
            credentials,
            stream: false,
            supplied: None,
        }))),
        Operation::Embedding => embedding_request(case, credentials),
        Operation::Text => Ok(Request::Text(Box::new(TextCall {
            model: case.model.clone(),
            messages: messages(case)?,
            optional: optional(case)?,
            credentials,
            acompletion: case.acompletion,
        }))),
    }
}

fn embedding_request(case: &Case, credentials: Credentials) -> Result<Request, Box<HelperError>> {
    let Some(value) = case.embedding.as_ref() else {
        return Err(failure("ValueError", "an embedding case names its body"));
    };
    let body: EmbeddingRequest = decode(value)?;
    Ok(Request::Embedding(Box::new(EmbeddingCall {
        model: case.model.clone(),
        request: body,
        credentials,
    })))
}
