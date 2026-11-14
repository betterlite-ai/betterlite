use super::*;

#[tokio::test]
async fn a_307_post_is_followed_within_the_origin() {
    let server = TestServer::start(|request| {
        if request.uri == "/moved" {
            answer(
                200,
                "application/json",
                "{\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{}}",
            )
        } else {
            relocated(307, "/moved")
        }
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let message = receive(&mut streams).await;
    assert_eq!(response_id(&message), Some(RequestId::Number(7)));
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].uri, "/moved");
}

#[tokio::test]
async fn a_302_on_a_post_is_not_followed_and_names_the_location() {
    let server = TestServer::start(|_| relocated(302, "/elsewhere")).await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let message = receive(&mut streams).await;
    let (code, text) = error_of(&message);
    assert_eq!(code, -32600);
    assert!(text.contains("/elsewhere"), "{text}");
    assert!(text.contains("not followed"), "{text}");
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn a_twenty_first_redirect_is_not_followed() {
    let server = TestServer::start(|_| relocated(307, "/mcp")).await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let message = receive(&mut streams).await;
    let (code, text) = error_of(&message);
    assert_eq!(code, -32600);
    assert!(text.contains("not followed"), "{text}");
    assert_eq!(server.requests().len(), 21);
}

#[tokio::test]
async fn a_202_resolves_the_request_as_invalid() {
    let server = TestServer::start(|_| answer(202, "", "")).await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32600);
    assert_eq!(text, "server answered a request with 202 Accepted");
}

#[tokio::test]
async fn a_404_before_a_session_is_method_not_found() {
    let server = TestServer::start(|_| answer(404, "text/plain", "missing")).await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32601);
    assert_eq!(text, "Not Found");
}

#[tokio::test]
async fn a_404_after_a_session_is_session_terminated() {
    let server = TestServer::start(|request| {
        if request.method == Method::POST
            && request.body.starts_with(b"{\"jsonrpc\":\"2.0\",\"id\":1")
        {
            let mut response = answer(
                200,
                "application/json",
                "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}",
            );
            response
                .headers_mut()
                .insert(SESSION_HEADER, HeaderValue::from_static("session-1"));
            return response;
        }
        answer(404, "text/plain", "missing")
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(1, "initialize")).await;
    assert_eq!(
        response_id(&receive(&mut streams).await),
        Some(RequestId::Number(1))
    );
    send(&mut streams, request(2, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32600);
    assert_eq!(text, "Session terminated");
}

#[tokio::test]
async fn a_json_error_body_wins_over_the_status() {
    let server = TestServer::start(|_| {
        answer(
            400,
            "application/json",
            "{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32602,\"message\":\"bad params\"}}",
        )
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(5, "tools/list")).await;
    let message = receive(&mut streams).await;
    let (code, text) = error_of(&message);
    assert_eq!(code, -32602);
    assert_eq!(text, "bad params");
    match &message {
        JsonRpcMessage::Error(error) => assert_eq!(error.id, Some(RequestId::Number(5))),
        JsonRpcMessage::Request(_)
        | JsonRpcMessage::Notification(_)
        | JsonRpcMessage::Response(_) => {
            panic!("expected a JSON-RPC error")
        }
    }
}

#[tokio::test]
async fn a_500_without_a_json_body_is_internal_error() {
    let server = TestServer::start(|_| answer(500, "text/plain", "boom")).await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32603);
    assert_eq!(text, "Server returned an error response");
}

#[tokio::test]
async fn an_unknown_content_type_is_invalid() {
    let server = TestServer::start(|_| answer(200, "text/plain", "text")).await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32600);
    assert_eq!(text, "Unexpected content type: text/plain");
}

#[tokio::test]
async fn the_protocol_version_header_is_cleared_for_initialize() {
    let server = TestServer::start(|_| {
        answer(
            200,
            "application/json",
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}",
        )
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let SdkTransport::Streamable(http) = &transport else {
        panic!("expected the streamable transport")
    };
    http.set_protocol_version("2025-06-18");
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(1, "tools/list")).await;
    send(&mut streams, request(1, "initialize")).await;
    let requests = server.requests();
    assert_eq!(
        note_header(&requests[0], &VERSION_HEADER).as_deref(),
        Some("2025-06-18")
    );
    assert_eq!(note_header(&requests[1], &VERSION_HEADER), None);
}

#[tokio::test]
async fn an_sse_response_rewrites_the_id_and_reconnects_with_last_event_id() {
    let server = TestServer::start(|request| {
        if request.method == Method::POST {
            return streamed(vec![
                Bytes::from_static(b"event: message\nid: evt-1\ndata:\n\n"),
                Bytes::from_static(
                    b"event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{\"level\":\"info\",\"data\":\"x\"}}\n\n",
                ),
            ]);
        }
        streamed(vec![Bytes::from_static(
            b"event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":\"server-9\",\"result\":{}}\n\n",
        )])
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let notification = receive(&mut streams).await;
    assert!(matches!(notification, JsonRpcMessage::Notification(_)));
    let response = receive(&mut streams).await;
    assert_eq!(response_id(&response), Some(RequestId::Number(7)));
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].method, Method::GET);
    assert_eq!(
        note_header(&requests[1], &LAST_EVENT_HEADER).as_deref(),
        Some("evt-1")
    );
}

#[tokio::test]
async fn an_sse_event_over_the_limit_fails_the_request() {
    let server = TestServer::start(|_| {
        let payload = format!("event: message\ndata: {}\n\n", "x".repeat(1_048_557));
        streamed(vec![Bytes::from(payload)])
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32000);
    assert!(text.contains("1048576"), "{text}");
}

#[tokio::test]
async fn the_session_id_is_kept_and_the_delete_is_sent_on_close() {
    let server = TestServer::start(|request| {
        if request.method == Method::DELETE && request.uri == "/mcp" {
            return relocated(307, "/mcp2");
        }
        if request.method == Method::DELETE {
            return answer(200, "", "");
        }
        let mut response = answer(
            200,
            "application/json",
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}",
        );
        response
            .headers_mut()
            .insert(SESSION_HEADER, HeaderValue::from_static("session-1"));
        response
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(1, "initialize")).await;
    let _response = receive(&mut streams).await;
    <SdkStreams as Transport<RoleClient>>::close(&mut streams)
        .await
        .unwrap();
    let requests = server.requests();
    let delete = requests.last().unwrap();
    assert_eq!(delete.method, Method::DELETE);
    assert_eq!(delete.uri, "/mcp2");
    assert_eq!(
        note_header(delete, &SESSION_HEADER).as_deref(),
        Some("session-1")
    );
}

#[tokio::test]
async fn a_get_stream_failure_reaches_the_handler() {
    let server = TestServer::start(|request| {
        if request.method == Method::POST
            && request.body.starts_with(b"{\"jsonrpc\":\"2.0\",\"id\":1")
        {
            let mut response = answer(
                200,
                "application/json",
                "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}",
            );
            response
                .headers_mut()
                .insert(SESSION_HEADER, HeaderValue::from_static("session-1"));
            return response;
        }
        if request.method == Method::POST {
            return answer(202, "", "");
        }
        answer(500, "text/plain", "boom")
    })
    .await;
    let (mut transport, handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(1, "initialize")).await;
    let _response = receive(&mut streams).await;
    send(&mut streams, initialized()).await;
    assert!(
        <SdkStreams as Transport<RoleClient>>::receive(&mut streams)
            .await
            .is_none()
    );
    let recorded = handler.recorded.lock().unwrap().clone().unwrap();
    assert_eq!(recorded.kind(), StreamFailureKind::Http);
    let gets = server
        .requests()
        .into_iter()
        .filter(|request| request.method == Method::GET)
        .count();
    assert_eq!(gets, 2);
}

#[tokio::test]
async fn a_get_stream_delivers_server_messages() {
    let server = TestServer::start(|request| {
        if request.method == Method::POST
            && request.body.starts_with(b"{\"jsonrpc\":\"2.0\",\"id\":1")
        {
            let mut response = answer(
                200,
                "application/json",
                "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}",
            );
            response
                .headers_mut()
                .insert(SESSION_HEADER, HeaderValue::from_static("session-1"));
            return response;
        }
        if request.method == Method::POST {
            return answer(202, "", "");
        }
        streamed(vec![Bytes::from_static(
            b"event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{\"level\":\"info\",\"data\":\"x\"}}\n\n",
        )])
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(1, "initialize")).await;
    let _response = receive(&mut streams).await;
    send(&mut streams, initialized()).await;
    let message = receive(&mut streams).await;
    assert!(matches!(message, JsonRpcMessage::Notification(_)));
}

#[tokio::test]
async fn an_sse_event_at_the_size_limit_is_accepted() {
    let prefix = "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{\"level\":\"info\",\"data\":\"";
    let suffix = "\"}}";
    let padding = 1_048_556 - prefix.len() - suffix.len();
    let json = format!("{prefix}{}{suffix}", "x".repeat(padding));
    let server = TestServer::start(move |_| {
        let event = format!("event: message\ndata: {json}\n\n");
        streamed(vec![Bytes::from(event)])
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    assert!(matches!(
        receive(&mut streams).await,
        JsonRpcMessage::Notification(_)
    ));
}

#[tokio::test]
async fn an_sse_event_with_crlf_at_the_size_limit_is_accepted() {
    let prefix = "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{\"level\":\"info\",\"data\":\"";
    let suffix = "\"}}";
    let padding = 1_048_556 - prefix.len() - suffix.len();
    let json = format!("{prefix}{}{suffix}", "x".repeat(padding));
    let server = TestServer::start(move |_| {
        let event = format!("event: message\r\ndata: {json}\r\n\r\n");
        streamed(vec![Bytes::from(event)])
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    assert!(matches!(
        receive(&mut streams).await,
        JsonRpcMessage::Notification(_)
    ));
}

#[tokio::test]
async fn consecutive_events_each_count_from_zero() {
    let prefix = "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{\"level\":\"info\",\"data\":\"";
    let suffix = "\"}}";
    let padding = 600_000 - prefix.len() - suffix.len();
    let json = format!("{prefix}{}{suffix}", "x".repeat(padding));
    let server = TestServer::start(move |_| {
        let event = format!("event: message\ndata: {json}\n\nevent: message\ndata: {json}\n\n");
        streamed(vec![Bytes::from(event)])
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    assert!(matches!(
        receive(&mut streams).await,
        JsonRpcMessage::Notification(_)
    ));
    assert!(matches!(
        receive(&mut streams).await,
        JsonRpcMessage::Notification(_)
    ));
}
