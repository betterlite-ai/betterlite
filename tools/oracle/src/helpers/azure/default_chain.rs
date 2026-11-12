use std::collections::BTreeMap;

use betterlite_azure::azure_default_chain::environment::ChainEnvironment;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::azure_default_chain::{AzureCredentialType, Settings};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::OrHelperError as _;
use crate::helpers::{Helper, HelperError, HelperResult};

mod fields;
mod fixture;
mod report;
mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Infer { steps } => Ok(report::infer_steps(steps)),
            Input::Provider { steps } => steps
                .iter()
                .map(report::provider_value)
                .collect::<Result<Vec<Value>, Box<HelperError>>>()
                .map(|values| json!({ "steps": values })),
            Input::Chain { steps } => steps
                .iter()
                .map(report::chain_value)
                .collect::<Result<Vec<Value>, Box<HelperError>>>()
                .map(|values| json!({ "steps": values })),
            Input::TryBoundary { steps } => steps
                .iter()
                .map(report::boundary_value)
                .collect::<Result<Vec<Value>, Box<HelperError>>>()
                .map(|values| json!({ "steps": values })),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Infer { steps: Vec<InferStep> },
    Provider { steps: Vec<ProviderStep> },
    Chain { steps: Vec<ChainStep> },
    TryBoundary { steps: Vec<TryStep> },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InferStep {
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProviderStep {
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) credential: Option<String>,
    #[serde(default)]
    pub(super) scope: Option<String>,
    #[serde(default)]
    pub(super) construct_errors: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ChainStep {
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) credential: Option<String>,
    #[serde(default)]
    pub(super) scope: Option<String>,
    #[serde(default)]
    pub(super) construct_errors: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) scripts: Vec<fixture::ScriptStep>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TryStep {
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    pub(super) scope: String,
    #[serde(default)]
    pub(super) construct_errors: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) scripts: Vec<fixture::ScriptStep>,
    #[serde(default = "default_invoke")]
    pub(super) invoke: bool,
}

const fn default_invoke() -> bool {
    true
}

pub(super) fn environment_of(
    variables: &BTreeMap<String, String>,
) -> Result<ChainEnvironment, Box<HelperError>> {
    let settings: Settings = ConfigLoader {
        lookup: variables.clone(),
    }
    .load("")
    .map(|config| config.sections.azure_default_chain)
    .or_config_error()?;
    Ok(ChainEnvironment::from_settings(&settings, variables))
}

fn unsupported(name: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: format!("unknown credential {name}"),
    })
}

pub(super) fn requested(
    name: Option<&str>,
) -> Result<Option<AzureCredentialType>, Box<HelperError>> {
    name.map_or(Ok(None), |value| {
        AzureCredentialType::parse(value)
            .map(Some)
            .ok_or_else(|| unsupported(value))
    })
}
