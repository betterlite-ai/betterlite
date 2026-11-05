#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::agents::json::AgentJson;
use rstest::rstest;

use super::{ProviderCall, ProviderError, PydanticAIProviderConfig};
use crate::pydantic_ai_handler::{HandlerError, PydanticAIHandler};
use crate::pydantic_ai_submit::tests::{Counter, ScriptedTransport, json};
use crate::pydantic_ai_transformation::send::PydanticAITransformation;

const SUBMITTED: &str = r#"{"result": {"id": "task-1", "status": {"state": "submitted"}}}"#;
const COMPLETED: &str = r#"{"result": {"id": "task-1", "status": {"state": "completed"}, "history": [
    {"role": "user", "messageId": "q", "parts": [{"kind": "text", "text": "q"}]},
    {"role": "agent", "messageId": "answer", "parts": [{"kind": "text", "text": "abcdefghij"}]}
]}}"#;
const AGENT: Option<&str> = Some("http://agent.test");

fn provider(bodies: &[&str]) -> PydanticAIProviderConfig<ScriptedTransport> {
    PydanticAIProviderConfig {
        handler: PydanticAIHandler {
            transformation: PydanticAITransformation {
                transport: ScriptedTransport::answering(bodies),
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
) -> ProviderCall<'call, AgentJson> {
    ProviderCall {
        request_id: "req-1",
        params,
        api_base,
        kwargs,
    }
}

#[tokio::test]
async fn pydantic_ai_provider_defaults_to_a_sixty_second_timeout() {
    let provider = provider(&[COMPLETED]);
    let (params, extra) = (json(r#"{"message": {"parts": []}}"#), kwargs("{}"));
    provider
        .handle_non_streaming(&call(&params, AGENT, &extra), &mut Counter(0))
        .await
        .unwrap();
    let recorded = provider.handler.transformation.transport.recorded();
    assert_eq!(recorded[0].timeout, Duration::from_secs(60));
    assert_eq!(recorded[0].headers.len(), 1);
}

#[tokio::test]
async fn pydantic_ai_provider_reads_timeout_and_extra_headers() {
    let provider = provider(&[SUBMITTED, COMPLETED]);
    let params = json(r#"{"message": {"parts": []}}"#);
    let extra = kwargs(r#"{"timeout": 2.5, "agent_extra_headers": {"X-Trace": "t-1"}}"#);
    provider
        .handle_non_streaming(&call(&params, AGENT, &extra), &mut Counter(0))
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
    let provider = provider(&[COMPLETED]);
    let params = json(r#"{"message": {"parts": []}}"#);
    let extra = kwargs(r#"{"timeout": null, "agent_extra_headers": null, "chunk_size": null}"#);
    provider
        .handle_non_streaming(&call(&params, AGENT, &extra), &mut Counter(0))
        .await
        .unwrap();
    let recorded = provider.handler.transformation.transport.recorded();
    assert_eq!(recorded[0].timeout, Duration::from_secs(60));
}

#[rstest]
#[case::timeout_text(r#"{"timeout": "slow"}"#)]
#[case::headers_list(r#"{"agent_extra_headers": ["a"]}"#)]
#[case::zero_chunk(r#"{"chunk_size": 0}"#)]
#[case::negative_delay(r#"{"delay_ms": -1}"#)]
#[tokio::test]
async fn pydantic_ai_provider_rejects_wrong_typed_kwargs(#[case] text: &str) {
    let provider = provider(&[COMPLETED]);
    let (params, extra) = (json("{}"), kwargs(text));
    let error = provider
        .handle_streaming(&call(&params, AGENT, &extra), &mut Counter(0))
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
async fn pydantic_ai_provider_chunks_the_stream_by_chunk_size_and_delay() {
    let provider = provider(&[COMPLETED]);
    let params = json(r#"{"message": {"parts": []}}"#);
    let extra = kwargs(r#"{"chunk_size": 4, "delay_ms": 7}"#);
    let steps = provider
        .handle_streaming(&call(&params, AGENT, &extra), &mut Counter(0))
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
    let provider = provider(&[]);
    let (params, extra) = (json("{}"), kwargs("{}"));
    let error = provider
        .handle_streaming(&call(&params, api_base, &extra), &mut Counter(0))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProviderError::Handler(HandlerError::ApiBaseRequired)
    ));
}

#[tokio::test]
async fn pydantic_ai_provider_non_streaming_requires_an_api_base() {
    let provider = provider(&[]);
    let (params, extra) = (json("{}"), kwargs("{}"));
    let error = provider
        .handle_non_streaming(&call(&params, None, &extra), &mut Counter(0))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProviderError::Handler(HandlerError::ApiBaseRequired)
    ));
}
