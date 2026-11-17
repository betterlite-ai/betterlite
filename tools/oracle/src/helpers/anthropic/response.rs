use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use serde::Deserialize;

mod fixture;
mod state;
mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    id: String,
    status: u16,
    raw_text: String,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    messages: Vec<Json>,
    #[serde(default)]
    optional_params: IndexMap<String, Json>,
    #[serde(default)]
    litellm_params: IndexMap<String, Json>,
    #[serde(default)]
    request_data: IndexMap<String, Json>,
    #[serde(default)]
    json_mode: Option<bool>,
    now: i64,
    #[serde(default)]
    disable_add_prefix_to_prompt: bool,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        fixture::run(input)
    }
}
