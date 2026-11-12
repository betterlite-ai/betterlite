use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use secrecy::SecretString;

use super::config;
use super::error::LangFlowError;
use super::request::{RequestBody, RequestInput, transform_request};

pub struct RunInput<'request> {
    pub model: &'request str,
    pub messages: &'request [Message],
    pub optional_params: &'request Object,
    pub api_base: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
    pub stream: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRun {
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: RequestBody,
    pub fake_stream: bool,
}

// LiteLLM's handler order: validate_environment, then get_complete_url, then transform_request
pub fn prepare_run(input: &RunInput<'_>) -> Result<PreparedRun, LangFlowError> {
    let mut headers = BTreeMap::new();
    config::validate_environment(&mut headers, input.api_key);
    let url = config::complete_url(input.api_base, input.model, input.optional_params)?;
    let body = transform_request(&RequestInput {
        messages: input.messages,
        optional_params: input.optional_params,
    })?;
    // LangFlow's should_fake_stream is true exactly for an explicit stream request
    let fake_stream = input.stream == Some(true);
    Ok(PreparedRun {
        url,
        headers,
        body,
        fake_stream,
    })
}
