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

fn project(model: &Value, object: &Value, data: &Value, prompt: u32, total: u32) -> Value {
    json!({
        "model": model, "object": object, "data": data,
        "usage": {"prompt_tokens": prompt, "total_tokens": total},
    })
}

fn jina_reply(
    call: &Call,
    key: Option<&SecretString>,
    request: &Object,
    hook: &mut Vec<Value>,
) -> Result<Option<Value>, Failed> {
    let Some(reply) = &call.reply else {
        return Ok(None);
    };
    let response =
        jina_embedding::transform_embedding_response(reply.body.as_bytes(), key, request, |seen| {
            hook.push(json!({
                "input_is_builtin": seen.input == jina_embedding::BuiltinInput,
                "api_key": seen.api_key.map(|secret| secret.expose_secret().to_owned()),
                "complete_input_dict": seen.complete_input_dict,
                "original_response": seen.original_response,
            }));
        })
        .map_err(|error| failure("JSONDecodeError", None, error.to_string()))?;
    Ok(Some(project(
        &json!(response.model),
        &json!(response.object),
        &json!(response.data),
        response.usage.prompt_tokens,
        response.usage.total_tokens,
    )))
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
        Ok(reply) => Ok(reply.map(|parsed| {
            project(
                &json!(parsed.model),
                &json!(parsed.object),
                &json!(parsed.data),
                parsed.usage.prompt_tokens,
                parsed.usage.total_tokens,
            )
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

fn infinity_request(call: &Call) -> Object {
    let mut optional = call.optional.clone();
    infinity_embedding::map_openai_params(&call.options, &mut optional);
    infinity_embedding::transform_embedding_request(&call.model, &call.input, &optional)
}

fn carried_error(supplied: &Failure) -> Result<Value, Failed> {
    match infinity_embedding::error::get_error_class(
        &supplied.message,
        supplied.status,
        &supplied.headers,
    ) {
        InfinityEmbeddingError::Provider {
            status_code,
            message,
            headers,
        } => Ok(json!({
            "status_code": status_code, "message": message,
            "headers": headers, "url": SYNTHETIC_REQUEST_URL,
        })),
        other @ (InfinityEmbeddingError::MissingApiBase
        | InfinityEmbeddingError::Json { .. }
        | InfinityEmbeddingError::Shape { .. }) => {
            Err(failure("InfinityError", None, other.to_string()))
        }
    }
}

fn infinity(call: &Call) -> crate::helpers::HelperResult {
    let key = call.api_key.clone().map(SecretString::from);
    let headers =
        infinity_embedding::validate_environment(&call.headers, key.as_ref(), &settings(call)?);
    let carried = call.error.as_ref().map(carried_error).transpose()?;
    let url = infinity_embedding::get_complete_url(call.base.as_deref())
        .map_err(|error| failure("ValueError", None, error.to_string()))?;
    Ok(json!({
        "url": url,
        "headers": headers,
        "request": infinity_request(call),
        "response": infinity_reply(call)?,
        "error": carried,
    }))
}

#[path = "embedding_host_plan/tests.rs"]
mod tests;
