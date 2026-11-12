use crate::cache_key::request::{CacheKeyRequest, KeyParam, LiteLlmParams, RequestCacheControl};
use crate::cache_key::value::{KeyObject, KeyValue};
use crate::types::control::DynamicCacheControl;

use super::HandlerError;

pub const LOGGING_OBJECT_KWARG: &str = "litellm_logging_obj";
pub const CACHE_KEY_KWARG: &str = "cache_key";
pub const INPUT_KWARG: &str = "input";
const METADATA_KWARG: &str = "metadata";
const LITELLM_METADATA_KWARG: &str = "litellm_metadata";
const LITELLM_PARAMS_KWARG: &str = "litellm_params";
const CACHE_KWARG: &str = "cache";
const STREAM_KWARG: &str = "stream";

// LiteLLM's _drop_logging_obj_from_kwargs: the handler lives on the logging object, so keeping it here closes a reference cycle
#[must_use]
pub fn drop_logging_obj_from_kwargs(mut request_kwargs: KeyObject) -> KeyObject {
    request_kwargs.shift_remove(LOGGING_OBJECT_KWARG);
    request_kwargs
}

// LiteLLM's convert_args_to_kwargs: positional arguments take the parameter names of the original function's signature
#[must_use]
pub fn convert_args_to_kwargs(parameters: &[String], args: &[KeyValue]) -> KeyObject {
    let mut projected = KeyObject::new();
    for (name, value) in parameters.iter().zip(args) {
        projected.insert(name.clone(), value.clone());
    }
    projected
}

pub fn request_cache_key(kwargs: &KeyObject) -> Result<Option<String>, HandlerError> {
    optional_field(
        kwargs.get(CACHE_KEY_KWARG),
        "cache_key",
        "a string",
        string_of,
    )
}

pub fn request_stream(kwargs: &KeyObject) -> Result<bool, HandlerError> {
    match kwargs.get(STREAM_KWARG) {
        None | Some(KeyValue::Null) => Ok(false),
        Some(KeyValue::Bool(stream)) => Ok(*stream),
        Some(_) => Err(HandlerError::FieldType {
            field: "stream",
            expected: "a boolean",
        }),
    }
}

pub fn remove_null_metadata(kwargs: &mut KeyObject) {
    if matches!(kwargs.get(METADATA_KWARG), None | Some(KeyValue::Null)) {
        kwargs.shift_remove(METADATA_KWARG);
    }
}

// one key request from the flat request kwargs: the cache dict, metadata and litellm_params are read by name, the rest keep request order
pub fn key_request(kwargs: &KeyObject) -> Result<CacheKeyRequest, HandlerError> {
    let params = params_of(kwargs);
    let metadata = optional_field(
        kwargs.get(METADATA_KWARG),
        "metadata",
        "an object",
        object_of,
    )?;
    let litellm_metadata = optional_field(
        kwargs.get(LITELLM_METADATA_KWARG),
        "litellm_metadata",
        "an object",
        object_of,
    )?;
    let litellm_params = optional_field(
        kwargs.get(LITELLM_PARAMS_KWARG),
        "litellm_params",
        "an object",
        object_of,
    )?
    .map(|fields| litellm_params(&fields))
    .transpose()?;
    let cache_control = optional_field(kwargs.get(CACHE_KWARG), "cache", "an object", object_of)?
        .map(|fields| request_cache_control(&fields))
        .transpose()?;
    Ok(CacheKeyRequest {
        params,
        metadata,
        litellm_metadata,
        litellm_params,
        cache_control,
    })
}

pub fn request_control(kwargs: &KeyObject) -> Result<DynamicCacheControl, HandlerError> {
    let Some(fields) = optional_field(kwargs.get(CACHE_KWARG), "cache", "an object", object_of)?
    else {
        return Ok(DynamicCacheControl::default());
    };
    Ok(DynamicCacheControl {
        ttl: optional_field(fields.get("ttl"), "cache.ttl", "an integer", integer_of)?,
        namespace: optional_field(
            fields.get("namespace"),
            "cache.namespace",
            "a string",
            string_of,
        )?,
        s_maxage: optional_field(
            fields.get("s-maxage"),
            "cache.s-maxage",
            "an integer",
            integer_of,
        )?,
        s_max_age: optional_field(
            fields.get("s-max-age"),
            "cache.s-max-age",
            "an integer",
            integer_of,
        )?,
        no_cache: optional_field(
            fields.get("no-cache"),
            "cache.no-cache",
            "a boolean",
            bool_of,
        )?,
        no_store: optional_field(
            fields.get("no-store"),
            "cache.no-store",
            "a boolean",
            bool_of,
        )?,
    })
}

// LiteLLM's _should_store_result_in_cache reads cache.no-store with a False default and compares it to True
pub fn no_store_requested(kwargs: &KeyObject) -> Result<bool, HandlerError> {
    let Some(fields) = optional_field(kwargs.get(CACHE_KWARG), "cache", "an object", object_of)?
    else {
        return Ok(false);
    };
    let no_store = optional_field(
        fields.get("no-store"),
        "cache.no-store",
        "a boolean",
        bool_of,
    )?;
    Ok(no_store == Some(true))
}

pub fn set_input_param(request: &mut CacheKeyRequest, input: &KeyValue) {
    match request
        .params
        .iter_mut()
        .find(|param| param.name == INPUT_KWARG)
    {
        Some(param) => param.value = input.clone(),
        None => request.params.push(KeyParam {
            name: INPUT_KWARG.to_owned(),
            value: input.clone(),
        }),
    }
}

// B4298 keys every embedding input on its own input, so a preset key must not stand in for it
pub fn clear_preset_key(request: &mut CacheKeyRequest) {
    if let Some(params) = request.litellm_params.as_mut() {
        params.preset_cache_key = None;
    }
}

fn params_of(kwargs: &KeyObject) -> Vec<KeyParam> {
    kwargs
        .iter()
        .filter(|(name, _value)| {
            !matches!(
                name.as_str(),
                METADATA_KWARG | LITELLM_METADATA_KWARG | LITELLM_PARAMS_KWARG | CACHE_KWARG
            )
        })
        .map(|(name, value)| KeyParam {
            name: name.clone(),
            value: value.clone(),
        })
        .collect()
}

fn litellm_params(fields: &KeyObject) -> Result<LiteLlmParams, HandlerError> {
    let preset = optional_field(
        fields.get("preset_cache_key"),
        "litellm_params.preset_cache_key",
        "a string",
        string_of,
    )?;
    let metadata = optional_field(
        fields.get("metadata"),
        "litellm_params.metadata",
        "an object",
        object_of,
    )?;
    let litellm_metadata = optional_field(
        fields.get("litellm_metadata"),
        "litellm_params.litellm_metadata",
        "an object",
        object_of,
    )?;
    Ok(LiteLlmParams {
        preset_cache_key: preset,
        metadata,
        litellm_metadata,
        file_name: optional_file_name(fields.get("file_name")),
    })
}

fn request_cache_control(fields: &KeyObject) -> Result<RequestCacheControl, HandlerError> {
    let namespace = optional_field(
        fields.get("namespace"),
        "cache.namespace",
        "a string",
        string_of,
    )?;
    let use_cache = optional_field(
        fields.get("use-cache"),
        "cache.use-cache",
        "a boolean",
        bool_of,
    )?;
    Ok(RequestCacheControl {
        namespace,
        use_cache: use_cache.unwrap_or(false),
    })
}

fn optional_file_name(value: Option<&KeyValue>) -> Option<KeyValue> {
    match value {
        None | Some(KeyValue::Null) => None,
        Some(present) => Some(present.clone()),
    }
}

fn optional_field<Value, Extract>(
    value: Option<&KeyValue>,
    field: &'static str,
    expected: &'static str,
    extract: Extract,
) -> Result<Option<Value>, HandlerError>
where
    Extract: FnOnce(&KeyValue) -> Option<Value>,
{
    match value {
        None | Some(KeyValue::Null) => Ok(None),
        Some(present) => extract(present)
            .map(Some)
            .ok_or(HandlerError::FieldType { field, expected }),
    }
}

fn integer_of(value: &KeyValue) -> Option<i64> {
    if let KeyValue::Number(number) = value {
        return number.as_i64();
    }
    None
}

fn string_of(value: &KeyValue) -> Option<String> {
    if let KeyValue::String(text) = value {
        return Some(text.clone());
    }
    None
}

const fn bool_of(value: &KeyValue) -> Option<bool> {
    if let KeyValue::Bool(flag) = value {
        return Some(*flag);
    }
    None
}

fn object_of(value: &KeyValue) -> Option<KeyObject> {
    if let KeyValue::Object(fields) = value {
        return Some(fields.clone());
    }
    None
}
