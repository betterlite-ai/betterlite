use betterlite_prompts::arguments::json::{Json, Object};

use super::error::SapChatError;
use super::prompt_module::build_prompt_module;
use crate::sap_models::{validate_orchestration_request, validate_sap_message};

pub fn transform_request(
    model: &str,
    messages: &[Json],
    optional_params: &Object,
) -> Result<Json, SapChatError> {
    let mut params = optional_params.clone();
    params.shift_remove("deployment_url");
    let template = messages
        .iter()
        .map(validate_sap_message)
        .collect::<Result<Vec<_>, _>>()?;
    let placeholder_values = params
        .shift_remove("placeholder_values")
        .filter(|value| !matches!(value, Json::Null));
    let fallbacks = fallback_entries(&mut params)?;
    params.shift_remove("stream");
    let stream = stream_options(params.shift_remove("stream_options"))?;
    params.shift_remove("tool_choice");
    let primary = build_prompt_module(model, &template, params)?;
    let mut extra = Vec::with_capacity(fallbacks.len());
    for entry in &fallbacks {
        extra.push(fallback_module(entry)?);
    }
    let mut config = Object::from([(String::from("modules"), merge_modules(primary, extra))]);
    if !stream.is_empty() {
        config.insert(String::from("stream"), Json::Object(stream));
    }
    let mut body = Object::from([(String::from("config"), Json::Object(config))]);
    if let Some(values) = placeholder_values {
        body.insert(String::from("placeholder_values"), values);
    }
    validate_orchestration_request(&Json::Object(body)).map_err(SapChatError::from)
}

fn fallback_entries(params: &mut Object) -> Result<Vec<Json>, SapChatError> {
    match params.shift_remove("fallback_sap_modules") {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::List(entries)) => Ok(entries),
        Some(_) => Err(SapChatError::Fallback(
            "`fallback_sap_modules` must be a list",
        )),
    }
}

fn merge_modules(primary: Json, extra: Vec<Json>) -> Json {
    if extra.is_empty() {
        return primary;
    }
    let mut all = Vec::with_capacity(extra.len());
    all.push(primary);
    all.extend(extra);
    Json::List(all)
}

fn fallback_module(entry: &Json) -> Result<Json, SapChatError> {
    let Json::Object(entry_fields) = entry else {
        return Err(SapChatError::Fallback(
            "each entry in `fallback_sap_modules` must be an object",
        ));
    };
    let mut fields = entry_fields.clone();
    let Some(named) = fields
        .shift_remove("model")
        .filter(|value| !matches!(value, Json::Null))
    else {
        return Err(SapChatError::Fallback(
            "Each entry in `fallback_sap_modules` must include a 'model' key.",
        ));
    };
    let Json::Text(model_name) = named else {
        return Err(SapChatError::Fallback(
            "Each entry in `fallback_sap_modules` must include a 'model' key.",
        ));
    };
    let template = match fields.shift_remove("messages") {
        None | Some(Json::Null) => Vec::new(),
        Some(Json::List(messages)) => messages,
        Some(_) => {
            return Err(SapChatError::Fallback(
                "`fallback_sap_modules` messages must be a list",
            ));
        }
    };
    build_prompt_module(
        model_name
            .strip_prefix("sap/")
            .unwrap_or(model_name.as_str()),
        &template,
        fields,
    )
}

fn stream_options(value: Option<Json>) -> Result<Object, SapChatError> {
    let Some(options) = value.filter(|given| !matches!(given, Json::Null)) else {
        return Ok(Object::new());
    };
    let Json::Object(fields) = options else {
        return Err(SapChatError::Field("`stream_options` must be an object"));
    };
    let mut stream = Object::new();
    for name in ["chunk_size", "delimiters"] {
        if let Some(given) = fields.get(name) {
            stream.insert(name.to_owned(), given.clone());
        }
    }
    Ok(stream)
}
