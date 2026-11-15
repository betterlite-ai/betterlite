use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::{ChatMessage, Tool};

use crate::compaction_history::compactor::{CompactionCounter, ResponsesPrefix};
use crate::compaction_history::contracts::{
    Candidate, Deployment, ModelLimits, NativeCapability, NativeSupport, SummaryRequest,
};
use crate::compaction_history::error::{CompactionError, ErrorKind};

pub(super) fn messages(raw: &str) -> Vec<ChatMessage> {
    serde_json::from_str(raw).unwrap()
}

pub(super) fn json(raw: &str) -> Json {
    serde_json::from_str(raw).unwrap()
}

pub(super) fn items(raw: &str) -> Vec<Json> {
    serde_json::from_str(raw).unwrap()
}

pub(super) fn deployment(
    provider: bool,
    capability: NativeCapability,
    window: Option<i64>,
    output: Option<i64>,
) -> Deployment {
    Deployment {
        model: "compactor-model".to_owned(),
        custom_llm_provider: Some("anthropic".to_owned()),
        limits: ModelLimits {
            max_input_tokens: window,
            max_output_tokens: output,
        },
        native: NativeSupport {
            provider,
            capability,
        },
    }
}

pub(super) fn candidate(model: &str, deployments: Vec<Deployment>) -> Candidate {
    Candidate {
        model: model.to_owned(),
        deployments,
    }
}

pub(super) fn request(max_tokens: i64) -> SummaryRequest {
    SummaryRequest {
        model: "parent-model".to_owned(),
        messages: messages(r#"[{"role":"user","content":"older"}]"#),
        max_tokens,
        tools: None,
        system: None,
        user: None,
    }
}

pub(super) struct FixedCounter {
    pub(super) value: i64,
}

#[async_trait]
impl CompactionCounter for FixedCounter {
    async fn count(&self, _request: &SummaryRequest) -> Result<i64, CompactionError> {
        Ok(self.value)
    }
}

pub(super) struct RecordingCounter {
    pub(super) seen: Mutex<Vec<SummaryRequest>>,
}

#[async_trait]
impl CompactionCounter for RecordingCounter {
    async fn count(&self, request: &SummaryRequest) -> Result<i64, CompactionError> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        Ok(0)
    }
}

pub(super) struct FailingCounter;

#[async_trait]
impl CompactionCounter for FailingCounter {
    async fn count(&self, request: &SummaryRequest) -> Result<i64, CompactionError> {
        Err(CompactionError {
            kind: ErrorKind::Rejected,
            model: request.model.clone(),
            reason: "the counter failed".to_owned(),
            source: None,
        })
    }
}

pub(super) struct CheckedConverter {
    pub(super) items: Mutex<Vec<usize>>,
    pub(super) requests: Mutex<Vec<Json>>,
    pub(super) tools: Mutex<Vec<usize>>,
}

impl ResponsesPrefix for CheckedConverter {
    fn messages(
        &self,
        items: &[Json],
        request: &Json,
    ) -> Result<Vec<ChatMessage>, CompactionError> {
        self.items
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(items.len());
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        Ok(messages(r#"[{"role":"user","content":"older"}]"#))
    }

    fn tools(&self, tools: &[Json]) -> Result<Vec<Tool>, CompactionError> {
        self.tools
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(tools.len());
        Ok(Vec::new())
    }
}
