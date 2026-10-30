mod block;
pub mod rate;
pub mod schema;
mod tests;
mod weekday;
pub mod window;

use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use rate::OffPeakRateError;
use schema::OffPeakBlock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenRates {
    pub input_rate: Decimal,
    pub output_rate: Decimal,
    pub cache_read_rate: Decimal,
    pub cache_creation_rate: Decimal,
    pub reasoning_rate: Option<Decimal>,
}

impl TokenRates {
    #[must_use]
    pub const fn billed_reasoning_rate(&self) -> Decimal {
        if let Some(rate) = self.reasoning_rate {
            return rate;
        }
        self.output_rate
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct BaseCosts {
    pub prompt_rate: Decimal,
    pub completion_rate: Decimal,
    pub cache_creation_rate: Decimal,
    pub cache_creation_above_1hr_rate: Option<Decimal>,
    pub cache_read_rate: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ResolvedBaseCosts {
    pub prompt_rate: Decimal,
    pub completion_rate: Decimal,
    pub cache_creation_rate: Decimal,
    pub cache_creation_above_1hr_rate: Decimal,
    pub cache_read_rate: Decimal,
}

pub fn apply_off_peak_pricing(
    off_peak_pricing: Option<&OffPeakBlock>,
    moment: DateTime<Utc>,
    rates: TokenRates,
    database: &TimeZoneDatabase,
) -> Result<TokenRates, OffPeakRateError> {
    let Some(block) = off_peak_pricing else {
        return Ok(rates);
    };
    if !block::is_off_peak(block, moment, database) {
        return Ok(rates);
    }
    let off_peak_reasoning_rate = rate::parse_rate(
        block.reasoning_token_cost.as_ref(),
        "output_cost_per_reasoning_token",
    )?;
    Ok(TokenRates {
        input_rate: rate::parse_rate(block.input_cost_per_token.as_ref(), "input_cost_per_token")?
            .unwrap_or(rates.input_rate),
        output_rate: rate::parse_rate(
            block.output_cost_per_token.as_ref(),
            "output_cost_per_token",
        )?
        .unwrap_or(rates.output_rate),
        cache_read_rate: rate::parse_rate(
            block.cache_read_input_token_cost.as_ref(),
            "cache_read_input_token_cost",
        )?
        .unwrap_or(rates.cache_read_rate),
        cache_creation_rate: rate::parse_rate(
            block.cache_creation_cost.as_ref(),
            "cache_creation_input_token_cost",
        )?
        .unwrap_or(rates.cache_creation_rate),
        reasoning_rate: off_peak_reasoning_rate.or(rates.reasoning_rate),
    })
}

impl From<BaseCosts> for TokenRates {
    fn from(base_costs: BaseCosts) -> Self {
        Self {
            input_rate: base_costs.prompt_rate,
            output_rate: base_costs.completion_rate,
            cache_read_rate: base_costs.cache_read_rate,
            cache_creation_rate: base_costs.cache_creation_rate,
            reasoning_rate: None,
        }
    }
}

pub fn apply_off_peak_to_base_costs(
    off_peak_pricing: Option<&OffPeakBlock>,
    moment: DateTime<Utc>,
    base_costs: BaseCosts,
    database: &TimeZoneDatabase,
) -> Result<ResolvedBaseCosts, OffPeakRateError> {
    let above_1hr_rate = base_costs.cache_creation_above_1hr_rate;
    let rates = apply_off_peak_pricing(
        off_peak_pricing,
        moment,
        TokenRates::from(base_costs),
        database,
    )?;
    let cache_creation_above_1hr_rate = above_1hr_rate.unwrap_or(rates.cache_creation_rate);
    Ok(ResolvedBaseCosts {
        prompt_rate: rates.input_rate,
        completion_rate: rates.output_rate,
        cache_creation_rate: rates.cache_creation_rate,
        cache_creation_above_1hr_rate,
        cache_read_rate: rates.cache_read_rate,
    })
}
