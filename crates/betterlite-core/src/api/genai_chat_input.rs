#![expect(
    clippy::disallowed_types,
    reason = "GenAI parts may carry vendor JSON, and function args, responses and JSON schemas are arbitrary caller JSON"
)]
mod capabilities;
pub mod input;
mod messages;
mod response_format;
mod tests;
mod tools;

use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::chat::{ChatRequest, ResponseFormat, Stop, Tool};
use betterlite_openai::openai_base_config::CatalogCapabilities;
use indexmap::IndexMap;
use thiserror::Error;

use crate::schema::{SchemaError, SchemaLimits};

use self::capabilities::supported_response_format;
use self::input::{GenerateContentRequest, GenerateLiteLLMParams, SystemInstruction};
use self::messages::messages;
use self::tools::{tool_choice, tools_to_openai};

pub struct TranslationContext<'call> {
    pub resolution: &'call ResolutionContext<'call>,
    pub model_capabilities: &'call CatalogCapabilities,
    pub limits: SchemaLimits,
}

pub type TranslatedCompletion = (ChatRequest, Option<GenerateLiteLLMParams>);

#[derive(Debug, Error)]
pub enum GenAiInputError {
    #[error("the GenAI schema cannot be used: {0}")]
    Schema(#[from] SchemaError),
    #[error("the GenAI request cannot be encoded: {0}")]
    Encode(#[from] serde_json::Error),
}

pub fn translate_generate_content(
    request: &GenerateContentRequest,
    litellm_params: Option<&GenerateLiteLLMParams>,
    context: &TranslationContext<'_>,
) -> Result<TranslatedCompletion, GenAiInputError> {
    let system = request
        .system_instruction
        .as_ref()
        .or(request.system_instruction_snake.as_ref());
    let response_format = supported_response_format(request, litellm_params, context)?;
    let tools = tools_to_openai(
        request.tools.as_deref().unwrap_or_default(),
        &context.limits,
    )?;
    let completion = completion_request(request, system, tools, response_format)?;
    Ok((completion, litellm_params.cloned()))
}

fn completion_request(
    request: &GenerateContentRequest,
    system: Option<&SystemInstruction>,
    tools: Vec<Tool>,
    response_format: Option<ResponseFormat>,
) -> Result<ChatRequest, GenAiInputError> {
    let config = request.generation_config.as_ref();
    Ok(ChatRequest {
        model: Some(request.model.clone()),
        messages: messages(&request.contents, system)?,
        stream: false,
        stream_options: None,
        temperature: config.and_then(|fields| fields.temperature.clone()),
        top_p: config.and_then(|fields| fields.top_p.clone()),
        presence_penalty: None,
        frequency_penalty: None,
        choice_count: None,
        max_tokens: config.and_then(|fields| fields.max_output_tokens),
        max_completion_tokens: None,
        seed: None,
        stop: config
            .and_then(|fields| fields.stop_sequences.clone())
            .map(Stop::Many),
        logprobs: None,
        top_logprobs: None,
        user: None,
        service_tier: None,
        reasoning_effort: None,
        tools: (!tools.is_empty()).then_some(tools),
        tool_choice: tool_choice(
            request
                .tool_config
                .as_ref()
                .or(request.tool_config_snake.as_ref()),
        ),
        parallel_tool_calls: None,
        response_format,
        fallbacks: None,
        context_window_fallbacks: None,
        content_policy_fallbacks: None,
        disable_fallbacks: None,
        passthrough: IndexMap::new(),
    })
}
