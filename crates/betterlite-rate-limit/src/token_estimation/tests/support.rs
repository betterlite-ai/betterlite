#![cfg(test)]

use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_models::decisions::Json;

use super::RequestData;
use super::estimate::TokenEstimateRequest;
use super::services::{
    GenAiTranslation, GenAiTranslationRequest, InputTokenCounter, ResponsesInput,
    ResponsesInputRequest, TokenCounterRequest, TranslatedChat,
};

pub(crate) fn request(text: &str) -> RequestData {
    serde_json::from_str(text).expect("a request dictionary")
}

pub(crate) fn value(text: &str) -> Json {
    serde_json::from_str(text).expect("a json value")
}

pub(crate) fn chat_reply(messages: &str) -> TranslatedChat {
    TranslatedChat {
        messages: value(messages),
        tools: None,
        tool_choice: None,
    }
}

pub(crate) fn estimate<'request>(
    data: &'request RequestData,
    call_type: Option<&'request str>,
    min_configured_tpm_limit: Option<i64>,
    configured_output_tokens: Option<i64>,
) -> TokenEstimateRequest<'request> {
    TokenEstimateRequest {
        data,
        call_type,
        model: None,
        min_configured_tpm_limit,
        configured_output_tokens,
    }
}

#[derive(Clone)]
pub(crate) struct SeenTranslation {
    pub(crate) model: String,
    pub(crate) contents: Json,
    pub(crate) config: Option<Json>,
    pub(crate) system_instruction: Option<Json>,
    pub(crate) system_instruction_snake: Option<Json>,
    pub(crate) tools: Option<Json>,
    pub(crate) tool_config: Option<Json>,
    pub(crate) tool_config_snake: Option<Json>,
}

pub(crate) struct FixedTranslation {
    pub(crate) reply: Option<TranslatedChat>,
}

pub(crate) fn no_translation() -> FixedTranslation {
    FixedTranslation { reply: None }
}

impl GenAiTranslation for FixedTranslation {
    fn translate_generate_content(
        &self,
        _request: &GenAiTranslationRequest<'_>,
    ) -> Option<TranslatedChat> {
        self.reply.clone()
    }
}

pub(crate) struct RecordingTranslation {
    pub(crate) reply: Option<TranslatedChat>,
    pub(crate) seen: Mutex<Vec<SeenTranslation>>,
}

impl GenAiTranslation for RecordingTranslation {
    fn translate_generate_content(
        &self,
        request: &GenAiTranslationRequest<'_>,
    ) -> Option<TranslatedChat> {
        self.seen
            .lock()
            .expect("the translation recording lock")
            .push(SeenTranslation {
                model: request.model.to_owned(),
                contents: request.contents.clone(),
                config: request.config.cloned(),
                system_instruction: request.system_instruction.cloned(),
                system_instruction_snake: request.system_instruction_snake.cloned(),
                tools: request.tools.cloned(),
                tool_config: request.tool_config.cloned(),
                tool_config_snake: request.tool_config_snake.cloned(),
            });
        self.reply.clone()
    }
}

#[derive(Clone)]
pub(crate) struct SeenResponses {
    pub(crate) input: Json,
    pub(crate) instructions: Option<String>,
}

pub(crate) struct FixedResponses {
    pub(crate) messages: Vec<Json>,
}

impl ResponsesInput for FixedResponses {
    fn transform_responses_api_input(&self, _request: &ResponsesInputRequest<'_>) -> Vec<Json> {
        self.messages.clone()
    }
}

pub(crate) struct RecordingResponses {
    pub(crate) messages: Vec<Json>,
    pub(crate) seen: Mutex<Vec<SeenResponses>>,
}

impl ResponsesInput for RecordingResponses {
    fn transform_responses_api_input(&self, request: &ResponsesInputRequest<'_>) -> Vec<Json> {
        self.seen
            .lock()
            .expect("the responses recording lock")
            .push(SeenResponses {
                input: request.input.clone(),
                instructions: request.instructions.map(str::to_owned),
            });
        self.messages.clone()
    }
}

#[derive(Clone)]
pub(crate) struct SeenCount {
    pub(crate) model: String,
    pub(crate) messages: Option<Vec<Json>>,
    pub(crate) text: Option<Json>,
    pub(crate) tools: Option<Vec<Json>>,
    pub(crate) tool_choice: Option<Json>,
}

pub(crate) struct RecordingCounter {
    pub(crate) tokens: Option<i64>,
    pub(crate) seen: Mutex<Vec<SeenCount>>,
}

#[async_trait]
impl InputTokenCounter for RecordingCounter {
    async fn count_tokens(&self, request: &TokenCounterRequest<'_>) -> Option<i64> {
        self.seen
            .lock()
            .expect("the counter recording lock")
            .push(SeenCount {
                model: request.model.to_owned(),
                messages: request.messages.map(<[Json]>::to_vec),
                text: request.text.cloned(),
                tools: request.tools.map(<[Json]>::to_vec),
                tool_choice: request.tool_choice.cloned(),
            });
        self.tokens
    }
}
