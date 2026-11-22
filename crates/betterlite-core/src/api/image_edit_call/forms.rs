use betterlite_context::non_default_params::Params;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::controls::multipart::{FormField, FormValue, flatten_form_field_values};
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};

use super::call::{ImageEditCall, MAX_FORM_DEPTH};
use super::error::ImageEditError;

// kwargs.headers first, then extra_headers: a caller's explicit map wins, exactly as merged_extra_headers
pub(crate) fn merged_headers(call: &ImageEditCall) -> Result<Option<ImageHeaders>, ImageEditError> {
    let mut merged = ImageParams::new();
    if let Some(Json::Object(entries)) = call.kwargs.get("headers") {
        merged.extend(entries.to_owned());
    }
    if let Some(extra) = &call.extra_headers {
        merged.extend(extra.to_owned());
    }
    if merged.is_empty() {
        return Ok(None);
    }
    let mut headers = ImageHeaders::new();
    for (name, value) in merged {
        let Json::Text(text) = value else {
            return Err(ImageEditError::HeaderText { name });
        };
        headers.insert(name, text);
    }
    Ok(Some(headers))
}

// flatten_form_field_values(non_default_params, extra_body): non-default fields first, then extra_body's, so a repeated name keeps its position and takes extra_body's value
pub(crate) fn form_fields(
    call: &ImageEditCall,
    non_default: &Params,
) -> Result<Vec<FormField>, ImageEditError> {
    let source = JsonValue::Object(non_default.clone().into_iter().collect());
    let body = call.extra_body.as_ref().map(|params| {
        JsonValue::Object(
            params
                .iter()
                .map(|(name, value)| (name.clone(), json_value(value)))
                .collect(),
        )
    });
    Ok(flatten_form_field_values(
        &[Some(&source), body.as_ref()],
        MAX_FORM_DEPTH,
    )?)
}

// the kwargs map as the filtered Params filter_out_litellm_params works on
#[must_use]
pub(crate) fn control_params(params: &ImageParams) -> Params {
    params
        .iter()
        .map(|(name, value)| (name.clone(), json_value(value)))
        .collect()
}

fn json_value(value: &Json) -> JsonValue {
    match value {
        Json::Null => JsonValue::Null,
        Json::Bool(flag) => JsonValue::Bool(*flag),
        Json::Number(number) => JsonValue::Number(number.clone()),
        Json::Text(text) => JsonValue::Text(text.clone()),
        Json::List(items) => JsonValue::Array(items.iter().map(json_value).collect()),
        Json::Object(entries) => JsonValue::Object(
            entries
                .iter()
                .map(|(name, item)| (name.clone(), json_value(item)))
                .collect(),
        ),
    }
}

// a dict update over the mapped options: a repeated name keeps its first position and takes the new value
pub(crate) fn merge_form_fields(options: &mut ImageParams, fields: Vec<FormField>) {
    for field in fields {
        let value = match field.value {
            FormValue::Single(text) => Json::Text(text),
            FormValue::Repeated(values) => Json::List(values.into_iter().map(Json::Text).collect()),
        };
        options.insert(field.name, value);
    }
}

// image_edit_request_params.update(non_default_params) for the bedrock, stability and BFL branches
pub(crate) fn merge_non_default(
    options: &mut ImageParams,
    non_default: &Params,
) -> Result<(), ImageEditError> {
    for (name, value) in non_default {
        options.insert(
            name.clone(),
            serde_json::from_slice(&serde_json::to_vec(value)?)?,
        );
    }
    Ok(())
}
