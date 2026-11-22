mod calls;
mod entry;

use std::collections::BTreeMap;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::runtime::Builder;

// ahealth_check inputs: the params map, the arguments it takes and the scripted dependencies around it
#[derive(Deserialize)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Deserialize)]
pub(crate) struct Step {
    pub(crate) params: serde_json::Map<String, Value>,
    pub(crate) mode: Option<String>,
    pub(crate) prompt: Option<String>,
    pub(crate) inputs: Option<Vec<String>>,
    // litellm.model_cost as the case scripts it: a model's entry, absent when the model is not a key
    pub(crate) catalog: BTreeMap<String, Option<String>>,
    // get_llm_provider's answer, or None for the failure that names the requested model
    pub(crate) resolution: Option<Resolution>,
    pub(crate) cheapest: Vec<String>,
    // the probe reply's hidden_params.headers; None is the reply that carries no headers value
    pub(crate) headers: Option<BTreeMap<String, String>>,
    pub(crate) failure: Option<String>,
    pub(crate) raw_request: Option<RawRequest>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct Resolution {
    pub(crate) model: String,
    pub(crate) provider: String,
}

// RawRequestTypedDict as the probe call records it
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct RawRequest {
    #[serde(rename = "raw_request_api_base")]
    pub(crate) api_base: Option<String>,
    #[serde(rename = "raw_request_body")]
    pub(crate) body: Option<Value>,
    #[serde(rename = "raw_request_headers")]
    pub(crate) headers: Option<Value>,
    pub(crate) error: Option<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| {
                Box::new(HelperError {
                    class: "RuntimeError".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            })?;
        let mut results = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            results.push(runtime.block_on(entry::run_step(step))?);
        }
        Ok(Value::Array(results))
    }
}
