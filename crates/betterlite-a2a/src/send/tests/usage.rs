use std::collections::BTreeMap;

use betterlite_core::agents::provider::a2a::JsonRpcId;

use crate::completion_bridge::tests::Source;
use crate::completion_bridge::tests::support::{provider, recorder};
use crate::send::asend_message;
use crate::send::call::SendCall;
use crate::send::deps::{BridgeDeps, NativeDeps};

use super::support::{
    Reply, buffered_message, counter, json, logging_state, pool, pricing_params, request, tls,
    with_agent,
};

#[tokio::test]
async fn send_usage_sets_the_token_counts_and_pricing_params() {
    let (outcome, _seen) = with_agent(Reply::Json(buffered_message("hi")), |base| async move {
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
            request: request("hello world"),
            api_base: Some(base),
            litellm_params: pricing_params(),
            agent_id: Some("agent-7".to_owned()),
            agent_extra_headers: None,
        };
        let mut logging = logging_state(Some("logging-trace"));
        let result = asend_message(call, Some(&mut logging), &deps, &mut bridge).await;
        (result, logging)
    })
    .await;

    let result = outcome.0.expect("the native send answers");
    assert_eq!(result.id, Some(JsonRpcId::String("rpc-1".to_owned())));
    let logging = outcome.1;
    assert_eq!(
        logging.model_call_details.get("usage"),
        Some(&json(
            r#"{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}"#
        ))
    );
    assert_eq!(
        logging.model_call_details.get("agent_id"),
        Some(&json(r#""agent-7""#))
    );
    assert_eq!(
        logging.model_call_details.get("litellm_params"),
        Some(&json(
            r#"{"cost_per_query":3,"input_cost_per_token":0.0001}"#
        ))
    );
    assert_eq!(logging.model, "a2a_agent/fake-agent");
    assert_eq!(logging.custom_llm_provider.as_deref(), Some("a2a_agent"));
}

#[tokio::test]
async fn send_usage_merges_pricing_into_existing_logging_params() {
    let (outcome, _seen) = with_agent(Reply::Json(buffered_message("hi")), |base| async move {
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
            litellm_params: pricing_params(),
            agent_id: None,
            agent_extra_headers: None,
        };
        let mut logging = logging_state(Some("logging-trace"));
        logging
            .model_call_details
            .insert("litellm_params".to_owned(), json(r#"{"agent_id":"kept"}"#));
        let result = asend_message(call, Some(&mut logging), &deps, &mut bridge).await;
        (result, logging)
    })
    .await;

    assert!(outcome.0.is_ok());
    let logging = outcome.1;
    assert_eq!(
        logging.model_call_details.get("litellm_params"),
        Some(&json(
            r#"{"agent_id":"kept","cost_per_query":3,"input_cost_per_token":0.0001}"#
        ))
    );
    assert_eq!(logging.model_call_details.get("agent_id"), None);
}

#[tokio::test]
async fn send_usage_without_a_logging_object_still_answers() {
    let (outcome, _seen) = with_agent(Reply::Json(buffered_message("hi")), |base| async move {
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
            litellm_params: pricing_params(),
            agent_id: Some("agent-7".to_owned()),
            agent_extra_headers: None,
        };
        asend_message(call, None, &deps, &mut bridge).await
    })
    .await;

    let result = outcome.expect("the send answers");
    assert_eq!(result.id, Some(JsonRpcId::String("rpc-1".to_owned())));
}
