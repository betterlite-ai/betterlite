use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::completion_bridge::tests::support::parsed;

use super::support::{
    Reply, SendFixture, details, json, logging_state, message_frame, params_with_provider, posted,
    pricing_params, send_call, status_frame, stream_call, with_agent,
};

#[rstest]
#[case::model("model", r#""a2a_agent/fake-agent""#)]
#[case::provider("custom_llm_provider", r#""a2a_agent""#)]
#[case::call_type("call_type", r#""asend_message_streaming""#)]
#[case::agent_id("agent_id", r#""agent-7""#)]
#[case::metadata("metadata", r#"{"user_id":"u-1"}"#)]
#[case::usage(
    "usage",
    r#"{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}"#
)]
#[case::stream("stream", "false")]
#[tokio::test]
async fn stream_logging_builds_an_object_when_absent(#[case] key: &str, #[case] expected: &str) {
    let reply = Reply::Sse(vec![status_frame("working"), message_frame("hi")]);
    let (outcome, _seen) = Box::pin(with_agent(reply, |base| async move {
        let mut fixture = SendFixture::new();
        let mut call = stream_call(send_call("hello world", Some(base), pricing_params()));
        call.agent_id = Some("agent-7".to_owned());
        call.metadata = Some(json(r#"{"user_id":"u-1"}"#));
        call.proxy_server_request = Some(json(r#"{"path":"/v1/a2a/message"}"#));
        Box::pin(fixture.stream(call, None)).await
    }))
    .await;

    assert_eq!(outcome.dispatch.len(), 1);
    let logging = &outcome.dispatch[0].logging;
    assert_eq!(logging.call_type, "asend_message_streaming");
    assert_eq!(logging.model, "a2a_agent/fake-agent");
    assert_eq!(logging.custom_llm_provider.as_deref(), Some("a2a_agent"));
    assert_eq!(logging.litellm_trace_id, "generated-1");
    assert_eq!(outcome.issued, vec!["generated-1".to_owned()]);
    let built_params = json(
        r#"{"cost_per_query":3,"input_cost_per_token":0.0001,"metadata":{"user_id":"u-1"},"proxy_server_request":{"path":"/v1/a2a/message"}}"#,
    );
    assert_eq!(Json::Object(logging.litellm_params.clone()), built_params);
    assert_eq!(Json::Object(logging.optional_params.clone()), built_params);
    assert_eq!(details(logging, key), Some(json(expected)));
}

#[tokio::test]
async fn stream_logging_reuses_the_given_object() {
    let reply = Reply::Sse(vec![message_frame("hi")]);
    let (outcome, _seen) = Box::pin(with_agent(reply, |base| async move {
        let mut fixture = SendFixture::new();
        let mut call = stream_call(send_call("hello", Some(base), BTreeMap::new()));
        call.agent_id = Some("agent-7".to_owned());
        let logging = logging_state(Some("logging-trace"));
        Box::pin(fixture.stream(call, Some(logging))).await
    }))
    .await;

    assert_eq!(outcome.dispatch.len(), 1);
    let logging = &outcome.dispatch[0].logging;
    assert_eq!(logging.call_type, "acompletion");
    assert_eq!(logging.model, "a2a_agent/fake-agent");
    assert_eq!(logging.custom_llm_provider.as_deref(), Some("a2a_agent"));
    assert_eq!(logging.litellm_trace_id, "logging-trace");
    assert!(logging.litellm_params.is_empty());
    assert!(logging.optional_params.is_empty());
    assert_eq!(details(logging, "agent_id"), Some(json(r#""agent-7""#)));
    assert!(outcome.issued.is_empty());
}

#[tokio::test]
async fn stream_iterator_yields_the_agent_chunks() {
    let reply = Reply::Sse(vec![status_frame("working"), message_frame("hi")]);
    let (outcome, seen) = Box::pin(with_agent(reply, |base| async move {
        let mut fixture = SendFixture::new();
        let call = stream_call(send_call(
            "hello world",
            Some(base),
            params_with_provider(None),
        ));
        Box::pin(fixture.stream(call, None)).await
    }))
    .await;

    let collected = outcome.events;
    assert_eq!(collected.len(), 2);
    let first = collected[0]
        .as_ref()
        .expect("the status chunk is forwarded");
    assert_eq!(first["result"]["kind"], parsed(r#""status-update""#));
    assert_eq!(first["result"]["status"]["state"], parsed(r#""working""#));
    let second = collected[1]
        .as_ref()
        .expect("the message chunk is forwarded");
    assert_eq!(second["result"]["kind"], parsed(r#""message""#));
    assert_eq!(second["result"]["parts"][0]["text"], parsed(r#""hi""#));
    assert_eq!(second["id"], parsed(r#""request-1""#));
    assert_eq!(outcome.dispatch.len(), 1);
    assert_eq!(
        details(&outcome.dispatch[0].logging, "usage"),
        Some(json(
            r#"{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}"#
        ))
    );
    assert!(
        posted(&seen)
            .body
            .contains("\"method\":\"SendStreamingMessage\""),
        "the streaming client sends the streaming method"
    );
}

#[tokio::test]
async fn stream_iterator_reports_a_stream_error() {
    let reply = Reply::Sse(vec!["data: not-json\n\n".to_owned()]);
    let (outcome, _seen) = Box::pin(with_agent(reply, |base| async move {
        let mut fixture = SendFixture::new();
        let call = stream_call(send_call("hello", Some(base), params_with_provider(None)));
        Box::pin(fixture.stream(call, None)).await
    }))
    .await;

    assert_eq!(outcome.events.len(), 1);
    outcome.events[0]
        .as_ref()
        .expect_err("the stream error is reported");
    assert!(outcome.dispatch.is_empty());
}
