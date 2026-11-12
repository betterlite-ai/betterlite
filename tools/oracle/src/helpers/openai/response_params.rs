use std::collections::BTreeMap;

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::response_params::capabilities::CUSTOM_LLM_PROVIDER;
use betterlite_openai::response_params::params::{ResponsesOptions, ResponsesParams};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{Helper, HelperError, HelperResult};

mod calls;
mod fixture;
mod tests;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    #[serde(default)]
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) provider: Option<ProviderId>,
    #[serde(default)]
    pub(crate) params: ResponsesParams,
    #[serde(default)]
    pub(crate) model_cost: BTreeMap<String, Value>,
    #[serde(default)]
    pub(crate) drop_params: bool,
    #[serde(default)]
    pub(crate) global_drop_params: bool,
    #[serde(default)]
    pub(crate) body: Option<Value>,
}

impl Probe {
    pub(crate) fn options(&self) -> ResponsesOptions {
        ResponsesOptions {
            provider: self.provider.unwrap_or(CUSTOM_LLM_PROVIDER),
            model: self.model.clone(),
            params: self.params.clone(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probes {
    pub(crate) probes: Vec<Probe>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Call {
    MinimumOutputTokens(Probes),
    #[serde(rename = "reasoning_unsupported_drop_or_error")]
    ReasoningRejected(Probes),
    Gpt5TemperatureDefaultEffort(Probes),
    TopPWithReasoning(Probes),
    UnknownCatalogModel(Probes),
    ReasoningSummaryWithoutEffort(Probes),
    ResponsesParameterSet(Probes),
    ProviderCapabilities(Probes),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Call;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| {
                Box::new(HelperError {
                    class: String::from("RuntimeError"),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            })?;
        runtime.block_on(calls::run(input))
    }
}
