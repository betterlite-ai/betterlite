use std::str::FromStr as _;

use betterlite_auth::reservation_cost::ModelInfoSource;
use betterlite_auth::reservation_cost::error::ModelInfoError;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;

use crate::lens_inference::request_body::SYSTEM;

mod tests;

const INPUT_RATE: &str = "input_cost_per_token";
const OUTPUT_RATE: &str = "output_cost_per_token";
const INPUT_200K_RATE: &str = "input_cost_per_token_above_200k_tokens";
const OUTPUT_200K_RATE: &str = "output_cost_per_token_above_200k_tokens";
const INPUT_128K_RATE: &str = "input_cost_per_token_above_128k_tokens";
const OUTPUT_128K_RATE: &str = "output_cost_per_token_above_128k_tokens";
const PROMPT_OVERHEAD_TOKENS: f64 = 1024.0;
const COMPLETION_TOKENS: f64 = 4096.0;

// the deployment's own rates win over the catalog, and extra router fields are ignored
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct DeploymentParams {
    pub model: String,
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub input_cost_per_token: Option<Decimal>,
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub output_cost_per_token: Option<Decimal>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Deployment {
    pub litellm_params: DeploymentParams,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prices {
    pub input_cost_per_token: Decimal,
    pub output_cost_per_token: Decimal,
    pub input_cost_per_token_above_200k_tokens: Decimal,
    pub output_cost_per_token_above_200k_tokens: Decimal,
    pub input_cost_per_token_above_128k_tokens: Decimal,
    pub output_cost_per_token_above_128k_tokens: Decimal,
}

#[derive(Debug, thiserror::Error)]
pub enum QuoteError {
    #[error("no analysis deployments are configured")]
    NoDeployments,
    #[error("prices for model {model} are unavailable: {source}")]
    Prices {
        model: String,
        #[source]
        source: ModelInfoError,
    },
    #[error("no price is published for model {model}")]
    UnknownModel { model: String },
    #[error("model {model} has no {rate}")]
    MissingRate { model: String, rate: &'static str },
    #[error("model {model} has a {rate} that is not a finite decimal")]
    InvalidRate { model: String, rate: &'static str },
    #[error("model {model} has a negative {rate}: {value}")]
    NegativeRate {
        model: String,
        rate: &'static str,
        value: Decimal,
    },
    #[error("the lens estimate is outside the decimal range")]
    Overflow,
}

fn rate(model: &str, info: &JsonObject, key: &'static str) -> Result<Option<Decimal>, QuoteError> {
    match info.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => {
            Decimal::from_str(&number.to_string())
                .map(Some)
                .map_err(|_| QuoteError::InvalidRate {
                    model: model.to_owned(),
                    rate: key,
                })
        }
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(QuoteError::InvalidRate {
                model: model.to_owned(),
                rate: key,
            })
        }
    }
}

fn base_rate(model: &str, info: &JsonObject, key: &'static str) -> Result<Decimal, QuoteError> {
    let amount = rate(model, info, key)?.ok_or_else(|| QuoteError::MissingRate {
        model: model.to_owned(),
        rate: key,
    })?;
    if amount.is_sign_negative() {
        return Err(QuoteError::NegativeRate {
            model: model.to_owned(),
            rate: key,
            value: amount,
        });
    }
    Ok(amount)
}

// a tier the catalog leaves out or nulls prices at zero, which LiteLLM's Prices model rejects
fn tier_rate(model: &str, info: &JsonObject, key: &'static str) -> Result<Decimal, QuoteError> {
    Ok(rate(model, info, key)?.unwrap_or(Decimal::ZERO))
}

fn prices_from_info(model: &str, info: &JsonObject) -> Result<Prices, QuoteError> {
    Ok(Prices {
        input_cost_per_token: base_rate(model, info, INPUT_RATE)?,
        output_cost_per_token: base_rate(model, info, OUTPUT_RATE)?,
        input_cost_per_token_above_200k_tokens: tier_rate(model, info, INPUT_200K_RATE)?,
        output_cost_per_token_above_200k_tokens: tier_rate(model, info, OUTPUT_200K_RATE)?,
        input_cost_per_token_above_128k_tokens: tier_rate(model, info, INPUT_128K_RATE)?,
        output_cost_per_token_above_128k_tokens: tier_rate(model, info, OUTPUT_128K_RATE)?,
    })
}

fn deployment_prices(
    deployment: &Deployment,
    prices: &dyn ModelInfoSource,
) -> Result<Prices, QuoteError> {
    let params = &deployment.litellm_params;
    if let (Some(input), Some(output)) = (params.input_cost_per_token, params.output_cost_per_token)
    {
        for (key, value) in [(INPUT_RATE, input), (OUTPUT_RATE, output)] {
            if value.is_sign_negative() {
                return Err(QuoteError::NegativeRate {
                    model: params.model.clone(),
                    rate: key,
                    value,
                });
            }
        }
        return Ok(Prices {
            input_cost_per_token: input,
            output_cost_per_token: output,
            input_cost_per_token_above_200k_tokens: Decimal::ZERO,
            output_cost_per_token_above_200k_tokens: Decimal::ZERO,
            input_cost_per_token_above_128k_tokens: Decimal::ZERO,
            output_cost_per_token_above_128k_tokens: Decimal::ZERO,
        });
    }
    let info = prices
        .get_model_info(&params.model)
        .map_err(|source| QuoteError::Prices {
            model: params.model.clone(),
            source,
        })?
        .ok_or_else(|| QuoteError::UnknownModel {
            model: params.model.clone(),
        })?;
    prices_from_info(&params.model, &info)
}

fn float_amount(rate: Decimal) -> f64 {
    rate.to_f64().unwrap_or(f64::INFINITY)
}

// inference.quote: the worst-case prompt plus a full completion, at the highest rate any deployment charges
pub fn quote(
    deployments: &[Deployment],
    prompt: &str,
    prices: &dyn ModelInfoSource,
) -> Result<Decimal, QuoteError> {
    if deployments.is_empty() {
        return Err(QuoteError::NoDeployments);
    }
    let mut input_rate = 0.0_f64;
    let mut output_rate = 0.0_f64;
    for deployment in deployments {
        let deployment = deployment_prices(deployment, prices)?;
        input_rate = input_rate
            .max(float_amount(deployment.input_cost_per_token))
            .max(float_amount(
                deployment.input_cost_per_token_above_200k_tokens,
            ))
            .max(float_amount(
                deployment.input_cost_per_token_above_128k_tokens,
            ));
        output_rate = output_rate
            .max(float_amount(deployment.output_cost_per_token))
            .max(float_amount(
                deployment.output_cost_per_token_above_200k_tokens,
            ))
            .max(float_amount(
                deployment.output_cost_per_token_above_128k_tokens,
            ));
    }
    let overhead = prompt
        .len()
        .checked_add(SYSTEM.len())
        .ok_or(QuoteError::Overflow)?;
    let tokens = u32::try_from(overhead).map_err(|_| QuoteError::Overflow)?;
    let input = (f64::from(tokens) + PROMPT_OVERHEAD_TOKENS) * input_rate;
    let output = COMPLETION_TOKENS * output_rate;
    Decimal::from_f64((input + output) * 2.0_f64).ok_or(QuoteError::Overflow)
}
