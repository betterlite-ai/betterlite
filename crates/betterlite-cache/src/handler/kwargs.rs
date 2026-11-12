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
    match kwargs.get(CACHE_KEY_KWARG) {
        None | Some(KeyValue::Null) => Ok(None),
        Some(KeyValue::String(key)) => Ok(Some(key.clone())),
        Some(_) => Err(HandlerError::FieldType {
            field: "cache_key",
            expected: "a string",
        }),
    }
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
    let mut params = Vec::new();
    for (name, value) in kwargs {
        match name.as_str() {
            METADATA_KWARG | LITELLM_METADATA_KWARG | LITELLM_PARAMS_KWARG | CACHE_KWARG => {}
            _ => params.push(KeyParam {
                name: name.clone(),
                value: value.clone(),
            }),
        }
    }
    Ok(CacheKeyRequest {
        params,
        metadata: optional_object(kwargs.get(METADATA_KWARG), "metadata")?,
        litellm_metadata: optional_object(kwargs.get(LITELLM_METADATA_KWARG), "litellm_metadata")?,
        litellm_params: match kwargs.get(LITELLM_PARAMS_KWARG) {
            None | Some(KeyValue::Null) => None,
            Some(KeyValue::Object(fields)) => Some(litellm_params(fields)?),
            Some(_) => {
                return Err(HandlerError::FieldType {
                    field: "litellm_params",
                    expected: "an object",
                });
            }
        },
        cache_control: match cache_fields(kwargs)? {
            None => None,
            Some(fields) => Some(request_cache_control(fields)?),
        },
    })
}

pub fn request_control(kwargs: &KeyObject) -> Result<DynamicCacheControl, HandlerError> {
    match cache_fields(kwargs)? {
        None => Ok(DynamicCacheControl::default()),
        Some(fields) => Ok(DynamicCacheControl {
            ttl: optional_integer(fields.get("ttl"), "cache.ttl")?,
            namespace: optional_string(fields.get("namespace"), "cache.namespace")?,
            s_maxage: optional_integer(fields.get("s-maxage"), "cache.s-maxage")?,
            s_max_age: optional_integer(fields.get("s-max-age"), "cache.s-max-age")?,
            no_cache: optional_bool(fields.get("no-cache"), "cache.no-cache")?,
            no_store: optional_bool(fields.get("no-store"), "cache.no-store")?,
        }),
    }
}

// LiteLLM's _should_store_result_in_cache reads cache.no-store with a False default and compares it to True
pub fn no_store_requested(kwargs: &KeyObject) -> Result<bool, HandlerError> {
    match cache_fields(kwargs)? {
        None => Ok(false),
        Some(fields) => {
            let flag = optional_bool(fields.get("no-store"), "cache.no-store")?;
            Ok(flag == Some(true))
        }
    }
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

fn cache_fields(kwargs: &KeyObject) -> Result<Option<&KeyObject>, HandlerError> {
    match kwargs.get(CACHE_KWARG) {
        None | Some(KeyValue::Null) => Ok(None),
        Some(KeyValue::Object(fields)) => Ok(Some(fields)),
        Some(_) => Err(HandlerError::FieldType {
            field: "cache",
            expected: "an object",
        }),
    }
}

fn optional_object(
    value: Option<&KeyValue>,
    field: &'static str,
) -> Result<Option<KeyObject>, HandlerError> {
    match value {
        None | Some(KeyValue::Null) => Ok(None),
        Some(KeyValue::Object(fields)) => Ok(Some(fields.clone())),
        Some(_) => Err(HandlerError::FieldType {
            field,
            expected: "an object",
        }),
    }
}

fn litellm_params(fields: &KeyObject) -> Result<LiteLlmParams, HandlerError> {
    Ok(LiteLlmParams {
        preset_cache_key: match fields.get("preset_cache_key") {
            None | Some(KeyValue::Null) => None,
            Some(KeyValue::String(key)) => Some(key.clone()),
            Some(_) => {
                return Err(HandlerError::FieldType {
                    field: "litellm_params.preset_cache_key",
                    expected: "a string",
                });
            }
        },
        metadata: optional_object(fields.get("metadata"), "litellm_params.metadata")?,
        litellm_metadata: optional_object(
            fields.get("litellm_metadata"),
            "litellm_params.litellm_metadata",
        )?,
        file_name: match fields.get("file_name") {
            None | Some(KeyValue::Null) => None,
            Some(value) => Some(value.clone()),
        },
    })
}

fn request_cache_control(fields: &KeyObject) -> Result<RequestCacheControl, HandlerError> {
    Ok(RequestCacheControl {
        namespace: optional_string(fields.get("namespace"), "cache.namespace")?,
        use_cache: match fields.get("use-cache") {
            None | Some(KeyValue::Null) => false,
            Some(KeyValue::Bool(flag)) => *flag,
            Some(_) => {
                return Err(HandlerError::FieldType {
                    field: "cache.use-cache",
                    expected: "a boolean",
                });
            }
        },
    })
}

fn optional_integer(
    value: Option<&KeyValue>,
    field: &'static str,
) -> Result<Option<i64>, HandlerError> {
    match value {
        None | Some(KeyValue::Null) => Ok(None),
        Some(KeyValue::Number(number)) => number.as_i64().map_or_else(
            || {
                Err(HandlerError::FieldType {
                    field,
                    expected: "an integer",
                })
            },
            |integer| Ok(Some(integer)),
        ),
        Some(_) => Err(HandlerError::FieldType {
            field,
            expected: "an integer",
        }),
    }
}

fn optional_string(
    value: Option<&KeyValue>,
    field: &'static str,
) -> Result<Option<String>, HandlerError> {
    match value {
        None | Some(KeyValue::Null) => Ok(None),
        Some(KeyValue::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(HandlerError::FieldType {
            field,
            expected: "a string",
        }),
    }
}

const fn optional_bool(
    value: Option<&KeyValue>,
    field: &'static str,
) -> Result<Option<bool>, HandlerError> {
    match value {
        None | Some(KeyValue::Null) => Ok(None),
        Some(KeyValue::Bool(flag)) => Ok(Some(*flag)),
        Some(_) => Err(HandlerError::FieldType {
            field,
            expected: "a boolean",
        }),
    }
}
