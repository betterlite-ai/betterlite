use betterlite_auth::reservation_cost::ModelInfoSource;
use betterlite_auth::reservation_cost::error::ModelInfoError;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive as _, ToPrimitive as _};
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
    pub input_cost_above_200k: Decimal,
    pub output_cost_above_200k: Decimal,
    pub input_cost_above_128k: Decimal,
    pub output_cost_above_128k: Decimal,
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

#[derive(Clone, Copy)]
enum Rule {
    Required,
    Optional,
}

fn missing_rate(model: &str, key: &'static str, rule: Rule) -> Result<Decimal, QuoteError> {
    match rule {
        Rule::Required => Err(QuoteError::MissingRate {
            model: model.to_owned(),
            rate: key,
        }),
        Rule::Optional => Ok(Decimal::ZERO),
    }
}

fn rate(
    model: &str,
    info: &JsonObject,
    key: &'static str,
    rule: Rule,
) -> Result<Decimal, QuoteError> {
    let Some(value) = info.get(key) else {
        return missing_rate(model, key, rule);
    };
    if matches!(value, Json::Null) {
        return missing_rate(model, key, rule);
    }
    let Json::Number(number) = value else {
        return Err(QuoteError::InvalidRate {
            model: model.to_owned(),
            rate: key,
        });
    };
    // the catalog and serde_json spell a rate like 1.25e-05, which Decimal's FromStr refuses
    let Ok(amount) = parse_decimal_lexeme(&number.to_string()) else {
        return Err(QuoteError::InvalidRate {
            model: model.to_owned(),
            rate: key,
        });
    };
    if matches!(rule, Rule::Required) && amount.is_sign_negative() {
        return Err(QuoteError::NegativeRate {
            model: model.to_owned(),
            rate: key,
            value: amount,
        });
    }
    Ok(amount)
}

// a tier the catalog leaves out or nulls prices at zero, which LiteLLM's Prices model rejects
fn prices_from_info(model: &str, info: &JsonObject) -> Result<Prices, QuoteError> {
    Ok(Prices {
        input_cost_per_token: rate(model, info, INPUT_RATE, Rule::Required)?,
        output_cost_per_token: rate(model, info, OUTPUT_RATE, Rule::Required)?,
        input_cost_above_200k: rate(model, info, INPUT_200K_RATE, Rule::Optional)?,
        output_cost_above_200k: rate(model, info, OUTPUT_200K_RATE, Rule::Optional)?,
        input_cost_above_128k: rate(model, info, INPUT_128K_RATE, Rule::Optional)?,
        output_cost_above_128k: rate(model, info, OUTPUT_128K_RATE, Rule::Optional)?,
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
            input_cost_above_200k: Decimal::ZERO,
            output_cost_above_200k: Decimal::ZERO,
            input_cost_above_128k: Decimal::ZERO,
            output_cost_above_128k: Decimal::ZERO,
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
        let rates = deployment_prices(deployment, prices)?;
        let input = rates
            .input_cost_per_token
            .max(rates.input_cost_above_200k)
            .max(rates.input_cost_above_128k);
        let output = rates
            .output_cost_per_token
            .max(rates.output_cost_above_200k)
            .max(rates.output_cost_above_128k);
        input_rate = input_rate.max(input.to_f64().unwrap_or(f64::INFINITY));
        output_rate = output_rate.max(output.to_f64().unwrap_or(f64::INFINITY));
    }
    let overhead = prompt
        .len()
        .checked_add(SYSTEM.len())
        .ok_or(QuoteError::Overflow)?;
    let Ok(tokens) = u32::try_from(overhead) else {
        return Err(QuoteError::Overflow);
    };
    // float_arithmetic is denied; a fused multiply-add computes the same sum, product and doubling
    let input = f64::from(tokens)
        .mul_add(1.0_f64, PROMPT_OVERHEAD_TOKENS)
        .mul_add(input_rate, 0.0_f64);
    let output = COMPLETION_TOKENS.mul_add(output_rate, 0.0_f64);
    Decimal::from_f64(input.mul_add(1.0_f64, output).mul_add(2.0_f64, 0.0_f64))
        .ok_or(QuoteError::Overflow)
}
