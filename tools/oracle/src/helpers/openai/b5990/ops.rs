use std::collections::BTreeMap;

use betterlite_openai::chat::ChatMessage;
use betterlite_openai::huggingface_chat::HuggingfaceEnv;
use betterlite_openai::huggingface_chat::error::HuggingfaceChatError;
use betterlite_openai::huggingface_chat::headers::validate_environment;
use betterlite_openai::huggingface_chat::mapping::{ProviderMapping, ProviderMappingSource};
use betterlite_openai::huggingface_chat::request::transform_request;
use betterlite_openai::huggingface_chat::url::get_complete_url;
use betterlite_openai::openai_base_config::Object;
use secrecy::SecretString;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

use super::input::{Input, Step};

pub(crate) fn run(input: &Input) -> HelperResult {
    let mut results = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        results.push(execute(step)?);
    }
    Ok(Value::Array(results))
}

fn execute(step: &Step) -> HelperResult {
    match step {
        Step::CompleteUrl {
            model,
            api_base,
            env,
        } => complete_url(model, api_base.as_deref(), env),
        Step::ValidateEnvironment { headers, api_key } => {
            let key = api_key.as_deref().map(SecretString::from);
            Ok(json!(validate_environment(headers, key.as_ref())))
        }
        Step::TransformRequest {
            model,
            messages,
            optional_params,
            api_base,
            mapping,
        } => request(
            model,
            messages,
            optional_params,
            api_base.as_deref(),
            mapping.as_ref(),
        ),
    }
}

fn complete_url(
    model: &str,
    api_base: Option<&str>,
    env: &BTreeMap<String, String>,
) -> HelperResult {
    let environment = HuggingfaceEnv {
        hf_api_base: env.get("HF_API_BASE").cloned(),
        huggingface_api_base: env.get("HUGGINGFACE_API_BASE").cloned(),
    };
    get_complete_url(api_base, model, &environment)
        .map(Value::String)
        .map_err(|error| failure(&error))
}

fn request(
    model: &str,
    messages: &[ChatMessage],
    optional_params: &Object,
    api_base: Option<&str>,
    mapping: Option<&ProviderMapping>,
) -> HelperResult {
    let source = RecordedSource(mapping.cloned());
    transform_request(model, messages, optional_params, api_base, &source)
        .map(Value::Object)
        .map_err(|error| failure(&error))
}

struct RecordedSource(Option<ProviderMapping>);

impl ProviderMappingSource for RecordedSource {
    fn mapping(&self, model_id: &str) -> Result<ProviderMapping, HuggingfaceChatError> {
        self.0
            .clone()
            .ok_or_else(|| HuggingfaceChatError::NoProviderMapping {
                model: model_id.to_owned(),
            })
    }
}

fn failure(error: &HuggingfaceChatError) -> Box<HelperError> {
    Box::new(HelperError {
        class: class_name(error).to_owned(),
        status: status_code(error),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

const fn class_name(error: &HuggingfaceChatError) -> &'static str {
    match error {
        HuggingfaceChatError::Huggingface(_) => "HuggingfaceError",
        HuggingfaceChatError::MissingProviderPrefix { .. }
        | HuggingfaceChatError::MappingMissing
        | HuggingfaceChatError::NoProviderMapping { .. }
        | HuggingfaceChatError::MappingTooLarge { .. }
        | HuggingfaceChatError::MappingDecode { .. }
        | HuggingfaceChatError::Content(_)
        | HuggingfaceChatError::MessageEncode { .. } => "ValueError",
    }
}

const fn status_code(error: &HuggingfaceChatError) -> Option<u16> {
    match error {
        HuggingfaceChatError::Huggingface(source) => Some(source.status_code),
        HuggingfaceChatError::Content(source) => Some(source.status_code()),
        HuggingfaceChatError::MissingProviderPrefix { .. }
        | HuggingfaceChatError::MappingMissing
        | HuggingfaceChatError::NoProviderMapping { .. }
        | HuggingfaceChatError::MappingTooLarge { .. }
        | HuggingfaceChatError::MappingDecode { .. }
        | HuggingfaceChatError::MessageEncode { .. } => None,
    }
}
