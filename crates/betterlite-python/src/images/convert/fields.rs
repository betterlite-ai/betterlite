use super::{ConvertError, DEFAULT_GENERATION_MODEL, OPENAI_PROVIDER};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use betterlite_sdk::call_setup::RequestMap;
use std::time::Duration;

pub(crate) fn text(arguments: &RequestMap, name: &str) -> Result<Option<String>, ConvertError> {
    match arguments.get(name) {
        Some(Json::Text(value)) => Ok(Some(value.clone())),
        Some(Json::Null) | None => Ok(None),
        Some(_other) => Err(field(name, "text")),
    }
}

pub(crate) fn boolean(arguments: &RequestMap, name: &str) -> Result<Option<bool>, ConvertError> {
    match arguments.get(name) {
        Some(Json::Bool(value)) => Ok(Some(*value)),
        Some(Json::Null) | None => Ok(None),
        Some(_other) => Err(field(name, "a boolean")),
    }
}

pub(crate) fn strings(arguments: &RequestMap, name: &str) -> Result<Vec<String>, ConvertError> {
    let Some(value) = arguments.get(name) else {
        return Ok(Vec::new());
    };
    let Json::List(items) = value else {
        if matches!(value, Json::Null) {
            return Ok(Vec::new());
        }
        return Err(field(name, "a list of text values"));
    };
    let mut found = Vec::with_capacity(items.len());
    for item in items {
        let Json::Text(text) = item else {
            return Err(field(name, "a list of text values"));
        };
        found.push(text.clone());
    }
    Ok(found)
}

// main.py merges the headers kwarg under extra_headers, which wins on a shared name
pub(crate) fn headers(arguments: &RequestMap) -> Result<Headers, ConvertError> {
    let mut merged = Headers::new();
    merge_headers(&mut merged, arguments, "headers")?;
    merge_headers(&mut merged, arguments, "extra_headers")?;
    Ok(merged)
}

fn merge_headers(
    target: &mut Headers,
    arguments: &RequestMap,
    name: &str,
) -> Result<(), ConvertError> {
    let Some(value) = arguments.get(name) else {
        return Ok(());
    };
    let Json::Object(fields) = value else {
        if matches!(value, Json::Null) {
            return Ok(());
        }
        return Err(field(name, "an object of text values"));
    };
    for (header, entry) in fields {
        let Json::Text(text) = entry else {
            return Err(field(name, "an object of text values"));
        };
        target.insert(header.clone(), text.clone());
    }
    Ok(())
}

pub(crate) fn timeout(arguments: &RequestMap) -> Result<Option<Duration>, ConvertError> {
    match arguments.get("timeout") {
        Some(Json::Number(value)) => {
            let seconds = value.as_f64().ok_or_else(|| field("timeout", "a number"))?;
            Duration::try_from_secs_f64(seconds)
                .map(Some)
                .map_err(|source| ConvertError::Timeout { source })
        }
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(field("timeout", "a number"))
        }
    }
}

// the model and provider a call resolves to, with dall-e-2 and openai standing in when neither is given
pub(crate) fn target(arguments: &RequestMap) -> Result<(String, String), ConvertError> {
    let model = text(arguments, "model")?.unwrap_or_else(|| DEFAULT_GENERATION_MODEL.to_owned());
    let provider = text(arguments, "custom_llm_provider")?;
    Ok(resolve_target(&model, provider.as_deref()))
}

// get_llm_provider with an openai-only provider set: a prefix names the provider, otherwise openai
pub(crate) fn resolve_target(model: &str, provider: Option<&str>) -> (String, String) {
    let mut name = model;
    let mut found = provider;
    if let Some((prefix, rest)) = model.split_once('/')
        && (found.is_none() || found == Some(prefix))
    {
        name = rest;
        if found.is_none() {
            found = Some(prefix);
        }
    }
    let resolved_provider = found.map_or_else(|| OPENAI_PROVIDER.to_owned(), str::to_owned);
    (name.to_owned(), resolved_provider)
}

pub(crate) fn field(name: &str, expected: &'static str) -> ConvertError {
    ConvertError::Field {
        name: name.to_owned(),
        expected,
    }
}
