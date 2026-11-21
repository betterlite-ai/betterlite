use betterlite_prompts::arguments::json::{Json, Object};

use crate::api::genai::chat_call::ForwardedFields;
use crate::api::genai_chat_input::input::{
    Contents, GenAiTool, GenerateContentRequest as TypedRequest,
    GenerateLiteLLMParams as TypedParams, GenerationConfig, SystemInstruction, ToolConfig,
};
use crate::genai_setup::params::{GenerateContentLiteLLMParams as GenAiParams, RequestFieldError};

use super::GenerateContentRequest;
use super::caller::Caller;
use super::fields::{decode, first_non_empty, object_field};

// the typed request translate_generate_content_to_completion builds, and what the adapter forwards beside it
pub(super) struct AdapterInput {
    pub(super) request: TypedRequest,
    pub(super) forwarded: ForwardedFields,
}

// the values the adapter reads: the caller's contents, the four kwargs members, and the config dict
pub(super) fn adapter_input(
    request: &GenerateContentRequest<'_>,
    caller: &Caller,
    config: &Object,
) -> Result<AdapterInput, RequestFieldError> {
    let contents: Contents = decode(request.contents, "contents", "a content list")?;
    let system_instruction =
        first_non_empty(&caller.kwargs, ["systemInstruction", "system_instruction"])
            .map(|value| {
                decode::<SystemInstruction>(value, "systemInstruction", "a system instruction")
            })
            .transpose()?;
    let tools = caller
        .chat_tools
        .as_ref()
        .map(|value| decode::<Vec<GenAiTool>>(value, "tools", "a tool list"))
        .transpose()?;
    let tool_config = first_non_empty(&caller.kwargs, ["toolConfig", "tool_config"])
        .map(|value| decode::<ToolConfig>(value, "toolConfig", "a tool config"))
        .transpose()?;
    let generation_config = generation_config_of(config)?;
    Ok(AdapterInput {
        request: TypedRequest {
            model: caller.model.clone(),
            contents,
            system_instruction,
            system_instruction_snake: None,
            tools,
            tool_config,
            tool_config_snake: None,
            generation_config,
        },
        forwarded: ForwardedFields {
            metadata: object_field(&caller.kwargs, "metadata")?,
            extra_headers: request.extra_headers.cloned(),
            proxy_server_request: object_field(&caller.kwargs, "proxy_server_request")?,
        },
    })
}

// the setup result's config dict becomes the typed generation config, and an empty dict stays none
fn generation_config_of(config: &Object) -> Result<Option<GenerationConfig>, RequestFieldError> {
    if config.is_empty() {
        return Ok(None);
    }
    let value = Json::Object(config.clone());
    decode::<GenerationConfig>(&value, "generationConfig", "a generation config").map(Some)
}

// the adapter reads the setup result's params, with LiteLLM's retry loop count narrowed to an unsigned one
pub(super) fn adapter_params(params: &GenAiParams) -> TypedParams {
    TypedParams {
        api_base: params.api_base.clone(),
        api_key: params.api_key.clone(),
        api_version: params.api_version.clone(),
        custom_llm_provider: params.custom_llm_provider.clone(),
        max_retries: retry_count(params.max_retries),
        organization: params.organization.clone(),
    }
}

// a negative max_retries means no retry loop, and the typed params carry an unsigned count
fn retry_count(max_retries: Option<i64>) -> Option<u32> {
    match max_retries {
        None => None,
        Some(value) if value.is_negative() => Some(0_u32),
        Some(value) => Some(u32::try_from(value).unwrap_or(u32::MAX)),
    }
}
