use betterlite_context::owned_parameter_names::is_litellm_owned_kwarg;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::VideoGenerationRequest;
use super::error::VideoGenerationCallError;
use super::providers::VideoGenerationConfig;
use crate::api::bfl::edit::media::SuppliedMedia;
use crate::api::video_records::VideoCreateOptionalRequestParams;
use crate::api::videos_http_transport::video_config::{Headers, Params};

const EXTRA_BODY: &str = "extra_body";
const TIMEOUT: &str = "timeout";
const INPUT_REFERENCE: &str = "input_reference";
const CHARACTERS: &str = "characters";
const STRING: &str = "a string";
const MAP: &str = "a map";
const TEXT_MAP: &str = "a map of strings";
const CHARACTER_LIST: &str = "a list of string maps";
const MEDIA: &str = "a file reference";

// get_requested_video_generation_optional_param: the merged call data minus the nulls and the owned names, the loose
// keywords over it, extra_body last and flat, then the timeout removed; the loose map loses its extra_body on the way
pub fn requested_optional_params(
    call: &mut VideoGenerationRequest,
) -> Result<VideoCreateOptionalRequestParams, VideoGenerationCallError> {
    let params = requested_params(call)?;
    typed_options(params, call)
}

// get_optional_params_video_generation: the provider's map, then the extra_body entries over it key by key
pub fn provider_request_params<Config>(
    config: &Config,
    model: &str,
    drop_params: bool,
    options: &VideoCreateOptionalRequestParams,
) -> Params
where
    Config: VideoGenerationConfig,
{
    let mut mapped = config.map_openai_params(options, model, drop_params);
    if let Some(extra_body) = &options.extra_body {
        for (name, value) in extra_body {
            mapped.insert(name.clone(), Json::Text(value.clone()));
        }
        mapped.shift_remove(EXTRA_BODY);
    }
    mapped
}

// the options as the dict LiteLLM builds them, before they are narrowed to the typed struct
fn requested_params(call: &mut VideoGenerationRequest) -> Result<Params, VideoGenerationCallError> {
    let loose_extra_body = call.kwargs.shift_remove(EXTRA_BODY);
    // local_vars.update(kwargs): every loose keyword lands over the named arguments, nulls included
    let mut optional = named_params(call);
    for (name, value) in &call.kwargs {
        optional.insert(name.clone(), value.clone());
    }
    // steps 1 and 2: a null value removes the entry, and LiteLLM-owned names never travel as options
    optional.retain(|_, value| !matches!(value, Json::Null));
    optional.retain(|name, _| !is_litellm_owned_kwarg(name));
    // step 3: the loose map's non-null entries lay over the result again
    let mut cleaned = call.kwargs.clone();
    cleaned.retain(|_, value| !matches!(value, Json::Null));
    cleaned.retain(|name, _| !is_litellm_owned_kwarg(name));
    optional.extend(cleaned);
    // step 4: the top-level extra_body, then the loose one, each entry only when it is not null
    let mut merged = Params::new();
    let top_level = Json::Object(call.extra_body.clone().unwrap_or_default());
    merge_extra_body(&mut merged, Some(&top_level));
    merge_extra_body(&mut merged, loose_extra_body.as_ref());
    if !merged.is_empty() {
        merged.retain(|name, _| !is_litellm_owned_kwarg(name));
        if !merged.is_empty() {
            optional.insert(EXTRA_BODY.to_owned(), Json::Object(merged.clone()));
            for (name, value) in &merged {
                optional.insert(name.clone(), value.clone());
            }
        }
    }
    // step 5: last, so no timeout reaches the provider as an option
    optional.shift_remove(TIMEOUT);
    Ok(optional)
}

// local_vars as the named arguments fill it: prompt, model and the two option-wide keys are read on their own
fn named_params(call: &VideoGenerationRequest) -> Params {
    let mut params = Params::new();
    for (name, value) in [
        ("seconds", &call.seconds),
        ("size", &call.size),
        ("user", &call.user),
    ] {
        if let Some(text) = value {
            params.insert(name.to_owned(), Json::Text(text.clone()));
        }
    }
    if let Some(provider) = &call.custom_llm_provider {
        params.insert(
            "custom_llm_provider".to_owned(),
            Json::Text(provider.clone()),
        );
    }
    if let Some(headers) = &call.extra_headers {
        let mut entries = Params::new();
        for (name, value) in headers {
            entries.insert(name.clone(), Json::Text(value.clone()));
        }
        params.insert("extra_headers".to_owned(), Json::Object(entries));
    }
    if let Some(query) = &call.extra_query {
        params.insert("extra_query".to_owned(), Json::Object(query.clone()));
    }
    if let Some(timeout) = call.timeout.and_then(Number::from_f64) {
        params.insert(TIMEOUT.to_owned(), Json::Number(timeout));
    }
    params
}

fn merge_extra_body(merged: &mut Params, candidate: Option<&Json>) {
    let Some(Json::Object(entries)) = candidate else {
        return;
    };
    for (name, value) in entries {
        if !matches!(value, Json::Null) {
            merged.insert(name.clone(), value.clone());
        }
    }
}

// the requested options narrowed to VideoCreateOptionalRequestParams: a declared field the value cannot fill is
// refused by name, and every key the type does not declare (extra_query among them) goes no further
fn typed_options(
    mut params: Params,
    call: &VideoGenerationRequest,
) -> Result<VideoCreateOptionalRequestParams, VideoGenerationCallError> {
    let reference = match params.shift_remove(INPUT_REFERENCE) {
        None => call.input_reference.clone(),
        Some(value) => Some(supplied_media(value)?),
    };
    let image = params.shift_remove("image");
    let parameters = match params.shift_remove("parameters") {
        None => None,
        Some(Json::Object(fields)) => Some(fields),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(VideoGenerationCallError::FieldType {
                field: "parameters",
                expected: MAP,
            });
        }
    };
    Ok(VideoCreateOptionalRequestParams {
        input_reference: reference,
        image,
        parameters,
        model: take_text(&mut params, "model")?,
        resolution: take_text(&mut params, "resolution")?,
        seconds: take_text(&mut params, "seconds")?,
        size: take_text(&mut params, "size")?,
        characters: take_characters(&mut params)?,
        user: take_text(&mut params, "user")?,
        extra_headers: take_text_map(&mut params, "extra_headers")?,
        extra_body: take_text_map(&mut params, EXTRA_BODY)?,
    })
}

// FileTypes as the JSON a caller can hand in: a path or URL, or a list of them
fn supplied_media(value: Json) -> Result<SuppliedMedia, VideoGenerationCallError> {
    match value {
        Json::Text(text) => Ok(SuppliedMedia::Text(text)),
        Json::List(items) => {
            let mut media = Vec::with_capacity(items.len());
            for item in items {
                media.push(supplied_media(item)?);
            }
            Ok(SuppliedMedia::List(media))
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => {
            Err(VideoGenerationCallError::FieldType {
                field: INPUT_REFERENCE,
                expected: MEDIA,
            })
        }
    }
}

// a declared field holding a string
fn take_text(
    params: &mut Params,
    field: &'static str,
) -> Result<Option<String>, VideoGenerationCallError> {
    match params.shift_remove(field) {
        None => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(VideoGenerationCallError::FieldType {
                field,
                expected: STRING,
            })
        }
    }
}

// a declared field holding dict[str, str]: extra_headers and extra_body
fn take_text_map(
    params: &mut Params,
    field: &'static str,
) -> Result<Option<Headers>, VideoGenerationCallError> {
    let Some(value) = params.shift_remove(field) else {
        return Ok(None);
    };
    let Json::Object(fields) = value else {
        return Err(VideoGenerationCallError::FieldType {
            field,
            expected: TEXT_MAP,
        });
    };
    let mut map = Headers::new();
    for (name, value) in fields {
        let Json::Text(text) = value else {
            return Err(VideoGenerationCallError::FieldType {
                field,
                expected: TEXT_MAP,
            });
        };
        map.insert(name, text);
    }
    Ok(Some(map))
}

// a declared field holding list[dict[str, str]]
fn take_characters(
    params: &mut Params,
) -> Result<Option<Vec<IndexMap<String, String>>>, VideoGenerationCallError> {
    let Some(value) = params.shift_remove(CHARACTERS) else {
        return Ok(None);
    };
    let Json::List(items) = value else {
        return Err(invalid_characters());
    };
    let mut characters = Vec::with_capacity(items.len());
    for item in items {
        let Json::Object(fields) = item else {
            return Err(invalid_characters());
        };
        let mut character = IndexMap::new();
        for (name, value) in fields {
            let Json::Text(text) = value else {
                return Err(invalid_characters());
            };
            character.insert(name, text);
        }
        characters.push(character);
    }
    Ok(Some(characters))
}

fn invalid_characters() -> VideoGenerationCallError {
    VideoGenerationCallError::FieldType {
        field: CHARACTERS,
        expected: CHARACTER_LIST,
    }
}

#[cfg(test)]
mod tests;
