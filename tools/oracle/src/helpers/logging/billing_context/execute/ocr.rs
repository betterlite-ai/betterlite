use betterlite_core::cost::context::pricing::{PricingQuery, ocr_info};
use serde_json::{Value, json};

use super::super::model::CaseLookup;
use super::super::projection::optional_decimal;
use super::super::{Call, Failure, failure};
use super::required;

pub(super) fn ocr(call: &Call) -> Result<Value, Failure> {
    let input = required(call.pricing.as_ref(), "pricing")?;
    let lookup = CaseLookup::from(call.catalog.as_slice());
    let Some(pricing) = ocr_info(
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
    Ok(json!({
        "ocr_cost_per_page": optional_decimal(pricing.ocr_cost_per_page),
        "ocr_cost_per_credit": optional_decimal(pricing.ocr_cost_per_credit),
        "annotation_cost_per_page": optional_decimal(pricing.annotation_cost_per_page),
    }))
}
