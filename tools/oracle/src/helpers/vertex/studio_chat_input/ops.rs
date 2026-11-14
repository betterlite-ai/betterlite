use std::collections::BTreeSet;
use std::future::{Future, ready};

use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_prompts::message_content::Message;
use betterlite_vertex::gemini_options::params::ReasoningSupport;
use betterlite_vertex::studio_chat::{supported_openai_params, transform_messages};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use crate::helpers::{HelperError, HelperResult};

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    reasoning_models: Vec<String>,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    op: Operation,
    #[serde(default)]
    model: String,
    #[serde(default)]
    messages: Vec<Message>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Operation {
    Params,
    Messages,
}

struct Catalog(BTreeSet<String>);

impl ReasoningSupport for Catalog {
    fn supports_reasoning(&self, model: &str) -> bool {
        self.0.contains(model)
    }
}

struct NoFetch;

impl RemoteMediaFetcher for NoFetch {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(Err(ImageFetchError::Attempts {
            url: url.to_owned(),
        }))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        let url = urls.first().cloned().unwrap_or_default();
        ready(Err(ImageFetchError::Attempts { url }))
    }
}

pub(super) fn run(input: &Input) -> HelperResult {
    let catalog = Catalog(input.reasoning_models.iter().cloned().collect());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| runtime_error(&error))?;
    let results: Vec<Value> = input
        .steps
        .iter()
        .map(|step| answer(&runtime, step, &catalog))
        .collect();
    Ok(json!({"results": results}))
}

fn answer(runtime: &Runtime, step: &Step, catalog: &Catalog) -> Value {
    match step.op {
        Operation::Params => json!({"params": supported_openai_params(&step.model, catalog)}),
        Operation::Messages => {
            match runtime.block_on(transform_messages(&step.messages, &step.model, &NoFetch)) {
                Ok(history) => json!({"contents": history.contents}),
                Err(error) => json!({"error": {
                    "status": error.status(),
                    "type": error.error_type(),
                    "param": error.param(),
                    "code": error.code(),
                }}),
            }
        }
    }
}

fn runtime_error(error: &std::io::Error) -> Box<HelperError> {
    Box::new(HelperError {
        class: "RuntimeError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
