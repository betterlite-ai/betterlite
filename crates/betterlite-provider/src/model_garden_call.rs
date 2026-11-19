use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::compatible_line_stream::Mode;
use betterlite_vertex::model_garden::{ModelGardenTargetInput, model_garden_target};
use betterlite_vertex::model_policy::error::VertexAiError;
use betterlite_vertex::model_urls::error::VertexUrlError;
use indexmap::IndexMap;
use secrecy::SecretString;
use thiserror::Error;
use uuid::Uuid;

use crate::compatible_chat_call::error::CompatibleChatError;
use crate::compatible_chat_call::execution::CompatibleChatExecution;
use crate::compatible_chat_call::send::CompatibleChatOutcome;

mod tests;

// the source hands the OpenAI-like handler this provider name, which also prefixes the reply's model
const CUSTOM_LLM_PROVIDER: &str = "vertex_ai";

pub struct ModelGardenCall<'call> {
    pub model: &'call str,
    pub messages: &'call [ChatMessage],
    pub optional_params: &'call IndexMap<String, Json>,
    pub api_base: Option<&'call str>,
    pub vertex_location: Option<&'call str>,
    pub vertex_project: Option<&'call str>,
    pub resolved_project: &'call str,
    pub access_token: &'call SecretString,
    pub timeout: Option<Duration>,
    pub mode: Mode,
    pub identifier: fn() -> Uuid,
    pub clock: fn() -> i64,
}

#[derive(Debug, Error)]
enum CallError {
    #[error(transparent)]
    Target(#[from] VertexUrlError),
    #[error(transparent)]
    Chat(Box<CompatibleChatError>),
}

// the source wraps every failure inside its try in a Vertex 500, including an inner error that carries a status
pub async fn completion(
    call: &ModelGardenCall<'_>,
    client: &ProxiedClient,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
) -> Result<CompatibleChatOutcome, VertexAiError> {
    compatible_call(call, client, state, env)
        .await
        .map_err(|error| VertexAiError {
            status_code: 500,
            message: error.to_string(),
            headers: None,
        })
}

async fn compatible_call(
    call: &ModelGardenCall<'_>,
    client: &ProxiedClient,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
) -> Result<CompatibleChatOutcome, CallError> {
    let mut options = call.optional_params.clone();
    normalized_stream(&mut options)?;
    let target = model_garden_target(&ModelGardenTargetInput {
        model: call.model,
        api_base: call.api_base,
        vertex_location: call.vertex_location,
        vertex_project: call.vertex_project,
        resolved_project: call.resolved_project,
    })?;
    let execution = CompatibleChatExecution {
        model: &target.model,
        messages: call.messages,
        custom_llm_provider: CUSTOM_LLM_PROVIDER,
        api_base: Some(target.api_base.as_str()),
        api_key: Some(call.access_token),
        headers: None,
        optional_params: &options,
        custom_endpoint: false,
        timeout: call.timeout,
        mode: call.mode,
        fake_stream: false,
        transform_messages: None,
        streaming_decoder: None,
        identifier: call.identifier,
        clock: call.clock,
    };
    execution
        .execute(client, state, env)
        .await
        .map_err(|error| CallError::Chat(Box::new(error)))
}

// the source reads the option without popping it and writes the boolean back, so the body always carries one
fn normalized_stream(options: &mut IndexMap<String, Json>) -> Result<(), CallError> {
    let stream = match options.get("stream") {
        None | Some(Json::Null) => false,
        Some(Json::Bool(flag)) => *flag,
        Some(Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            return Err(CallError::Chat(Box::new(CompatibleChatError::OptionType {
                field: "stream",
                expected: "a boolean",
            })));
        }
    };
    options.insert("stream".to_owned(), Json::Bool(stream));
    Ok(())
}
