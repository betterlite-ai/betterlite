use betterlite_models::controls::extra_body::check_extra_body;
use betterlite_models::controls::json_value::JsonValue;

use super::error::NonDefaultParamsError;
use super::names::{EXTRA_BODY_PROVIDERS, EXTRA_BODY_ROUTING_KEYS, OPENAI_COMPATIBLE_PROVIDERS};
use super::{JsonObject, Params};

pub struct ProviderSpecificRequest<'request> {
    pub passed_params: &'request Params,
    pub custom_llm_provider: &'request str,
    pub openai_params: &'request [&'request str],
    pub additional_drop_params: &'request [String],
}

fn requested_extra_body(passed_params: &Params) -> Result<JsonObject, NonDefaultParamsError> {
    match passed_params.get("extra_body") {
        None | Some(JsonValue::Null) => Ok(JsonObject::new()),
        Some(JsonValue::Object(fields)) => Ok(fields.clone()),
        Some(_) => Err(NonDefaultParamsError::WrongType {
            field: "extra_body",
            expected: "an object",
        }),
    }
}

// OpenAI-style providers take unknown params inside extra_body; every other provider gets them at the top level.
pub fn add_provider_specific_params(
    mut optional_params: Params,
    request: &ProviderSpecificRequest<'_>,
) -> Result<Params, NonDefaultParamsError> {
    let provider = request.custom_llm_provider;
    let drops = request.additional_drop_params;
    if !EXTRA_BODY_PROVIDERS.contains(&provider) && !OPENAI_COMPATIBLE_PROVIDERS.contains(&provider)
    {
        for (key, value) in request.passed_params {
            if !request.openai_params.contains(&key.as_str())
                && !matches!(value, JsonValue::Null)
                && !drops.contains(key)
            {
                optional_params.insert(key.clone(), value.clone());
            }
        }
        return Ok(optional_params);
    }
    if drops.iter().any(|param| param == "extra_body") {
        return Ok(optional_params);
    }
    let mut extra_body = requested_extra_body(request.passed_params)?;
    for (key, value) in request.passed_params {
        if key != "extra_body"
            && !request.openai_params.contains(&key.as_str())
            && !matches!(value, JsonValue::Null)
        {
            extra_body.insert(key.clone(), value.clone());
        }
    }
    let mut merged = match optional_params.get("extra_body") {
        Some(JsonValue::Object(existing)) => existing.clone(),
        _ => JsonObject::new(),
    };
    merged.extend(extra_body);
    merged
        .retain(|key, _| !EXTRA_BODY_ROUTING_KEYS.contains(&key.as_str()) && !drops.contains(key));
    let checked = JsonValue::Object(merged);
    check_extra_body(&checked)?;
    optional_params.insert("extra_body".to_owned(), checked);
    Ok(optional_params)
}
