use betterlite_models::call_helpers::metadata::{CallParams, InternalMetadata};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::datadog_llm_obs::values::{as_text, mapping_field};
use crate::latency_alerts::api_base::ApiBaseResolver;

use super::error::HangingRequestError;

pub(crate) fn call_params(request_data: &IndexMap<String, Json>) -> CallParams {
    let Some(params) = mapping_field(request_data, "litellm_params") else {
        return CallParams::default();
    };
    let litellm_metadata = mapping_field(params, "litellm_metadata")
        .cloned()
        .map(|entries| InternalMetadata { entries });
    CallParams {
        metadata: mapping_field(params, "metadata").cloned(),
        litellm_metadata,
    }
}

pub(crate) fn field_text<'source>(
    source: &'source IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<&'source str>, HangingRequestError> {
    match source.get(field) {
        Some(Json::Null) | None => Ok(None),
        Some(value) => as_text(value)
            .map(Some)
            .ok_or(HangingRequestError::FieldType { field }),
    }
}

pub(crate) fn deployment_api_base(
    resolver: &dyn ApiBaseResolver,
    request_data: &IndexMap<String, Json>,
    model: &str,
) -> Result<Option<String>, HangingRequestError> {
    let Some(deployment) = mapping_field(request_data, "deployment") else {
        return Ok(None);
    };
    let empty = IndexMap::new();
    match deployment.get("litellm_params") {
        Some(Json::Object(params)) => Ok(resolver.resolve(model, params)?),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Ok(None)
        }
        None => Ok(resolver.resolve(model, &empty)?),
    }
}
