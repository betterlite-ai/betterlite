use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;

use crate::chat::ChatMessage;
use crate::chat_content::{ChatContent, ChatContentError, Mode, RefuseDownloads};

use super::PROVIDER;
use super::history::{fill_reasoning_content, thinking_active};
use super::messages::{convert_messages, forward_or_collapse};
use super::tools::drop_unsupported_tools;

pub struct RequestInput<'request> {
    pub model: &'request str,
    pub messages: &'request [ChatMessage],
    pub optional_params: &'request Object,
    pub supports_vision: bool,
    pub supports_reasoning: bool,
}

pub fn transform_request(
    request: &RequestInput<'_>,
    mode: Mode,
) -> Result<Object, ChatContentError> {
    let dropped = drop_unsupported_tools(request.optional_params);
    let prompt: Vec<Message> = convert_messages(request.messages)?;
    let history = if thinking_active(request.supports_reasoning, &dropped.params) {
        fill_reasoning_content(&prompt)
    } else {
        prompt
    };
    let prepared = forward_or_collapse(&history, request.supports_vision);
    let typed: Vec<ChatMessage> = convert_messages(&prepared)?;
    let environment = BTreeMap::new();
    let content = ChatContent {
        model: request.model,
        messages: &typed,
        optional_params: &dropped.params,
        provider: Some(PROVIDER),
        api_base: None,
        global_api_base: None,
        openai_system_messages_first: false,
        environment: &environment,
        downloads: &RefuseDownloads,
    };
    content.transform(mode)?.to_body()
}
