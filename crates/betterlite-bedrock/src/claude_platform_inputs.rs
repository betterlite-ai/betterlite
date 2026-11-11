pub mod body;
pub mod endpoint;
pub mod error;
pub mod route;
pub mod signing;
pub mod workspace;

mod tests;

use std::cmp::Ordering;

use crate::model_routes::ParamValue;

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
    fn is_effective(self) -> bool {
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
fn select<const N: usize>(operands: [Option<Candidate<'_>>; N]) -> Option<Candidate<'_>> {
    let mut selected = None;
    for operand in operands {
        if operand.is_some_and(Candidate::is_effective) {
            return operand;
        }
        selected = operand;
    }
    selected
}

fn select_text<const N: usize>(values: [Option<&str>; N]) -> Option<&str> {
    let mut selected = None;
    for value in values {
        if value.is_some_and(|text| !text.is_empty()) {
            return value;
        }
        selected = value;
    }
    selected
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
