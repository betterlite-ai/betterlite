use crate::api::api_boundaries::ApiLimits;
use crate::api::messages_chat_output::output::MessageKind;
use crate::api::messages_responses_input::items::MessagePart;
use crate::api::messages_responses_input::records::MessageRecord;
use crate::api::messages_responses_input::translate::messages_to_input;
use crate::api::messages_thinking::session::decode_messages_thinking;

use super::ENCRYPTED_REASONING_INCLUDE;
use super::error::MessagesResponsesOutputError;
use super::options::{
    translate_context_management, translate_output_format, translate_thinking,
    translate_tool_choice, translate_tools,
};
use super::outputs::{
    DeveloperMessage, DeveloperRole, ResponsesContextManagement, ResponsesInputItem,
    ResponsesReasoning, ResponsesRequestRecord, ResponsesText, ResponsesTool, ResponsesToolChoice,
};
use super::records::{
    AnthropicRequestRecord, MetadataRecord, SettingsRecord, SystemBlock, SystemContent,
};

const SESSION_ID_LIMIT: usize = 64;

pub fn translate_request(
    request: &AnthropicRequestRecord,
    settings: &SettingsRecord,
    include_encrypted_reasoning: bool,
    limits: &ApiLimits,
) -> Result<ResponsesRequestRecord, MessagesResponsesOutputError> {
    let options = request_options(request, settings)?;
    let developer_parts = system_developer_parts(request.system.as_ref());
    let instructions = system_instructions(request.system.as_ref(), developer_parts.is_empty());
    let input = messages_input(&request.messages, developer_parts)?;
    let (user, prompt_cache_key) = user_fields(request.metadata.as_ref(), limits)?;
    Ok(ResponsesRequestRecord {
        model: request.model.clone(),
        input,
        include: include_encrypted_reasoning.then(|| vec![ENCRYPTED_REASONING_INCLUDE.to_owned()]),
        instructions,
        max_output_tokens: request.max_tokens.filter(|tokens| *tokens != 0),
        temperature: request.temperature,
        top_p: request.top_p,
        tools: options.tools,
        tool_choice: options.tool_choice,
        reasoning: options.reasoning,
        text: options.text,
        context_management: options.context_management,
        user,
        prompt_cache_key,
    })
}

struct RequestOptions {
    tools: Option<Vec<ResponsesTool>>,
    tool_choice: Option<ResponsesToolChoice>,
    reasoning: Option<ResponsesReasoning>,
    text: Option<ResponsesText>,
    context_management: Option<Vec<ResponsesContextManagement>>,
}

fn request_options(
    request: &AnthropicRequestRecord,
    settings: &SettingsRecord,
) -> Result<RequestOptions, MessagesResponsesOutputError> {
    let reasoning = match request.thinking.as_ref() {
        Some(thinking) => translate_thinking(thinking, request.output_config.as_ref(), settings)?,
        None => None,
    };
    let context_management = match request.context_management.as_ref() {
        Some(management) => translate_context_management(management)?,
        None => None,
    };
    Ok(RequestOptions {
        tools: request
            .tools
            .as_deref()
            .filter(|tools| !tools.is_empty())
            .map(translate_tools),
        tool_choice: request.tool_choice.as_ref().map(translate_tool_choice),
        reasoning,
        text: translate_output_format(request),
        context_management,
    })
}

fn messages_input(
    messages: &[MessageRecord],
    developer_parts: Vec<MessagePart>,
) -> Result<Vec<ResponsesInputItem>, MessagesResponsesOutputError> {
    let mut input: Vec<ResponsesInputItem> = messages_to_input(messages)
        .map_err(|source| MessagesResponsesOutputError::Messages { source })?
        .into_iter()
        .map(ResponsesInputItem::Item)
        .collect();
    if !developer_parts.is_empty() {
        input.insert(
            0,
            ResponsesInputItem::Developer(DeveloperMessage {
                r#type: MessageKind::Message,
                role: DeveloperRole::Developer,
                content: developer_parts,
            }),
        );
    }
    Ok(input)
}

fn system_developer_parts(system: Option<&SystemContent>) -> Vec<MessagePart> {
    let Some(SystemContent::Blocks(blocks)) = system else {
        return Vec::new();
    };
    if !blocks
        .iter()
        .any(|block| block.prompt_cache_breakpoint.is_some())
    {
        return Vec::new();
    }
    blocks.iter().filter_map(developer_part).collect()
}

fn developer_part(block: &SystemBlock) -> Option<MessagePart> {
    if block.r#type.as_deref() != Some("text") {
        return None;
    }
    let text = block.text.as_deref().filter(|text| !text.is_empty())?;
    Some(MessagePart::Text {
        text: text.to_owned(),
        prompt_cache_breakpoint: block.prompt_cache_breakpoint,
    })
}

fn system_instructions(system: Option<&SystemContent>, no_developer_parts: bool) -> Option<String> {
    if !no_developer_parts {
        return None;
    }
    match system? {
        SystemContent::Text(text) => (!text.is_empty()).then(|| text.clone()),
        SystemContent::Blocks(blocks) if !blocks.is_empty() => {
            let texts: Vec<&str> = blocks.iter().filter_map(text_part).collect();
            Some(texts.join("\n"))
        }
        SystemContent::Blocks(_) => None,
    }
}

fn text_part(block: &SystemBlock) -> Option<&str> {
    if block.r#type.as_deref() != Some("text") {
        return None;
    }
    block.text.as_deref().filter(|text| !text.is_empty())
}

fn user_fields(
    metadata: Option<&MetadataRecord>,
    limits: &ApiLimits,
) -> Result<(Option<String>, Option<String>), MessagesResponsesOutputError> {
    let Some(user_id) = metadata.and_then(|record| record.user_id.as_deref()) else {
        return Ok((None, None));
    };
    let user = Some(user_id.chars().take(SESSION_ID_LIMIT).collect());
    let prompt_cache_key = decode_messages_thinking(user_id.as_bytes(), limits)
        .map_err(|source| MessagesResponsesOutputError::SessionKey { source })?;
    Ok((user, prompt_cache_key))
}
