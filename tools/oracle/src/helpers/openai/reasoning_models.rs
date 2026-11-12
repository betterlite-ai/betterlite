use std::collections::{BTreeMap, BTreeSet};

use betterlite_openai::chat::ChatMessage;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::reasoning_models::ResolvedModel;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{Helper, HelperError, HelperResult};

mod calls;
mod fixture;
mod tests;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) chat_models: BTreeSet<String>,
    #[serde(default)]
    pub(crate) text_models: BTreeSet<String>,
    #[serde(default)]
    pub(crate) model_cost: BTreeMap<String, Value>,
    #[serde(default)]
    pub(crate) requested: Object,
    #[serde(default)]
    pub(crate) existing: Object,
    #[serde(default)]
    pub(crate) drop_params: bool,
    #[serde(default)]
    pub(crate) global_drop_params: bool,
    #[serde(default)]
    pub(crate) resolved: Option<ResolvedModel>,
    #[serde(default)]
    pub(crate) effective_effort: Option<String>,
    #[serde(default)]
    pub(crate) messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probes {
    pub(crate) probes: Vec<Probe>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Call {
    Gpt5SearchAllowlist(Probes),
    Gpt5ChatExclusion(Probes),
    Gpt6FamilySelection(Probes),
    DictEffortNormalization(Probes),
    XhighOptIn(Probes),
    LowMinimalOptOut(Probes),
    CatalogBeforeDefaultKey(Probes),
    TemperatureWhileThinking(Probes),
    SamplingWithNone(Probes),
    O1DeveloperRoles(Probes),
    AudioTokenParameter(Probes),
    Gpt5RegularAllowlist(Probes),
    VersionBoundaries(Probes),
    OSeriesCapabilityFallback(Probes),
    OSeriesTemperatureLimit(Probes),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Call;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| helper_error("RuntimeError", error.to_string()))?;
        runtime.block_on(calls::run(input))
    }
}

pub(crate) fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
