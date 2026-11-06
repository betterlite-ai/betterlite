use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_http::proxy::limits::{ReadError, read_bounded};
use reqwest::StatusCode;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::generic_guardrail_contracts::GuardrailMessage;
use crate::javelin::AppliedGuardrailsHeader;

type Error = PangeaGuardrailError;

pub const DEFAULT_API_BASE: &str = "https://ai-guard.aws.us.pangea.cloud";
pub const API_KEY_VARIABLE: &str = "PANGEA_API_KEY";
pub const API_BASE_VARIABLE: &str = "PANGEA_API_BASE";
pub const SUPPORTED_EVENT_HOOKS: [EventHook; 2] = [EventHook::PreCall, EventHook::PostCall];
const GUARD_API: &str = "v1beta/guard";
const COMPLETION_CALL_TYPES: [&str; 2] = ["text_completion", "atext_completion"];
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const BLOCKED: &str = "Violated Pangea guardrail policy";
const BACKEND: &str = "Error in Pangea Guardrail";
const MISSING_KEY: &str = "Pangea API Key not found. Set PANGEA_API_KEY environment variable or pass it in litellm_params.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventHook {
    PreCall,
    PostCall,
}

#[derive(Debug, Error)]
pub enum PangeaGuardrailError {
    #[error("Pangea guardrail name is required")]
    MissingGuardrailName,
    // LiteLLM raises its PangeaGuardrailMissingSecrets exception here instead.
    #[error("{MISSING_KEY}")]
    MissingSecrets,
    #[error("the Pangea AI Guard request failed")]
    Request { source: reqwest::Error },
    #[error("the Pangea AI Guard answered with status {status}")]
    HttpStatus { status: StatusCode },
    #[error("the Pangea AI Guard answer is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the Pangea AI Guard answer is not valid JSON")]
    ResponseJson { source: serde_json::Error },
    #[error("{BLOCKED}")]
    Blocked,
    #[error("the Pangea AI Guard answer has no {field} to transform")]
    MissingData { field: &'static str },
    #[error("a transformed text completion needs exactly one message with content")]
    InvalidTextCompletion,
}

impl PangeaGuardrailError {
    // LiteLLM raises HTTPException(status_code=400) for a block and wraps every other failure in a 500.
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        if matches!(self, Self::Blocked) {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }

    #[must_use]
    pub fn detail(&self, guardrail_name: &str) -> PangeaErrorDetail {
        let blocked = matches!(self, Self::Blocked);
        let message = if blocked { BLOCKED } else { BACKEND };
        PangeaErrorDetail {
            error: message,
            guardrail_name: guardrail_name.to_owned(),
            exceptions: (!blocked).then(|| self.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PangeaErrorDetail {
    pub error: &'static str,
    pub guardrail_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exceptions: Option<String>,
}

// The api_key field stays plain text here because this is the config vocabulary the UI serializes, not a runtime secret holder.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PangeaGuardrailConfigModel {
    pub api_key: Option<String>,
    pub api_base: Option<String>,
    pub pangea_input_recipe: Option<String>,
    pub pangea_output_recipe: Option<String>,
}

impl GuardrailConfigModelIdentity for PangeaGuardrailConfigModel {
    fn ui_friendly_name() -> &'static str {
        "Pangea Guardrail"
    }
}

#[derive(Debug, Clone, Default)]
pub struct PangeaGuardrailParams {
    pub guardrail_name: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub pangea_input_recipe: Option<String>,
    pub pangea_output_recipe: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RequestData {
    pub messages: Option<Vec<GuardrailMessage>>,
    pub prompt: Option<GuardrailValue>,
    pub tools: Option<GuardrailValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GuardrailResponse {
    Model { choices: Vec<GuardrailValue> },
    TextCompletion { choices: Vec<GuardrailValue> },
}

#[derive(Debug, Clone, Serialize)]
struct AiGuardRequest {
    debug: bool,
    input: AiGuardInput,
    event_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    recipe: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct AiGuardInput {
    messages: Option<Vec<GuardrailMessage>>,
    tools: Option<GuardrailValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    choices: Option<Vec<GuardrailValue>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct AiGuardResponse {
    result: AiGuardResult,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct AiGuardResult {
    blocked: Option<bool>,
    transformed: Option<bool>,
    output: Option<AiGuardOutput>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct AiGuardOutput {
    messages: Option<Vec<GuardrailMessage>>,
    choices: Option<Vec<GuardrailValue>>,
}

#[derive(Debug)]
pub struct PangeaGuardrail {
    pub guardrail_name: String,
    pub api_key: SecretString,
    pub api_base: String,
    pub input_recipe: Option<String>,
    pub output_recipe: Option<String>,
    pub client: reqwest::Client,
}

impl PangeaGuardrail {
    pub fn new<L>(
        params: PangeaGuardrailParams,
        lookup: &L,
        client: reqwest::Client,
    ) -> Result<Self, Error>
    where
        L: EnvironmentLookup,
    {
        let name = params.guardrail_name.ok_or(Error::MissingGuardrailName)?;
        let env_key = lookup
            .lookup(API_KEY_VARIABLE)
            .filter(|value| !value.is_empty());
        let configured_key = params.api_key.or_else(|| env_key.map(SecretString::from));
        let api_key = configured_key.ok_or(Error::MissingSecrets)?;
        let env_base = lookup
            .lookup(API_BASE_VARIABLE)
            .filter(|value| !value.is_empty());
        let api_base = params
            .api_base
            .or(env_base)
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        Ok(Self {
            guardrail_name: name,
            api_key,
            api_base,
            input_recipe: params.pangea_input_recipe,
            output_recipe: params.pangea_output_recipe,
            client,
        })
    }

    async fn call_ai_guard(
        &self,
        event_type: &'static str,
        input: AiGuardInput,
        recipe: Option<String>,
    ) -> Result<AiGuardResult, Error> {
        let request = AiGuardRequest {
            debug: false,
            input,
            event_type,
            recipe,
        };
        let mut response = self
            .client
            .post(format!("{}/{GUARD_API}", self.api_base))
            .bearer_auth(self.api_key.expose_secret())
            .json(&request)
            .send()
            .await
            .map_err(|source| Error::Request { source })?;
        let status = response.status();
        if status.is_client_error() || status.is_server_error() {
            return Err(Error::HttpStatus { status });
        }
        let read = read_bounded(&mut response, MAX_RESPONSE_BYTES).await;
        let bytes = read.map_err(|error| match error {
            ReadError::TooLarge { limit } => Error::ResponseTooLarge { limit },
            ReadError::Failed(source) => Error::Request { source },
        })?;
        let parsed: AiGuardResponse =
            serde_json::from_slice(&bytes).map_err(|source| Error::ResponseJson { source })?;
        if parsed.result.blocked == Some(true) {
            return Err(Error::Blocked);
        }
        Ok(parsed.result)
    }

    pub async fn async_pre_call_hook<A>(
        &self,
        data: &mut RequestData,
        call_type: &str,
        applied: &mut A,
    ) -> Result<bool, Error>
    where
        A: AppliedGuardrailsHeader,
    {
        let completion = COMPLETION_CALL_TYPES.contains(&call_type);
        let input_messages = if completion {
            Some(vec![user_message(
                data.prompt
                    .clone()
                    .ok_or(Error::MissingData { field: "prompt" })?,
            )])
        } else {
            data.messages.clone()
        };
        let input = AiGuardInput {
            messages: input_messages,
            tools: data.tools.clone(),
            choices: None,
        };
        let answer = self
            .call_ai_guard("input", input, self.input_recipe.clone())
            .await?;
        applied.record_applied_guardrail(&self.guardrail_name);
        if answer.transformed != Some(true) {
            return Ok(false);
        }
        let output_messages = answer.output.and_then(|output| output.messages);
        let messages = output_messages.ok_or(Error::MissingData { field: "messages" })?;
        if completion {
            let [message] = messages.as_slice() else {
                return Err(Error::InvalidTextCompletion);
            };
            let content = message.extra.get("content");
            let transformed_content = content.ok_or(Error::InvalidTextCompletion)?;
            data.prompt = Some(transformed_content.clone());
        } else {
            data.messages = Some(messages);
        }
        Ok(true)
    }

    pub async fn async_post_call_success_hook<A>(
        &self,
        data: &mut RequestData,
        response: &mut GuardrailResponse,
        applied: &mut A,
    ) -> Result<bool, Error>
    where
        A: AppliedGuardrailsHeader,
    {
        let (request_messages, sent_choices) = match response {
            GuardrailResponse::Model { choices } => {
                let Some(messages) = data.messages.clone() else {
                    return Ok(false);
                };
                (messages, choices.clone())
            }
            GuardrailResponse::TextCompletion { choices } => {
                let prompt = data
                    .prompt
                    .clone()
                    .ok_or(Error::MissingData { field: "prompt" })?;
                (vec![user_message(prompt)], choices.clone())
            }
        };
        let input = AiGuardInput {
            messages: Some(request_messages),
            tools: data.tools.clone(),
            choices: Some(sent_choices),
        };
        let answer = self
            .call_ai_guard("output", input, self.output_recipe.clone())
            .await?;
        applied.record_applied_guardrail(&self.guardrail_name);
        if answer.transformed != Some(true) {
            return Ok(false);
        }
        let transformed_choices = answer.output.and_then(|output| output.choices);
        let transformed = transformed_choices.ok_or(Error::MissingData { field: "choices" })?;
        match response {
            GuardrailResponse::Model { choices }
            | GuardrailResponse::TextCompletion { choices } => *choices = transformed,
        }
        Ok(true)
    }
}

fn user_message(content: GuardrailValue) -> GuardrailMessage {
    let extra = BTreeMap::from([("content".to_owned(), content)]);
    GuardrailMessage {
        role: "user".to_owned(),
        extra,
    }
}

pub trait CallbackRegistry {
    fn add_litellm_callback(&mut self, guardrail: Arc<PangeaGuardrail>);
}

pub fn initialize_guardrail<R, L>(
    params: PangeaGuardrailParams,
    lookup: &L,
    client: reqwest::Client,
    registry: &mut R,
) -> Result<Arc<PangeaGuardrail>, Error>
where
    R: CallbackRegistry,
    L: EnvironmentLookup,
{
    let guardrail = Arc::new(PangeaGuardrail::new(params, lookup, client)?);
    registry.add_litellm_callback(Arc::clone(&guardrail));
    Ok(guardrail)
}

#[path = "pangea/tests/config_model.rs"]
mod config_model;
#[path = "pangea/tests/environment.rs"]
mod environment;
#[path = "pangea/tests/missing_api_key.rs"]
mod missing_api_key;
mod tests;
#[path = "pangea/tests/text_completion.rs"]
mod text_completion;
