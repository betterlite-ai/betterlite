pub mod body;
pub mod endpoint;
pub mod error;
pub mod signing;
pub mod workspace;

mod tests;

use std::cmp::Ordering;

use crate::model_routes::{ParamMap, ParamValue};

pub const CLAUDE_PLATFORM_SERVICE_NAME: &str = "aws-external-anthropic";
pub const CLAUDE_PLATFORM_BEDROCK_ROUTE: &str = "claude_platform/";
pub const OVERRIDE_PARAMS_KEY: &str = "claude_platform_unsupported_params";
pub const NON_REQUEST_PARAMS: [&str; 5] = [
    "workspace_id",
    "aws_workspace_id",
    "anthropic_workspace_id",
    "anthropic-workspace-id",
    OVERRIDE_PARAMS_KEY,
];
pub const DEFAULT_UNSUPPORTED: [&str; 1] = ["context_management"];

#[derive(Clone, Copy)]
pub(super) enum Candidate<'source> {
    Param {
        field: &'static str,
        value: &'source ParamValue,
    },
    Text(&'source str),
}

impl Candidate<'_> {
    fn is_effective(&self) -> bool {
        match self {
            Self::Text(value) => !value.is_empty(),
            Self::Param { value, .. } => is_effective(value),
        }
    }

    fn render(self) -> Result<String, error::PlatformInputError> {
        match self {
            Self::Text(value) => Ok(value.to_owned()),
            Self::Param { field, value } => render_param(field, value),
        }
    }
}

// Python's `a or b or c` keeps the last operand when none of them is truthy
fn select_matching<T, const N: usize>(
    operands: [Option<T>; N],
    is_effective: fn(&T) -> bool,
) -> Option<T> {
    let mut selected = None;
    for operand in operands {
        if operand.as_ref().is_some_and(is_effective) {
            return operand;
        }
        selected = operand;
    }
    selected
}

pub(super) fn optional_candidate<'source>(
    params: &'source ParamMap,
    key: &'static str,
) -> Option<Candidate<'source>> {
    params
        .get(key)
        .map(|value| Candidate::Param { field: key, value })
}

pub(super) fn is_effective(value: &ParamValue) -> bool {
    match value {
        ParamValue::Null => false,
        ParamValue::Bool(flag) => *flag,
        ParamValue::Number(number) => number
            .as_f64()
            .is_none_or(|parsed| parsed.partial_cmp(&0.0_f64) != Some(Ordering::Equal)),
        ParamValue::Text(text) => !text.is_empty(),
        ParamValue::List(items) => !items.is_empty(),
        ParamValue::Mapping(fields) => !fields.is_empty(),
    }
}

pub(super) fn render_param(
    field: &'static str,
    value: &ParamValue,
) -> Result<String, error::PlatformInputError> {
    match value {
        ParamValue::Text(text) => Ok(text.clone()),
        ParamValue::Bool(true) => Ok("True".to_owned()),
        ParamValue::Bool(false) => Ok("False".to_owned()),
        ParamValue::Number(number) => Ok(number.to_string()),
        ParamValue::Null => Ok(String::new()),
        ParamValue::List(_) | ParamValue::Mapping(_) => {
            Err(error::PlatformInputError::ValueType { field })
        }
    }
}
