use betterlite_config::cost_adjustments::CostAdjustmentMap;
use betterlite_cost::adjustments::{apply_cost_discount, apply_cost_margin};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::json;
use thiserror::Error;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    base_cost: String,
    provider: Option<String>,
    #[serde(default)]
    discount_config: CostAdjustmentMap,
    #[serde(default)]
    margin_config: CostAdjustmentMap,
}

#[derive(Debug, Error)]
enum Failure {
    #[error("invalid base cost: {0}")]
    BaseCost(String),
    #[error(transparent)]
    Adjustment(#[from] betterlite_cost::adjustments::AdjustmentError),
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let base_cost = Decimal::from_str_exact(&input.base_cost).map_err(|error| {
            Box::<crate::helpers::HelperError>::from(Failure::BaseCost(error.to_string()))
        })?;
        let discount =
            apply_cost_discount(base_cost, input.provider.as_deref(), &input.discount_config)
                .map_err(|error| {
                    Box::<crate::helpers::HelperError>::from(Failure::Adjustment(error))
                })?;
        let margin = apply_cost_margin(base_cost, input.provider.as_deref(), &input.margin_config)
            .map_err(|error| {
                Box::<crate::helpers::HelperError>::from(Failure::Adjustment(error))
            })?;
        Ok(json!({
            "discount": {"final_cost": discount.final_cost, "discount_percent": discount.discount_percent, "saved_amount": discount.saved_amount},
            "margin": {"final_cost": margin.final_cost, "margin_percent": margin.margin_percent, "fixed_amount": margin.fixed_amount, "total_amount": margin.total_amount}
        }))
    }
}

impl From<Failure> for Box<crate::helpers::HelperError> {
    fn from(failure: Failure) -> Self {
        Self::new(crate::helpers::HelperError {
            class: "RuntimeError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: failure.to_string(),
        })
    }
}
