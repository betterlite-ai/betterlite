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

fn reply_value(reply: &betterlite_openai::embedding::EmbeddingResponse) -> HelperResult {
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

pub(crate) fn execute(step: &Step) -> HelperResult {
    match step {
        Step::ChatSupportedParams => Ok(params_value(&CHAT_PARAMS)),
        Step::ChatDefaults { defaults } => {
            Ok(match VolcEngineChatConfig::try_from(defaults.clone()) {
                Ok(config) => Value::Object(config.config()),
                Err(_) => failure(),
            })
        }
        Step::ChatMap {
            params,
            optional_params,
        } => {
            let mut optional = optional_params.clone();
            Ok(match map_chat_params(params, &mut optional) {
                Ok(()) => Value::Object(optional),
                Err(_) => failure(),
            })
        }
        Step::EmbeddingSupportedParams => Ok(params_value(&EMBEDDING_PARAMS)),
        Step::EmbeddingMap {
            params,
            optional_params,
            drop_params,
        } => {
            let mut optional = optional_params.clone();
            Ok(
                match map_embedding_params(params, &mut optional, *drop_params) {
                    Ok(()) => Value::Object(optional),
                    Err(_) => failure(),
                },
            )
        }
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
        Step::EmbeddingHeaders { api_key } => Ok(match api_key {
            None => failure(),
            Some(key) => serde_json::to_value(validate_environment(
                &Headers::new(),
                Some(&SecretString::from(key.clone())),
            )?)?,
        }),
        Step::EmbeddingReply { model, body } => match decode_reply(body.as_bytes(), model) {
            Ok(reply) => reply_value(&reply),
            Err(_) => Ok(failure()),
        },
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
        } => error_value(&get_error_class(message, *status, headers.clone())),
    }
}
