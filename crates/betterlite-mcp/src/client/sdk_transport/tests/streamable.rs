use std::any::TypeId;

use rmcp::ServiceError;
use rmcp::transport::{DynamicTransportError, Transport as _};

use crate::client::cleanup::http_client::ResponseHead;
use crate::client::http_transport::capture::CaptureRecord;
use crate::client::http_transport::error::McpHttpError;
use crate::client::sdk_transport::error::SdkError;
use crate::client::sessions::error::SessionError;
use crate::client::tools::result::describe_failure;

use super::raw_server::{MISSING_BYTES, RawResponse, RawTestServer};
use super::*;

fn status_of(error: &SdkError) -> u16 {
    let SdkError::Http(http) = error else {
        panic!("expected an HTTP status error, got {error}");
    };
    let McpHttpError::Status(head) = http.as_ref() else {
        panic!("expected an HTTP status error, got {error}");
    };
    head.status
}

fn initializing_server<Reply>(
    reply: Reply,
) -> impl Fn(&RecordedRequest) -> Response + Send + Sync + 'static
where
    Reply: Fn(&RecordedRequest) -> Response + Send + Sync + 'static,
{
    move |request| {
        if request.method == Method::POST
            && request.body.starts_with(b"{\"jsonrpc\":\"2.0\",\"id\":1")
        {
            return with_session(answer(
                200,
                "application/json",
                "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}",
            ));
        }
        reply(request)
    }
}

fn streamable_event(data: &'static str) -> Bytes {
    Bytes::from(format!("event: message\ndata: {data}\n\n"))
}

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
    assert_eq!(requests[1].method, Method::POST);
    assert_eq!(requests[1].uri, "/moved");
}

#[tokio::test]
async fn a_302_on_a_post_is_not_followed_and_names_the_location() {
    let server = TestServer::start(|_| relocated(302, "/elsewhere")).await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32_600_i32);
    assert_eq!(
        text,
        format!("(redirected to {}; not followed)", server.url("/elsewhere"))
    );
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn a_twenty_first_redirect_is_not_followed() {
    let server = TestServer::start(|_| relocated(307, "/mcp")).await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32_600_i32);
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
    assert_eq!(code, -32_600_i32);
    assert_eq!(text, "server answered a request with 202 Accepted");
}

#[tokio::test]
async fn a_404_before_a_session_is_method_not_found() {
    let server = TestServer::start(|_| answer(404, "text/plain", "missing")).await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32_601_i32);
    assert_eq!(text, "Not Found");
}

#[tokio::test]
async fn a_404_after_a_session_is_session_terminated() {
    let server = TestServer::start(initializing_server(|_| {
        answer(404, "text/plain", "missing")
    }))
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
    assert_eq!(code, -32_600_i32);
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
    assert_eq!(code, -32_602_i32);
    assert_eq!(text, "bad params");
    let JsonRpcMessage::Error(error) = &message else {
        panic!("expected a JSON-RPC error")
    };
    assert_eq!(error.id, Some(RequestId::Number(5)));
}

#[tokio::test]
async fn a_json_body_that_is_not_a_json_rpc_error_is_a_status_error() {
    let server = TestServer::start(|_| {
        answer(
            400,
            "application/json",
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}",
        )
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    let error = streams.send(request(7, "tools/list")).await.err().unwrap();
    assert_eq!(status_of(&error), 400);
}

#[tokio::test]
async fn a_cut_error_body_is_a_status_error() {
    let server = RawTestServer::start(Arc::new(|_: &RecordedRequest| {
        RawResponse::new(
            500,
            "application/json",
            b"{\"jsonrpc\":\"2.0\",\"id\":1,",
            MISSING_BYTES,
        )
    }))
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    let error = streams.send(request(7, "tools/list")).await.err().unwrap();
    assert_eq!(status_of(&error), 500);
}

#[tokio::test]
async fn a_500_without_a_json_body_is_a_status_error() {
    let server = TestServer::start(|_| answer(500, "text/plain", "boom")).await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    let error = streams.send(request(7, "tools/list")).await.err().unwrap();
    assert_eq!(status_of(&error), 500);
}

#[test]
fn a_transport_send_status_error_is_described_as_the_http_status_error() {
    let head = ResponseHead {
        status: 401,
        headers: HeaderMap::new(),
        capture: Some(CaptureRecord {
            exchange: Exchange {
                method: Method::POST,
                url: Url::parse("http://mcp.test/mcp").unwrap(),
                request_headers: HeaderMap::new(),
                request_body: Bytes::new(),
                status: 401,
                response_headers: HeaderMap::new(),
            },
            preview: "denied".to_owned(),
        }),
    };
    let dynamic = DynamicTransportError::from_parts(
        "streamable-http",
        TypeId::of::<()>(),
        Box::new(SdkError::from(McpHttpError::Status(Box::new(head)))),
    );
    let error: SessionError<SdkError> = SessionError::Runtime(Arc::new(SdkError::Service(
        ServiceError::TransportSend(dynamic),
    )));
    let failure = describe_failure(&error);
    assert_eq!(failure.error_type, "HTTPStatusError");
    assert_eq!(
        failure.message,
        "Client error '401 Unauthorized' for url 'http://mcp.test/mcp'\nFor more information check: https://developer.mozilla.org/en-US/docs/Web/HTTP/Status/401"
    );
}

#[test]
fn a_direct_status_error_is_described_as_the_http_status_error() {
    let error: SessionError<SdkError> = SessionError::Runtime(Arc::new(SdkError::from(
        McpHttpError::Status(Box::new(ResponseHead {
            status: 401,
            headers: HeaderMap::new(),
            capture: None,
        })),
    )));
    let failure = describe_failure(&error);
    assert_eq!(failure.error_type, "HTTPStatusError");
    assert_eq!(
        failure.message,
        "Client error '401 Unauthorized' for url ''\nFor more information check: https://developer.mozilla.org/en-US/docs/Web/HTTP/Status/401"
    );
}

#[test]
fn a_runtime_error_that_is_not_a_status_error_keeps_its_type_name() {
    let error: SessionError<SdkError> = SessionError::Runtime(Arc::new(SdkError::SessionClosed));
    let failure = describe_failure(&error);
    assert_eq!(failure.error_type, "SdkError");
    assert_eq!(failure.message, "the MCP session is closed");
}

#[tokio::test]
async fn an_unknown_content_type_is_invalid() {
    let server = TestServer::start(|_| answer(200, "text/plain", "text")).await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32_600_i32);
    assert_eq!(text, "Unexpected content type: text/plain");
}

#[tokio::test]
async fn an_empty_content_type_is_invalid() {
    let server = TestServer::start(|_| answer(200, "", "")).await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32_600_i32);
    assert_eq!(text, "Unexpected content type: ");
}

#[tokio::test]
async fn a_json_body_that_is_not_json_rpc_is_a_parse_error() {
    let server = TestServer::start(|_| answer(200, "application/json", "not-json")).await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32_700_i32);
    assert!(
        text.starts_with("Failed to parse JSON response: "),
        "{text}"
    );
}

#[tokio::test]
async fn a_cut_json_body_is_a_parse_error() {
    let server = RawTestServer::start(Arc::new(|_: &RecordedRequest| {
        RawResponse::new(
            200,
            "application/json",
            b"{\"jsonrpc\":\"2.0\",\"id\":1,",
            MISSING_BYTES,
        )
    }))
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32_700_i32);
    assert!(
        text.starts_with("Failed to parse JSON response: "),
        "{text}"
    );
    assert_eq!(
        text,
        "Failed to parse JSON response: the MCP HTTP response body could not be read"
    );
}

#[tokio::test]
async fn an_sse_message_that_is_not_json_rpc_is_a_parse_error() {
    let server =
        TestServer::start(|_| streamed(vec![Bytes::from_static(b"data: not-json\n\n")])).await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32_700_i32);
    assert!(text.starts_with("Failed to parse SSE message: "), "{text}");
}

#[tokio::test]
async fn a_text_event_stream_media_type_with_parameters_is_sse() {
    let server = TestServer::start(|_| {
        let mut response = streamed(vec![streamable_event(
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{\"level\":\"info\",\"data\":\"x\"}}",
        )]);
        response.headers_mut().insert(
            CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream; charset=utf-8"),
        );
        response
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
async fn a_text_event_stream_suffix_is_not_sse() {
    let server = TestServer::start(|_| answer(200, "text/event-streaming", "x")).await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32_000_i32);
    assert_eq!(
        text,
        "SSE stream failed: Expected response with content type 'text/event-stream', got 'text/event-streaming'."
    );
}

#[tokio::test]
async fn an_sse_stream_that_ends_without_an_id_answers_connection_closed() {
    let server =
        TestServer::start(|_| streamed(vec![Bytes::from_static(b"event: message\ndata:\n\n")]))
            .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32_000_i32);
    assert_eq!(text, "SSE stream ended without a response");
    assert_eq!(server.count(&Method::GET), 0);
}

#[tokio::test]
async fn reconnection_gives_up_after_two_attempts() {
    let server = TestServer::start(|request| {
        if request.method == Method::POST {
            return streamed(vec![Bytes::from_static(
                b"event: message\nid: evt-1\nretry: 0\ndata:\n\n",
            )]);
        }
        answer(500, "text/plain", "boom")
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32_000_i32);
    assert_eq!(
        text,
        "SSE stream ended and reconnection attempts were exhausted"
    );
    assert_eq!(server.count(&Method::GET), 2);
}

#[tokio::test]
async fn a_reconnection_that_is_not_sse_keeps_retrying() {
    let server = TestServer::start(|request| {
        if request.method == Method::POST {
            return streamed(vec![Bytes::from_static(
                b"event: message\nid: evt-1\nretry: 0\ndata:\n\n",
            )]);
        }
        answer(200, "application/json", "{}")
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32_000_i32);
    assert_eq!(
        text,
        "SSE stream ended and reconnection attempts were exhausted"
    );
    assert_eq!(server.count(&Method::GET), 2);
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

#[tokio::test(start_paused = true)]
async fn an_sse_response_rewrites_the_id_and_reconnects_with_last_event_id() {
    let server = TestServer::start(|request| {
        if request.method == Method::POST {
            return streamed(vec![
                Bytes::from_static(b"event: message\nid: evt-1\ndata:\n\n"),
                streamable_event(
                    "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{\"level\":\"info\",\"data\":\"x\"}}",
                ),
            ]);
        }
        streamed(vec![streamable_event(
            "{\"jsonrpc\":\"2.0\",\"id\":\"server-9\",\"result\":{}}",
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
    assert_eq!(code, -32_000_i32);
    assert_eq!(
        text,
        "SSE stream failed: Server-sent event exceeded the 1048576 byte limit."
    );
}

#[tokio::test]
async fn an_sse_event_with_three_data_lines_over_the_limit_fails_the_request() {
    let json = event_json(400_000);
    let server = TestServer::start(move |_| {
        streamed(vec![Bytes::from(format!(
            "event: message\ndata: {json}\ndata: {json}\ndata: {json}\n\n"
        ))])
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32_000_i32);
    assert_eq!(
        text,
        "SSE stream failed: Server-sent event exceeded the 1048576 byte limit."
    );
}

#[tokio::test]
async fn a_crlf_event_with_three_data_lines_over_the_limit_fails_the_request() {
    let json = event_json(400_000);
    let server = TestServer::start(move |_| {
        streamed(vec![Bytes::from(format!(
            "event: message\r\ndata: {json}\r\ndata: {json}\r\ndata: {json}\r\n\r\n"
        ))])
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32_000_i32);
    assert_eq!(
        text,
        "SSE stream failed: Server-sent event exceeded the 1048576 byte limit."
    );
}

#[tokio::test]
async fn an_sse_event_at_the_size_limit_is_accepted() {
    let json = event_json(1_048_556);
    let server = TestServer::start(move |_| {
        streamed(vec![Bytes::from(format!(
            "event: message\ndata: {json}\n\n"
        ))])
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
    let json = event_json(1_048_556);
    let server = TestServer::start(move |_| {
        streamed(vec![Bytes::from(format!(
            "event: message\r\ndata: {json}\r\n\r\n"
        ))])
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
    let json = event_json(600_000);
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

#[tokio::test]
async fn the_session_id_is_kept_and_the_delete_is_sent_on_close() {
    let server = TestServer::start(|request| {
        if request.method == Method::DELETE && request.uri == "/mcp" {
            return relocated(307, "/mcp2");
        }
        if request.method == Method::DELETE {
            return answer(200, "", "");
        }
        with_session(answer(
            200,
            "application/json",
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}",
        ))
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(1, "initialize")).await;
    let _response = receive(&mut streams).await;
    streams.close().await.unwrap();
    let requests = server.requests();
    let delete = requests.last().unwrap();
    assert_eq!(delete.method, Method::DELETE);
    assert_eq!(delete.uri, "/mcp2");
    assert_eq!(
        note_header(delete, &SESSION_HEADER).as_deref(),
        Some("session-1")
    );
}

#[tokio::test(start_paused = true)]
async fn a_get_stream_failure_reaches_the_handler() {
    let server = TestServer::start(initializing_server(|request| {
        if request.method == Method::POST {
            answer(202, "", "")
        } else {
            answer(500, "text/plain", "boom")
        }
    }))
    .await;
    let (mut transport, handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(1, "initialize")).await;
    let _response = receive(&mut streams).await;
    send(&mut streams, initialized()).await;
    assert!(receive_end(&mut streams).await);
    let recorded = handler.recorded.lock().unwrap().clone().unwrap();
    assert_eq!(recorded.kind(), StreamFailureKind::Http);
    assert_eq!(server.count(&Method::GET), 2);
}

#[tokio::test]
async fn a_get_stream_delivers_server_messages() {
    let server = TestServer::start(initializing_server(|request| {
        if request.method == Method::POST {
            return answer(202, "", "");
        }
        streamed(vec![streamable_event(
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{\"level\":\"info\",\"data\":\"x\"}}",
        )])
    }))
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(1, "initialize")).await;
    let _response = receive(&mut streams).await;
    send(&mut streams, initialized()).await;
    let message = receive(&mut streams).await;
    assert!(matches!(message, JsonRpcMessage::Notification(_)));
    let requests = server.requests();
    let get = requests.last().unwrap();
    assert_eq!(get.method, Method::GET);
    assert_eq!(
        note_header(get, &SESSION_HEADER).as_deref(),
        Some("session-1")
    );
    assert_eq!(
        note_header(get, &ACCEPT).as_deref(),
        Some("application/json, text/event-stream")
    );
    assert_eq!(
        note_header(get, &CACHE_CONTROL).as_deref(),
        Some("no-store")
    );
}

#[tokio::test]
async fn a_get_stream_retries_with_the_last_event_id() {
    let server = TestServer::start(initializing_server(|request| {
        if request.method == Method::POST {
            return answer(202, "", "");
        }
        if request.headers.contains_key(LAST_EVENT_HEADER) {
            return answer(500, "text/plain", "boom");
        }
        streamed(vec![Bytes::from_static(
            b"id: g-1\nretry: 0\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{\"level\":\"info\",\"data\":\"x\"}}\n\n",
        )])
    }))
    .await;
    let (mut transport, handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(1, "initialize")).await;
    let _response = receive(&mut streams).await;
    send(&mut streams, initialized()).await;
    assert!(matches!(
        receive(&mut streams).await,
        JsonRpcMessage::Notification(_)
    ));
    assert!(receive_end(&mut streams).await);
    let recorded = handler.recorded.lock().unwrap().clone().unwrap();
    assert_eq!(recorded.kind(), StreamFailureKind::Http);
    let gets: Vec<Option<String>> = server
        .requests()
        .iter()
        .filter(|request| request.method == Method::GET)
        .map(|request| note_header(request, &LAST_EVENT_HEADER))
        .collect();
    let resumed = Some("g-1".to_owned());
    assert_eq!(gets, vec![None, resumed.clone(), resumed]);
}

#[tokio::test]
async fn a_get_stream_parse_error_is_a_value_error() {
    let server = TestServer::start(initializing_server(|request| {
        if request.method == Method::POST {
            return answer(202, "", "");
        }
        streamed(vec![Bytes::from_static(b"data: not-json\n\n")])
    }))
    .await;
    let (mut transport, handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(1, "initialize")).await;
    let _response = receive(&mut streams).await;
    send(&mut streams, initialized()).await;
    assert!(receive_end(&mut streams).await);
    let recorded = handler.recorded.lock().unwrap().clone().unwrap();
    assert_eq!(recorded.kind(), StreamFailureKind::ValueError);
    assert_eq!(server.count(&Method::GET), 1);
}

#[tokio::test(start_paused = true)]
async fn a_get_stream_with_a_non_sse_body_is_retried() {
    let server = TestServer::start(initializing_server(|request| {
        if request.method == Method::POST {
            return answer(202, "", "");
        }
        answer(200, "application/json", "{}")
    }))
    .await;
    let (mut transport, handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(1, "initialize")).await;
    let _response = receive(&mut streams).await;
    send(&mut streams, initialized()).await;
    assert!(receive_end(&mut streams).await);
    let recorded = handler.recorded.lock().unwrap().clone().unwrap();
    assert_eq!(recorded.kind(), StreamFailureKind::Http);
    assert_eq!(server.count(&Method::GET), 2);
}

#[tokio::test(start_paused = true)]
async fn a_redirected_get_stream_is_not_opened() {
    let server = TestServer::start(initializing_server(|request| {
        if request.method == Method::POST {
            return answer(202, "", "");
        }
        relocated(302, "http://other.test/mcp")
    }))
    .await;
    let (mut transport, handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(1, "initialize")).await;
    let _response = receive(&mut streams).await;
    send(&mut streams, initialized()).await;
    let pending = timeout(Duration::from_millis(2000), streams.receive()).await;
    assert!(
        pending.is_err(),
        "a redirected GET stream must not resolve receive"
    );
    assert_eq!(server.count(&Method::GET), 1);
    assert!(handler.recorded.lock().unwrap().is_none());
}
