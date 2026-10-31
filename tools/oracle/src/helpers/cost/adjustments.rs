use betterlite_config::cost_adjustments::{
    CostAdjustmentMap, discount_entry, margin_entry, resolve_adjustment_entries,
};
use betterlite_config::loader::ConfigError;
use betterlite_cost::adjustments::{apply_cost_discount, apply_cost_margin};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
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
    BaseCost(#[source] rust_decimal::Error),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Adjustment(#[from] betterlite_cost::adjustments::AdjustmentError),
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let base_cost = Decimal::from_str_exact(&input.base_cost)
            .map_err(|error| Box::<crate::helpers::HelperError>::from(Failure::BaseCost(error)))?;
        let discount_config = resolve_adjustment_entries(
            "litellm_settings.cost_discount_config",
            input.discount_config.clone(),
            discount_entry,
        )
        .map_err(|error| Box::<crate::helpers::HelperError>::from(Failure::Config(error)))?;
        let margin_config = resolve_adjustment_entries(
            "litellm_settings.cost_margin_config",
            input.margin_config.clone(),
            margin_entry,
        )
        .map_err(|error| Box::<crate::helpers::HelperError>::from(Failure::Config(error)))?;
        let discount = apply_cost_discount(base_cost, input.provider.as_deref(), &discount_config)
            .map_err(|error| {
                Box::<crate::helpers::HelperError>::from(Failure::Adjustment(error))
            })?;
        let margin = apply_cost_margin(base_cost, input.provider.as_deref(), &margin_config)
            .map_err(|error| {
                Box::<crate::helpers::HelperError>::from(Failure::Adjustment(error))
            })?;
        let mut output = json!({
            "discount": {"final_cost": discount.final_cost, "discount_percent": discount.discount_percent, "saved_amount": discount.saved_amount},
            "margin": {"final_cost": margin.final_cost, "margin_percent": margin.margin_percent, "fixed_amount": margin.fixed_amount, "total_amount": margin.total_amount}
        });
        canonical_numbers(&mut output);
        Ok(output)
    }
}

fn canonical_numbers(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            for nested in fields.values_mut() {
                canonical_numbers(nested);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                canonical_numbers(item);
            }
        }
        serde_json::Value::Number(number) => {
            if let Some(float) = number.as_f64()
                && let Some(canonical) = serde_json::Number::from_f64(float)
            {
                *number = canonical;
            }
        }
        serde_json::Value::String(text) => {
            if let Ok(decimal) = Decimal::from_str_exact(text)
                && let Some(float) = decimal.to_f64()
                && let Some(canonical) = serde_json::Number::from_f64(float)
            {
                *value = serde_json::Value::Number(canonical);
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) => {}
    }
}

impl From<Failure> for Box<crate::helpers::HelperError> {
    fn from(failure: Failure) -> Self {
        let class = match &failure {
            Failure::BaseCost(_) | Failure::Adjustment(_) => "RuntimeError",
            Failure::Config(_) => "ConfigError",
        };
        Self::new(crate::helpers::HelperError {
            class: class.to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: failure.to_string(),
        })
    }
}
