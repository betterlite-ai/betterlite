use betterlite_models::catalog::BundledCatalog;
use betterlite_tokenizer::selection::{
    fix_model_name, openai_tokenizer_encoding_name, uses_legacy_message_accounting,
};
use betterlite_tokenizer::tools::{
    ToolLimits, format_function_definitions, parse_tool_definitions,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    EncodingName { model: String },
    FixModelName { model: String },
    LegacyAccounting { model: String },
    FormatTools { tools: Value },
}

pub(crate) struct Op;

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::EncodingName { model } => Ok(json!(
                openai_tokenizer_encoding_name(model, &BundledCatalog::default().get()).name()
            )),
            Input::FixModelName { model } => Ok(json!(fix_model_name(
                model,
                &BundledCatalog::default().get()
            ))),
            Input::LegacyAccounting { model } => Ok(json!(uses_legacy_message_accounting(
                model,
                &BundledCatalog::default().get()
            ))),
            Input::FormatTools { tools } => {
                let bytes =
                    serde_json::to_vec(tools).map_err(|error| failure(error.to_string()))?;
                let definitions = parse_tool_definitions(&bytes, ToolLimits::default())
                    .map_err(|error| failure(error.to_string()))?;
                let formatted = format_function_definitions(&definitions)
                    .map_err(|error| failure(error.to_string()))?;
                Ok(Value::String(formatted))
            }
        }
    }
}
