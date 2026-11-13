use std::collections::BTreeMap;
use std::sync::Arc;
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
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::assets;
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};
use secrecy::SecretString;
use serde_json::{Value, json};
use thiserror::Error;

use crate::helpers::{HelperError, HelperResult};

use super::input::{Input, ResponseStep, Step, UrlStep, ValidateEnvironmentStep};

#[derive(Debug, Error)]
enum B5848Error {
    #[error(transparent)]
    Rerank(#[from] HuggingfaceRerankError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("the case carries no token counter for the response")]
    Counter,
}

pub(crate) fn run(input: &Input) -> HelperResult {
    let needs_encoder = input
        .steps
        .iter()
        .any(|step| matches!(step, Step::Response(response) if response.token_counts.is_none()));
    let encoder = if needs_encoder {
        Some(BundledCl100k::build()?)
    } else {
        None
    };
    let mut outcomes = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        outcomes.push(run_step(step, encoder.as_ref()).map_err(helper_error)?);
    }
    Ok(Value::Array(outcomes))
}

fn helper_error(error: B5848Error) -> Box<HelperError> {
    let status = match &error {
        B5848Error::Rerank(HuggingfaceRerankError::Huggingface(source)) => Some(source.status_code),
        B5848Error::Rerank(_) | B5848Error::Json(_) | B5848Error::Counter => None,
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

fn run_step(step: &Step, encoder: Option<&BundledCl100k>) -> Result<Value, B5848Error> {
    match step {
        Step::Url(url) => Ok(url_value(url)),
        Step::SupportedParams(_) => Ok(json!(SUPPORTED_RERANK_PARAMS)),
        Step::Params(params) => Ok(serde_json::to_value(map_cohere_rerank_params(
            params.non_default_params.as_ref(),
        ))?),
        Step::ValidateEnvironment(environment) => Ok(validate_value(environment)),
        Step::Request(request) => Ok(serde_json::to_value(transform_rerank_request(
            &request.params,
        )?)?),
        Step::Response(response) => response_value(response, encoder),
    }
}

fn url_value(step: &UrlStep) -> Value {
    let environment = environment(&step.env);
    json!({
        "base": get_api_base(step.api_base.as_deref(), &environment),
        "complete": get_complete_url(step.api_base.as_deref(), &environment),
    })
}

fn validate_value(step: &ValidateEnvironmentStep) -> Value {
    let key = step.api_key.as_deref().map(SecretString::from);
    json!(validate_environment(&step.headers, key.as_ref()))
}

fn response_value(
    step: &ResponseStep,
    encoder: Option<&BundledCl100k>,
) -> Result<Value, B5848Error> {
    let recorded = step
        .token_counts
        .as_ref()
        .map(|counts| RecordedCounter::new(counts));
    let counter: &dyn TextTokenCounter = if let Some(counter) = &recorded {
        counter
    } else if let Some(encoder) = encoder {
        encoder
    } else {
        return Err(B5848Error::Counter);
    };
    let request = ResponseRequest {
        model: &step.model,
        status_code: Some(step.status),
        body: step.body.as_bytes(),
        request_data: &step.request_data,
    };
    let response = transform_rerank_response(&request, counter, &FixedId(&step.id))?;
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
        match self.outcomes.get(index) {
            Some(Some(value)) => Ok(*value),
            Some(None) | None => Err(CountError::MissingInput),
        }
    }
}

struct BundledCl100k {
    engine: Engine,
}

impl BundledCl100k {
    fn build() -> Result<Self, Box<HelperError>> {
        let Some(data) = assets::bundled(Encoding::Cl100kBase) else {
            return Err(failure("cl100k_base is not bundled"));
        };
        let engine = TiktokenEngine::from_bpe_file(Encoding::Cl100kBase, &data)
            .map_err(|error| failure(&error.to_string()))?;
        Ok(Self {
            engine: Engine::Tiktoken(Arc::new(engine)),
        })
    }
}

impl TextTokenCounter for BundledCl100k {
    fn count(&self, _model: &str, text: &str) -> Result<u64, CountError> {
        let tokens = self.engine.encode(text).map_err(CountError::Engine)?;
        u64::try_from(tokens.len()).map_err(|_error| CountError::Overflow)
    }
}

fn failure(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "B5848Tokenizer".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
