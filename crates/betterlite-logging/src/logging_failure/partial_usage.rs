use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use serde_json::Number;
use std::str::FromStr as _;

use crate::fields::timestamps::with_fraction;
use crate::fields::usage::Usage;
use crate::fields::usage::project::usage_json;
use crate::logging_state::LoggingState;

pub trait AssembledResponse {
    fn usage(&self) -> Option<&Usage>;
}

// ast-grep-ignore: no-long-names
pub fn record_partial_usage_for_failure(
    state: &mut LoggingState,
    usage: &Usage,
    response_cost: Decimal,
) {
    state
        .model_call_details
        .insert("combined_usage_object".to_owned(), usage_json(usage));
    state
        .model_call_details
        .insert("response_cost".to_owned(), money_json(response_cost));
}

// ast-grep-ignore: no-long-names
pub fn record_assembled_response_for_failure<Response, Cost>(
    state: &mut LoggingState,
    assembled: &Response,
    response_cost: Cost,
) -> Option<Decimal>
where
    Response: AssembledResponse,
    Cost: FnOnce(&Response) -> Option<Decimal>,
{
    let usage = assembled.usage()?;
    let cost = response_cost(assembled).unwrap_or(Decimal::ZERO);
    // ast-grep-ignore: no-long-names
    record_partial_usage_for_failure(state, usage, cost);
    Some(cost)
}

fn money_json(value: Decimal) -> Json {
    match Number::from_str(&with_fraction(value).to_string()) {
        Ok(number) => Json::Number(number),
        Err(_error) => Json::Null,
    }
}
