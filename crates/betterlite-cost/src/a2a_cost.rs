use rust_decimal::Decimal;
use thiserror::Error;

mod tests;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct A2aCostParams {
    pub response_cost: Option<Decimal>,
    pub cost_per_query: Option<Decimal>,
    pub input_cost_per_token: Option<Decimal>,
    pub output_cost_per_token: Option<Decimal>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct A2aUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum A2aCostError {
    #[error("the A2A cost arithmetic overflowed a decimal")]
    Overflow,
}

pub fn calculate_a2a_cost(
    params: &A2aCostParams,
    usage: Option<A2aUsage>,
) -> Result<Decimal, A2aCostError> {
    if let Some(response_cost) = params.response_cost {
        return Ok(response_cost);
    }
    if let Some(cost_per_query) = params.cost_per_query {
        return Ok(cost_per_query);
    }
    if params.input_cost_per_token.is_none() && params.output_cost_per_token.is_none() {
        return Ok(Decimal::ZERO);
    }
    let Some(counted) = usage else {
        return Ok(Decimal::ZERO);
    };
    let input = multiply(counted.prompt_tokens, params.input_cost_per_token)?;
    let output = multiply(counted.completion_tokens, params.output_cost_per_token)?;
    input.checked_add(output).ok_or(A2aCostError::Overflow)
}

fn multiply(tokens: u64, rate: Option<Decimal>) -> Result<Decimal, A2aCostError> {
    Decimal::from(tokens)
        .checked_mul(rate.unwrap_or(Decimal::ZERO))
        .ok_or(A2aCostError::Overflow)
}
