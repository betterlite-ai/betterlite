use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_openai::huggingface_chat::HuggingfaceEnv;
use betterlite_openai::huggingface_rerank::error::HuggingfaceRerankError;
use betterlite_openai::huggingface_rerank::headers::validate_environment;
use betterlite_openai::huggingface_rerank::params::{
    SUPPORTED_RERANK_PARAMS, map_cohere_rerank_params, transform_rerank_request,
};
use betterlite_openai::huggingface_rerank::response::{ResponseRequest, transform_rerank_response};
use betterlite_openai::huggingface_rerank::url::{get_api_base, get_complete_url};
use betterlite_openai::huggingface_rerank::{RequestIdSource, TextTokenCounter};
use betterlite_tokenizer::count::error::CountError;
use secrecy::SecretString;
use serde_json::{Value, json};
use thiserror::Error;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::input::{Input, ResponseStep, Step, UrlStep};

#[derive(Debug, Error)]
enum B5848Error {
    #[error(transparent)]
    Rerank(#[from] HuggingfaceRerankError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub(crate) fn run(input: &Input) -> HelperResult {
    let mut outcomes = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        outcomes.push(run_step(step).map_err(|error| helper_error(&error))?);
    }
    Ok(Value::Array(outcomes))
}

fn helper_error(error: &B5848Error) -> Box<HelperError> {
    let status = match error {
        B5848Error::Rerank(HuggingfaceRerankError::Huggingface(source)) => Some(source.status_code),
        B5848Error::Rerank(_) | B5848Error::Json(_) => None,
    };
    Box::new(HelperError {
        class: "HuggingfaceRerankError".to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn run_step(step: &Step) -> Result<Value, B5848Error> {
    match step {
        Step::Url(url) => Ok(url_value(url)),
        Step::SupportedParams => Ok(json!(SUPPORTED_RERANK_PARAMS)),
        Step::Params(params) => Ok(serde_json::to_value(map_cohere_rerank_params(
            params.non_default_params.as_ref(),
        ))?),
        Step::ValidateEnvironment(call) => {
            let key = call.api_key.as_deref().map(SecretString::from);
            Ok(json!(validate_environment(&call.headers, key.as_ref())))
        }
        Step::Request(request) => Ok(serde_json::to_value(transform_rerank_request(
            &request.params,
        )?)?),
        Step::Response(response) => response_value(response),
    }
}

fn url_value(step: &UrlStep) -> Value {
    let environment = environment(&step.env);
    json!({
        "base": get_api_base(step.api_base.as_deref(), &environment),
        "complete": get_complete_url(step.api_base.as_deref(), &environment),
    })
}

fn response_value(step: &ResponseStep) -> Result<Value, B5848Error> {
    let counter = RecordedCounter::new(&step.token_counts);
    let request = ResponseRequest {
        model: &step.model,
        status_code: Some(step.status),
        body: step.body.as_bytes(),
        request_data: &step.request_data,
    };
    let response = transform_rerank_response(&request, &counter, &FixedId(&step.id))?;
    Ok(serde_json::to_value(response)?)
}

fn environment(env: &BTreeMap<String, String>) -> HuggingfaceEnv {
    HuggingfaceEnv {
        hf_api_base: env.get("HF_API_BASE").cloned(),
        huggingface_api_base: env.get("HUGGINGFACE_API_BASE").cloned(),
    }
}

struct FixedId<'id>(&'id str);

impl RequestIdSource for FixedId<'_> {
    fn new_id(&self) -> String {
        self.0.to_owned()
    }
}

struct RecordedCounter {
    outcomes: Vec<Option<u64>>,
    calls: Mutex<usize>,
}

impl RecordedCounter {
    fn new(outcomes: &[Option<u64>]) -> Self {
        Self {
            outcomes: outcomes.to_vec(),
            calls: Mutex::new(0),
        }
    }
}

impl TextTokenCounter for RecordedCounter {
    fn count(&self, _model: &str, _text: &str) -> Result<u64, CountError> {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        let index = *calls;
        *calls = calls.saturating_add(1);
        drop(calls);
        match self.outcomes.get(index) {
            Some(Some(value)) => Ok(*value),
            Some(None) | None => Err(CountError::MissingInput),
        }
    }
}
