use betterlite_config::loader::EnvironmentLookup;
use betterlite_prompts::builtin::Field;
use betterlite_prompts::factory::{Output, PromptFactory, prompt_factory};
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Number;

use super::error::TritonError;
use super::reply::decode_reply;
use super::request::TransformInput;

pub const MAX_TOKENS_ENV: &str = "DEFAULT_MAX_TOKENS_FOR_TRITON";

pub fn max_tokens_default(environment: &dyn EnvironmentLookup) -> Result<i64, TritonError> {
    let text = environment.lookup(MAX_TOKENS_ENV).unwrap_or_default();
    let configured = betterlite_env::number::<i64>(MAX_TOKENS_ENV, &text)
        .map_err(|source| TritonError::Environment { source })?;
    Ok(configured.unwrap_or(2000))
}

pub struct TritonGenerateConfig<'prompts> {
    pub prompts: &'prompts PromptFactory,
    pub now: DateTime<Utc>,
    pub max_tokens_default: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GenerateRequest {
    pub text_input: Option<String>,
    pub parameters: IndexMap<String, Field>,
    pub stream: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GenerateReply {
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GenerateBody {
    #[serde(deserialize_with = "Option::deserialize")]
    text_output: Option<String>,
}

impl TritonGenerateConfig<'_> {
    pub async fn transform_request(
        &self,
        input: &TransformInput<'_>,
    ) -> Result<GenerateRequest, TritonError> {
        let prompt =
            match prompt_factory(self.prompts, input.model, input.messages, None, self.now).await {
                Ok(Output::Text(text)) => Some(text),
                Ok(Output::None) => None,
                Ok(Output::Ollama(_)) => return Err(TritonError::UnexpectedPrompt),
                Err(source) => return Err(TritonError::Prompt { source }),
            };
        let mut parameters = input.optional_params.clone();
        let stream = match parameters.shift_remove("stream") {
            None | Some(Field::Null) => false,
            Some(Field::Bool(value)) => value,
            Some(_) => return Err(TritonError::NotABoolean { name: "stream" }),
        };
        let max_tokens = match parameters.shift_remove("max_tokens") {
            None => self.max_tokens_default,
            Some(Field::Number(number)) => number
                .as_i64()
                .ok_or(TritonError::NotAnInteger { name: "max_tokens" })?,
            Some(_) => return Err(TritonError::NotAnInteger { name: "max_tokens" }),
        };
        parameters.insert(
            String::from("max_tokens"),
            Field::Number(Number::from(max_tokens)),
        );
        Ok(GenerateRequest {
            text_input: prompt,
            parameters,
            stream,
        })
    }
}

pub fn transform_response(status_code: u16, body: &[u8]) -> Result<GenerateReply, TritonError> {
    let decoded: GenerateBody = decode_reply(status_code, body)?;
    Ok(GenerateReply {
        text: decoded.text_output,
    })
}
