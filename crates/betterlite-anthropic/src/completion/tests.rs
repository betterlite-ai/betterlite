#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::response::{
    AssistantMessage, ChatChoice, ChatResponse, ChatUsage, MessageProviderFields,
};
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::{Hub, builtin_entries};
use betterlite_tokenizer::count::default_encoding::{DefaultEncoding, DefaultEngine};
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::assets;
use betterlite_tokenizer::engine::huggingface::HuggingFaceEngine;
use betterlite_tokenizer::engine::selection::SelectionError;
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};
use chrono::{DateTime, TimeZone as _, Utc};
use indexmap::IndexMap;

use crate::completion::prompt::CustomPromptEntry;

mod cases;
mod corpus;
mod invariants;
mod options;
mod prompts;
mod responses;
mod streams;

pub(crate) const USER_PROMPT: &str = "\n\nHuman: hello\n\nAssistant: ";

pub(crate) fn object(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).unwrap()
}

// claude-2 gets a custom template, so claude-instant-1 still goes through the prompt factory
pub(crate) fn templates() -> BTreeMap<String, CustomPromptEntry> {
    let entry: CustomPromptEntry = serde_json::from_str(
        r#"{"roles":{"user":{"pre_message":"U:","post_message":"|"}},"initial_prompt_value":"<","final_prompt_value":">"}"#,
    )
    .unwrap();
    BTreeMap::from([("claude-2".to_owned(), entry)])
}

pub(crate) fn clock() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap()
}

pub(crate) fn factory() -> PromptFactory {
    PromptFactory {
        cache: Mutex::new(builtin_entries()),
        hub: Hub::Unreachable,
    }
}

pub(crate) fn tiktoken() -> Engine {
    let data = assets::bundled(Encoding::Cl100kBase).unwrap();
    Engine::Tiktoken(Arc::new(
        TiktokenEngine::from_bpe_file(Encoding::Cl100kBase, &data).unwrap(),
    ))
}

pub(crate) fn user_messages() -> Vec<Message> {
    serde_json::from_str(r#"[{"role":"user","content":"hello"}]"#).unwrap()
}

pub(crate) fn blank_response(content: Option<&str>) -> ChatResponse {
    ChatResponse {
        id: "chatcmpl-kept".to_owned(),
        object: "chat.completion".to_owned(),
        created: 1,
        model: "unset".to_owned(),
        choices: vec![ChatChoice {
            index: 0,
            message: AssistantMessage {
                role: "assistant".to_owned(),
                content: content.map(str::to_owned),
                tool_calls: None,
                annotations: None,
                audio: None,
                refusal: None,
                provider_specific_fields: MessageProviderFields::default(),
            },
            finish_reason: None,
            logprobs: None,
            provider_specific_fields: BTreeMap::new(),
        }],
        usage: ChatUsage::default(),
        system_fingerprint: None,
        service_tier: None,
    }
}

// the default encoding answers with the Anthropic tokenizer, so it differs from the explicit cl100k one
pub(crate) struct AnthropicDefault;

#[async_trait]
impl DefaultEngine for AnthropicDefault {
    async fn load_default(&self) -> Result<Engine, SelectionError> {
        let engine = HuggingFaceEngine::from_bytes(assets::ANTHROPIC).unwrap();
        Ok(Engine::HuggingFace(Arc::new(engine)))
    }
}

pub(crate) fn default_encoding() -> DefaultEncoding {
    DefaultEncoding::new(Arc::new(AnthropicDefault))
}
