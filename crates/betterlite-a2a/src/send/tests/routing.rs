use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::provider::a2a::JsonRpcId;
use rstest::rstest;

use crate::completion_bridge::tests::support::{model_response, recorded_calls};
use crate::send::error::{SendCallError, SendEntryError};

use super::support::{
    Reply, SendFixture, buffered_message, messages, params_with_provider, posted, send_call,
    with_agent,
};

#[tokio::test]
async fn send_routing_takes_the_completion_bridge_for_a_non_empty_provider() {
    let mut fixture = SendFixture::new();
    fixture.completion.answer = Ok(model_response("bridge answer"));
    let call = send_call(
        "hello world",
        None,
        params_with_provider(Some(AgentJson::String("langgraph".to_owned()))),
    );
    let result = fixture.send(call, None).await.expect("the bridge answers");

    let calls = recorded_calls(&fixture.completion);
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
    let (outcome, seen) = Box::pin(with_agent(
        Reply::Json(buffered_message("hi")),
        |base| async move {
            let mut fixture = SendFixture::new();
            let call = send_call("hello", Some(base), params_with_provider(provider_value));
            let result = fixture.send(call, None).await;
            (result, recorded_calls(&fixture.completion))
        },
    ))
    .await;

    outcome.0.expect("the native send answers");
    assert!(outcome.1.is_empty());
    assert!(posted(&seen).body.contains("\"method\":\"SendMessage\""));
}

#[tokio::test]
async fn send_routing_rejects_a_non_text_provider() {
    let mut fixture = SendFixture::new();
    let call = send_call(
        "hello",
        None,
        params_with_provider(Some(AgentJson::Bool(true))),
    );

    let result = fixture.send(call, None).await;

    assert!(matches!(
        result,
        Err(SendEntryError::Call(SendCallError::NotText {
            field: "custom_llm_provider"
        }))
    ));
    assert!(recorded_calls(&fixture.completion).is_empty());
}
