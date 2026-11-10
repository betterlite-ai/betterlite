use betterlite_logging::spend_bodies::support::{NodeAllocator, spend_value};
use betterlite_logging::spend_sanitize::value::{SpendMapping, SpendValue, to_json};
use betterlite_models::decisions::Json;
use serde_json::Value;

use crate::helpers::{HelperError, HelperResult};

pub(super) type Decoded = Result<SpendValue, Box<HelperError>>;

pub(super) fn decode(value: &Value, nodes: &mut NodeAllocator) -> Decoded {
    let decoded: Json =
        serde_json::from_value(value.clone()).map_err(|error| invalid(&error.to_string()))?;
    Ok(spend_value(&decoded, nodes))
}

pub(super) fn mapping_of(value: &SpendValue) -> Result<&SpendMapping, Box<HelperError>> {
    let SpendValue::Mapping(mapping) = value else {
        return Err(invalid("the input must be a JSON object"));
    };
    Ok(mapping)
}

pub(super) fn mapping_json(value: &SpendMapping) -> HelperResult {
    serde_json::to_value(to_json(&SpendValue::Mapping(value.clone())))
        .map_err(|error| invalid(&error.to_string()))
}

pub(super) fn invalid(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
