use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::volcengine_chat::params::{
    SUPPORTED_OPENAI_PARAMS as CHAT_PARAMS, map_openai_params as map_chat_params,
};
use betterlite_openai::volcengine_chat::settings::VolcEngineChatConfig;
use betterlite_openai::volcengine_common::{VolcEngineError, base_url, headers as common_headers};
use betterlite_openai::volcengine_embedding::error::get_error_class;
use betterlite_openai::volcengine_embedding::params::{
    SUPPORTED_OPENAI_PARAMS as EMBEDDING_PARAMS, map_openai_params as map_embedding_params,
};
use betterlite_openai::volcengine_embedding::reply::decode_reply;
use betterlite_openai::volcengine_embedding::request::{
    get_complete_url, transform_embedding_request, validate_environment,
};
use secrecy::SecretString;
use serde_json::Value;

use crate::helpers::HelperResult;

use super::input::Step;

fn failure() -> Value {
    let mut value = Object::new();
    value.insert(String::from("ok"), Value::Bool(false));
    Value::Object(value)
}

fn reply_value(reply: &EmbeddingResponse) -> HelperResult {
    let mut value = Object::new();
    value.insert(String::from("ok"), Value::Bool(true));
    value.insert(String::from("reply"), serde_json::to_value(reply)?);
    Ok(Value::Object(value))
}

fn error_value(error: &VolcEngineError) -> HelperResult {
    let mut value = Object::new();
    value.insert(String::from("status_code"), Value::from(error.status_code));
    value.insert(
        String::from("message"),
        Value::String(error.message.clone()),
    );
    value.insert(
        String::from("headers"),
        serde_json::to_value(&error.headers)?,
    );
    Ok(Value::Object(value))
}

fn params_value(names: &[&str]) -> Value {
    Value::Array(
        names
            .iter()
            .map(|name| Value::String((*name).to_owned()))
            .collect(),
    )
}

fn chat_defaults(defaults: &Object) -> Value {
    VolcEngineChatConfig::try_from(defaults.clone())
        .map_or_else(|_| failure(), |config| Value::Object(config.config()))
}

fn chat_map(params: &Object, optional_params: &Object) -> Value {
    let mut optional = optional_params.clone();
    if map_chat_params(params, &mut optional).is_err() {
        return failure();
    }
    Value::Object(optional)
}

fn embedding_map(params: &Object, optional_params: &Object, drop_params: bool) -> Value {
    let mut optional = optional_params.clone();
    map_embedding_params(params, &mut optional, drop_params)
        .map_or_else(|_| failure(), |()| Value::Object(optional))
}

fn embedding_headers(api_key: Option<&str>) -> HelperResult {
    let secret = api_key.map(|key| SecretString::from(key.to_owned()));
    validate_environment(&Headers::new(), secret.as_ref()).map_or_else(
        |_| Ok(failure()),
        |headers| Ok(serde_json::to_value(headers)?),
    )
}

fn embedding_reply(model: &str, body: &str) -> HelperResult {
    let Ok(reply) = decode_reply(body.as_bytes(), model) else {
        return Ok(failure());
    };
    reply_value(&reply)
}

pub(crate) fn execute(step: &Step) -> HelperResult {
    match step {
        Step::ChatSupportedParams => Ok(params_value(&CHAT_PARAMS)),
        Step::ChatDefaults { defaults } => Ok(chat_defaults(defaults)),
        Step::ChatMap {
            params,
            optional_params,
        } => Ok(chat_map(params, optional_params)),
        Step::EmbeddingSupportedParams => Ok(params_value(&EMBEDDING_PARAMS)),
        Step::EmbeddingMap {
            params,
            optional_params,
            drop_params,
        } => Ok(embedding_map(params, optional_params, *drop_params)),
        Step::EmbeddingUrl { api_base } => Ok(Value::String(get_complete_url(api_base.as_deref()))),
        Step::EmbeddingBody {
            model,
            input,
            optional_params,
        } => Ok(Value::Object(transform_embedding_request(
            model,
            input,
            optional_params,
        ))),
        Step::EmbeddingHeaders { api_key } => embedding_headers(api_key.as_deref()),
        Step::EmbeddingReply { model, body } => embedding_reply(model, body),
        Step::BaseUrl { api_base } => Ok(Value::String(base_url(api_base.as_deref()))),
        Step::Headers {
            api_key,
            extra_headers,
        } => Ok(serde_json::to_value(common_headers(
            &SecretString::from(api_key.clone()),
            extra_headers,
        ))?),
        Step::ErrorClass {
            message,
            status,
            headers,
        } => error_value(&get_error_class(message, *status, headers)),
    }
}
