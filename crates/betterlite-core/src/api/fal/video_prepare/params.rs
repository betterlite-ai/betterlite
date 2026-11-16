use betterlite_models::decisions::Json;
use serde_json::Number;

use super::VideoParams;
use super::duration::{DurationValue, duration_value};
use super::error::FalVideoError;
use super::profile::{VideoProfile, profile_for_model};
use super::size::size_params;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 7] = [
    "model",
    "prompt",
    "input_reference",
    "seconds",
    "size",
    "user",
    "extra_headers",
];

pub fn map_openai_params(
    options: &VideoParams,
    model: &str,
    _drop_params: bool,
) -> Result<VideoParams, FalVideoError> {
    let profile = profile_for_model(model);
    let mut mapped = VideoParams::new();
    match options.get("input_reference") {
        Some(Json::Text(reference)) => {
            let value = if profile.reference_as_list {
                Json::List(vec![Json::Text(reference.clone())])
            } else {
                Json::Text(reference.clone())
            };
            mapped.insert(profile.reference_key.to_owned(), value);
        }
        Some(_) => return Err(FalVideoError::ReferenceNotText),
        None => {}
    }
    if let Some(seconds) = options.get("seconds") {
        mapped.extend(duration_params(seconds, profile)?);
    }
    if let Some(size) = options.get("size") {
        mapped.extend(size_params(size, profile));
    }
    if let Some(Json::Text(user)) = options.get("user") {
        mapped.insert("end_user_id".to_owned(), Json::Text(user.clone()));
    }
    for (key, value) in options {
        if !SUPPORTED_OPENAI_PARAMS.contains(&key.as_str()) {
            mapped.insert(key.clone(), value.clone());
        }
    }
    Ok(mapped)
}

fn duration_params(seconds: &Json, profile: &VideoProfile) -> Result<VideoParams, FalVideoError> {
    let mut params = VideoParams::new();
    match duration_value(seconds) {
        None => return Err(FalVideoError::SecondsNotNumeric),
        Some(DurationValue::Auto) => {
            if !profile.integer_duration {
                params.insert("duration".to_owned(), Json::Text("auto".to_owned()));
            }
        }
        Some(DurationValue::Numeric(text)) => {
            let value = if profile.integer_duration {
                json_integer(&text)?
            } else {
                Json::Text(text)
            };
            params.insert("duration".to_owned(), value);
        }
    }
    Ok(params)
}

fn json_integer(text: &str) -> Result<Json, FalVideoError> {
    // the text is digits with an optional sign, so it reads back as the same JSON number
    text.parse::<Number>()
        .map(Json::Number)
        .map_err(|_| FalVideoError::SecondsNotNumeric)
}
