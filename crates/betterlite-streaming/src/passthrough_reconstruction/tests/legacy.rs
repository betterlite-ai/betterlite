use crate::passthrough_reconstruction::error::ReconstructionError;
use crate::passthrough_reconstruction::events::RawSseChunk;
use crate::passthrough_reconstruction::legacy::{
    AssemblyRequest, AssemblyServices, assemble_response_legacy,
};
use crate::streamed_response_assembly::response::Response;

use super::support::{
    FIXED_NOW, FixtureCosts, FixtureCounter, ScriptedConverter, fixture_hooks, text_chunk,
};

const START_EVENT: &str =
    "data: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-3-5-sonnet\"}}";
const STOP_EVENT: &str =
    "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}";

fn delta_event(text: &str) -> String {
    format!(
        "data: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"{text}\"}}}}"
    )
}

fn text_of(response: Option<Response>) -> (String, String) {
    let Some(Response::Chat(chat)) = response else {
        panic!("the legacy reconstruction builds a chat response");
    };
    (
        chat.choices[0].message.content.clone().unwrap_or_default(),
        chat.choices[0].finish_reason.clone(),
    )
}

#[tokio::test]
async fn assembles_every_converted_event() {
    let costs = FixtureCosts;
    let counter = FixtureCounter;
    let hooks = fixture_hooks();
    let services = AssemblyServices {
        costs: &costs,
        counter: &counter,
        logging: None,
        hooks: &hooks,
    };
    let mut converter = ScriptedConverter::default();
    let chunks = [text_chunk(&format!(
        "{START_EVENT}\n\n{}\n\n{}\n\n{STOP_EVENT}",
        delta_event("Hello"),
        delta_event(" world"),
    ))];
    let request = AssemblyRequest {
        chunks: &chunks,
        converter: &mut converter,
        now: FIXED_NOW,
    };
    let response = assemble_response_legacy(request, &services).await.unwrap();
    let (content, finish_reason) = text_of(response);
    assert_eq!(content, "Hello world");
    assert_eq!(finish_reason, "stop");
    assert_eq!(converter.seen.len(), 4);
}

#[tokio::test]
async fn skips_only_a_whole_done_line() {
    let costs = FixtureCosts;
    let counter = FixtureCounter;
    let hooks = fixture_hooks();
    let services = AssemblyServices {
        costs: &costs,
        counter: &counter,
        logging: None,
        hooks: &hooks,
    };
    let mut converter = ScriptedConverter::default();
    let chunks = [text_chunk(&format!(
        "{START_EVENT}\n\ndata: [DONE]\n\n{}\n\ndata:[DONE]\n\n{STOP_EVENT}",
        delta_event("a [DONE] b"),
    ))];
    let request = AssemblyRequest {
        chunks: &chunks,
        converter: &mut converter,
        now: FIXED_NOW,
    };
    let response = assemble_response_legacy(request, &services).await.unwrap();
    assert_eq!(text_of(response).0, "a [DONE] b");
    assert!(!converter.seen.iter().any(|event| event == "data: [DONE]"));
    assert!(converter.seen.iter().any(|event| event == "data:[DONE]"));
    assert!(
        converter
            .seen
            .iter()
            .any(|event| event.contains("a [DONE] b"))
    );
}

#[tokio::test]
async fn matches_a_done_marker_on_the_trimmed_whole_line() {
    let costs = FixtureCosts;
    let counter = FixtureCounter;
    let hooks = fixture_hooks();
    let services = AssemblyServices {
        costs: &costs,
        counter: &counter,
        logging: None,
        hooks: &hooks,
    };
    let mut converter = ScriptedConverter::default();
    let chunks = [text_chunk(&format!(
        "{START_EVENT}\n\n{STOP_EVENT}\n\ndata: [DONE] more\n\nevent: done\n  data: [DONE]"
    ))];
    let request = AssemblyRequest {
        chunks: &chunks,
        converter: &mut converter,
        now: FIXED_NOW,
    };
    let response = assemble_response_legacy(request, &services).await.unwrap();
    assert!(response.is_some());
    assert!(
        converter
            .seen
            .iter()
            .any(|event| event == "data: [DONE] more")
    );
    assert!(
        !converter
            .seen
            .iter()
            .any(|event| event == "event: done\n  data: [DONE]")
    );
}

#[tokio::test]
async fn skips_a_malformed_event_after_logging_it() {
    let costs = FixtureCosts;
    let counter = FixtureCounter;
    let hooks = fixture_hooks();
    let services = AssemblyServices {
        costs: &costs,
        counter: &counter,
        logging: None,
        hooks: &hooks,
    };
    let mut converter = ScriptedConverter {
        seen: Vec::new(),
        malformed: Some("boom"),
        ended: None,
        failed: None,
    };
    let chunks = [text_chunk(&format!(
        "{START_EVENT}\n\ndata: {{boom}}\n\n{}\n\n{STOP_EVENT}",
        delta_event("ok"),
    ))];
    let request = AssemblyRequest {
        chunks: &chunks,
        converter: &mut converter,
        now: FIXED_NOW,
    };
    let response = assemble_response_legacy(request, &services).await.unwrap();
    assert_eq!(text_of(response).0, "ok");
    assert_eq!(
        hooks.logger.messages(),
        vec![
            "Building complete streaming response from 1 chunks".to_owned(),
            "Skipping non-JSON SSE event: data: {boom}".to_owned(),
        ]
    );
}

#[tokio::test]
async fn stops_the_chunk_at_an_ended_event() {
    let costs = FixtureCosts;
    let counter = FixtureCounter;
    let hooks = fixture_hooks();
    let services = AssemblyServices {
        costs: &costs,
        counter: &counter,
        logging: None,
        hooks: &hooks,
    };
    let mut converter = ScriptedConverter {
        seen: Vec::new(),
        malformed: None,
        ended: Some("END-HERE"),
        failed: None,
    };
    let chunks = [
        text_chunk(&format!(
            "{START_EVENT}\n\n{}\n\ndata: {{{{END-HERE}}}}\n\n{}",
            delta_event("a"),
            delta_event("b"),
        )),
        text_chunk(&delta_event("c")),
    ];
    let request = AssemblyRequest {
        chunks: &chunks,
        converter: &mut converter,
        now: FIXED_NOW,
    };
    let response = assemble_response_legacy(request, &services).await.unwrap();
    assert_eq!(text_of(response).0, "ac");
    assert_eq!(converter.seen.len(), 4);
    assert!(converter.seen[2].contains("END-HERE"));
    assert!(converter.seen[3].contains("\"c\""));
    assert!(!converter.seen.iter().any(|event| event.contains("\"b\"")));
}

#[tokio::test]
async fn returns_a_conversion_failure() {
    let costs = FixtureCosts;
    let counter = FixtureCounter;
    let hooks = fixture_hooks();
    let services = AssemblyServices {
        costs: &costs,
        counter: &counter,
        logging: None,
        hooks: &hooks,
    };
    let mut converter = ScriptedConverter {
        seen: Vec::new(),
        malformed: None,
        ended: None,
        failed: Some("BROKEN"),
    };
    let chunks = [text_chunk("data: {BROKEN}")];
    let request = AssemblyRequest {
        chunks: &chunks,
        converter: &mut converter,
        now: FIXED_NOW,
    };
    let result = assemble_response_legacy(request, &services).await;
    assert!(matches!(
        result,
        Err(ReconstructionError::Conversion { .. })
    ));
}

#[tokio::test]
async fn answers_none_without_a_convertible_event() {
    let costs = FixtureCosts;
    let counter = FixtureCounter;
    let hooks = fixture_hooks();
    let services = AssemblyServices {
        costs: &costs,
        counter: &counter,
        logging: None,
        hooks: &hooks,
    };
    let mut converter = ScriptedConverter::default();
    let chunks = [text_chunk("event: message_start")];
    let request = AssemblyRequest {
        chunks: &chunks,
        converter: &mut converter,
        now: FIXED_NOW,
    };
    let response = assemble_response_legacy(request, &services).await.unwrap();
    assert!(response.is_none());
}

#[tokio::test]
async fn refuses_a_chunk_that_is_not_utf8() {
    let costs = FixtureCosts;
    let counter = FixtureCounter;
    let hooks = fixture_hooks();
    let services = AssemblyServices {
        costs: &costs,
        counter: &counter,
        logging: None,
        hooks: &hooks,
    };
    let mut converter = ScriptedConverter::default();
    let chunks = [RawSseChunk::Bytes(vec![0xff])];
    let request = AssemblyRequest {
        chunks: &chunks,
        converter: &mut converter,
        now: FIXED_NOW,
    };
    let result = assemble_response_legacy(request, &services).await;
    assert!(matches!(result, Err(ReconstructionError::Event(_))));
}
