#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_models::decisions::Json;
use futures_util::StreamExt as _;
use indexmap::IndexMap;
use rstest::rstest;

use crate::passthrough_stream_lifecycle::chunk_processor::chunk_processor;
use crate::passthrough_stream_lifecycle::cost_injection::{
    StreamCostInjection, cost_injection_active,
};
use crate::passthrough_stream_lifecycle::support::{
    Harness, json, object, ok_chunks, processor_call,
};
use crate::passthrough_stream_logging::object::StreamLoggingObject;
use crate::passthrough_stream_logging::types::EndpointType;

const VERTEX_URL: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/p/locations/l/publishers/google/models/gemini:streamRawPredict";

#[test]
fn the_flag_needs_the_global_switch() {
    assert!(!cost_injection_active(
        false,
        Some("gpt-4o"),
        EndpointType::OpenAi,
        "/v1/chat/completions"
    ));
    assert!(cost_injection_active(
        true,
        Some("gpt-4o"),
        EndpointType::OpenAi,
        "/v1/chat/completions"
    ));
}

#[test]
fn the_flag_needs_a_model_name() {
    assert!(!cost_injection_active(
        true,
        None,
        EndpointType::Anthropic,
        "/v1/messages"
    ));
    assert!(!cost_injection_active(
        true,
        Some(""),
        EndpointType::Anthropic,
        "/v1/messages"
    ));
    assert!(cost_injection_active(
        true,
        Some("claude-3"),
        EndpointType::Anthropic,
        "/v1/messages"
    ));
}

#[test]
fn anthropic_and_openai_allow_injection() {
    assert!(cost_injection_active(
        true,
        Some("claude-3"),
        EndpointType::Anthropic,
        "/v1/messages"
    ));
    assert!(cost_injection_active(
        true,
        Some("gpt-4o"),
        EndpointType::OpenAi,
        "/v1/chat/completions"
    ));
}

#[test]
fn vertex_allows_only_the_raw_predict_routes() {
    assert!(cost_injection_active(
        true,
        Some("gemini-1.5-pro"),
        EndpointType::VertexAi,
        VERTEX_URL
    ));
    assert!(cost_injection_active(
        true,
        Some("gemini-1.5-pro"),
        EndpointType::VertexAi,
        "https://us-central1-aiplatform.googleapis.com/v1/projects/p/locations/l/publishers/google/models/gemini:rawPredict"
    ));
    assert!(!cost_injection_active(
        true,
        Some("gemini-1.5-pro"),
        EndpointType::VertexAi,
        "https://us-central1-aiplatform.googleapis.com/v1/projects/p/locations/l/publishers/google/models/gemini:streamGenerateContent"
    ));
}

#[rstest]
#[case(EndpointType::Gemini)]
#[case(EndpointType::Tinyfish)]
#[case(EndpointType::Generic)]
#[case(EndpointType::Decisions)]
fn other_endpoints_never_allow_injection(#[case] endpoint_type: EndpointType) {
    assert!(!cost_injection_active(
        true,
        Some("gpt-4o"),
        endpoint_type,
        VERTEX_URL
    ));
}

#[tokio::test]
async fn an_unswitched_stream_keeps_the_hot_path() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let mut call = processor_call(
        Arc::clone(&lifecycle),
        object(&[], IndexMap::new()),
        EndpointType::OpenAi,
    );
    call.request_body = Some(json("{\"model\":\"gpt-4o\"}"));
    let chunks = vec![b"data: {}\n\n".to_vec()];
    let collected: Vec<_> = chunk_processor(call, ok_chunks(chunks.clone()))
        .collect()
        .await;
    let yielded: Vec<Vec<u8>> = collected.into_iter().map(|item| item.unwrap()).collect();
    assert_eq!(yielded, chunks);
    assert!(harness.cost_injection.calls().is_empty());
}

#[tokio::test]
async fn the_switch_turns_injection_on_for_a_known_model() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(true);
    let mut call = processor_call(
        Arc::clone(&lifecycle),
        object(&[], IndexMap::new()),
        EndpointType::OpenAi,
    );
    call.request_body = Some(json("{\"model\":\"gpt-4o\"}"));
    drop(
        chunk_processor(call, ok_chunks(vec![b"data: {}\n\n".to_vec()]))
            .collect::<Vec<_>>()
            .await,
    );
    assert_eq!(
        harness.cost_injection.calls(),
        vec![(b"data: {}\n\n".to_vec(), "gpt-4o".to_owned())]
    );
}

#[tokio::test]
async fn a_missing_model_keeps_the_hot_path() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(true);
    let call = processor_call(
        Arc::clone(&lifecycle),
        object(&[("model", Json::Null)], IndexMap::new()),
        EndpointType::OpenAi,
    );
    drop(
        chunk_processor(call, ok_chunks(vec![b"data: {}\n\n".to_vec()]))
            .collect::<Vec<_>>()
            .await,
    );
    assert!(harness.cost_injection.calls().is_empty());
}

struct ClearingInjection {
    calls: Mutex<usize>,
}

impl StreamCostInjection for ClearingInjection {
    fn inject(
        &self,
        frames: &[u8],
        _model_name: &str,
        logging_obj: &StreamLoggingObject,
    ) -> Vec<u8> {
        let mut calls = self.calls.lock().unwrap();
        *calls = calls.saturating_add(1);
        drop(calls);
        logging_obj.edit(&mut |state| {
            state.model_call_details.shift_remove("model");
        });
        frames.to_vec()
    }
}

#[tokio::test]
async fn the_flag_is_resolved_once_per_stream() {
    let injection = Arc::new(ClearingInjection {
        calls: Mutex::new(0),
    });
    let harness =
        Harness::new(&["2024-01-01T00:00:01+00:00"])
            .with_injection(Arc::<ClearingInjection>::clone(&injection));
    let lifecycle = harness.lifecycle(true);
    let logging_obj = object(
        &[("model", Json::Text("gpt-4o".to_owned()))],
        IndexMap::new(),
    );
    let call = processor_call(Arc::clone(&lifecycle), logging_obj, EndpointType::OpenAi);
    let chunks = vec![b"data: {}\n\n".to_vec(), b"data: {}\n\n".to_vec()];
    let collected: Vec<_> = chunk_processor(call, ok_chunks(chunks)).collect().await;
    assert_eq!(collected.len(), 2);
    assert_eq!(*injection.calls.lock().unwrap(), 2);
}
