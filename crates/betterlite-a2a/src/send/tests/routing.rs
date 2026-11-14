use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::provider::a2a::JsonRpcId;
use rstest::rstest;

use crate::completion_bridge::tests::Source;
use crate::completion_bridge::tests::support::{
    model_response, provider, recorded_calls, recorder,
};
use crate::send::asend_message;
use crate::send::call::SendCall;
use crate::send::deps::{BridgeDeps, NativeDeps};
use crate::send::error::{SendCallError, SendEntryError};

use super::support::{
    Reply, buffered_message, counter, messages, params_with_provider, pool, posted, request, tls,
    with_agent,
};

#[tokio::test]
async fn send_routing_takes_the_completion_bridge_for_a_non_empty_provider() {
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
    let mut completion = recorder();
    completion.answer = Ok(model_response("bridge answer"));
    let providers = provider();
    let mut source = Source::default();
    let mut bridge = BridgeDeps {
        completion: &completion,
        providers: &providers,
        source: &mut source,
    };
    let call = SendCall {
        a2a_client: None,
        request: request("hello world"),
        api_base: None,
        litellm_params: params_with_provider(Some(AgentJson::String("langgraph".to_owned()))),
        agent_id: None,
        agent_extra_headers: None,
    };

    let result = asend_message(call, None, &deps, &mut bridge)
        .await
        .expect("the bridge answers");

    let calls = recorded_calls(&completion);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "langgraph/agent");
    assert!(!calls[0].stream);
    assert_eq!(calls[0].messages, messages("hello world"));
    assert_eq!(result.id, Some(JsonRpcId::String("request-1".to_owned())));
    let answer = result.result.expect("the bridge result is a map");
    let parts = answer
        .get("parts")
        .and_then(AgentJson::as_array)
        .expect("the bridge answer carries parts");
    assert_eq!(
        parts[0].get("text").and_then(AgentJson::as_str),
        Some("bridge answer")
    );
}

#[rstest]
#[case::absent(None)]
#[case::null(Some(AgentJson::Null))]
#[case::empty(Some(AgentJson::String(String::new())))]
#[tokio::test]
async fn send_routing_uses_native_for_an_absent_null_or_empty_provider(
    #[case] provider_value: Option<AgentJson>,
) {
    let (outcome, seen) = with_agent(Reply::Json(buffered_message("hi")), |base| async move {
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
        let call = SendCall {
            a2a_client: None,
            request: request("hello"),
            api_base: Some(base),
            litellm_params: params_with_provider(provider_value),
            agent_id: None,
            agent_extra_headers: None,
        };
        let result = asend_message(call, None, &deps, &mut bridge).await;
        (result, recorded_calls(&completion))
    })
    .await;

    assert!(outcome.0.is_ok());
    assert!(outcome.1.is_empty());
    assert!(posted(&seen).body.contains("\"method\":\"SendMessage\""));
}

#[tokio::test]
async fn send_routing_rejects_a_non_text_provider() {
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
    let call = SendCall {
        a2a_client: None,
        request: request("hello"),
        api_base: None,
        litellm_params: params_with_provider(Some(AgentJson::Bool(true))),
        agent_id: None,
        agent_extra_headers: None,
    };

    let result = asend_message(call, None, &deps, &mut bridge).await;

    assert!(matches!(
        result,
        Err(SendEntryError::Call(SendCallError::NotText {
            field: "custom_llm_provider"
        }))
    ));
    assert!(recorded_calls(&completion).is_empty());
}
