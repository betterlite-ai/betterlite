mod fixtures;
mod reports;
mod tests;

use betterlite_openai::openai_base_config::{Headers, Object};
use serde::Deserialize;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InferenceStep {
    pub(crate) endpoint: String,
    pub(crate) request_data: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RouterStep {
    pub(crate) endpoint: String,
    pub(crate) router_models: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeploymentStep {
    pub(crate) endpoint: String,
    pub(crate) model_group: String,
    pub(crate) served_models: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlStep {
    pub(crate) model: String,
    pub(crate) endpoint: String,
    #[serde(default)]
    pub(crate) litellm_params: Object,
    pub(crate) request_query_params: Option<Object>,
    pub(crate) api_base: Option<String>,
    pub(crate) env_base: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StreamStep {
    pub(crate) endpoint: String,
    pub(crate) chunks: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RelayStep {
    pub(crate) endpoint: String,
    pub(crate) model: String,
    pub(crate) chunks: Vec<String>,
    pub(crate) model_call_details: Option<Object>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NonstreamStep {
    pub(crate) endpoint: String,
    pub(crate) model: String,
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) headers: Headers,
    pub(crate) body: Object,
    #[serde(default)]
    pub(crate) request_data: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KeyStep {
    pub(crate) api_key: Option<String>,
    pub(crate) env_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ValidationStep {
    #[serde(default)]
    pub(crate) headers: Headers,
    pub(crate) api_key: Option<String>,
    pub(crate) params_key: Option<String>,
    pub(crate) configured_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    BodyModelInferenceEndpoint {
        inference_steps: Vec<InferenceStep>,
    },
    RouterModelInPath {
        router_steps: Vec<RouterStep>,
    },
    ForeignDeployment {
        deployment_steps: Vec<DeploymentStep>,
    },
    ApiVersionRemoval {
        removals: Vec<String>,
        url_steps: Vec<UrlStep>,
    },
    ResponsesStreamLog {
        stream_steps: Vec<StreamStep>,
    },
    MessagesRelayLog {
        relay_steps: Vec<RelayStep>,
    },
    NonstreamUsage {
        nonstream_steps: Vec<NonstreamStep>,
    },
    PassthroughAuthInputs {
        key_steps: Vec<KeyStep>,
        validation_steps: Vec<ValidationStep>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        reports::run(input)
    }
}
