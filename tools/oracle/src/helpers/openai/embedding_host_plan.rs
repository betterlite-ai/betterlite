use std::collections::BTreeMap;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::embedding_host_settings::{Raw, Settings};
use betterlite_openai::infinity_embedding::error::{InfinityEmbeddingError, SYNTHETIC_REQUEST_URL};
use betterlite_openai::infinity_embedding::{self, decode_infinity_embedding};
use betterlite_openai::jina_embedding;
use betterlite_openai::openai_base_config::{Headers, Object};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

type Failed = Box<crate::helpers::HelperError>;

#[derive(Deserialize)]
pub(crate) struct Input {
    jina: Call,
    infinity: Call,
}

#[derive(Deserialize)]
struct Call {
    model: String,
    input: Value,
    #[serde(default)]
    options: Object,
    #[serde(default)]
    optional: Object,
    base: Option<String>,
    api_key: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    headers: Headers,
    reply: Option<Reply>,
    error: Option<Failure>,
}

#[derive(Deserialize)]
struct Reply {
    body: String,
    status: u16,
}

#[derive(Deserialize)]
struct Failure {
    status: u16,
    message: String,
    #[serde(default)]
    headers: Headers,
}

fn failure(class: &str, status: Option<u16>, message: String) -> Failed {
    Box::new(crate::helpers::HelperError {
        class: class.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        Ok(json!({"jina": jina(&input.jina)?, "infinity": infinity(&input.infinity)?}))
    }
}

fn settings(call: &Call) -> Result<Settings, Failed> {
    Raw::default()
        .convert(&call.env)
        .map_err(|error| failure("ConfigError", None, error.to_string()))
}

fn jina_reply(
    call: &Call,
    key: Option<&SecretString>,
    request: &Object,
    hook: &mut Vec<Value>,
) -> Result<Option<Value>, Failed> {
    call.reply
        .as_ref()
        .map(|reply| {
            let response = jina_embedding::transform_embedding_response(
                reply.body.as_bytes(),
                key,
                request,
                |observed| {
                    hook.push(json!({
                        "input_is_builtin": observed.input == jina_embedding::BuiltinInput,
                        "api_key": observed.api_key.map(|secret| secret.expose_secret().to_owned()),
                        "complete_input_dict": observed.complete_input_dict,
                        "original_response": observed.original_response,
                    }));
                },
            )
            .map_err(|error| failure("JSONDecodeError", None, error.to_string()))?;
            Ok(json!({
                "model": response.model, "object": response.object, "data": response.data,
                "usage": {
                    "prompt_tokens": response.usage.prompt_tokens,
                    "total_tokens": response.usage.total_tokens,
                },
            }))
        })
        .transpose()
}

fn jina(call: &Call) -> crate::helpers::HelperResult {
    let key = call.api_key.clone().map(SecretString::from);
    let credentials =
        jina_embedding::resolve_credentials(call.base.as_deref(), key.as_ref(), &settings(call)?);
    let resolved_key = credentials.api_key.as_ref();
    let mut optional = call.optional.clone();
    jina_embedding::map_openai_params(&call.options, &mut optional);
    let request = jina_embedding::transform_embedding_request(&call.model, &call.input, &optional);
    let mut hook = Vec::new();
    let response = jina_reply(call, resolved_key, &request, &mut hook)?;
    Ok(json!({
        "api_base": credentials.api_base,
        "api_key": resolved_key.map(|secret| secret.expose_secret().to_owned()),
        "url": jina_embedding::get_complete_url(call.base.as_deref()),
        "headers": jina_embedding::validate_environment(&call.headers, resolved_key),
        "request": request,
        "response": response,
        "hook": hook,
    }))
}

fn infinity_reply(call: &Call) -> Result<Option<Value>, Failed> {
    let decoded = call
        .reply
        .as_ref()
        .map(|reply| decode_infinity_embedding(reply.body.as_bytes(), reply.status))
        .transpose();
    match decoded {
        Ok(response) => Ok(response.map(|reply| {
            json!({
                "model": reply.model, "object": reply.object, "data": reply.data,
                "usage": {
                    "prompt_tokens": reply.usage.prompt_tokens,
                    "total_tokens": reply.usage.total_tokens,
                },
            })
        })),
        Err(InfinityEmbeddingError::Json {
            status_code,
            message,
            ..
        }) => Err(failure("InfinityError", Some(status_code), message)),
        Err(
            error @ (InfinityEmbeddingError::MissingApiBase
            | InfinityEmbeddingError::Provider { .. }
            | InfinityEmbeddingError::Shape { .. }),
        ) => Err(failure("ValidationError", None, error.to_string())),
    }
}

fn infinity(call: &Call) -> crate::helpers::HelperResult {
    let key = call.api_key.clone().map(SecretString::from);
    let url = infinity_embedding::get_complete_url(call.base.as_deref())
        .map_err(|error| failure("ValueError", None, error.to_string()))?;
    let mut optional = call.optional.clone();
    infinity_embedding::map_openai_params(&call.options, &mut optional);
    let error = call.error.as_ref().map(|supplied| {
        json!({
            "status_code": supplied.status, "message": supplied.message,
            "headers": supplied.headers, "url": SYNTHETIC_REQUEST_URL,
        })
    });
    Ok(json!({
        "url": url,
        "headers": infinity_embedding::validate_environment(
            &call.headers, key.as_ref(), &settings(call)?),
        "request": infinity_embedding::transform_embedding_request(
            &call.model, &call.input, &optional),
        "response": infinity_reply(call)?,
        "error": error,
    }))
}

#[path = "embedding_host_plan/tests.rs"]
mod tests;
