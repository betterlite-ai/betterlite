#![cfg(test)]

use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use rstest::rstest;

use super::records::Chunk;
use super::response::{ChatResponse, Response, TextResponse};
use super::{AssemblyError, MetadataServices, Request, StreamLogging, stream_chunk_builder};
use crate::stream_chunk_shaping::records::{JsonObject, JsonValue};

mod cases;
mod support;

use support::{FixtureCosts, FixtureCounter, chunks, hooks};

const FIXED_NOW: i64 = 1_700_000_000;

struct Harness<'fixture> {
    chunks: Option<Vec<Chunk>>,
    messages: bool,
    prompt: Option<Box<dyn Fn() -> Result<i64, CountError> + Send + Sync>>,
    logging: Option<&'fixture dyn StreamLogging>,
    metadata: Option<&'fixture MetadataServices<'fixture>>,
    counter: FixtureCounter,
    costs: FixtureCosts,
    now: i64,
}

impl<'fixture> Harness<'fixture> {
    fn new(chunks: Option<Vec<Chunk>>) -> Self {
        Self {
            chunks,
            messages: false,
            prompt: None,
            logging: None,
            metadata: None,
            counter: FixtureCounter::default(),
            costs: FixtureCosts::default(),
            now: FIXED_NOW,
        }
    }

    async fn run(&self) -> (Result<Option<Response>, AssemblyError>, Vec<String>) {
        let hooks = hooks();
        let messages: Option<&[Message]> = if self.messages { Some(&[]) } else { None };
        let request = Request {
            chunks: self.chunks.as_deref(),
            messages,
            count_prompt_tokens: self.prompt.as_deref(),
            logging: self.logging,
            costs: &self.costs,
            counter: &self.counter,
            metadata: self.metadata,
            now: self.now,
        };
        let result = stream_chunk_builder(&request, &hooks).await;
        (result, hooks.logger.messages())
    }
}

fn ok(result: Result<Option<Response>, AssemblyError>) -> Response {
    match result {
        Ok(Some(response)) => response,
        Ok(None) => panic!("expected an assembled response, got None"),
        Err(error) => panic!("expected an assembled response, got {error:?}"),
    }
}

fn failed(result: Result<Option<Response>, AssemblyError>) -> AssemblyError {
    match result {
        Ok(response) => panic!("expected an assembly error, got {response:?}"),
        Err(error) => error,
    }
}

fn chat(response: Response) -> ChatResponse {
    match response {
        Response::Chat(inner) => *inner,
        Response::Text(inner) => panic!("expected a chat response, got {inner:?}"),
    }
}

fn text(response: Response) -> TextResponse {
    match response {
        Response::Text(inner) => inner,
        Response::Chat(inner) => panic!("expected a text response, got {inner:?}"),
    }
}

async fn chat_of(harness: &Harness<'_>) -> ChatResponse {
    let (result, _logs) = harness.run().await;
    chat(ok(result))
}

fn value(source: &str) -> JsonValue {
    serde_json::from_str(source).expect("fixture value parses")
}

fn object(source: &str) -> JsonObject {
    serde_json::from_str(source).expect("fixture object parses")
}

const PLAIN_STREAM: &str = r#"[{"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o", "choices": [{"index": 0, "delta": {"role": "assistant", "content": "A"}}]}]"#;

const COST_STREAM_HEAD: &str = r#"[{"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o", "choices": [{"index": 0, "delta": {"role": "assistant", "content": "A"}}]}, {"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15"#;

const HIDDEN_STREAM_TAIL: &str =
    r#", "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}}]"#;

fn plain_stream() -> Vec<Chunk> {
    chunks(PLAIN_STREAM)
}

fn cost_stream(cost: Option<&str>) -> Vec<Chunk> {
    let mut stream = COST_STREAM_HEAD.to_owned();
    if let Some(cost) = cost {
        stream.push_str(r#", "cost": "#);
        stream.push_str(cost);
    }
    stream.push_str("}}]");
    chunks(&stream)
}

fn hidden_stream(hidden: &str) -> Vec<Chunk> {
    let mut stream = String::from(
        r#"[{"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o", "choices": [{"index": 0, "delta": {"role": "assistant", "content": "A"}}]}, {"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}], "_hidden_params": "#,
    );
    stream.push_str(hidden);
    stream.push_str(HIDDEN_STREAM_TAIL);
    chunks(&stream)
}

#[derive(Clone, Copy, Debug)]
enum Proof {
    EmptyAndErrorBoundary,
    TextCompletionAssembly,
    SimpleTextPath,
    ComplexContentOrder,
    ProviderFieldsCitations,
    UsageHiddenMetadata,
    ReportedCostPrecedence,
    UsageCostStamping,
}

#[rstest]
#[case::empty_and_error_boundary(Proof::EmptyAndErrorBoundary)]
#[case::text_completion_assembly(Proof::TextCompletionAssembly)]
#[case::simple_text_path(Proof::SimpleTextPath)]
#[case::complex_content_order(Proof::ComplexContentOrder)]
#[case::provider_fields_citations(Proof::ProviderFieldsCitations)]
#[case::usage_hidden_metadata(Proof::UsageHiddenMetadata)]
#[case::reported_cost_precedence(Proof::ReportedCostPrecedence)]
#[case::usage_cost_stamping(Proof::UsageCostStamping)]
#[tokio::test]
async fn streamed_response_assembly_cases(#[case] proof: Proof) {
    match proof {
        Proof::EmptyAndErrorBoundary => cases::empty_and_error_boundary().await,
        Proof::TextCompletionAssembly => cases::text_completion_assembly().await,
        Proof::SimpleTextPath => cases::simple_text_path().await,
        Proof::ComplexContentOrder => cases::complex_content_order().await,
        Proof::ProviderFieldsCitations => cases::provider_fields_citations().await,
        Proof::UsageHiddenMetadata => cases::usage_hidden_metadata().await,
        Proof::ReportedCostPrecedence => cases::reported_cost_precedence().await,
        Proof::UsageCostStamping => cases::usage_cost_stamping().await,
    }
}
