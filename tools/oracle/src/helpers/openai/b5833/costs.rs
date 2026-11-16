use betterlite_models::price_fields::nested::SearchContextPricing;
use betterlite_openai::groq_cost::{ServerToolUse, web_search_cost};
use rust_decimal::Decimal;
use serde_json::{Value, json};

use super::{Call, helper_error};
use betterlite_oracle_helper_api::error::HelperError;

pub(super) fn cost_values(calls: &[Call]) -> Result<Vec<Value>, Box<HelperError>> {
    let mut values = Vec::with_capacity(calls.len());
    for call in calls {
        let rate = call.medium_rate.as_deref().map(parse_rate).transpose()?;
        let pricing = SearchContextPricing {
            search_context_size_low: None,
            search_context_size_medium: rate,
            search_context_size_high: None,
        };
        let usage = ServerToolUse {
            web_search_requests: call.web_search_requests,
            browser_open_requests: call.browser_open_requests,
        };
        let used_pricing = call.medium_rate.is_some().then_some(&pricing);
        let cost = web_search_cost(Some(&usage), used_pricing)
            .map_err(|error| helper_error("CostFailure", None, error.to_string()))?;
        values.push(json!({"cost": format!("{cost:.12}")}));
    }
    Ok(values)
}

fn parse_rate(text: &str) -> Result<Decimal, Box<HelperError>> {
    text.parse::<Decimal>().map_err(|_error| {
        helper_error(
            "InvalidRate",
            None,
            format!("the medium rate is not a decimal: {text}"),
        )
    })
}
