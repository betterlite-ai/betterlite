use rust_decimal::prelude::ToPrimitive as _;

use super::contracts::{InputBudget, ModelLimits, OutputAllowance};
use super::error::{CompactionError, ErrorKind};

pub const COMPACTOR_BUDGET_RATIO: f64 = 0.9;

pub fn budget(
    model: &str,
    limits: ModelLimits,
    allowance: OutputAllowance,
    ratio: f64,
) -> Result<InputBudget, CompactionError> {
    let output = allowance
        .max_completion_tokens
        .or(allowance.max_output_tokens)
        .or(allowance.max_tokens)
        .or(limits.max_output_tokens);
    if output.is_some_and(|value| value <= 0) {
        return Err(CompactionError {
            kind: ErrorKind::Rejected,
            model: model.to_owned(),
            reason: "The output allowance must be a positive integer".to_owned(),
        });
    }
    let available = match (limits.max_input_tokens, output) {
        (Some(window), Some(reserved)) => {
            let scaled = window.to_f64().unwrap_or_default();
            let truncated = scaled.mul_add(ratio, 0.0).to_i64().unwrap_or(i64::MAX);
            Some(truncated.saturating_sub(reserved))
        }
        (Some(_), None) | (None, Some(_)) | (None, None) => None,
    };
    Ok(InputBudget {
        window: limits.max_input_tokens,
        available,
    })
}
