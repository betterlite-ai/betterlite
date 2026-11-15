use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::agents::json::AgentJson;
use rstest::rstest;

use crate::client::{A2AClientSettings, create_a2a_client};
use crate::send::error::{SendCallError, SendEntryError};

use super::support::{
    CARD_PATH, CUSTOM_CARD_PATH, Reply, SendFixture, buffered_message, card_requests, header,
    logging_state, message_frame, params_with_provider, pool, posted, request_with_id, send_call,
    stream_call, tls, with_agent, with_agent_advertising_a_dead_port,
};

#[tokio::test]
async fn send_client_sends_a_generated_trace_id_and_the_agent_id() {
    let reply = Reply::Json(buffered_message("hi"));
    let (outcome, seen) = Box::pin(with_agent(reply, |base| async move {
        let mut fixture = SendFixture::new();
        let mut call = send_call("hello", Some(base), params_with_provider(None));
        call.agent_id = Some("agent-7".to_owned());
        fixture.send(call, None).await
    }))
    .await;

    outcome.expect("the send succeeds");
    assert_eq!(card_requests(&seen).len(), 1);
    let posted = posted(&seen);
    assert_eq!(
        posted.headers.get("x-litellm-trace-id").map(String::as_str),
        Some("generated-1")
    );
    assert_eq!(
        posted.headers.get("x-litellm-agent-id").map(String::as_str),
        Some("agent-7")
    );
    assert_eq!(
        header(&seen, CARD_PATH, "x-litellm-trace-id").as_deref(),
        Some("generated-1")
    );
}

#[tokio::test]
async fn send_client_prefers_the_logging_trace_id() {
    let reply = Reply::Json(buffered_message("hi"));
    let (outcome, seen) = Box::pin(with_agent(reply, |base| async move {
        let mut fixture = SendFixture::new();
        let call = send_call("hello", Some(base), params_with_provider(None));
        let mut logging = logging_state(Some("logging-trace"));
        fixture.send(call, Some(&mut logging)).await
    }))
    .await;

    outcome.expect("the send succeeds");
    let posted = posted(&seen);
    assert_eq!(
        posted.headers.get("x-litellm-trace-id").map(String::as_str),
        Some("logging-trace")
    );
    assert_eq!(posted.headers.get("x-litellm-agent-id"), None);
}

#[tokio::test]
async fn send_client_generates_a_trace_id_for_an_empty_logging_trace() {
    let reply = Reply::Json(buffered_message("hi"));
    let (outcome, seen) = Box::pin(with_agent(reply, |base| async move {
        let mut fixture = SendFixture::new();
        let call = send_call("hello", Some(base), params_with_provider(None));
        let mut logging = logging_state(Some(""));
        fixture.send(call, Some(&mut logging)).await
    }))
    .await;

    outcome.expect("the send succeeds");
    let posted = posted(&seen);
    assert_eq!(
        posted.headers.get("x-litellm-trace-id").map(String::as_str),
        Some("generated-1")
    );
}

#[tokio::test]
async fn send_client_lets_agent_headers_override_the_litellm_headers() {
    let reply = Reply::Json(buffered_message("hi"));
    let (outcome, seen) = Box::pin(with_agent(reply, |base| async move {
        let mut fixture = SendFixture::new();
        let mut call = send_call("hello", Some(base), params_with_provider(None));
        call.agent_id = Some("agent-7".to_owned());
        call.agent_extra_headers = Some(BTreeMap::from([
            ("X-LiteLLM-Trace-Id".to_owned(), "agent-trace".to_owned()),
            ("X-LiteLLM-Agent-Id".to_owned(), "agent-header".to_owned()),
            ("x-team".to_owned(), "blue".to_owned()),
        ]));
        fixture.send(call, None).await
    }))
    .await;

    outcome.expect("the send succeeds");
    let posted = posted(&seen);
    assert_eq!(
        posted.headers.get("x-litellm-trace-id").map(String::as_str),
        Some("agent-trace")
    );
    assert_eq!(
        posted.headers.get("x-litellm-agent-id").map(String::as_str),
        Some("agent-header")
    );
    assert_eq!(
        posted.headers.get("x-team").map(String::as_str),
        Some("blue")
    );
}

#[tokio::test]
async fn send_client_requires_an_api_base_without_a_client() {
    let mut fixture = SendFixture::new();
    let call = send_call("hello", None, params_with_provider(None));

    let result = fixture.send(call, None).await;

    assert!(matches!(
        result,
        Err(SendEntryError::Call(SendCallError::MissingClient))
    ));
}

#[tokio::test]
async fn stream_client_requires_an_api_base_without_a_client() {
    let mut fixture = SendFixture::new();
    let call = stream_call(send_call("hello", None, params_with_provider(None)));

    let error = Box::pin(fixture.start_error(call, None)).await;

    assert!(matches!(error, SendCallError::MissingClient));
}

#[tokio::test]
async fn stream_client_uses_the_provided_client_without_fetching_the_card_again() {
    let reply = Reply::Sse(vec![message_frame("hi")]);
    let (outcome, seen) = Box::pin(with_agent(reply, |base| async move {
        let pool = pool();
        let environment = BTreeMap::new();
        let settings = A2AClientSettings {
            base_url: base,
            timeout: Some(Duration::from_secs(5)),
            tls: tls(),
            extra_headers: BTreeMap::new(),
            streaming: true,
            relative_card_path: None,
        };
        let client = create_a2a_client(&pool, settings, &environment)
            .await
            .expect("the test client builds");
        let mut fixture = SendFixture::new();
        let mut call = stream_call(send_call("hello", None, params_with_provider(None)));
        call.a2a_client = Some(client);
        Box::pin(fixture.stream(call, None)).await
    }))
    .await;

    assert_eq!(outcome.events.len(), 1);
    outcome.events[0]
        .as_ref()
        .expect("the event is not an error");
    assert_eq!(card_requests(&seen).len(), 1);
}

#[tokio::test]
async fn send_client_uses_the_provided_client_without_fetching_the_card_again() {
    let reply = Reply::Json(buffered_message("hi"));
    let (outcome, seen) = Box::pin(with_agent(reply, |base| async move {
        let pool = pool();
        let environment = BTreeMap::new();
        let settings = A2AClientSettings {
            base_url: base,
            timeout: Some(Duration::from_secs(5)),
            tls: tls(),
            extra_headers: BTreeMap::new(),
            streaming: false,
            relative_card_path: None,
        };
        let client = create_a2a_client(&pool, settings, &environment)
            .await
            .expect("the test client builds");
        let mut fixture = SendFixture::new();
        let mut call = send_call("hello", None, params_with_provider(None));
        call.a2a_client = Some(client);
        fixture.send(call, None).await
    }))
    .await;

    outcome.expect("the send succeeds");
    assert_eq!(card_requests(&seen).len(), 1);
    assert!(posted(&seen).body.contains("\"method\":\"SendMessage\""));
}

#[tokio::test]
async fn send_client_uses_the_configured_agent_card_path() {
    let reply = Reply::Json(buffered_message("hi"));
    let (outcome, seen) = Box::pin(with_agent(reply, |base| async move {
        let mut fixture = SendFixture::new();
        let mut call = send_call("hello", Some(base), params_with_provider(None));
        call.litellm_params.insert(
            "agent_card_path".to_owned(),
            AgentJson::String("custom/card.json".to_owned()),
        );
        fixture.send(call, None).await
    }))
    .await;

    outcome.expect("the send succeeds");
    let cards = card_requests(&seen);
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].path, CUSTOM_CARD_PATH);
    assert!(posted(&seen).body.contains("hello"));
}

#[tokio::test]
async fn send_client_retries_with_the_agent_card_url_and_api_base() {
    let reply = Reply::Json(buffered_message("retried"));
    let (outcome, seen) = Box::pin(with_agent_advertising_a_dead_port(
        reply,
        |base| async move {
            let mut fixture = SendFixture::new();
            let call = send_call("hello", Some(base), params_with_provider(None));
            fixture.send(call, None).await
        },
    ))
    .await;

    outcome.expect("the send succeeds");
    assert_eq!(
        seen.iter().filter(|entry| entry.path == "/").count(),
        1,
        "the advertised agent only saw the retried message"
    );
    assert!(posted(&seen).body.contains("\"method\":\"SendMessage\""));
}

#[tokio::test]
async fn stream_client_retries_with_the_agent_card_url_and_api_base() {
    let reply = Reply::Sse(vec![message_frame("retried")]);
    let (outcome, seen) = Box::pin(with_agent_advertising_a_dead_port(
        reply,
        |base| async move {
            let mut fixture = SendFixture::new();
            let call = stream_call(send_call("hello", Some(base), params_with_provider(None)));
            Box::pin(fixture.stream(call, None)).await
        },
    ))
    .await;

    assert_eq!(outcome.events.len(), 1);
    outcome.events[0]
        .as_ref()
        .expect("the event is not an error");
    assert_eq!(
        seen.iter().filter(|entry| entry.path == "/").count(),
        1,
        "the advertised agent only saw the retried message"
    );
}

#[rstest]
#[case::from_the_logging_object(Some("logging-trace"), "request-1", "logging-trace")]
#[case::from_the_request_id(None, "request-1", "request-1")]
#[case::from_the_request_id_for_an_empty_logging_trace(Some(""), "request-1", "request-1")]
#[case::generated_for_an_empty_id(None, "", "generated-1")]
#[tokio::test]
async fn stream_client_trace_id_fallbacks(
    #[case] logging_trace: Option<&str>,
    #[case] request_id: &str,
    #[case] expected_trace: &str,
) {
    let reply = Reply::Sse(vec![message_frame("hi")]);
    let (outcome, seen) = Box::pin(with_agent(reply, |base| async move {
        let mut fixture = SendFixture::new();
        let mut call = stream_call(send_call("hello", Some(base), params_with_provider(None)));
        call.request = request_with_id(request_id, "hello");
        call.agent_id = Some("agent-7".to_owned());
        let logging = logging_trace.map(|trace| logging_state(Some(trace)));
        Box::pin(fixture.stream(call, logging)).await
    }))
    .await;

    assert_eq!(outcome.events.len(), 1);
    outcome.events[0]
        .as_ref()
        .expect("the event is not an error");
    let posted = posted(&seen);
    assert_eq!(
        posted.headers.get("x-litellm-trace-id").map(String::as_str),
        Some(expected_trace)
    );
    assert_eq!(
        posted.headers.get("x-litellm-agent-id").map(String::as_str),
        Some("agent-7")
    );
}

#[tokio::test]
async fn stream_client_lets_agent_headers_override_the_litellm_headers() {
    let reply = Reply::Sse(vec![message_frame("hi")]);
    let (outcome, seen) = Box::pin(with_agent(reply, |base| async move {
        let mut fixture = SendFixture::new();
        let mut call = stream_call(send_call("hello", Some(base), params_with_provider(None)));
        call.agent_id = Some("agent-7".to_owned());
        call.agent_extra_headers = Some(BTreeMap::from([(
            "X-LiteLLM-Trace-Id".to_owned(),
            "agent-trace".to_owned(),
        )]));
        Box::pin(fixture.stream(call, None)).await
    }))
    .await;

    assert_eq!(outcome.events.len(), 1);
    let posted = posted(&seen);
    assert_eq!(
        posted.headers.get("x-litellm-trace-id").map(String::as_str),
        Some("agent-trace")
    );
}
