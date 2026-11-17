use betterlite_anthropic::option_primitives::context_management::map_context_management;
use betterlite_models::decisions::Json;

use crate::converse_params::error::ConverseParamsError;
use crate::converse_params::wire::transcode;
use crate::model_routes::{ParamMap, ParamValue};
use crate::request_policy::validation::validate_request_metadata;
use crate::request_policy::{Params, RequestValue};

const AUTO_TIER: &str = "auto";
const CONTEXT_MANAGEMENT_FIELD: &str = "context_management";
const DEFAULT_TIER: &str = "default";
const REQUEST_METADATA_FIELD: &str = "requestMetadata";
const SERVICE_TIER_FIELD: &str = "serviceTier";
const SERVICE_TIER_TYPE_FIELD: &str = "type";
const SUPPORTED_TIERS: [&str; 3] = ["default", "flex", "priority"];

// Bedrock has no "auto" tier; every other unknown or non-string value leaves the mapped field alone.
pub fn map_service_tier_param(value: &str, optional_params: &mut ParamMap) {
    let tier = if value == AUTO_TIER {
        DEFAULT_TIER
    } else {
        value
    };
    if !SUPPORTED_TIERS.contains(&tier) {
        return;
    }
    let block = ParamMap::from([(
        SERVICE_TIER_TYPE_FIELD.to_owned(),
        ParamValue::Text(tier.to_owned()),
    )]);
    optional_params.insert(SERVICE_TIER_FIELD.to_owned(), ParamValue::Mapping(block));
}

// Only a dict goes to B596's validator, and the value is stored once it passed.
pub fn map_request_metadata_param(
    value: &ParamValue,
    optional_params: &mut ParamMap,
) -> Result<(), ConverseParamsError> {
    let ParamValue::Mapping(fields) = value else {
        return Ok(());
    };
    let pairs = fields
        .iter()
        .map(|(key, value)| Ok((key.clone(), metadata_value(value)?)))
        .collect::<Result<Params, ConverseParamsError>>()?;
    validate_request_metadata(&RequestValue::Mapping(pairs))?;
    optional_params.insert(REQUEST_METADATA_FIELD.to_owned(), value.clone());
    Ok(())
}

fn metadata_value(value: &ParamValue) -> Result<RequestValue, ConverseParamsError> {
    match value {
        ParamValue::Text(text) => Ok(RequestValue::Text(text.clone())),
        other => Ok(RequestValue::Raw(
            serde_json::to_string(other).map_err(ConverseParamsError::Encode)?,
        )),
    }
}

// A list runs the B2201 mapper and assigns only a mapped result; a dict passes through unchanged.
pub fn map_context_management_param(
    value: &ParamValue,
    optional_params: &mut ParamMap,
) -> Result<(), ConverseParamsError> {
    match value {
        ParamValue::Mapping(_) => {
            optional_params.insert(CONTEXT_MANAGEMENT_FIELD.to_owned(), value.clone());
        }
        ParamValue::List(_) => {
            let encoded: Json = transcode(value)?;
            if let Some(mapped) = map_context_management(&encoded)? {
                optional_params.insert(CONTEXT_MANAGEMENT_FIELD.to_owned(), transcode(&mapped)?);
            }
        }
        ParamValue::Null | ParamValue::Bool(_) | ParamValue::Number(_) | ParamValue::Text(_) => {}
    }
    Ok(())
}
