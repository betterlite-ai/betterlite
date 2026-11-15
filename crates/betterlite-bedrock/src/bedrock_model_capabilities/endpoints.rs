use crate::model_routes::{ParamMap, ParamValue};

use super::CapabilityError;

const RESPONSES_ENDPOINT: &str = "/v1/responses";
const SUPPORTED_ENDPOINTS_FIELD: &str = "supported_endpoints";

pub fn supports_openai_responses(
    model: Option<&str>,
    model_cost: &ParamMap,
) -> Result<bool, CapabilityError> {
    let Some(name) = model.filter(|value| !value.is_empty()) else {
        return Ok(false);
    };
    for candidate in [name.to_owned(), format!("bedrock/{name}")] {
        let Some(ParamValue::Mapping(fields)) = model_cost.get(&candidate) else {
            continue;
        };
        if endpoints_list_responses(fields.get(SUPPORTED_ENDPOINTS_FIELD))? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn endpoints_list_responses(endpoints: Option<&ParamValue>) -> Result<bool, CapabilityError> {
    match endpoints {
        None | Some(ParamValue::Null) => Ok(false),
        Some(ParamValue::Bool(flag)) => {
            if *flag {
                Err(not_iterable())
            } else {
                Ok(false)
            }
        }
        Some(ParamValue::Number(number)) => {
            if number.as_f64() == Some(0.0) {
                Ok(false)
            } else {
                Err(not_iterable())
            }
        }
        Some(ParamValue::Text(text)) => Ok(text.contains(RESPONSES_ENDPOINT)),
        Some(ParamValue::List(items)) => Ok(items
            .iter()
            .any(|item| matches!(item, ParamValue::Text(text) if text == RESPONSES_ENDPOINT))),
        Some(ParamValue::Mapping(fields)) => Ok(fields.contains_key(RESPONSES_ENDPOINT)),
    }
}

const fn not_iterable() -> CapabilityError {
    CapabilityError::NotIterable {
        field: SUPPORTED_ENDPOINTS_FIELD,
    }
}
