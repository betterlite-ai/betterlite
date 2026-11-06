use std::sync::Arc;
use std::time::{Duration, Instant};

use betterlite_core::a2a::exception_mapping::{A2AException, map_a2a_exception};
use betterlite_core::agents::provider::a2a::JsonRpcId;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use futures_util::StreamExt as _;
use http::Method;
use http::header::HeaderMap;
use rstest::rstest;
use tokio::net::TcpListener;
use tokio::time::timeout;
use url::Url;

use super::{
    AgentState, CARD_PATH, Reply, buffered_message, card_json, create_a2a_client, environment,
    limited_client, message_frame, pooled_client, run_agent, send_message, send_request, settings,
    start_agent, status_frame, team_of, tls_policy, user_agent_of,
};
use crate::client::NativeA2aSdk;
use crate::client::convert::{to_compat_stream_response, to_core_send_message_request};
use crate::client::error::{ClientError, transport_error};
use crate::client::records::compat::SendMessageRequest;
use crate::client::records::core::{
    CoreArtifact, CoreArtifactUpdate, CoreMessage, CorePart, CoreStreamEvent,
};
use crate::client::send::stream_messages;
use crate::errors::A2AError;

fn interfaces_card(interfaces: &str) -> String {
    format!(
        r#"{{"name":"fake-agent","supportedInterfaces":[{interfaces}],"capabilities":{{"streaming":true}},"defaultInputModes":["text"],"defaultOutputModes":["text"],"skills":[]}}"#
    )
}

fn request_id() -> JsonRpcId {
    let raw = r#""request-1""#;
    serde_json::from_str(raw).unwrap()
}

#[rstest]
#[case::multi_interface("multi")]
#[case::rest_only_card("rest-only")]
#[case::tenant_interface("tenant")]
#[tokio::test]
async fn interface_selection(#[case] scenario: &str) {
    let (listener, base) = start_agent().await;
    let interfaces = match scenario {
        "multi" => format!(
            r#"{{"url":"{base}/rest","protocolBinding":"HTTP+JSON","protocolVersion":"1.0"}},{{"url":"{base}/rpc","protocolBinding":"JSONRPC","protocolVersion":"1.0"}}"#
        ),
        "rest-only" => format!(
            r#"{{"url":"{base}/rest","protocolBinding":"HTTP+JSON","protocolVersion":"1.0"}}"#
        ),
        _ => format!(
            r#"{{"url":"{base}/rpc","protocolBinding":"JSONRPC","protocolVersion":"1.0","tenant":"acme"}}"#
        ),
    };
    let state = AgentState {
        card: interfaces_card(&interfaces),
        reply: Reply::Json(buffered_message("hi")),
        seen: Arc::default(),
    };
    let pool = pooled_client();
    let request = send_request("hello");
    let agent_base = base.clone();
    let (created, seen) = run_agent(
        listener,
        state,
        Box::pin(async move {
            let client = create_a2a_client(
                &pool,
                settings(&agent_base, false, Duration::from_secs(5)),
                &environment(None),
            )
            .await?;
            let endpoint = client.endpoint.clone();
            let sent = send_message(&client, &request).await;
            Ok::<_, A2AError>((endpoint, sent))
        }),
    )
    .await;
    if scenario == "rest-only" {
        let error = created.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("no compatible transports found.")
        );
        assert!(seen.iter().all(|entry| entry.path == CARD_PATH));
        return;
    }
    let (endpoint, sent) = created.unwrap();
    assert_eq!(endpoint.as_str(), format!("{base}/rpc"));
    sent.unwrap();
    assert_eq!(team_of(&seen, "/rpc").as_deref(), Some("blue"));
    if scenario == "tenant" {
        let body = seen
            .iter()
            .find(|entry| entry.path == "/rpc")
            .map(|entry| entry.body.clone())
            .unwrap_or_default();
        assert!(body.contains(r#""tenant":"acme""#));
    }
}

#[tokio::test]
async fn stream_survives_gaps_beyond_timeout() {
    let (listener, base) = start_agent().await;
    let state = AgentState {
        card: card_json(&base, true),
        reply: Reply::SseGaps(vec![
            (0, status_frame()),
            (300, status_frame()),
            (300, status_frame()),
        ]),
        seen: Arc::default(),
    };
    let pool = pooled_client();
    let request = send_request("hello");
    let (chunks, _seen) = run_agent(
        listener,
        state,
        Box::pin(async move {
            let client = create_a2a_client(
                &pool,
                settings(&base, true, Duration::from_millis(500)),
                &environment(None),
            )
            .await
            .unwrap();
            stream_messages(&client, &request)
                .await
                .collect::<Vec<_>>()
                .await
        }),
    )
    .await;
    assert_eq!(chunks.len(), 3);
    assert!(chunks.iter().all(Result::is_ok));
}

#[tokio::test]
async fn redirect_not_followed() {
    let (listener, base) = start_agent().await;
    let target = format!("{base}/moved");
    let state = AgentState {
        card: card_json(&base, true),
        reply: Reply::Redirect(target),
        seen: Arc::default(),
    };
    let pool = pooled_client();
    let request = send_request("hello");
    let (outcome, seen) = run_agent(
        listener,
        state,
        Box::pin(async move {
            let client = create_a2a_client(
                &pool,
                settings(&base, false, Duration::from_secs(5)),
                &environment(None),
            )
            .await
            .unwrap();
            send_message(&client, &request).await
        }),
    )
    .await;
    assert!(matches!(
        outcome,
        Err(ClientError::Status { status: 307, .. })
    ));
    assert!(seen.iter().all(|entry| entry.path != "/moved"));
}

#[test]
fn connection_words_in_urls_stay_generic() {
    let json_rpc = ClientError::JsonRpc {
        url: "https://connect.example.com/rpc".to_owned(),
        code: -32601,
        message: "method not found".to_owned(),
    };
    let mapped = map_a2a_exception(&NativeA2aSdk, &json_rpc, None, None, None);
    assert!(matches!(mapped, A2AException::Other { .. }));
    let timeout = ClientError::Timeout {
        url: "https://connect.example.com/rpc".to_owned(),
        timeout: Duration::from_secs(1),
        source: None,
    };
    let mapped_timeout = map_a2a_exception(&NativeA2aSdk, &timeout, None, None, None);
    assert!(matches!(mapped_timeout, A2AException::Other { .. }));
}

#[test]
fn core_message_defaults() {
    let message: CoreMessage = serde_json::from_str(r#"{"role":"ROLE_AGENT"}"#).unwrap();
    assert_eq!(message.common.message_id, "");
    assert!(message.parts.is_empty());
    let response = to_compat_stream_response(CoreStreamEvent::Message(message), request_id());
    let json = serde_json::to_string(&response.result).unwrap();
    assert!(json.contains(r#""messageId":"""#));
    assert!(json.contains(r#""parts":[]"#));
}

#[test]
fn wrapped_base64_decodes() {
    let request: SendMessageRequest = serde_json::from_str(
        r#"{"id":"request-1","params":{"message":{"messageId":"m-1","role":"user","parts":[{"kind":"file","file":{"bytes":"aG\nk="}}]}}}"#,
    )
    .unwrap();
    let core = to_core_send_message_request(request).unwrap();
    let CorePart::Raw(part) = &core.message.parts[0] else {
        panic!("expected a raw part");
    };
    assert_eq!(part.raw, "aGk=");
}

#[test]
fn artifact_update_defaults() {
    let update = CoreArtifactUpdate {
        task_id: "t-1".to_owned(),
        context_id: None,
        artifact: CoreArtifact {
            artifact_id: "a-1".to_owned(),
            name: None,
            description: None,
            parts: Vec::new(),
            metadata: None,
            extensions: Vec::new(),
        },
        append: None,
        last_chunk: None,
        metadata: None,
    };
    let response = to_compat_stream_response(CoreStreamEvent::ArtifactUpdate(update), request_id());
    let json = serde_json::to_string(&response.result).unwrap();
    assert!(json.contains(r#""append":false"#));
    assert!(json.contains(r#""lastChunk":false"#));
}

#[tokio::test]
async fn url_only_card_uses_core_transport() {
    let (listener, base) = start_agent().await;
    let state = AgentState {
        card: card_json(&base, true),
        reply: Reply::Json(buffered_message("hi")),
        seen: Arc::default(),
    };
    let pool = pooled_client();
    let request = send_request("hello");
    let (outcome, seen) = run_agent(
        listener,
        state,
        Box::pin(async move {
            let client = create_a2a_client(
                &pool,
                settings(&base, false, Duration::from_secs(5)),
                &environment(None),
            )
            .await
            .unwrap();
            send_message(&client, &request).await
        }),
    )
    .await;
    outcome.unwrap();
    let posted = seen.iter().find(|entry| entry.path == "/").unwrap();
    assert!(posted.body.contains(r#""method":"SendMessage""#));
    assert_eq!(posted.version.as_deref(), Some("1.0"));
}

#[tokio::test]
async fn message_then_status_ends_the_stream() {
    let (listener, base) = start_agent().await;
    let state = AgentState {
        card: card_json(&base, true),
        reply: Reply::Sse(vec![message_frame("first"), status_frame()]),
        seen: Arc::default(),
    };
    let pool = pooled_client();
    let request = send_request("hello");
    let (chunks, _seen) = run_agent(
        listener,
        state,
        Box::pin(async move {
            let client = create_a2a_client(
                &pool,
                settings(&base, true, Duration::from_secs(5)),
                &environment(None),
            )
            .await
            .unwrap();
            stream_messages(&client, &request)
                .await
                .collect::<Vec<_>>()
                .await
        }),
    )
    .await;
    assert_eq!(chunks.len(), 1);
    chunks[0].as_ref().unwrap();
}

#[tokio::test]
async fn sse_error_event_fails_the_stream() {
    let (listener, base) = start_agent().await;
    let state = AgentState {
        card: card_json(&base, true),
        reply: Reply::Sse(vec![String::from("event: error\ndata: boom\n\n")]),
        seen: Arc::default(),
    };
    let pool = pooled_client();
    let request = send_request("hello");
    let (chunks, _seen) = run_agent(
        listener,
        state,
        Box::pin(async move {
            let client = create_a2a_client(
                &pool,
                settings(&base, true, Duration::from_secs(5)),
                &environment(None),
            )
            .await
            .unwrap();
            stream_messages(&client, &request)
                .await
                .collect::<Vec<_>>()
                .await
        }),
    )
    .await;
    assert_eq!(chunks.len(), 1);
    assert!(matches!(
        &chunks[0],
        Err(ClientError::SseError { message }) if message.contains("boom")
    ));
}

#[tokio::test]
async fn additional_interfaces_choose_jsonrpc() {
    let (listener, base) = start_agent().await;
    let card = format!(
        r#"{{"name":"fake-agent","url":"{base}/rest","preferredTransport":"HTTP+JSON","additionalInterfaces":[{{"url":"{base}/rpc","transport":"JSONRPC"}}],"capabilities":{{"streaming":true}},"defaultInputModes":["text"],"defaultOutputModes":["text"],"skills":[]}}"#
    );
    let state = AgentState {
        card,
        reply: Reply::Json(buffered_message("hi")),
        seen: Arc::default(),
    };
    let pool = pooled_client();
    let request = send_request("hello");
    let agent_base = base.clone();
    let (outcome, seen) = run_agent(
        listener,
        state,
        Box::pin(async move {
            let client = create_a2a_client(
                &pool,
                settings(&agent_base, false, Duration::from_secs(5)),
                &environment(None),
            )
            .await
            .unwrap();
            let endpoint = client.endpoint.clone();
            let sent = send_message(&client, &request).await;
            (endpoint, sent)
        }),
    )
    .await;
    let (endpoint, sent) = outcome;
    assert_eq!(endpoint.as_str(), format!("{base}/rpc"));
    sent.unwrap();
    assert_eq!(team_of(&seen, "/rpc").as_deref(), Some("blue"));
}

#[tokio::test]
async fn stream_stall_is_a_timeout() {
    let (listener, base) = start_agent().await;
    let state = AgentState {
        card: card_json(&base, true),
        reply: Reply::SseGaps(vec![(0, status_frame()), (1500, status_frame())]),
        seen: Arc::default(),
    };
    let pool = pooled_client();
    let request = send_request("hello");
    let (chunks, _seen) = run_agent(
        listener,
        state,
        Box::pin(async move {
            let client = create_a2a_client(
                &pool,
                settings(&base, true, Duration::from_millis(500)),
                &environment(None),
            )
            .await
            .unwrap();
            stream_messages(&client, &request)
                .await
                .collect::<Vec<_>>()
                .await
        }),
    )
    .await;
    assert_eq!(chunks.len(), 2);
    chunks[0].as_ref().unwrap();
    let Err(error) = &chunks[1] else {
        panic!("expected a timeout error");
    };
    assert!(matches!(error, ClientError::Timeout { .. }));
    assert!(matches!(
        map_a2a_exception(&NativeA2aSdk, error, None, None, None),
        A2AException::Other { .. }
    ));
}

#[tokio::test]
async fn connect_timeout_classifies_as_timeout() {
    let pool = pooled_client();
    let request = OutboundRequest {
        method: Method::POST,
        url: Url::parse("http://127.0.0.1:1/").unwrap(),
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Empty,
        deadline: Duration::from_millis(100),
    };
    let Err(failure) = pool.send(request).await else {
        panic!("expected a connect failure");
    };
    let TransportError::Connect(source) = failure else {
        panic!("expected a connect failure");
    };
    let classified = transport_error(
        "http://127.0.0.1:1/",
        Duration::from_millis(100),
        TransportError::ConnectTimeout(source),
    );
    assert!(matches!(classified, ClientError::Timeout { .. }));
}

#[tokio::test]
async fn streaming_head_timeout() {
    let (listener, base) = start_agent().await;
    let state = AgentState {
        card: card_json(&base, true),
        reply: Reply::Slow(Duration::from_millis(500), buffered_message("late")),
        seen: Arc::default(),
    };
    let pool = pooled_client();
    let request = send_request("hello");
    let (chunks, _seen) = run_agent(
        listener,
        state,
        Box::pin(async move {
            let client = create_a2a_client(
                &pool,
                settings(&base, true, Duration::from_millis(100)),
                &environment(None),
            )
            .await
            .unwrap();
            stream_messages(&client, &request)
                .await
                .collect::<Vec<_>>()
                .await
        }),
    )
    .await;
    assert_eq!(chunks.len(), 1);
    assert!(matches!(&chunks[0], Err(ClientError::Timeout { .. })));
}

#[tokio::test]
async fn streaming_fallback_body_stall_times_out() {
    let (listener, base) = start_agent().await;
    let state = AgentState {
        card: card_json(&base, true),
        reply: Reply::StallJson(500),
        seen: Arc::default(),
    };
    let pool = pooled_client();
    let request = send_request("hello");
    let (chunks, _seen) = run_agent(
        listener,
        state,
        Box::pin(async move {
            let client = create_a2a_client(
                &pool,
                settings(&base, true, Duration::from_millis(100)),
                &environment(None),
            )
            .await
            .unwrap();
            stream_messages(&client, &request)
                .await
                .collect::<Vec<_>>()
                .await
        }),
    )
    .await;
    assert_eq!(chunks.len(), 1);
    assert!(matches!(&chunks[0], Err(ClientError::Timeout { .. })));
}

#[tokio::test]
async fn buffered_trickle_succeeds() {
    let (listener, base) = start_agent().await;
    let body = buffered_message("late");
    let (first, rest) = body.split_at(40);
    let (second, third) = rest.split_at(40);
    let chunks = vec![
        (0, first.to_owned()),
        (300, second.to_owned()),
        (300, third.to_owned()),
    ];
    let state = AgentState {
        card: card_json(&base, true),
        reply: Reply::JsonGaps(chunks),
        seen: Arc::default(),
    };
    let pool = pooled_client();
    let request = send_request("hello");
    let (outcome, _seen) = run_agent(
        listener,
        state,
        Box::pin(async move {
            let client = create_a2a_client(
                &pool,
                settings(&base, false, Duration::from_millis(500)),
                &environment(None),
            )
            .await
            .unwrap();
            send_message(&client, &request).await
        }),
    )
    .await;
    outcome.unwrap();
}

#[tokio::test]
async fn reqwest_read_timeout_classifies_as_timeout() {
    let (listener, base) = start_agent().await;
    let state = AgentState {
        card: card_json(&base, true),
        reply: Reply::SseGaps(vec![(300, status_frame())]),
        seen: Arc::default(),
    };
    let pool = pooled_client();
    let request = send_request("hello");
    let (chunks, _seen) = run_agent(
        listener,
        state,
        Box::pin(async move {
            let mut client = create_a2a_client(
                &pool,
                settings(&base, true, Duration::from_secs(5)),
                &environment(None),
            )
            .await
            .unwrap();
            client.http_client = limited_client(TransportLimits {
                connect_timeout: Duration::from_secs(1),
                read_timeout: Duration::from_millis(50),
                ..TransportLimits::DEFAULT
            });
            client.timeout = Duration::from_secs(5);
            stream_messages(&client, &request)
                .await
                .collect::<Vec<_>>()
                .await
        }),
    )
    .await;
    assert_eq!(chunks.len(), 1);
    assert!(matches!(&chunks[0], Err(ClientError::Timeout { .. })));
}

#[tokio::test]
async fn connect_phase_follows_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("https://{}", listener.local_addr().unwrap());
    let pool = limited_client(TransportLimits {
        connect_timeout: Duration::from_millis(200),
        read_timeout: Duration::from_millis(200),
        ..TransportLimits::DEFAULT
    });
    let mut settings = settings(&base, false, Duration::from_secs(2));
    settings.relative_card_path = Some("/".to_owned());
    let started = Instant::now();
    let outcome = timeout(
        Duration::from_secs(5),
        create_a2a_client(&pool, settings, &environment(None)),
    )
    .await;
    let Ok(Err(_failure)) = outcome else {
        panic!("the card fetch must fail at the configured timeout");
    };
    assert!(started.elapsed() >= Duration::from_millis(1500));
    drop(listener);
}

#[tokio::test]
async fn policy_user_agent_reaches_both_requests() {
    let (listener, base) = start_agent().await;
    let state = AgentState {
        card: card_json(&base, true),
        reply: Reply::Json(buffered_message("hi")),
        seen: Arc::default(),
    };
    let pool = pooled_client();
    let request = send_request("hello");
    let mut settings = settings(&base, false, Duration::from_secs(5));
    settings.tls = tls_policy("custom-ua/1");
    let (outcome, seen) = run_agent(
        listener,
        state,
        Box::pin(async move {
            let client = create_a2a_client(&pool, settings, &environment(None))
                .await
                .unwrap();
            send_message(&client, &request).await
        }),
    )
    .await;
    outcome.unwrap();
    assert_eq!(
        user_agent_of(&seen, CARD_PATH).as_deref(),
        Some("custom-ua/1")
    );
    assert_eq!(user_agent_of(&seen, "/").as_deref(), Some("custom-ua/1"));
}

#[rstest]
#[case::buffered(false)]
#[case::streaming_fallback(true)]
#[tokio::test]
async fn body_cap_is_enforced(#[case] streaming: bool) {
    let (listener, base) = start_agent().await;
    let body = buffered_message("late");
    let cap = body.len().saturating_sub(1);
    let state = AgentState {
        card: card_json(&base, true),
        reply: Reply::Json(body),
        seen: Arc::default(),
    };
    let pool = pooled_client();
    let request = send_request("hello");
    let (outcome, _seen) = run_agent(
        listener,
        state,
        Box::pin(async move {
            let mut client = create_a2a_client(
                &pool,
                settings(&base, streaming, Duration::from_secs(5)),
                &environment(None),
            )
            .await
            .unwrap();
            client.http_client = limited_client(TransportLimits {
                body_bytes: cap,
                ..TransportLimits::DEFAULT
            });
            send_message(&client, &request).await
        }),
    )
    .await;
    assert!(matches!(
        outcome,
        Err(ClientError::Request {
            source: TransportError::BodyTooLarge { .. },
            ..
        })
    ));
}

#[rstest]
#[case::buffered(false)]
#[case::streaming_fallback(true)]
#[tokio::test]
async fn body_cap_allows_exact_size(#[case] streaming: bool) {
    let (listener, base) = start_agent().await;
    let body = buffered_message("late");
    let cap = body.len();
    let state = AgentState {
        card: card_json(&base, true),
        reply: Reply::Json(body),
        seen: Arc::default(),
    };
    let pool = pooled_client();
    let request = send_request("hello");
    let (outcome, _seen) = run_agent(
        listener,
        state,
        Box::pin(async move {
            let mut client = create_a2a_client(
                &pool,
                settings(&base, streaming, Duration::from_secs(5)),
                &environment(None),
            )
            .await
            .unwrap();
            client.http_client = limited_client(TransportLimits {
                body_bytes: cap,
                ..TransportLimits::DEFAULT
            });
            send_message(&client, &request).await
        }),
    )
    .await;
    outcome.unwrap();
}

#[tokio::test]
async fn network_mock_never_reaches_the_agent() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut pool = pooled_client();
    pool.network_mock = true;
    let mut settings = settings(&base, false, Duration::from_secs(2));
    settings.relative_card_path = Some("/".to_owned());
    let _outcome = create_a2a_client(&pool, settings, &environment(None)).await;
    let accepted = timeout(Duration::from_millis(200), listener.accept()).await;
    assert!(matches!(accepted, Err(_elapsed)));
}
