use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::response_input::{ResponsesInput, ResponsesItem};
use betterlite_prompts::arguments::json as prompts;

use crate::api::responses_provider_contract::records::Params;

use super::EXTRA_BODY_FIELD;
use super::error::ManusResponsesError;

pub(super) fn to_prompts(value: &Json) -> prompts::Json {
    match value {
        Json::Null => prompts::Json::Null,
        Json::Bool(flag) => prompts::Json::Bool(*flag),
        Json::Number(number) => prompts::Json::Number(number.clone()),
        Json::Text(text) => prompts::Json::Text(text.clone()),
        Json::List(items) => prompts::Json::List(items.iter().map(to_prompts).collect()),
        Json::Object(fields) => prompts::Json::Object(
            fields
                .iter()
                .map(|(name, entry)| (name.clone(), to_prompts(entry)))
                .collect(),
        ),
    }
}

pub(super) fn to_prompts_object(params: &Params) -> prompts::Object {
    params
        .iter()
        .map(|(name, value)| (name.clone(), to_prompts(value)))
        .collect()
}

pub(super) fn from_prompts_object(object: &prompts::Object) -> Result<Params, ManusResponsesError> {
    let raw = serde_json::to_value(object)?;
    let params: Params = serde_json::from_value(raw)?;
    Ok(params)
}

pub(super) fn responses_input(value: &Json) -> Result<ResponsesInput, ManusResponsesError> {
    match value {
        Json::Text(text) => Ok(ResponsesInput::Text(text.clone())),
        Json::List(items) => Ok(ResponsesInput::Items(
            items
                .iter()
                .map(|item| ResponsesItem(to_prompts(item)))
                .collect(),
        )),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => {
            Err(ManusResponsesError::InputShape)
        }
    }
}

pub(super) fn extra_body(params: &Params) -> Result<Option<Params>, ManusResponsesError> {
    match params.get(EXTRA_BODY_FIELD) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => {
            if fields.is_empty() {
                return Ok(None);
            }
            Ok(Some(
                fields
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect(),
            ))
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(ManusResponsesError::ExtraBodyShape)
        }
    }
}

pub(super) fn text<'params>(
    params: &'params Params,
    field: &'static str,
) -> Result<Option<&'params str>, ManusResponsesError> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    if let Json::Text(text) = value {
        return Ok(Some(text.as_str()));
    }
    if matches!(value, Json::Null) {
        return Ok(None);
    }
    Err(ManusResponsesError::ParamNotText { field })
}

pub(super) fn object_of(params: &Params) -> Json {
    Json::Object(
        params
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    )
}

pub(super) fn headers_object(headers: &BTreeMap<String, String>) -> Json {
    Json::Object(
        headers
            .iter()
            .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
            .collect(),
    )
}
