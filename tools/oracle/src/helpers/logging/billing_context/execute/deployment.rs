use std::collections::BTreeMap;

use betterlite_core::cost::context::pricing::{PricingQuery, deployment_info};
use serde_json::{Value, json};

use super::super::model::CaseLookup;
use super::super::projection::scalar_value;
use super::super::{Call, Failure, failure};
use super::required;

pub(super) fn deployment(call: &Call) -> Result<Value, Failure> {
    let input = required(call.pricing.as_ref(), "pricing")?;
    let lookup = CaseLookup::from(call.catalog.as_slice());
    let Some(record) = deployment_info(
        &PricingQuery {
            custom_pricing: input.custom_pricing,
            router_model_id: input.router_model_id.as_deref(),
            logging_params: input.litellm_params.as_ref(),
        },
        &lookup,
    )
    .map_err(failure)?
    else {
        return Ok(Value::Null);
    };
    let fields: BTreeMap<String, Value> = record
        .iter()
        .map(|(key, scalar)| (key.clone(), scalar_value(scalar)))
        .collect();
    Ok(json!({"fields": fields}))
}
