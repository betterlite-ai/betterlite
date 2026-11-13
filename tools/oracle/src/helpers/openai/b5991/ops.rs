use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_openai::huggingface_chat::HuggingfaceEnv;
use betterlite_openai::huggingface_chat::error::HuggingfaceError;
use betterlite_openai::huggingface_chat::headers::validate_environment;
use betterlite_openai::huggingface_embedding::PrepareRequest;
use betterlite_openai::huggingface_embedding::error::HuggingfaceEmbeddingError;
use betterlite_openai::huggingface_embedding::prepare;
use betterlite_openai::huggingface_embedding::request::{
    input_for_tag, process_optional_params, transform_input,
};
use betterlite_openai::huggingface_embedding::response::{
    OrdinaryEncoder, decode_embedding_response, process_embedding_response,
};
use betterlite_openai::huggingface_embedding::task::PipelineTagSource;
use betterlite_openai::huggingface_embedding::url::embedding_url;
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::assets;
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};
use secrecy::SecretString;
use serde_json::{Value, json};
use thiserror::Error;

use crate::helpers::{HelperError, HelperResult};

use super::input::{
    Input, PrepareStep, ResponseStep, Step, TransformInputStep, UrlStep, ValidateEnvironmentStep,
};

#[derive(Debug, Error)]
enum B5991Error {
    #[error(transparent)]
    Embedding(#[from] HuggingfaceEmbeddingError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub(crate) fn run(input: &Input) -> HelperResult {
    let encoder = BundledCl100k::build()?;
    let mut outcomes = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        outcomes.push(outcome(run_step(step, &encoder)));
    }
    Ok(Value::Array(outcomes))
}

fn outcome(result: Result<Value, B5991Error>) -> Value {
    match result {
        Ok(value) => json!({"ok": true, "value": value}),
        Err(error) => json!({"ok": false, "status": status_code(&error)}),
    }
}

const fn status_code(error: &B5991Error) -> Option<u16> {
    match error {
        B5991Error::Embedding(HuggingfaceEmbeddingError::Huggingface(source)) => {
            Some(source.status_code)
        }
        B5991Error::Embedding(
            HuggingfaceEmbeddingError::InvalidTaskType { .. }
            | HuggingfaceEmbeddingError::ResponseTooLarge { .. }
            | HuggingfaceEmbeddingError::ResponseDecode { .. }
            | HuggingfaceEmbeddingError::RequestEncode { .. }
            | HuggingfaceEmbeddingError::SectionNotObject { .. }
            | HuggingfaceEmbeddingError::EmbeddingShape { .. }
            | HuggingfaceEmbeddingError::UsageOverflow,
        )
        | B5991Error::Json(_) => None,
    }
}

fn run_step(step: &Step, encoder: &BundledCl100k) -> Result<Value, B5991Error> {
    match step {
        Step::Prepare(prepare_step) => prepare_value(prepare_step),
        Step::TransformInput(transform_step) => transform_value(transform_step),
        Step::PipelineTag(tag_step) => Ok(Value::Object(input_for_tag(
            &tag_step.input,
            tag_step.pipeline_tag.as_deref(),
        )?)),
        Step::OptionalParams(params_step) => Ok(Value::Object(process_optional_params(
            params_step.data.clone(),
            &params_step.params(),
        )?)),
        Step::Response(response_step) => response_value(response_step, encoder),
        Step::ValidateEnvironment(validate_step) => Ok(validate_value(validate_step)),
        Step::Url(url_step) => Ok(url_value(url_step)),
    }
}

fn prepare_value(step: &PrepareStep) -> Result<Value, B5991Error> {
    let environment = environment(&step.env);
    let key = step.api_key.as_deref().map(SecretString::from);
    let source = RecordedSource::returning(step.pipeline_tag.as_deref());
    let request = PrepareRequest {
        model: &step.model,
        input: &step.input,
        optional_params: &step.optional_params,
        headers: &step.headers,
        api_key: key.as_ref(),
        api_base: step.api_base.as_deref(),
        env: &environment,
    };
    let prepared = prepare(&request, &source)?;
    let body: Value = serde_json::from_str(&prepared.body)?;
    Ok(json!({
        "url": prepared.url,
        "headers": prepared.headers,
        "body": body,
        "lookups": source.lookups(),
    }))
}

fn transform_value(step: &TransformInputStep) -> Result<Value, B5991Error> {
    let source = RecordedSource::returning(step.pipeline_tag.as_deref());
    let body = transform_input(&step.model, &step.input, &step.optional_params, &source)?;
    Ok(json!({"body": body, "lookups": source.lookups()}))
}

fn response_value(step: &ResponseStep, encoder: &BundledCl100k) -> Result<Value, B5991Error> {
    let bytes = serde_json::to_vec(&step.embeddings)?;
    let reply = decode_embedding_response(&bytes)?;
    let response = process_embedding_response(&reply, &step.model, &step.input, encoder)?;
    Ok(serde_json::to_value(response)?)
}

fn validate_value(step: &ValidateEnvironmentStep) -> Value {
    let key = step.api_key.as_deref().map(SecretString::from);
    let headers = validate_environment(&step.headers, key.as_ref());
    json!(headers)
}

fn url_value(step: &UrlStep) -> Value {
    let environment = environment(&step.env);
    Value::String(embedding_url(
        &step.model,
        step.api_base.as_deref(),
        &environment,
        step.task.as_deref(),
    ))
}

fn environment(env: &BTreeMap<String, String>) -> HuggingfaceEnv {
    HuggingfaceEnv {
        hf_api_base: env.get("HF_API_BASE").cloned(),
        huggingface_api_base: env.get("HUGGINGFACE_API_BASE").cloned(),
    }
}

struct RecordedSource {
    pipeline_tag: Option<String>,
    lookups: Mutex<Vec<String>>,
}

impl RecordedSource {
    fn returning(pipeline_tag: Option<&str>) -> Self {
        Self {
            pipeline_tag: pipeline_tag.map(str::to_owned),
            lookups: Mutex::new(Vec::new()),
        }
    }

    fn lookups(&self) -> Vec<String> {
        self.lookups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl PipelineTagSource for RecordedSource {
    fn tag(
        &self,
        model: &str,
        _task_type: Option<&str>,
    ) -> Result<Option<String>, HuggingfaceError> {
        self.lookups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model.to_owned());
        Ok(self.pipeline_tag.clone())
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

impl OrdinaryEncoder for BundledCl100k {
    fn encode_ordinary(&self, text: &str) -> usize {
        // the bundled vocabulary encodes any text; a failure shows as a token count the case rejects
        self.engine.encode(text).map_or(0, |tokens| tokens.len())
    }
}

fn failure(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "B5991Tokenizer".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
