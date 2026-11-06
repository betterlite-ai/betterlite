#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::agents::json::AgentJson;
use rstest::rstest;

use super::{ProviderCall, ProviderError, PydanticAIProviderConfig};
use crate::pydantic_ai_handler::{HandlerError, PydanticAIHandler};
use crate::pydantic_ai_submit::tests::{Counter, ScriptedTransport, json};
use crate::pydantic_ai_transformation::send::{
    ProjectedMessage, PydanticAITransformation, ReplayedStream,
};

const SUBMITTED: &str = r#"{"result": {"id": "task-1", "status": {"state": "submitted"}}}"#;
const COMPLETED: &str = r#"{"result": {"id": "task-1", "status": {"state": "completed"}, "history": [
    {"role": "user", "messageId": "q", "parts": [{"kind": "text", "text": "q"}]},
    {"role": "agent", "messageId": "answer", "parts": [{"kind": "text", "text": "abcdefghij"}]}
]}}"#;
const AGENT: Option<&str> = Some("http://agent.test");

fn provider(bodies: &[&str]) -> PydanticAIProviderConfig<ScriptedTransport, Counter> {
    PydanticAIProviderConfig {
        handler: PydanticAIHandler {
            transformation: PydanticAITransformation {
                transport: ScriptedTransport::answering(bodies),
                source: Counter(0),
            },
        },
    }
}

fn kwargs(text: &str) -> BTreeMap<String, AgentJson> {
    serde_json::from_str(text).unwrap()
}

fn call<'call>(
    params: &'call AgentJson,
    api_base: Option<&'call str>,
    kwargs: &'call BTreeMap<String, AgentJson>,
) -> ProviderCall<'call> {
    ProviderCall {
        request_id: "req-1",
        params,
        api_base,
        kwargs,
    }
}

#[tokio::test]
async fn pydantic_ai_provider_defaults_to_a_sixty_second_timeout() {
    let mut provider = provider(&[COMPLETED]);
    let (params, extra) = (json(r#"{"message": {"parts": []}}"#), kwargs("{}"));
    provider
        .handle::<ProjectedMessage>(&call(&params, AGENT, &extra))
        .await
        .unwrap();
    let recorded = provider.handler.transformation.transport.recorded();
    assert_eq!(recorded[0].timeout, Duration::from_secs(60));
    assert_eq!(recorded[0].headers.len(), 1);
}

#[tokio::test]
async fn pydantic_ai_provider_reads_timeout_and_extra_headers() {
    let mut provider = provider(&[SUBMITTED, COMPLETED]);
    let params = json(r#"{"message": {"parts": []}}"#);
    let extra = kwargs(r#"{"timeout": 2.5, "agent_extra_headers": {"X-Trace": "t-1"}}"#);
    provider
        .handle::<ProjectedMessage>(&call(&params, AGENT, &extra))
        .await
        .unwrap();
    let recorded = provider.handler.transformation.transport.recorded();
    assert_eq!(recorded.len(), 2);
    let sent: Vec<_> = recorded
        .iter()
        .map(|request| (request.timeout, request.headers["X-Trace"].as_str()))
        .collect();
    assert_eq!(sent, [(Duration::from_millis(2500), "t-1"); 2]);
}

#[tokio::test]
async fn pydantic_ai_provider_treats_null_kwargs_as_absent() {
    let mut provider = provider(&[COMPLETED]);
    let params = json(r#"{"message": {"parts": []}}"#);
    let extra = kwargs(r#"{"timeout": null, "agent_extra_headers": null, "chunk_size": null}"#);
    provider
        .handle::<ProjectedMessage>(&call(&params, AGENT, &extra))
        .await
        .unwrap();
    let recorded = provider.handler.transformation.transport.recorded();
    assert_eq!(recorded[0].timeout, Duration::from_secs(60));
}

#[rstest]
#[case::timeout_text(r#"{"timeout": "slow"}"#)]
#[case::headers_list(r#"{"agent_extra_headers": ["a"]}"#)]
#[case::delay_text(r#"{"delay_ms": "slow"}"#)]
#[tokio::test]
async fn pydantic_ai_provider_rejects_wrong_typed_kwargs(#[case] text: &str) {
    let mut provider = provider(&[COMPLETED]);
    let (params, extra) = (json("{}"), kwargs(text));
    let error = provider
        .handle::<ReplayedStream>(&call(&params, AGENT, &extra))
        .await
        .unwrap_err();
    assert!(matches!(error, ProviderError::Kwargs(_)));
    assert!(
        provider
            .handler
            .transformation
            .transport
            .recorded()
            .is_empty()
    );
}

#[rstest]
#[case::zero(0)]
#[case::negative(-1)]
#[tokio::test]
async fn pydantic_ai_provider_rejects_a_non_positive_chunk_size(#[case] chunk_size: i64) {
    let mut provider = provider(&[COMPLETED]);
    let (params, extra) = (
        json("{}"),
        BTreeMap::from([("chunk_size".to_owned(), AgentJson::from(chunk_size))]),
    );
    let error = provider
        .handle::<ReplayedStream>(&call(&params, AGENT, &extra))
        .await
        .unwrap_err();
    assert!(matches!(error, ProviderError::Kwargs(_)));
    assert!(
        provider
            .handler
            .transformation
            .transport
            .recorded()
            .is_empty()
    );
}

#[tokio::test]
async fn pydantic_ai_provider_non_streaming_ignores_the_pacing_kwargs() {
    let mut provider = provider(&[COMPLETED]);
    let params = json(r#"{"message": {"parts": []}}"#);
    let extra = kwargs(r#"{"chunk_size": 0, "delay_ms": -1}"#);
    provider
        .handle::<ProjectedMessage>(&call(&params, AGENT, &extra))
        .await
        .unwrap();
}

#[tokio::test]
async fn pydantic_ai_provider_streaming_treats_a_negative_delay_as_no_delay() {
    let mut provider = provider(&[COMPLETED]);
    let params = json(r#"{"message": {"parts": []}}"#);
    let extra = kwargs(r#"{"delay_ms": -5}"#);
    let steps = provider
        .handle::<ReplayedStream>(&call(&params, AGENT, &extra))
        .await
        .unwrap();
    assert!(steps.iter().all(|step| step.delay_after == Duration::ZERO));
}

#[tokio::test]
async fn pydantic_ai_provider_chunks_the_stream_by_chunk_size_and_delay() {
    let mut provider = provider(&[COMPLETED]);
    let params = json(r#"{"message": {"parts": []}}"#);
    let extra = kwargs(r#"{"chunk_size": 4, "delay_ms": 7}"#);
    let steps = provider
        .handle::<ReplayedStream>(&call(&params, AGENT, &extra))
        .await
        .unwrap();
    assert!(
        steps
            .iter()
            .any(|step| step.delay_after == Duration::from_millis(7))
    );
    let events: Vec<_> = steps.iter().map(|step| &step.event).collect();
    let text = serde_json::to_string(&events).unwrap();
    assert!(
        ["abcd", "efgh", "ij"]
            .iter()
            .all(|chunk| text.contains(&format!(r#""text":"{chunk}""#)))
    );
}

#[rstest]
#[case::missing(None)]
#[case::empty(Some(""))]
#[tokio::test]
async fn pydantic_ai_provider_streaming_needs_a_non_empty_api_base(#[case] api_base: Option<&str>) {
    let mut provider = provider(&[]);
    let (params, extra) = (json("{}"), kwargs("{}"));
    let error = provider
        .handle::<ReplayedStream>(&call(&params, api_base, &extra))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProviderError::Handler(HandlerError::ApiBaseRequired)
    ));
}

#[tokio::test]
async fn pydantic_ai_provider_non_streaming_requires_an_api_base() {
    let mut provider = provider(&[]);
    let (params, extra) = (json("{}"), kwargs("{}"));
    let error = provider
        .handle::<ProjectedMessage>(&call(&params, None, &extra))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProviderError::Handler(HandlerError::ApiBaseRequired)
    ));
}
