use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::agents::json::AgentJson;
use rstest::rstest;

use crate::client::{A2AClientSettings, create_a2a_client};
use crate::completion_bridge::tests::Source;
use crate::completion_bridge::tests::support::{collect_events, provider, recorder};
use crate::send::asend_message;
use crate::send::asend_message_streaming;
use crate::send::call::{SendCall, StreamCall};
use crate::send::deps::{BridgeDeps, NativeDeps};
use crate::send::error::{SendCallError, SendEntryError};

use super::support::{
    CARD_PATH, CUSTOM_CARD_PATH, Reply, StreamFixture, buffered_message, card_requests, counter,
    header, logging_state, message_frame, params_with_provider, pool, posted, recording_dispatch,
    request, request_with_id, tls, with_agent, with_agent_advertising_a_dead_port,
};

#[tokio::test]
async fn send_client_sends_a_generated_trace_id_and_the_agent_id() {
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
            litellm_params: params_with_provider(None),
            agent_id: Some("agent-7".to_owned()),
            agent_extra_headers: None,
        };
        asend_message(call, None, &deps, &mut bridge).await
    })
    .await;

    assert!(outcome.is_ok());
    assert_eq!(card_requests(&seen).len(), 1);
    let posted = posted(&seen);
    assert_eq!(
        posted.headers.get("x-litellm-trace-id").map(String::as_str),
        Some("generated-trace")
    );
    assert_eq!(
        posted.headers.get("x-litellm-agent-id").map(String::as_str),
        Some("agent-7")
    );
    assert_eq!(
        header(&seen, CARD_PATH, "x-litellm-trace-id").as_deref(),
        Some("generated-trace")
    );
}

#[tokio::test]
async fn send_client_prefers_the_logging_trace_id() {
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
            litellm_params: params_with_provider(None),
            agent_id: None,
            agent_extra_headers: None,
        };
        let mut logging = logging_state(Some("logging-trace"));
        asend_message(call, Some(&mut logging), &deps, &mut bridge).await
    })
    .await;

    assert!(outcome.is_ok());
    let posted = posted(&seen);
    assert_eq!(
        posted.headers.get("x-litellm-trace-id").map(String::as_str),
        Some("logging-trace")
    );
    assert_eq!(posted.headers.get("x-litellm-agent-id"), None);
}

#[tokio::test]
async fn send_client_lets_agent_headers_override_the_litellm_headers() {
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
            litellm_params: params_with_provider(None),
            agent_id: Some("agent-7".to_owned()),
            agent_extra_headers: Some(BTreeMap::from([
                ("X-LiteLLM-Trace-Id".to_owned(), "agent-trace".to_owned()),
                ("X-LiteLLM-Agent-Id".to_owned(), "agent-header".to_owned()),
                ("x-team".to_owned(), "blue".to_owned()),
            ])),
        };
        asend_message(call, None, &deps, &mut bridge).await
    })
    .await;

    assert!(outcome.is_ok());
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
        litellm_params: params_with_provider(None),
        agent_id: None,
        agent_extra_headers: None,
    };

    let result = asend_message(call, None, &deps, &mut bridge).await;

    assert!(matches!(
        result,
        Err(SendEntryError::Call(SendCallError::MissingClient))
    ));
}

#[tokio::test]
async fn stream_client_requires_an_api_base_without_a_client() {
    let mut fixture = StreamFixture::new();
    let completion = recorder();
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
        request: request("hello"),
        api_base: None,
        litellm_params: params_with_provider(None),
        agent_id: None,
        metadata: None,
        proxy_server_request: None,
        agent_extra_headers: None,
    };
    let deps = fixture.deps(dispatch);

    let result = asend_message_streaming(call, None, deps, &mut bridge).await;

    assert!(matches!(result, Err(SendCallError::MissingClient)));
}

#[tokio::test]
async fn send_client_uses_the_provided_client_without_fetching_the_card_again() {
    let (outcome, seen) = with_agent(Reply::Json(buffered_message("hi")), |base| async move {
        let pool = pool();
        let environment = BTreeMap::new();
        let settings = A2AClientSettings {
            base_url: base.clone(),
            timeout: Some(Duration::from_secs(5)),
            tls: tls(),
            extra_headers: BTreeMap::new(),
            streaming: false,
            relative_card_path: None,
        };
        let client = create_a2a_client(&pool, settings, &environment)
            .await
            .expect("the test client builds");
        let counter = counter();
        let tls = tls();
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
            a2a_client: Some(client),
            request: request("hello"),
            api_base: None,
            litellm_params: params_with_provider(None),
            agent_id: None,
            agent_extra_headers: None,
        };
        asend_message(call, None, &deps, &mut bridge).await
    })
    .await;

    assert!(outcome.is_ok());
    assert_eq!(card_requests(&seen).len(), 1);
    assert!(posted(&seen).body.contains("\"method\":\"SendMessage\""));
}

#[tokio::test]
async fn send_client_uses_the_configured_agent_card_path() {
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
        let mut litellm_params = params_with_provider(None);
        litellm_params.insert(
            "agent_card_path".to_owned(),
            AgentJson::String("custom/card.json".to_owned()),
        );
        let call = SendCall {
            a2a_client: None,
            request: request("hello"),
            api_base: Some(base),
            litellm_params,
            agent_id: None,
            agent_extra_headers: None,
        };
        asend_message(call, None, &deps, &mut bridge).await
    })
    .await;

    assert!(outcome.is_ok());
    let cards = card_requests(&seen);
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].path, CUSTOM_CARD_PATH);
    assert!(posted(&seen).body.contains("hello"));
}

#[tokio::test]
async fn send_client_retries_with_the_agent_card_url_and_api_base() {
    let (outcome, seen) = with_agent_advertising_a_dead_port(
        Reply::Json(buffered_message("retried")),
        |base| async move {
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
                litellm_params: params_with_provider(None),
                agent_id: None,
                agent_extra_headers: None,
            };
            asend_message(call, None, &deps, &mut bridge).await
        },
    )
    .await;

    assert!(outcome.is_ok());
    assert_eq!(
        seen.iter().filter(|entry| entry.path == "/").count(),
        1,
        "the advertised agent only saw the retried message"
    );
    assert!(posted(&seen).body.contains("\"method\":\"SendMessage\""));
}

#[tokio::test]
async fn stream_client_retries_with_the_agent_card_url_and_api_base() {
    let (outcome, seen) = with_agent_advertising_a_dead_port(
        Reply::Sse(vec![message_frame("retried")]),
        |base| async move {
            let mut fixture = StreamFixture::new();
            let completion = recorder();
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
                request: request("hello"),
                api_base: Some(base),
                litellm_params: params_with_provider(None),
                agent_id: None,
                metadata: None,
                proxy_server_request: None,
                agent_extra_headers: None,
            };
            let deps = fixture.deps(dispatch);
            let events = asend_message_streaming(call, None, deps, &mut bridge)
                .await
                .expect("the stream starts");
            collect_events(events).await
        },
    )
    .await;

    assert_eq!(outcome.len(), 1);
    assert!(outcome[0].is_ok());
    assert_eq!(
        seen.iter().filter(|entry| entry.path == "/").count(),
        1,
        "the advertised agent only saw the retried message"
    );
}

#[rstest]
#[case::from_the_logging_object(Some("logging-trace"), "request-1", "logging-trace")]
#[case::from_the_request_id(None, "request-1", "request-1")]
#[case::generated_for_an_empty_id(None, "", "generated-1")]
#[tokio::test]
async fn stream_client_trace_id_fallbacks(
    #[case] logging_trace: Option<&str>,
    #[case] request_id: &str,
    #[case] expected_trace: &str,
) {
    let (outcome, seen) = with_agent(Reply::Sse(vec![message_frame("hi")]), |base| async move {
        let mut fixture = StreamFixture::new();
        let completion = recorder();
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
            request: request_with_id(request_id, "hello"),
            api_base: Some(base),
            litellm_params: params_with_provider(None),
            agent_id: Some("agent-7".to_owned()),
            metadata: None,
            proxy_server_request: None,
            agent_extra_headers: None,
        };
        let logging = logging_trace.map(|trace| logging_state(Some(trace)));
        let deps = fixture.deps(dispatch);
        let events = asend_message_streaming(call, logging, deps, &mut bridge)
            .await
            .expect("the stream starts");
        collect_events(events).await
    })
    .await;

    assert_eq!(outcome.len(), 1);
    assert!(outcome[0].is_ok());
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
    let (outcome, seen) = with_agent(Reply::Sse(vec![message_frame("hi")]), |base| async move {
        let mut fixture = StreamFixture::new();
        let completion = recorder();
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
            request: request("hello"),
            api_base: Some(base),
            litellm_params: params_with_provider(None),
            agent_id: Some("agent-7".to_owned()),
            metadata: None,
            proxy_server_request: None,
            agent_extra_headers: Some(BTreeMap::from([(
                "X-LiteLLM-Trace-Id".to_owned(),
                "agent-trace".to_owned(),
            )])),
        };
        let deps = fixture.deps(dispatch);
        let events = asend_message_streaming(call, None, deps, &mut bridge)
            .await
            .expect("the stream starts");
        collect_events(events).await
    })
    .await;

    assert_eq!(outcome.len(), 1);
    let posted = posted(&seen);
    assert_eq!(
        posted.headers.get("x-litellm-trace-id").map(String::as_str),
        Some("agent-trace")
    );
}
