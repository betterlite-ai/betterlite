mod bases;
mod carriers;
mod control;
mod counting;
mod extracts;
mod models;
mod stamping;

use serde::Deserialize;

use crate::helpers::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    GptVersion { models: Vec<String> },
    ModelMap { probes: Vec<models::MapProbe> },
    OpenaiBases { probes: Vec<bases::Probe> },
    CarrierDetection(carriers::Call),
    RequestExtraction { probes: Vec<extracts::Probe> },
    BreakpointCounting { probes: Vec<counting::Probe> },
    DefaultControl { settings: Vec<Option<String>> },
    Stamping { probes: Vec<stamping::Probe> },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        Ok(match input {
            Input::GptVersion { models } => models::gpt_versions(models),
            Input::ModelMap { probes } => models::model_map(probes),
            Input::OpenaiBases { probes } => bases::bases(probes),
            Input::CarrierDetection(call) => carriers::carriers(call),
            Input::RequestExtraction { probes } => extracts::extracts(probes),
            Input::BreakpointCounting { probes } => counting::counts(probes),
            Input::DefaultControl { settings } => control::controls(settings),
            Input::Stamping { probes } => stamping::stamps(probes),
        })
    }
}
