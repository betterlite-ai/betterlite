use crate::passthrough_reconstruction::events::RawSseChunk;
use crate::passthrough_reconstruction::legacy::{
    AssemblyRequest, AssemblyServices, assemble_response_legacy,
};
use crate::passthrough_reconstruction::response::assemble_response;
use crate::streamed_response_assembly::response::Response;

use super::support::{
    FIXED_NOW, FixtureCosts, FixtureCounter, ScriptedConverter, fixture_hooks, text_chunk,
};

const START: &str = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-3-5-sonnet\"}}";
const BLOCK_START: &str = "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}";
const STOP: &str = "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}";

fn delta(text: &str) -> String {
    format!(
        "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"{text}\"}}}}"
    )
}

fn pure_text_chunks() -> Vec<RawSseChunk> {
    vec![text_chunk(&format!(
        "{START}\n\n{BLOCK_START}\n\n{}\n\n{}\n\n{STOP}",
        delta("Hello"),
        delta(" world"),
    ))]
}

fn content_of(response: Option<Response>) -> String {
    let Some(Response::Chat(chat)) = response else {
        panic!("the reconstruction builds a chat response");
    };
    chat.choices[0].message.content.clone().unwrap_or_default()
}

#[tokio::test]
async fn collapses_before_converting_a_pure_text_stream() {
    let costs = FixtureCosts;
    let counter = FixtureCounter;
    let hooks = fixture_hooks();
    let services = AssemblyServices {
        costs: &costs,
        counter: &counter,
        logging: None,
        hooks: &hooks,
    };
    let chunks = pure_text_chunks();
    let mut fast = ScriptedConverter::default();
    let fast_request = AssemblyRequest {
        chunks: &chunks,
        converter: &mut fast,
        now: FIXED_NOW,
    };
    let fast_response = assemble_response(fast_request, &services).await.unwrap();
    let mut legacy = ScriptedConverter::default();
    let legacy_request = AssemblyRequest {
        chunks: &chunks,
        converter: &mut legacy,
        now: FIXED_NOW,
    };
    let legacy_response = assemble_response_legacy(legacy_request, &services)
        .await
        .unwrap();
    assert_eq!(fast.seen.len(), 4);
    assert_eq!(legacy.seen.len(), 5);
    assert!(fast.seen[2].contains("\"text\":\"Hello world\""));
    let fast_encoded = serde_json::to_string(&fast_response).unwrap();
    assert_eq!(content_of(fast_response), "Hello world");
    assert_eq!(
        fast_encoded,
        serde_json::to_string(&legacy_response).unwrap()
    );
}

#[tokio::test]
async fn keeps_the_events_a_thinking_stream_carries() {
    let costs = FixtureCosts;
    let counter = FixtureCounter;
    let hooks = fixture_hooks();
    let services = AssemblyServices {
        costs: &costs,
        counter: &counter,
        logging: None,
        hooks: &hooks,
    };
    let thinking = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"hmm\"}}";
    let chunks = [text_chunk(&format!(
        "{START}\n\n{BLOCK_START}\n\n{thinking}\n\n{}\n\n{STOP}",
        delta("Hello"),
    ))];
    let mut converter = ScriptedConverter::default();
    let request = AssemblyRequest {
        chunks: &chunks,
        converter: &mut converter,
        now: FIXED_NOW,
    };
    let response = assemble_response(request, &services).await.unwrap();
    assert_eq!(converter.seen.len(), 5);
    assert!(converter.seen[2].contains("thinking_delta"));
    assert_eq!(content_of(response), "Hello");
}

#[tokio::test]
async fn falls_back_to_the_legacy_path_on_a_bad_frame() {
    let costs = FixtureCosts;
    let counter = FixtureCounter;
    let hooks = fixture_hooks();
    let services = AssemblyServices {
        costs: &costs,
        counter: &counter,
        logging: None,
        hooks: &hooks,
    };
    let chunks = [text_chunk(&format!(
        "{START}\n\ndata: {{bad}}\n\n{}\n\n{STOP}",
        delta("ok"),
    ))];
    let mut converter = ScriptedConverter::default();
    let request = AssemblyRequest {
        chunks: &chunks,
        converter: &mut converter,
        now: FIXED_NOW,
    };
    let response = assemble_response(request, &services).await.unwrap();
    assert_eq!(content_of(response), "ok");
    assert!(converter.seen.iter().any(|event| event == "data: {bad}"));
}
