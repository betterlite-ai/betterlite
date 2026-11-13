use betterlite_core::cost::context::{IdentityRequest, billing_model_name};
use serde_json::Value;

use super::super::model::{CaseLookup, cost_response};
use super::super::{Call, Failure, failure};
use super::NoInference;

pub(super) fn identity(call: &Call) -> Result<Value, Failure> {
    let Some(input) = call.identity.as_ref() else {
        return Err(failure("identity is required"));
    };
    let lookup = CaseLookup::from(call.catalog.as_slice());
    let response = input.response.as_ref().map(cost_response);
    let selected = billing_model_name(
        &IdentityRequest {
            model: input.model.as_deref(),
            response: response.as_ref(),
            base_model: input.base_model.as_deref(),
            custom_pricing: input.custom_pricing,
            custom_llm_provider: Some(&input.custom_llm_provider),
            router_model_id: input.router_model_id.as_deref(),
            region_name: input.region_name.as_deref(),
        },
        &lookup,
        &NoInference,
    );
    Ok(selected.map_or(Value::Null, Value::from))
}
