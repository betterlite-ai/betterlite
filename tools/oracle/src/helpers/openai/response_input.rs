use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::response_input::error::ResponseInputError;
use betterlite_openai::response_input::{ResponsesInput, transform_responses_request};
use betterlite_prompts::arguments::json::Object;
use serde::{Deserialize, Serialize};

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    pub(crate) model: String,
    pub(crate) input: ResponsesInput,
    #[serde(default)]
    pub(crate) params: Object,
    #[serde(default)]
    pub(crate) provider: Option<ProviderId>,
    #[serde(default)]
    pub(crate) model_info: Option<Object>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probes {
    pub(crate) probes: Vec<Probe>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Probes;

    fn run(input: &Self::Input) -> HelperResult {
        let mut results = Vec::with_capacity(input.probes.len());
        for probe in &input.probes {
            results.push(serde_json::to_value(body_of(probe)?)?);
        }
        Ok(serde_json::Value::Array(results))
    }
}

fn body_of(probe: &Probe) -> Result<Object, Box<HelperError>> {
    transform_responses_request(
        &probe.model,
        &probe.input,
        &probe.params,
        probe.provider.unwrap_or(ProviderId::Openai),
        probe.model_info.as_ref(),
    )
    .map_err(|error| helper_error(&error))
}

fn helper_error(error: &ResponseInputError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ResponseInputError"),
        status: error.http_status(),
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}
