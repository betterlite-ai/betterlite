use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Deserialize;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod stream;
mod success;
mod support;
mod views;

const GUARDRAIL_NAME: &str = "tool-permissions";
const DEFAULT_NOW: i64 = 1_700_000_000;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    rules: Vec<BTreeMap<String, GuardrailValue>>,
    default_action: String,
    on_disallowed_action: String,
    event_hook: String,
    default_on: bool,
    #[serde(default)]
    violation_message_template: Option<String>,
    mode: Mode,
    format: Format,
    #[serde(default)]
    data: BTreeMap<String, GuardrailValue>,
    #[serde(default)]
    response: Option<BTreeMap<String, GuardrailValue>>,
    #[serde(default)]
    chunks: Vec<BTreeMap<String, GuardrailValue>>,
    #[serde(default)]
    raw_chunks: Vec<String>,
    #[serde(default)]
    max_chunks: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Mode {
    Success,
    Streaming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Format {
    Openai,
    Anthropic,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.mode {
            Mode::Success => success::success(input),
            Mode::Streaming => stream::streaming(input),
        }
    }
}
