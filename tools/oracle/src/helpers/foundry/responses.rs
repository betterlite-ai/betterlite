use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod fixture;
mod steps;
mod tests;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

// a case tags each step with the route behaviour it checks, so one file covers several requests
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Step {
    Auth(AuthStep),
    DecisionsRoute(DecisionsStep),
    MissingBase(BaseStep),
    NoNativeWebsocket,
    Url(BaseStep),
}

#[derive(Deserialize)]
pub(crate) struct BaseStep {
    api_base: Option<String>,
    global_api_base: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Deserialize)]
pub(crate) struct AuthStep {
    api_base: Option<String>,
    api_key: Option<String>,
    global_api_base: Option<String>,
    global_api_key: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    params: Option<Object>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Deserialize)]
pub(crate) struct DecisionsStep {
    api_base: Option<String>,
    api_key: Option<String>,
    model: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        steps::run(input)
    }
}
