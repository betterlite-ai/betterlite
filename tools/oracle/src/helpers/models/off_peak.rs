use betterlite_models::off_peak::schema::OffPeakBlockEntry;
use betterlite_models::off_peak::{
    BaseCosts, ResolvedBaseCosts, TokenRates, apply_off_peak_pricing, apply_off_peak_to_base_costs,
};
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    off_peak_pricing: Option<OffPeakBlockEntry>,
    moment: DateTime<Utc>,
    rates: TokenRates,
    base_costs: BaseCosts,
}

#[derive(Serialize)]
struct Output {
    rates: RatesOutput,
    base_costs: ResolvedBaseCosts,
}

#[derive(Serialize)]
struct RatesOutput {
    #[serde(flatten)]
    rates: TokenRates,
    billed_reasoning_rate: Decimal,
}

impl From<TokenRates> for RatesOutput {
    fn from(rates: TokenRates) -> Self {
        Self {
            billed_reasoning_rate: rates.billed_reasoning_rate(),
            rates,
        }
    }
}

fn helper_error(class: &str, message: String) -> Box<crate::helpers::HelperError> {
    let error = crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    };
    Box::new(error)
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let off_peak_pricing = input
            .off_peak_pricing
            .as_ref()
            .and_then(OffPeakBlockEntry::as_block);
        let database = TimeZoneDatabase::from_env();
        let rates = apply_off_peak_pricing(off_peak_pricing, input.moment, input.rates, &database)
            .map_err(|error| helper_error("OffPeakRateError", error.to_string()))?;
        let base_costs = apply_off_peak_to_base_costs(
            off_peak_pricing,
            input.moment,
            input.base_costs,
            &database,
        )
        .map_err(|error| helper_error("OffPeakRateError", error.to_string()))?;
        let output = Output {
            rates: RatesOutput::from(rates),
            base_costs,
        };
        serde_json::to_value(output).map_err(|error| helper_error("Serialize", error.to_string()))
    }
}
