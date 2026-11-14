#![cfg(test)]

use async_trait::async_trait;
use betterlite_tokenizer::count::error::CountError;
use indexmap::IndexMap;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Mutex;

use crate::compress::assembly::get_dropped_tool_span_indices;
use crate::compress::budget::{
    BudgetSelection, BudgetSelectionRequest, select_kept_indices_for_budget,
};
use crate::compress::counting::TokenCounter;
use crate::embedding_scoring::{EmbeddingCall, EmbeddingExecutor};
use crate::message_normalization::content_to_text;
use crate::truncate_message::truncate_message;

use super::*;

const MODEL: &str = "gpt-4";

#[derive(Clone, Debug, PartialEq)]
enum CountCall {
    Text { model: String, text: String },
    Messages { model: String, messages: Vec<Value> },
}

struct FakeCounter {
    message_totals: Mutex<VecDeque<usize>>,
    text_overrides: BTreeMap<String, usize>,
    calls: Mutex<Vec<CountCall>>,
}

fn fixture_counter(totals: Vec<usize>) -> FakeCounter {
    FakeCounter {
        message_totals: Mutex::new(VecDeque::from(totals)),
        text_overrides: BTreeMap::new(),
        calls: Mutex::new(Vec::new()),
    }
}

#[async_trait]
impl TokenCounter for FakeCounter {
    async fn count_text(&self, model: &str, text: &str) -> Result<usize, CountError> {
        self.calls
            .lock()
            .expect("recorded calls")
            .push(CountCall::Text {
                model: model.to_owned(),
                text: text.to_owned(),
            });
        if let Some(tokens) = self.text_overrides.get(text) {
            return Ok(*tokens);
        }
        Ok(text.split_whitespace().count())
    }

    async fn count_messages(&self, model: &str, messages: &[Value]) -> Result<usize, CountError> {
        self.calls
            .lock()
            .expect("recorded calls")
            .push(CountCall::Messages {
                model: model.to_owned(),
                messages: messages.to_vec(),
            });
        if let Some(total) = self
            .message_totals
            .lock()
            .expect("message totals")
            .pop_front()
        {
            return Ok(total);
        }
        let mut total = 4_usize.saturating_mul(messages.len());
        for message in messages {
            let text = message
                .get("content")
                .map(content_to_text)
                .unwrap_or_default();
            total = total.saturating_add(text.split_whitespace().count());
        }
        Ok(total)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake embedding call failed")]
struct FakeEmbeddingFailure;

struct RecordingExecutor {
    embeddings: Vec<Vec<f64>>,
    calls: Mutex<Vec<Map<String, Value>>>,
}

#[async_trait]
impl EmbeddingExecutor for RecordingExecutor {
    type Failure = FakeEmbeddingFailure;

    async fn embed(&self, call: EmbeddingCall<'_>) -> Result<Vec<Vec<f64>>, Self::Failure> {
        self.calls
            .lock()
            .expect("recorded embedding calls")
            .push(call.parameters.clone());
        Ok(self.embeddings.clone())
    }
}

fn words(count: usize) -> String {
    (0..count)
        .map(|index| format!("w{index}"))
        .collect::<Vec<String>>()
        .join(" ")
}

fn request<'request>(
    messages: &'request [Value],
    trigger: usize,
    target: Option<usize>,
) -> CompressRequest<'request> {
    CompressRequest {
        messages,
        model: MODEL,
        call_type: "completion",
        compression_trigger: trigger,
        compression_target: target,
    }
}

async fn select_budget(
    normalized: &[Value],
    scores: &[f64],
    target: usize,
    kept: &[usize],
    spans: &[Vec<usize>],
) -> BudgetSelection {
    let request = BudgetSelectionRequest {
        normalized_messages: normalized,
        original_messages: normalized,
        combined_scores: scores,
        compression_target: target,
        model: MODEL,
        initial_kept_indices: kept,
        tool_exchange_spans: spans,
    };
    let counter = fixture_counter(Vec::new());
    select_kept_indices_for_budget(&request, &counter)
        .await
        .expect("selected")
}

mod budget_selection;
mod cache_key_extraction;
mod compress_behavior;
mod dropped_span_handling;
mod retrieval_tool_schema;
mod tool_exchange_atomicity;
