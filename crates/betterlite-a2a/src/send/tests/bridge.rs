use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::provider::a2a::JsonRpcId;

use crate::completion_bridge::dispatch::BridgeError;
use crate::completion_bridge::tests::Source;
use crate::completion_bridge::tests::support::{
    TestFailure, chunk, collect_events, parsed, provider, provider_calls, recorded_calls, recorder,
    selected_configs, summaries,
};
use crate::provider_selector::A2AProviderConfig;
use crate::send::asend_message;
use crate::send::asend_message_streaming;
use crate::send::call::{SendCall, StreamCall};
use crate::send::deps::{BridgeDeps, NativeDeps};
use crate::send::error::SendEntryError;

use super::support::{
    StreamFixture, counter, messages, params_with_provider, pool, recording_dispatch, request, tls,
};

#[tokio::test]
async fn bridge_passes_api_base_and_agent_headers_to_the_provider() {
    let counter = counter();
    let pool = pool();
    let tls = tls();
    let environment = BTreeMap::new();
    let ids = || "generated-trace".to_owned();
    let deps = NativeDeps {
        http: &pool,
        environment: &environment,
        tls: &tls,
        counter: &counter,
        trace_ids: &ids,
    };
    let completion = recorder();
    let providers = provider();
    let mut source = Source::default();
    let mut bridge = BridgeDeps {
        completion: &completion,
        providers: &providers,
        source: &mut source,
    };
    let agent_headers = BTreeMap::from([("x-team".to_owned(), "blue".to_owned())]);
    let params = params_with_provider(Some(AgentJson::String("langflow".to_owned())));
    let call = SendCall {
        a2a_client: None,
        request: request("hello world"),
        api_base: Some("https://agent.example".to_owned()),
        litellm_params: params.clone(),
        agent_id: None,
        agent_extra_headers: Some(agent_headers.clone()),
    };

    let result = asend_message(call, None, &deps, &mut bridge)
        .await
        .expect("the provider answers");

    assert_eq!(
        selected_configs(&providers),
        vec![A2AProviderConfig::LangFlow]
    );
    let calls = provider_calls(&providers);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].api_base.as_deref(), Some("https://agent.example"));
    assert_eq!(calls[0].agent_extra_headers, Some(agent_headers));
    assert_eq!(calls[0].litellm_params, params);
    assert!(recorded_calls(&completion).is_empty());
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
async fn stream_routing_uses_the_completion_bridge_for_a_non_empty_provider() {
    let mut fixture = StreamFixture::new();
    let mut completion = recorder();
    completion.chunks = Ok(vec![Ok(chunk("a")), Ok(chunk("b"))]);
    let providers = provider();
    let mut source = Source::default();
    let mut bridge = BridgeDeps {
        completion: &completion,
        providers: &providers,
        source: &mut source,
    };
    let (dispatch, _calls) = recording_dispatch();
    let call = StreamCall {
        a2a_client: None,
        request: request("hello world"),
        api_base: None,
        litellm_params: params_with_provider(Some(AgentJson::String("langgraph".to_owned()))),
        agent_id: None,
        metadata: None,
        proxy_server_request: None,
        agent_extra_headers: None,
    };

    let deps = fixture.deps(dispatch);
    let events = asend_message_streaming(call, None, deps, &mut bridge)
        .await
        .expect("the stream starts");
    let collected = collect_events(events).await;

    assert_eq!(
        summaries(&collected),
        vec![
            "task:submitted",
            "status:working:false",
            "artifact:ab",
            "status:completed:true",
        ]
    );
    for event in &collected {
        assert_eq!(
            event.as_ref().expect("the event is forwarded")["id"],
            parsed(r#""request-1""#)
        );
    }
    let calls = recorded_calls(&completion);
    assert_eq!(calls.len(), 1);
    assert!(calls[0].stream);
    assert_eq!(calls[0].model, "langgraph/agent");
    assert_eq!(calls[0].messages, messages("hello world"));
    assert!(selected_configs(&providers).is_empty());
    assert!(fixture.issued().is_empty());
}

#[tokio::test]
async fn stream_routing_selects_a_registered_provider() {
    let mut fixture = StreamFixture::new();
    let completion = recorder();
    let mut providers = provider();
    let event = parsed(
        r#"{"jsonrpc":"2.0","id":"provider","result":{"kind":"status-update","status":{"state":"working"},"final":false,"taskId":"t-1","contextId":"c-1"}}"#,
    );
    providers.events = Ok(vec![Ok(event.clone())]);
    let mut source = Source::default();
    let mut bridge = BridgeDeps {
        completion: &completion,
        providers: &providers,
        source: &mut source,
    };
    let (dispatch, _calls) = recording_dispatch();
    let agent_headers = BTreeMap::from([("x-team".to_owned(), "blue".to_owned())]);
    let call = StreamCall {
        a2a_client: None,
        request: request("hello"),
        api_base: Some("https://langflow.example".to_owned()),
        litellm_params: params_with_provider(Some(AgentJson::String(
            "pydantic_ai_agents".to_owned(),
        ))),
        agent_id: None,
        metadata: None,
        proxy_server_request: None,
        agent_extra_headers: Some(agent_headers.clone()),
    };

    let deps = fixture.deps(dispatch);
    let events = asend_message_streaming(call, None, deps, &mut bridge)
        .await
        .expect("the stream starts");
    let collected = collect_events(events).await;

    assert_eq!(
        selected_configs(&providers),
        vec![A2AProviderConfig::PydanticAI]
    );
    let calls = provider_calls(&providers);
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].api_base.as_deref(),
        Some("https://langflow.example")
    );
    assert_eq!(calls[0].agent_extra_headers, Some(agent_headers));
    assert_eq!(collected.len(), 1);
    assert_eq!(
        collected[0]
            .as_ref()
            .expect("the provider event is forwarded"),
        &event
    );
    assert_eq!(summaries(&collected), vec!["status:working:false"]);
    assert!(recorded_calls(&completion).is_empty());
}

#[tokio::test]
async fn stream_routing_reports_a_provider_failure() {
    let mut fixture = StreamFixture::new();
    let completion = recorder();
    let mut providers = provider();
    providers.events = Err(TestFailure("boom"));
    let mut source = Source::default();
    let mut bridge = BridgeDeps {
        completion: &completion,
        providers: &providers,
        source: &mut source,
    };
    let (dispatch, _calls) = recording_dispatch();
    let call = StreamCall {
        a2a_client: None,
        request: request("hello"),
        api_base: None,
        litellm_params: params_with_provider(Some(AgentJson::String("langflow".to_owned()))),
        agent_id: None,
        metadata: None,
        proxy_server_request: None,
        agent_extra_headers: None,
    };

    let deps = fixture.deps(dispatch);
    let events = asend_message_streaming(call, None, deps, &mut bridge)
        .await
        .expect("the stream starts");
    let collected = collect_events(events).await;

    assert_eq!(collected.len(), 1);
    assert!(matches!(
        &collected[0],
        Err(SendEntryError::Bridge(BridgeError::Provider(TestFailure(
            "boom"
        ))))
    ));
}
