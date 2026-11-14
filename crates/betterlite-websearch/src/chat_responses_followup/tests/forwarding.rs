use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use betterlite_context::agentic_loop_plan::{FollowupMessage, JsonMap};
use serde_json::json;
use tokio::task::yield_now;

use super::{ScriptedSearch, chat_calls, function_call, map, openai_call, responses_calls};
use crate::call_transformation::outcome::{SearchResult, search_outcome_text};
use crate::chat_responses_followup::chat::{FollowupRequest, build_chat_patch};
use crate::chat_responses_followup::responses::{
    ResponsesInput, ResponsesRequest, build_responses_patch,
};
use crate::chat_responses_followup::search::{SearchExecutor, SearchSurface, search_batch};
use crate::records::RichWebSearchInput;

struct OverlapProbe {
    running: AtomicUsize,
    peak: AtomicUsize,
}

#[async_trait]
impl SearchExecutor for OverlapProbe {
    async fn execute_search(
        &self,
        _query: &str,
        _kwargs: &JsonMap,
        _rich: Option<RichWebSearchInput>,
    ) -> SearchResult {
        let before = self.running.fetch_add(1, Ordering::SeqCst);
        self.peak
            .fetch_max(before.saturating_add(1), Ordering::SeqCst);
        yield_now().await;
        self.running.fetch_sub(1, Ordering::SeqCst);
        SearchResult::Succeeded {
            text: "result".to_owned(),
            response: None,
        }
    }
}

#[tokio::test]
async fn search_batch_runs_every_search_at_the_same_time() {
    let probe = OverlapProbe {
        running: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
    };
    let calls = chat_calls(json!([
        openai_call("call_1", json!({"query": "first"})),
        openai_call("call_2", json!({"query": "second"})),
        openai_call("call_3", json!({"query": "third"})),
    ]));
    let outcomes = search_batch(&probe, &calls, &map(json!({})), SearchSurface::Chat).await;
    assert_eq!(outcomes.len(), 3);
    assert_eq!(search_outcome_text(&outcomes[2]), "result");
    assert_eq!(probe.peak.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn chat_patch_hands_the_request_kwargs_to_the_search() {
    let searcher = Arc::new(ScriptedSearch::successful(&["result"]));
    let calls = chat_calls(json!([openai_call("call_1", json!({"query": "weather"}))]));
    let messages: Vec<FollowupMessage> = Vec::new();
    let optional_params = map(json!({}));
    let kwargs = map(json!({"marker": "kept"}));
    let request = FollowupRequest {
        model: "gpt-4o",
        messages: &messages,
        optional_params: &optional_params,
        kwargs: &kwargs,
    };
    build_chat_patch(searcher.as_ref(), &calls, "openai", &request)
        .await
        .unwrap();
    assert_eq!(searcher.recorded()[0].marker.as_deref(), Some("kept"));
}

#[tokio::test]
async fn responses_patch_hands_the_request_kwargs_to_the_search() {
    let searcher = Arc::new(ScriptedSearch::successful(&["result"]));
    let calls = responses_calls(json!([function_call(
        Some("call_1"),
        "{\"query\":\"weather\"}"
    )]));
    let input = ResponsesInput::Items(Vec::new());
    let optional_params = map(json!({}));
    let kwargs = map(json!({"marker": "kept"}));
    let request = ResponsesRequest {
        model: "gpt-4o",
        input: &input,
        optional_params: &optional_params,
        kwargs: &kwargs,
    };
    build_responses_patch(searcher.as_ref(), &calls, &request)
        .await
        .unwrap();
    assert_eq!(searcher.recorded()[0].marker.as_deref(), Some("kept"));
}
