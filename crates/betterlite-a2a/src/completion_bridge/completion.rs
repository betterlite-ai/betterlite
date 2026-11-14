use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;

use betterlite_core::agents::json::AgentJson;
use futures_util::stream::BoxStream;
use serde::{Deserialize, Serialize};

use super::wire::{ModelResponse, OpenAIMessage};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompletionCall {
    pub model: String,
    pub messages: Vec<OpenAIMessage>,
    pub stream: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, AgentJson>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct CompletionChunk {
    #[serde(default)]
    pub choices: Vec<ChunkChoice>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct ChunkChoice {
    #[serde(default)]
    pub delta: Option<ChunkDelta>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct ChunkDelta {
    #[serde(default)]
    pub content: Option<String>,
}

impl CompletionChunk {
    // LiteLLM reads chunk.choices[0].delta.content and skips every other choice
    #[must_use]
    pub fn first_content(&self) -> Option<&str> {
        self.choices
            .first()
            .and_then(|choice| choice.delta.as_ref())
            .and_then(|delta| delta.content.as_deref())
    }
}

// LiteLLM's litellm.acompletion(**completion_params), injected so A2A never names a provider
pub trait CompletionPort: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn complete(
        &self,
        call: CompletionCall,
    ) -> impl Future<Output = Result<ModelResponse, Self::Error>> + Send;

    fn stream(
        &self,
        call: CompletionCall,
    ) -> impl Future<Output = Result<StreamedChunks<Self::Error>, Self::Error>> + Send;
}

pub type StreamedChunks<E> = BoxStream<'static, Result<CompletionChunk, E>>;
