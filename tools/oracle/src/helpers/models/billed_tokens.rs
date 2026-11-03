use betterlite_models::billed_tokens::custom_cost;
use rust_decimal::Decimal;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> HelperResult {
        let field = |name: &str| input.get(name);
        let token_rates = field("custom_cost_per_token")
            .and_then(Value::as_object)
            .map(|rates| {
                Ok::<_, Box<HelperError>>((
                    decimal(rates.get("input_cost_per_token"))?,
                    decimal(rates.get("output_cost_per_token"))?,
                    optional_decimal(rates.get("cache_read_input_token_cost"))?,
                    optional_decimal(rates.get("cache_creation_input_token_cost"))?,
                ))
            })
            .transpose()?;
        let cost = custom_cost(
            integer(field("prompt_tokens"))?,
            integer(field("completion_tokens"))?,
            integer(field("cached_tokens"))?,
            integer(field("cache_creation_tokens"))?,
            token_rates,
            optional_decimal(field("custom_cost_per_second"))?,
            optional_decimal(field("response_time_ms"))?,
        )
        .map_err(|error| helper_error("BilledTokenError", error.to_string()))?;
        let Some((input_cost, output_cost)) = cost else {
            return Ok(Value::Null);
        };
        Ok(json!({
            "input_cost": input_cost.normalize().to_string(),
            "output_cost": output_cost.normalize().to_string(),
        }))
    }
}

fn decimal(value: Option<&Value>) -> Result<Decimal, Box<HelperError>> {
    let text = value
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            value
                .and_then(Value::as_number)
                .map(|number| number.to_string())
        })
        .ok_or_else(|| helper_error("InvalidInput", "expected a decimal".to_owned()))?;
    Decimal::from_str_exact(&text).map_err(|error| helper_error("InvalidInput", error.to_string()))
}

fn optional_decimal(value: Option<&Value>) -> Result<Option<Decimal>, Box<HelperError>> {
    value
        .filter(|value| !value.is_null())
        .map(|value| decimal(Some(value)))
        .transpose()
}

fn integer(value: Option<&Value>) -> Result<i64, Box<HelperError>> {
    match value {
        Some(value) => value
            .as_i64()
            .ok_or_else(|| helper_error("InvalidInput", "expected an integer".to_owned())),
        None => Ok(0),
    }
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
