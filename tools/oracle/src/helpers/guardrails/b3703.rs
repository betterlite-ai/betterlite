use std::str::FromStr as _;

use betterlite_guardrails::prompt_injection::keywords::generate_injection_keywords;
use betterlite_guardrails::prompt_injection::similarity::check_user_input_similarity;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    Keywords,
    Similarity {
        user_input: String,
        threshold: String,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Keywords => Ok(serde_json::json!(generate_injection_keywords())),
            Input::Similarity {
                user_input,
                threshold: threshold_text,
            } => {
                let threshold = Decimal::from_str(threshold_text).or_value_error()?;
                let keywords = generate_injection_keywords();
                Ok(serde_json::json!(
                    check_user_input_similarity(&keywords, user_input, threshold, None)
                        .or_value_error()?
                ))
            }
        }
    }
}
