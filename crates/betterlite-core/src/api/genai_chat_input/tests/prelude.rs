use betterlite_openai::chat::{ChatMessage, ChatRequest, MessageContent};
use serde_json::Value;

use crate::api::genai_chat_input::{
    GenerateContentRequest, GenerateLiteLLMParams, TranslatedCompletion, translate_generate_content,
};
use crate::schema::SchemaLimits;

pub(super) fn request(body: Value) -> GenerateContentRequest {
    serde_json::from_value(body).expect("a GenAI request")
}

pub(super) fn translate(body: Value) -> ChatRequest {
    translate_generate_content(&request(body), None, SchemaLimits::default())
        .expect("the translated request")
        .0
}

pub(super) fn translate_with(body: Value, params: &GenerateLiteLLMParams) -> TranslatedCompletion {
    translate_generate_content(&request(body), Some(params), SchemaLimits::default())
        .expect("the translated request")
}

pub(super) fn text_of(message: &ChatMessage) -> String {
    match message.content.as_ref().expect("message content") {
        MessageContent::Text(text) => text.clone(),
        MessageContent::Parts(parts) => parts
            .first()
            .and_then(|part| part.fields.get("text"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    }
}
