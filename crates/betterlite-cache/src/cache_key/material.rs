use betterlite_context::owned_parameter_names::is_litellm_owned_kwarg;
use betterlite_models::projections::parameter_names::api_parameter_names;

use super::error::CacheKeyError;
use super::request::{CacheKeyRequest, KeyParam};
use super::scope::{SEMANTIC_EXCLUDED_PARAMS, tenant_scope};
use super::value::{KeyObject, KeyValue, field};
use crate::types::semantic::SemanticCacheScope;

pub(crate) fn build(
    request: &CacheKeyRequest,
    include_provider_params: bool,
    semantic: Option<SemanticCacheScope>,
) -> Result<String, CacheKeyError> {
    let canonical = api_parameter_names();
    let mut material = String::new();
    for param in &request.params {
        if semantic.is_some() && SEMANTIC_EXCLUDED_PARAMS.contains(&param.name.as_str()) {
            continue;
        }
        if canonical.contains(param.name.as_str()) {
            let Some(value) = param_value(request, param)? else {
                continue;
            };
            push_param(&mut material, &param.name, &value);
            continue;
        }
        if include_provider_params
            && !is_litellm_owned_kwarg(&param.name)
            && !matches!(param.value, KeyValue::Null)
        {
            push_param(&mut material, &param.name, &param.value);
        }
    }
    if let Some(scope) = semantic {
        material.push_str(&tenant_scope(request, scope));
    }
    Ok(material)
}

fn push_param(material: &mut String, name: &str, value: &KeyValue) {
    material.push_str(name);
    material.push_str(": ");
    material.push_str(&value.render());
}

fn param_value(
    request: &CacheKeyRequest,
    param: &KeyParam,
) -> Result<Option<KeyValue>, CacheKeyError> {
    match param.name.as_str() {
        // A request routed through a caching group keys on the group, then on the router's model group, and last on the model name.
        "model" => {
            let litellm_params_metadata = request
                .litellm_params
                .as_ref()
                .and_then(|params| params.metadata.as_ref());
            let model_group = field(request.metadata.as_ref(), "model_group")
                .or_else(|| field(litellm_params_metadata, "model_group"));
            let caching_group = caching_group(request.metadata.as_ref(), model_group)?;
            Ok(Some(
                caching_group
                    .or_else(|| model_group.cloned())
                    .unwrap_or_else(|| param.value.clone()),
            ))
        }
        "file" => Ok(file_value(request, &param.value)),
        _ => match &param.value {
            KeyValue::Null => Ok(None),
            KeyValue::Bool(_)
            | KeyValue::Number(_)
            | KeyValue::String(_)
            | KeyValue::Array(_)
            | KeyValue::Object(_) => Ok(Some(param.value.clone())),
        },
    }
}

// LiteLLM returns str(group) for the first caching group that holds the model group.
fn caching_group(
    metadata: Option<&KeyObject>,
    model_group: Option<&KeyValue>,
) -> Result<Option<KeyValue>, CacheKeyError> {
    let Some(caching_groups) = field(metadata, "caching_groups") else {
        return Ok(None);
    };
    let KeyValue::Array(groups) = caching_groups else {
        return Err(CacheKeyError::CachingGroups);
    };
    for group in groups {
        let KeyValue::Array(members) = group else {
            return Err(CacheKeyError::CachingGroups);
        };
        if members.iter().any(|member| model_group == Some(member)) {
            return Ok(Some(group.clone()));
        }
    }
    Ok(None)
}

// Transcription caching identifies the upload by checksum first, then the file object's name, then a caller-supplied file name.
fn file_value(request: &CacheKeyRequest, param_value: &KeyValue) -> Option<KeyValue> {
    let file_name = match param_value {
        KeyValue::String(_) => Some(param_value.clone()),
        KeyValue::Null
        | KeyValue::Bool(_)
        | KeyValue::Number(_)
        | KeyValue::Array(_)
        | KeyValue::Object(_) => None,
    };
    let candidates = [
        field(request.metadata.as_ref(), "file_checksum").cloned(),
        file_name,
        field(request.metadata.as_ref(), "file_name").cloned(),
        request
            .litellm_params
            .as_ref()
            .and_then(|params| params.file_name.clone()),
    ];
    candidates.into_iter().flatten().next()
}
