use betterlite_context::owned_parameter_names::is_litellm_owned_kwarg;
use betterlite_models::decisions::Json;
use serde::de::DeserializeOwned;
use serde_json::Number;

use super::VideoGenerationRequest;
use super::error::VideoGenerationCallError;
use super::providers::VideoGenerationConfig;
use crate::api::bfl::edit::media::SuppliedMedia;
// ast-grep-ignore: no-long-names
use crate::api::video_records::VideoCreateOptionalRequestParams;
use crate::api::videos_http_transport::video_config::Params;

const EXTRA_BODY: &str = "extra_body";
const TIMEOUT: &str = "timeout";
const INPUT_REFERENCE: &str = "input_reference";
const CHARACTERS: &str = "characters";
const STRING: &str = "a string";
const MAP: &str = "a map";
const TEXT_MAP: &str = "a map of strings";
const MEDIA: &str = "a file reference";

// get_requested_video_generation_optional_param: nulls and owned names out, loose keywords over the rest, extra_body flat and last, timeout removed
pub fn requested_optional_params(
    call: &mut VideoGenerationRequest,
    // ast-grep-ignore: no-long-names
) -> Result<VideoCreateOptionalRequestParams, VideoGenerationCallError> {
    let mut params = requested_params(call)?;
    let reference = match params.shift_remove(INPUT_REFERENCE) {
        None => call.input_reference.clone(),
        Some(value) => Some(supplied_media(value)?),
    };
    // ast-grep-ignore: no-long-names
    Ok(VideoCreateOptionalRequestParams {
        input_reference: reference,
        image: params.shift_remove("image"),
        parameters: take(&mut params, "parameters", MAP)?,
        model: take(&mut params, "model", STRING)?,
        resolution: take(&mut params, "resolution", STRING)?,
        seconds: take(&mut params, "seconds", STRING)?,
        size: take(&mut params, "size", STRING)?,
        characters: take(&mut params, CHARACTERS, "a list of string maps")?,
        user: take(&mut params, "user", STRING)?,
        extra_headers: take(&mut params, "extra_headers", TEXT_MAP)?,
        extra_body: take(&mut params, EXTRA_BODY, TEXT_MAP)?,
    })
}

// the options as LiteLLM's dict builds them, before they are narrowed to the typed struct
fn requested_params(call: &mut VideoGenerationRequest) -> Result<Params, VideoGenerationCallError> {
    let loose_extra_body = call.kwargs.shift_remove(EXTRA_BODY);
    // local_vars.update(kwargs): every loose keyword lands over the named arguments, nulls included
    let mut optional = named_params(call);
    for (name, value) in &call.kwargs {
        optional.insert(name.clone(), value.clone());
    }
    // a null value removes the entry, and LiteLLM-owned names never travel as options
    optional.retain(|name, value| !matches!(value, Json::Null) && !is_litellm_owned_kwarg(name));
    // extra_body: a loose one replaced the named argument in local_vars, and the top level wins only without one
    let mut merged = Params::new();
    if let Some(loose) = &loose_extra_body {
        merge_extra_body(&mut merged, loose);
    } else if let Some(named) = &call.extra_body {
        merge_extra_body(&mut merged, &Json::Object(named.clone()));
    }
    merged.retain(|name, _| !is_litellm_owned_kwarg(name));
    if !merged.is_empty() {
        optional.insert(EXTRA_BODY.to_owned(), Json::Object(merged.clone()));
        optional.extend(merged);
    }
    // last, so no timeout reaches the provider as an option
    optional.shift_remove(TIMEOUT);
    Ok(optional)
}

// get_optional_params_video_generation: the provider's map, then the extra_body entries over it key by key
pub fn provider_request_params<Config>(
    config: &Config,
    model: &str,
    drop_params: bool,
    // ast-grep-ignore: no-long-names
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

fn merge_extra_body(merged: &mut Params, candidate: &Json) {
    let Json::Object(entries) = candidate else {
        return;
    };
    for (name, value) in entries {
        if !matches!(value, Json::Null) {
            merged.insert(name.clone(), value.clone());
        }
    }
}

// FileTypes as the JSON a caller can hand in: a path or URL, or a list of them
fn supplied_media(value: Json) -> Result<SuppliedMedia, VideoGenerationCallError> {
    match value {
        Json::Text(text) => Ok(SuppliedMedia::Text(text)),
        Json::List(items) => items
            .into_iter()
            .map(supplied_media)
            .collect::<Result<Vec<_>, _>>()
            .map(SuppliedMedia::List),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => {
            Err(VideoGenerationCallError::FieldType {
                field: INPUT_REFERENCE,
                expected: MEDIA,
            })
        }
    }
}

// a declared field holding the type its name says, read through B247's own value model
fn take<T>(
    params: &mut Params,
    field: &'static str,
    expected: &'static str,
) -> Result<Option<T>, VideoGenerationCallError>
where
    T: DeserializeOwned,
{
    let Some(value) = params.shift_remove(field) else {
        return Ok(None);
    };
    let encoded = serde_json::to_string(&value)
        .map_err(|_error| VideoGenerationCallError::FieldType { field, expected })?;
    serde_json::from_str(&encoded)
        .map(Some)
        .map_err(|_error| VideoGenerationCallError::FieldType { field, expected })
}

mod tests;
