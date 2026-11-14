use betterlite_core::agents::provider::a2a::JsonRpcId;

use super::support::{
    Reply, SendFixture, buffered_message, json, logging_state, pricing_params, send_call,
    with_agent,
};

#[tokio::test]
async fn send_usage_sets_the_token_counts_and_pricing_params() {
    let (outcome, _seen) = Box::pin(with_agent(
        Reply::Json(buffered_message("hi")),
        |base| async move {
            let mut fixture = SendFixture::new();
            let mut call = send_call("hello world", Some(base), pricing_params());
            call.agent_id = Some("agent-7".to_owned());
            let mut logging = logging_state(Some("logging-trace"));
            let result = fixture.send(call, Some(&mut logging)).await;
            (result, logging)
        },
    ))
    .await;

    let result = outcome.0.expect("the native send answers");
    assert_eq!(result.id, Some(JsonRpcId::String("request-1".to_owned())));
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
    let (outcome, _seen) = Box::pin(with_agent(
        Reply::Json(buffered_message("hi")),
        |base| async move {
            let mut fixture = SendFixture::new();
            let call = send_call("hello", Some(base), pricing_params());
            let mut logging = logging_state(Some("logging-trace"));
            logging
                .model_call_details
                .insert("litellm_params".to_owned(), json(r#"{"agent_id":"kept"}"#));
            let result = fixture.send(call, Some(&mut logging)).await;
            (result, logging)
        },
    ))
    .await;

    outcome.0.expect("the send answers");
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
    let (outcome, _seen) = Box::pin(with_agent(
        Reply::Json(buffered_message("hi")),
        |base| async move {
            let mut fixture = SendFixture::new();
            let mut call = send_call("hello", Some(base), pricing_params());
            call.agent_id = Some("agent-7".to_owned());
            fixture.send(call, None).await
        },
    ))
    .await;

    let result = outcome.expect("the send answers");
    assert_eq!(result.id, Some(JsonRpcId::String("request-1".to_owned())));
}
