use betterlite_anthropic::option_primitives::context_management::map_context_management;
use betterlite_models::decisions::Json;

use crate::converse_params::error::ConverseParamsError;
use crate::converse_params::wire::transcode;
use crate::model_routes::{ParamMap, ParamValue};
use crate::request_policy::validation::validate_request_metadata;
use crate::request_policy::{Params, RequestValue};

const AUTO_TIER: &str = "auto";
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
        .map(|(key, entry)| Ok((key.clone(), metadata_value(entry)?)))
        .collect::<Result<Params, ConverseParamsError>>()?;
    validate_request_metadata(&RequestValue::Mapping(pairs))?;
    optional_params.insert(REQUEST_METADATA_FIELD.to_owned(), value.clone());
    Ok(())
}

fn metadata_value(value: &ParamValue) -> Result<RequestValue, ConverseParamsError> {
    if let ParamValue::Text(text) = value {
        return Ok(RequestValue::Text(text.clone()));
    }
    Ok(RequestValue::Raw(
        serde_json::to_string(value).map_err(ConverseParamsError::Encode)?,
    ))
}

// A list runs the B2201 mapper and yields only a mapped result; a dict passes through unchanged.
pub fn context_management_value(
    value: &ParamValue,
) -> Result<Option<ParamValue>, ConverseParamsError> {
    match value {
        ParamValue::Mapping(_) => Ok(Some(value.clone())),
        ParamValue::List(_) => {
            let encoded: Json = transcode(value)?;
            map_context_management(&encoded)?
                .map(|mapped| transcode::<ParamValue, Json>(&mapped))
                .transpose()
        }
        ParamValue::Null | ParamValue::Bool(_) | ParamValue::Number(_) | ParamValue::Text(_) => {
            Ok(None)
        }
    }
}
