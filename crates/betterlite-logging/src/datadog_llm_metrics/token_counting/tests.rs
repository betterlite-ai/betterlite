#![cfg(test)]

use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::datadog_llm_metrics::token_counting::{
    ToolTokenCounter, ToolTokensError, tool_output_tokens,
};
use crate::datadog_llm_obs::records::{DdMessage, InputMessage, Message, ToolResult};

struct TextLengthCounter;

impl ToolTokenCounter for TextLengthCounter {
    type Failure = Infallible;

    fn count_text(
        &self,
        _model: &str,
        text: &str,
    ) -> impl Future<Output = Result<usize, Self::Failure>> + Send {
        ready(Ok(text.chars().count()))
    }
}

#[derive(Default)]
struct RecordingCounter {
    calls: Mutex<Vec<(String, String)>>,
}

impl RecordingCounter {
    fn calls(&self) -> Vec<(String, String)> {
        self.calls.lock().expect("the counter lock").clone()
    }
}

impl ToolTokenCounter for RecordingCounter {
    type Failure = Infallible;

    fn count_text(
        &self,
        model: &str,
        text: &str,
    ) -> impl Future<Output = Result<usize, Self::Failure>> + Send {
        let count = text.chars().count();
        self.calls
            .lock()
            .expect("the counter lock")
            .push((model.to_owned(), text.to_owned()));
        ready(Ok(count))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the tokenizer is unavailable")]
struct CounterFailure;

struct FailingCounter;

impl ToolTokenCounter for FailingCounter {
    type Failure = CounterFailure;

    fn count_text(
        &self,
        _model: &str,
        _text: &str,
    ) -> impl Future<Output = Result<usize, Self::Failure>> + Send {
        ready(Err(CounterFailure))
    }
}

struct HugeCounter;

impl ToolTokenCounter for HugeCounter {
    type Failure = Infallible;

    fn count_text(
        &self,
        _model: &str,
        _text: &str,
    ) -> impl Future<Output = Result<usize, Self::Failure>> + Send {
        ready(Ok(usize::MAX))
    }
}

fn tool_result(text: &str) -> ToolResult {
    ToolResult {
        name: "lookup".to_owned(),
        result: text.to_owned(),
        tool_id: "call-1".to_owned(),
        kind: "tool_result".to_owned(),
    }
}

fn message(results: Vec<ToolResult>) -> DdMessage {
    DdMessage::Message(Message {
        role: "assistant".to_owned(),
        content: Json::Text("calling the tool".to_owned()),
        reasoning_content: None,
        tool_calls: Vec::new(),
        tool_results: results,
    })
}

#[tokio::test]
async fn every_tool_result_across_messages_is_counted() {
    let messages = vec![
        message(vec![tool_result("ab"), tool_result("cde")]),
        message(vec![tool_result("f")]),
    ];
    let tokens = tool_output_tokens(&messages, "gpt-4o", &TextLengthCounter)
        .await
        .expect("the tool results count");
    assert_eq!(tokens, Some(6.0_f64));
}

#[tokio::test]
async fn messages_without_tool_results_count_nothing() {
    let messages = vec![message(Vec::new())];
    let tokens = tool_output_tokens(&messages, "gpt-4o", &TextLengthCounter)
        .await
        .expect("the messages count");
    assert_eq!(tokens, None);
}

#[tokio::test]
async fn no_messages_count_nothing() {
    let tokens = tool_output_tokens(&[], "gpt-4o", &TextLengthCounter)
        .await
        .expect("the empty conversation counts");
    assert_eq!(tokens, None);
}

#[tokio::test]
async fn the_model_and_each_result_text_reach_the_counter() {
    let counter = RecordingCounter::default();
    let messages = vec![message(vec![tool_result("first"), tool_result("second")])];
    let tokens = tool_output_tokens(&messages, "claude-3", &counter)
        .await
        .expect("the tool results count");
    assert_eq!(tokens, Some(11.0_f64));
    assert_eq!(
        counter.calls(),
        vec![
            ("claude-3".to_owned(), "first".to_owned()),
            ("claude-3".to_owned(), "second".to_owned()),
        ]
    );
}

#[tokio::test]
async fn an_empty_result_still_counts_as_a_tool_output() {
    let messages = vec![message(vec![tool_result("")])];
    let tokens = tool_output_tokens(&messages, "gpt-4o", &TextLengthCounter)
        .await
        .expect("the tool results count");
    assert_eq!(tokens, Some(0.0_f64));
}

#[tokio::test]
async fn input_and_raw_messages_are_skipped() {
    let messages = vec![
        DdMessage::Input(InputMessage {
            input: "not a tool result".to_owned(),
        }),
        DdMessage::Raw(IndexMap::new()),
        message(vec![tool_result("x")]),
    ];
    let tokens = tool_output_tokens(&messages, "gpt-4o", &TextLengthCounter)
        .await
        .expect("the tool results count");
    assert_eq!(tokens, Some(1.0_f64));
}

#[tokio::test]
async fn a_failing_counter_is_reported() {
    let messages = vec![message(vec![tool_result("x")])];
    let error = tool_output_tokens(&messages, "gpt-4o", &FailingCounter)
        .await
        .expect_err("the counter failure surfaces");
    assert!(matches!(error, ToolTokensError::Counter { .. }));
}

#[tokio::test]
async fn a_count_sum_past_u64_is_reported() {
    let messages = vec![message(vec![tool_result("a"), tool_result("b")])];
    let error = tool_output_tokens(&messages, "gpt-4o", &HugeCounter)
        .await
        .expect_err("the overflow surfaces");
    assert!(matches!(error, ToolTokensError::Overflow));
}
