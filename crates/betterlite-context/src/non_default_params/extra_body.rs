use betterlite_models::controls::extra_body::check_extra_body;
use betterlite_models::controls::json_value::JsonValue;

use super::error::NonDefaultParamsError;
use super::names::{EXTRA_BODY_PROVIDERS, EXTRA_BODY_ROUTING_KEYS, is_openai_compatible_provider};
use super::{JsonObject, Params};

pub struct ProviderSpecificRequest<'request> {
    pub passed_params: &'request Params,
    pub custom_llm_provider: &'request str,
    pub openai_params: &'request [&'request str],
    pub additional_drop_params: &'request [String],
}

pub fn add_provider_specific_params(
    mut optional_params: Params,
    request: &ProviderSpecificRequest<'_>,
) -> Result<Params, NonDefaultParamsError> {
    let provider = request.custom_llm_provider;
    let drops = request.additional_drop_params;
    if !EXTRA_BODY_PROVIDERS.contains(&provider) && !is_openai_compatible_provider(provider) {
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
    let mut extra_body = match request.passed_params.get("extra_body") {
        None | Some(JsonValue::Null) => JsonObject::new(),
        Some(JsonValue::Object(fields)) => fields.clone(),
        Some(_) => return Err(NonDefaultParamsError::WrongType("extra_body", "an object")),
    };
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
