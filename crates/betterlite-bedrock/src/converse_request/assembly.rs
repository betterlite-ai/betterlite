use betterlite_prompts::arguments::json::{Json, Object};

use crate::converse_params::inference::transform_inference_params;
use crate::converse_params::wire::transcode;
use crate::converse_request::CommonRequest;
use crate::converse_request::body::PreparedRequest;
use crate::converse_request::error::ConverseRequestError;
use crate::model_routes::{ParamMap, ParamValue};

const GUARDRAIL_FIELD: &str = "guardrailConfig";
const PERFORMANCE_FIELD: &str = "performanceConfig";
const SERVICE_TIER_FIELD: &str = "serviceTier";
const THINKING_FIELD: &str = "thinking";
const THINKING_OUTPUT_PATH: &str = "/usage/output_tokens_details";

// _transform_request_helper's body: the common fields in the order the pinned module builds them.
pub(super) fn common(
    prepared: PreparedRequest,
    system: Vec<Json>,
    tool_config: Option<Json>,
) -> Result<CommonRequest, ConverseRequestError> {
    let PreparedRequest {
        mut inference_params,
        additional,
        request_metadata,
        output_config,
    } = prepared;
    let guardrail_config = config_value(&mut inference_params, GUARDRAIL_FIELD)?;
    let performance_config = config_value(&mut inference_params, PERFORMANCE_FIELD)?;
    let service_tier = config_value(&mut inference_params, SERVICE_TIER_FIELD)?;
    let inference_config: Object = transcode(&transform_inference_params(inference_params))?;
    // The request fields are only emitted when any remain, but thinking is detected by key presence.
    let additional_fields: Option<Object> = (!additional.is_empty())
        .then(|| transcode(&additional))
        .transpose()?;
    let response_paths = additional_fields
        .as_ref()
        .is_some_and(|fields| fields.contains_key(THINKING_FIELD))
        .then(|| vec![THINKING_OUTPUT_PATH.to_owned()]);
    Ok(CommonRequest {
        inference_config,
        additional_model_request_fields: additional_fields,
        additional_model_response_field_paths: response_paths,
        system,
        guardrail_config,
        performance_config,
        service_tier,
        tool_config,
        request_metadata,
        output_config,
    })
}

// config_class(**value): a mapping unpacks, absence is skipped, and any other type is rejected.
fn config_value(
    params: &mut ParamMap,
    field: &'static str,
) -> Result<Option<Json>, ConverseRequestError> {
    match params.shift_remove(field) {
        None | Some(ParamValue::Null) => Ok(None),
        Some(value @ ParamValue::Mapping(_)) => Ok(Some(transcode(&value)?)),
        Some(
            ParamValue::Bool(_) | ParamValue::Number(_) | ParamValue::Text(_) | ParamValue::List(_),
        ) => Err(ConverseRequestError::FieldType {
            field,
            expected: "an object",
        }),
    }
}
