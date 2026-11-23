use super::convert::EmbeddingArguments;
use super::error::{CallFailure, Raised, raised};
use betterlite_context::embedding_optional_params::embedding_optional_params;
use betterlite_context::embedding_optional_params::error::EmbeddingParamsError;
use betterlite_context::embedding_optional_params::request::EmbeddingRequest as ParamsRequest;
use betterlite_context::non_default_params::Params;
use betterlite_context::non_default_params::filters::filter_out_litellm_params;
use betterlite_context::non_default_params::mapper::DropPolicy;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::embedding::{EmbeddingInput, EmbeddingRequest, EncodingFormat};
use serde_json::Number;
use std::collections::BTreeMap;
// main.py's default_params: never a provider argument, so a kwarg of this name stays out of the body
const EXCLUDED: [&str; 15] = [
    "user",
    "dimensions",
    "request_timeout",
    "api_base",
    "api_version",
    "api_key",
    "deployment_id",
    "organization",
    "base_url",
    "default_headers",
    "timeout",
    "max_retries",
    "encoding_format",
    "aembedding",
    "extra_headers",
];
const ENCODING_ENV: &str = "LITELLM_DEFAULT_EMBEDDING_ENCODING_FORMAT";
// get_optional_params_embeddings raises this text, and main.py's exception_type keeps it verbatim
const DIMENSIONS_MESSAGE: &str = "Setting dimensions is not supported for OpenAI `text-embedding-3` and later models. To drop it from the call, set `litellm.drop_params = True`.";
// The request LiteLLM's OpenAI embedding handler builds: typed fields plus every other body field.
pub(crate) fn build(
    arguments: &EmbeddingArguments,
    model: &str,
    provider: &str,
    litellm_drop_params: bool,
    environment: &BTreeMap<String, String>,
) -> Result<EmbeddingRequest, CallFailure> {
    let mut params = optional_params(arguments, model, provider, litellm_drop_params)?;
    let requested = requested_encoding(arguments.encoding_format.as_deref(), &params, environment);
    params.remove("encoding_format");
    // the mapping keeps or drops the caller's own value, so its presence decides and the typed argument fills it
    let dimensions = params.remove("dimensions").and(arguments.dimensions);
    let user = params.remove("user").and(arguments.user.clone());
    let mut fields = body_fields(&mut params)?;
    let encoding_format = encoding_field(requested, &mut fields);
    let value =
        serde_json::to_value(fields).map_err(|error| CallFailure::Invalid(error.to_string()))?;
    let passthrough =
        serde_json::from_value(value).map_err(|error| CallFailure::Invalid(error.to_string()))?;
    Ok(EmbeddingRequest {
        model: Some(model.to_owned()),
        input: arguments
            .input
            .clone()
            .unwrap_or_else(|| EmbeddingInput::Texts(Vec::new())),
        encoding_format,
        dimensions,
        user,
        passthrough,
    })
}
// main.py's non_default_params run through get_optional_params_embeddings for the resolved provider
fn optional_params(
    arguments: &EmbeddingArguments,
    model: &str,
    provider: &str,
    litellm_drop_params: bool,
) -> Result<Params, CallFailure> {
    let mut kwargs = Params::new();
    for (name, value) in &arguments.extra {
        kwargs.insert(name.clone(), json_value(value)?);
    }
    let filtered = filter_out_litellm_params(&kwargs, &EXCLUDED);
    let passed = passed_params(arguments, model, &filtered);
    let allowed = arguments.allowed_openai_params.clone().unwrap_or_default();
    let additional_drop = arguments.additional_drop_params.clone().unwrap_or_default();
    embedding_optional_params(&ParamsRequest {
        model,
        custom_llm_provider: provider,
        passed_params: &passed,
        special_params: &filtered,
        additional_drop_params: &additional_drop,
        allowed_openai_params: &allowed,
        drop_policy: DropPolicy {
            drop_params: arguments.drop_params,
            litellm_drop_params,
        },
        provider_config: None,
        mapper: None,
        supported: None,
    })
    .map_err(|error| params_failure(&error))
}
// get_optional_params_embeddings' locals(): the named arguments plus the kwargs it was handed
fn passed_params(arguments: &EmbeddingArguments, model: &str, filtered: &Params) -> Params {
    let mut passed = filtered.clone();
    passed.insert(String::from("model"), JsonValue::Text(model.to_owned()));
    if let Some(user) = &arguments.user {
        passed.insert(String::from("user"), JsonValue::Text(user.clone()));
    }
    if let Some(dimensions) = arguments.dimensions {
        passed.insert(
            String::from("dimensions"),
            JsonValue::Number(Number::from(dimensions)),
        );
    }
    if let Some(format) = &arguments.encoding_format {
        passed.insert(
            String::from("encoding_format"),
            JsonValue::Text(format.clone()),
        );
    }
    passed
}
fn params_failure(error: &EmbeddingParamsError) -> CallFailure {
    match error {
        EmbeddingParamsError::DimensionsUnsupported { .. } => raised(
            Raised::UnsupportedParams,
            String::from(DIMENSIONS_MESSAGE),
            None,
            None,
        ),
        EmbeddingParamsError::Unsupported { .. } => {
            raised(Raised::UnsupportedParams, error.to_string(), None, None)
        }
        EmbeddingParamsError::WrongType(..) => {
            raised(Raised::BadRequest, error.to_string(), None, None)
        }
        EmbeddingParamsError::MissingMapper { .. }
        | EmbeddingParamsError::Params(_)
        | EmbeddingParamsError::Mapper(_) => {
            raised(Raised::InternalServer, error.to_string(), None, None)
        }
    }
}
// encoding_format or the one the params carry or LITELLM_DEFAULT_EMBEDDING_ENCODING_FORMAT, dropping "none"
fn requested_encoding(
    explicit: Option<&str>,
    params: &Params,
    environment: &BTreeMap<String, String>,
) -> Option<String> {
    let from_params = match params.get("encoding_format") {
        Some(JsonValue::Text(text)) if !text.is_empty() => Some(text.clone()),
        Some(
            JsonValue::Text(_)
            | JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        )
        | None => None,
    };
    let requested = explicit
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or(from_params)
        .or_else(|| {
            environment
                .get(ENCODING_ENV)
                .filter(|value| !value.is_empty())
                .cloned()
        })?;
    if requested.trim().to_lowercase() == "none" {
        return None;
    }
    Some(requested)
}
fn encoding_field(
    requested: Option<String>,
    fields: &mut BTreeMap<String, Json>,
) -> Option<EncodingFormat> {
    match requested {
        Some(text) if text == "float" => Some(EncodingFormat::Float),
        Some(text) if text == "base64" => Some(EncodingFormat::Base64),
        Some(text) => {
            fields.insert(String::from("encoding_format"), Json::Text(text));
            None
        }
        None => None,
    }
}
// the OpenAI handler's raw post spreads extra_body into the body; every other optional param is a body field
fn body_fields(params: &mut Params) -> Result<BTreeMap<String, Json>, CallFailure> {
    let mut fields: BTreeMap<String, Json> = BTreeMap::new();
    if let Some(JsonValue::Object(body)) = params.remove("extra_body") {
        for (name, value) in body {
            fields.insert(name, json_from_value(&value)?);
        }
    }
    for (name, value) in params.iter() {
        fields.insert(name.clone(), json_from_value(value)?);
    }
    Ok(fields)
}
fn json_value(value: &Json) -> Result<JsonValue, CallFailure> {
    serde_json::to_string(value)
        .and_then(|text| serde_json::from_str::<JsonValue>(&text))
        .map_err(|error| CallFailure::Invalid(error.to_string()))
}
fn json_from_value(value: &JsonValue) -> Result<Json, CallFailure> {
    serde_json::to_string(value)
        .and_then(|text| serde_json::from_str::<Json>(&text))
        .map_err(|error| CallFailure::Invalid(error.to_string()))
}
