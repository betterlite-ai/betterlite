use betterlite_core::cost::call_price::error::CallPriceError;
use http::HeaderMap;
use rust_decimal::Decimal;
use thiserror::Error;

use crate::types::JsonObject;

pub const COST_HEADER: &str = "x-litellm-response-cost";

#[derive(Debug, Error)]
pub enum CostError {
    #[error("no price is known for model {model}")]
    Unpriced {
        model: String,
        #[source]
        source: Box<CallPriceError>,
    },
    #[error("the token count {tokens} does not fit the price lookup")]
    CountOverflow { tokens: u64 },
    #[error("the price of {model} overflows the cost total")]
    Overflow { model: String },
}

pub trait CostPerToken: Send + Sync {
    fn cost_per_token(
        &self,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
    ) -> Result<Decimal, CostError>;
}

#[must_use]
pub fn compute_cost(
    prices: &dyn CostPerToken,
    model: Option<&str>,
    input_tokens: u64,
    output_tokens: u64,
) -> Decimal {
    let Some(name) = model else {
        return Decimal::ZERO;
    };
    if input_tokens == 0 && output_tokens == 0 {
        return Decimal::ZERO;
    }
    match prices.cost_per_token(name, input_tokens, output_tokens) {
        Ok(cost) => cost,
        Err(error) => {
            tracing::debug!(%error, model = name, "harness endpoint: cost lookup failed");
            Decimal::ZERO
        }
    }
}

#[must_use]
pub fn header_cost(headers: &HeaderMap) -> Option<Decimal> {
    let raw = headers.get(COST_HEADER)?.to_str().ok()?;
    let text = raw.trim();
    text.parse()
        .or_else(|_error| Decimal::from_scientific(text))
        .ok()
}

#[must_use]
pub fn hidden_cost(hidden_params: &JsonObject) -> Option<Decimal> {
    let number = hidden_params.get("response_cost")?.as_number()?;
    let text = number.as_str();
    text.parse()
        .or_else(|_error| Decimal::from_scientific(text))
        .ok()
}
