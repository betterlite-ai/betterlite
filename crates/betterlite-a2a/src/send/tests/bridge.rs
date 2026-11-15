use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::provider::a2a::JsonRpcId;

use crate::completion_bridge::dispatch::{BridgeError, CompletionError};
use crate::completion_bridge::params::ParamsError;
use crate::completion_bridge::tests::support::{
    TestFailure, chunk, parsed, provider_calls, recorded_calls, selected_configs, summaries,
};
use crate::provider_selector::A2AProviderConfig;
use crate::send::error::SendEntryError;

use super::support::{SendFixture, messages, params_with_provider, send_call, stream_call};

#[tokio::test]
async fn bridge_passes_api_base_and_agent_headers_to_the_provider() {
    let mut fixture = SendFixture::new();
    let agent_headers = BTreeMap::from([("x-team".to_owned(), "blue".to_owned())]);
    let params = params_with_provider(Some(AgentJson::String("langflow".to_owned())));
    let mut call = send_call(
        "hello world",
        Some("https://agent.example".to_owned()),
        params.clone(),
    );
    call.agent_extra_headers = Some(agent_headers.clone());

    let result = fixture
        .send(call, None)
        .await
        .expect("the provider answers");

    assert_eq!(
        selected_configs(&fixture.providers),
        vec![A2AProviderConfig::LangFlow]
    );
    let calls = provider_calls(&fixture.providers);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].api_base.as_deref(), Some("https://agent.example"));
    assert_eq!(calls[0].agent_extra_headers, Some(agent_headers));
    assert_eq!(calls[0].litellm_params, params);
    assert!(recorded_calls(&fixture.completion).is_empty());
    assert_eq!(result.id, Some(JsonRpcId::String("provider".to_owned())));
    let answer = result.result.expect("the provider result is a map");
    let text = answer
        .get("parts")
        .and_then(AgentJson::as_array)
        .and_then(|parts| parts.first())
        .and_then(|part| part.get("text"))
        .and_then(AgentJson::as_str);
    assert_eq!(text, Some("from the provider"));
}

#[tokio::test]
async fn send_routing_reports_a_completion_failure() {
    let mut fixture = SendFixture::new();
    fixture.completion.answer = Err(TestFailure("boom"));
    let call = send_call(
        "hello",
        None,
        params_with_provider(Some(AgentJson::String("langgraph".to_owned()))),
    );

    let result = fixture.send(call, None).await;

    assert_eq!(recorded_calls(&fixture.completion).len(), 1);
    assert!(selected_configs(&fixture.providers).is_empty());
    assert!(matches!(
        result,
        Err(SendEntryError::Bridge(BridgeError::Completion(
            CompletionError::Completion(TestFailure("boom"))
        )))
    ));
}

#[tokio::test]
async fn stream_routing_uses_the_completion_bridge_for_a_non_empty_provider() {
    let mut fixture = SendFixture::new();
    fixture.completion.chunks = Ok(vec![Ok(chunk("a")), Ok(chunk("b"))]);
    let call = stream_call(send_call(
        "hello world",
        None,
        params_with_provider(Some(AgentJson::String("langgraph".to_owned()))),
    ));

    let outcome = Box::pin(fixture.stream(call, None)).await;

    assert_eq!(
        summaries(&outcome.events),
        vec![
            "task:submitted",
            "status:working:false",
            "artifact:ab",
            "status:completed:true",
        ]
    );
    assert!(
        outcome.events.iter().all(
            |event| event.as_ref().expect("the event is forwarded")["id"]
                == parsed(r#""request-1""#)
        ),
        "every streamed event carries the request id"
    );
    let calls = recorded_calls(&fixture.completion);
    assert_eq!(calls.len(), 1);
    assert!(calls[0].stream);
    assert_eq!(calls[0].model, "langgraph/agent");
    assert_eq!(calls[0].messages, messages("hello world"));
    assert!(selected_configs(&fixture.providers).is_empty());
    assert!(outcome.issued.is_empty());
}

#[tokio::test]
async fn stream_routing_selects_a_registered_provider() {
    let mut fixture = SendFixture::new();
    let event = parsed(
        r#"{"jsonrpc":"2.0","id":"provider","result":{"kind":"status-update","status":{"state":"working"},"final":false,"taskId":"t-1","contextId":"c-1"}}"#,
    );
    fixture.providers.events = Ok(vec![Ok(event.clone())]);
    let agent_headers = BTreeMap::from([("x-team".to_owned(), "blue".to_owned())]);
    let mut call = stream_call(send_call(
        "hello",
        Some("https://langflow.example".to_owned()),
        params_with_provider(Some(AgentJson::String("pydantic_ai_agents".to_owned()))),
    ));
    call.agent_extra_headers = Some(agent_headers.clone());

    let outcome = Box::pin(fixture.stream(call, None)).await;

    assert_eq!(
        selected_configs(&fixture.providers),
        vec![A2AProviderConfig::PydanticAI]
    );
    let calls = provider_calls(&fixture.providers);
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].api_base.as_deref(),
        Some("https://langflow.example")
    );
    assert_eq!(calls[0].agent_extra_headers, Some(agent_headers));
    assert_eq!(outcome.events.len(), 1);
    assert_eq!(
        outcome.events[0]
            .as_ref()
            .expect("the provider event is forwarded"),
        &event
    );
    assert_eq!(summaries(&outcome.events), vec!["status:working:false"]);
    assert!(recorded_calls(&fixture.completion).is_empty());
}

#[tokio::test]
async fn stream_routing_selects_a_bedrock_agentcore_provider() {
    let mut fixture = SendFixture::new();
    let mut params = params_with_provider(Some(AgentJson::String("bedrock".to_owned())));
    params.insert(
        "model".to_owned(),
        AgentJson::String("agentcore-runtime".to_owned()),
    );
    let call = stream_call(send_call("hello", None, params));

    let outcome = Box::pin(fixture.stream(call, None)).await;

    assert_eq!(
        selected_configs(&fixture.providers),
        vec![A2AProviderConfig::BedrockAgentCore]
    );
    assert!(recorded_calls(&fixture.completion).is_empty());
    assert!(outcome.events.is_empty());
}

#[tokio::test]
async fn stream_routing_reports_a_params_failure() {
    let mut fixture = SendFixture::new();
    let mut params = params_with_provider(Some(AgentJson::String("langgraph".to_owned())));
    params.insert("model".to_owned(), AgentJson::Bool(true));
    let call = stream_call(send_call("hello", None, params));

    let outcome = Box::pin(fixture.stream(call, None)).await;

    assert_eq!(outcome.events.len(), 1);
    assert!(matches!(
        &outcome.events[0],
        Err(SendEntryError::Bridge(BridgeError::Completion(
            CompletionError::Params(ParamsError::NotText { field: "model" })
        )))
    ));
    assert!(recorded_calls(&fixture.completion).is_empty());
    assert!(selected_configs(&fixture.providers).is_empty());
}

#[tokio::test]
async fn stream_routing_reports_a_provider_failure() {
    let mut fixture = SendFixture::new();
    fixture.providers.events = Err(TestFailure("boom"));
    let call = stream_call(send_call(
        "hello",
        None,
        params_with_provider(Some(AgentJson::String("langflow".to_owned()))),
    ));

    let outcome = Box::pin(fixture.stream(call, None)).await;

    assert_eq!(outcome.events.len(), 1);
    assert!(matches!(
        &outcome.events[0],
        Err(SendEntryError::Bridge(BridgeError::Provider(TestFailure(
            "boom"
        ))))
    ));
}
