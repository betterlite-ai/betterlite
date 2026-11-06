use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_http::proxy::limits::{ReadError, read_bounded};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

use crate::generic_guardrail_contracts::GuardrailMessage;
use crate::javelin::AppliedGuardrailsHeader;
use crate::pangea::config::PangeaGuardrailConfigModel;
use crate::pangea::error::PangeaGuardrailError;
use crate::pangea::wire::{
    AiGuardInput, AiGuardOutput, AiGuardRequest, AiGuardResponse, AiGuardResult,
};

pub const SUPPORTED_EVENT_HOOKS: [EventHook; 2] = [EventHook::PreCall, EventHook::PostCall];
const GUARD_API: &str = "v1beta/guard";
const COMPLETION_CALL_TYPES: [&str; 2] = ["text_completion", "atext_completion"];
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventHook {
    PreCall,
    PostCall,
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
        guardrail_name: Option<String>,
        config: PangeaGuardrailConfigModel,
        lookup: &L,
        client: reqwest::Client,
    ) -> Result<Self, PangeaGuardrailError>
    where
        L: EnvironmentLookup,
    {
        let name = guardrail_name.ok_or(PangeaGuardrailError::MissingGuardrailName)?;
        let settings = config.resolve(lookup)?;
        Ok(Self {
            guardrail_name: name,
            api_key: settings.api_key,
            api_base: settings.api_base,
            input_recipe: settings.input_recipe,
            output_recipe: settings.output_recipe,
            client,
        })
    }

    async fn call_ai_guard(
        &self,
        event_type: &'static str,
        input: AiGuardInput,
        recipe: Option<String>,
    ) -> Result<AiGuardResult, PangeaGuardrailError> {
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
            .map_err(|source| PangeaGuardrailError::Request { source })?;
        let status = response.status();
        if status.is_client_error() || status.is_server_error() {
            return Err(PangeaGuardrailError::HttpStatus { status });
        }
        let read = read_bounded(&mut response, MAX_RESPONSE_BYTES).await;
        let bytes = read.map_err(|error| match error {
            ReadError::TooLarge { limit } => PangeaGuardrailError::ResponseTooLarge { limit },
            ReadError::Failed(source) => PangeaGuardrailError::Request { source },
        })?;
        parse_answer(&bytes)
    }

    async fn run_hook<A>(
        &self,
        event_type: &'static str,
        input: AiGuardInput,
        recipe: Option<String>,
        applied: &mut A,
    ) -> Result<Option<AiGuardOutput>, PangeaGuardrailError>
    where
        A: AppliedGuardrailsHeader,
    {
        let answer = self.call_ai_guard(event_type, input, recipe).await?;
        applied.record_applied_guardrail(&self.guardrail_name);
        if answer.transformed != Some(true) {
            return Ok(None);
        }
        Ok(Some(answer.output.unwrap_or_default()))
    }

    pub async fn async_pre_call_hook<A>(
        &self,
        data: &mut RequestData,
        call_type: &str,
        applied: &mut A,
    ) -> Result<bool, PangeaGuardrailError>
    where
        A: AppliedGuardrailsHeader,
    {
        let completion = COMPLETION_CALL_TYPES.contains(&call_type);
        let sent_messages = if completion {
            Some(vec![completion_message(prompt(data)?)])
        } else {
            data.messages.clone()
        };
        let input = AiGuardInput {
            messages: sent_messages,
            tools: data.tools.clone(),
            choices: None,
        };
        let answer = self
            .run_hook("input", input, self.input_recipe.clone(), applied)
            .await?;
        let Some(output) = answer else {
            return Ok(false);
        };
        if completion {
            let messages = output.messages.unwrap_or_default();
            let [message] = messages.as_slice() else {
                return Err(PangeaGuardrailError::InvalidTextCompletion);
            };
            let content = message
                .extra
                .get("content")
                .ok_or(PangeaGuardrailError::InvalidTextCompletion)?;
            data.prompt = Some(content.clone());
        } else {
            let Some(messages) = output.messages else {
                return Err(PangeaGuardrailError::MissingData { field: "messages" });
            };
            data.messages = Some(messages);
        }
        Ok(true)
    }

    pub async fn async_post_call_success_hook<A>(
        &self,
        data: &mut RequestData,
        response: &mut GuardrailResponse,
        applied: &mut A,
    ) -> Result<bool, PangeaGuardrailError>
    where
        A: AppliedGuardrailsHeader,
    {
        let Some(input) = post_call_input(data, response)? else {
            return Ok(false);
        };
        let answer = self
            .run_hook("output", input, self.output_recipe.clone(), applied)
            .await?;
        let Some(output) = answer else {
            return Ok(false);
        };
        apply_post_call_output(response, output)
    }
}

fn prompt(data: &RequestData) -> Result<GuardrailValue, PangeaGuardrailError> {
    data.prompt
        .clone()
        .ok_or(PangeaGuardrailError::MissingData { field: "prompt" })
}

fn completion_message(content: GuardrailValue) -> GuardrailMessage {
    let extra = BTreeMap::from([("content".to_owned(), content)]);
    GuardrailMessage {
        role: "user".to_owned(),
        extra,
    }
}

fn parse_answer(bytes: &[u8]) -> Result<AiGuardResult, PangeaGuardrailError> {
    let parsed: AiGuardResponse = serde_json::from_slice(bytes)
        .map_err(|source| PangeaGuardrailError::ResponseJson { source })?;
    if parsed.result.blocked == Some(true) {
        return Err(PangeaGuardrailError::Blocked);
    }
    Ok(parsed.result)
}

fn post_call_input(
    data: &RequestData,
    response: &GuardrailResponse,
) -> Result<Option<AiGuardInput>, PangeaGuardrailError> {
    let choices = match response {
        GuardrailResponse::Model { choices } | GuardrailResponse::TextCompletion { choices } => {
            choices.clone()
        }
    };
    let messages = match response {
        GuardrailResponse::Model { .. } => {
            let Some(messages) = data.messages.clone() else {
                return Ok(None);
            };
            messages
        }
        GuardrailResponse::TextCompletion { .. } => vec![completion_message(prompt(data)?)],
    };
    Ok(Some(AiGuardInput {
        messages: Some(messages),
        tools: data.tools.clone(),
        choices: Some(choices),
    }))
}

fn apply_post_call_output(
    response: &mut GuardrailResponse,
    output: AiGuardOutput,
) -> Result<bool, PangeaGuardrailError> {
    let transformed = output
        .choices
        .ok_or(PangeaGuardrailError::MissingData { field: "choices" })?;
    match response {
        GuardrailResponse::Model { choices } | GuardrailResponse::TextCompletion { choices } => {
            *choices = transformed;
        }
    }
    Ok(true)
}

pub trait CallbackRegistry {
    fn add_litellm_callback(&mut self, guardrail: Arc<PangeaGuardrail>);
}

pub fn initialize_guardrail<R, L>(
    guardrail_name: Option<String>,
    config: PangeaGuardrailConfigModel,
    lookup: &L,
    client: reqwest::Client,
    registry: &mut R,
) -> Result<Arc<PangeaGuardrail>, PangeaGuardrailError>
where
    R: CallbackRegistry,
    L: EnvironmentLookup,
{
    let guardrail = Arc::new(PangeaGuardrail::new(
        guardrail_name,
        config,
        lookup,
        client,
    )?);
    registry.add_litellm_callback(Arc::clone(&guardrail));
    Ok(guardrail)
}
