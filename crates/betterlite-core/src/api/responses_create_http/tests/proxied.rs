use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::base_builder;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use http::Method;
use http::header::HeaderMap;
use url::Url;

use super::collect_payloads;
use crate::api::responses_create_http::service::{CreateRequest, CreateTransport, SentReply};

// The pooled client the handler builds when the caller supplies none, against a real HTTP/1.1 fixture
fn proxied_client() -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    ProxiedClient::build(
        base_builder().http1_only(),
        &policy,
        TransportLimits::DEFAULT,
    )
    .expect("the proxied client builds")
}

fn request(address: SocketAddr) -> CreateRequest {
    CreateRequest {
        method: Method::POST,
        url: Url::parse(&format!("http://{address}/v1/responses")).expect("the fixture URL"),
        headers: HeaderMap::new(),
        body: RequestBody::Json(Vec::new()),
        timeout: Duration::from_secs(5),
    }
}

// A 2xx streaming send hands the body over as SSE payloads
#[tokio::test]
async fn create_proxied_send_streams_sse() {
    let client = proxied_client();
    let streaming = serve(
        reply(
            "200 OK",
            &[("content-type", "text/event-stream")],
            "data: hello\n\ndata: [DONE]\n\n",
        ),
        Duration::ZERO,
    )
    .expect("the fixture serves");
    let SentReply::Streaming(mut source) =
        CreateTransport::send(&client, request(streaming.address), true)
            .await
            .expect("the streaming send")
    else {
        panic!("a 2xx streaming send hands the body over")
    };
    assert_eq!(collect_payloads(&mut source).await, vec!["hello", "[DONE]"]);
    let seen = streaming.finish().expect("the fixture request");
    assert_eq!(seen.line, "POST /v1/responses HTTP/1.1");
}

// A non-streaming send buffers the whole reply
#[tokio::test]
async fn create_proxied_send_buffers_json() {
    let client = proxied_client();
    let accepted = serve(
        reply(
            "200 OK",
            &[("content-type", "application/json")],
            r#"{"id":"resp_a"}"#,
        ),
        Duration::ZERO,
    )
    .expect("the fixture serves");
    let SentReply::Buffered(accepted_answer) =
        CreateTransport::send(&client, request(accepted.address), false)
            .await
            .expect("the buffered send")
    else {
        panic!("a non-streaming send buffers the reply")
    };
    assert_eq!(accepted_answer.status, 200);
    assert_eq!(accepted_answer.body, br#"{"id":"resp_a"}"#.to_vec());
    accepted.finish().expect("the fixture request");
}

// A non-2xx answer is buffered even when the caller asked for a stream
#[tokio::test]
async fn create_proxied_send_buffers_errors() {
    let client = proxied_client();
    let failed = serve(
        reply(
            "500 Internal Server Error",
            &[("content-type", "application/json")],
            r#"{"error":"boom"}"#,
        ),
        Duration::ZERO,
    )
    .expect("the fixture serves");
    let SentReply::Buffered(failed_answer) =
        CreateTransport::send(&client, request(failed.address), true)
            .await
            .expect("the streaming error send")
    else {
        panic!("a non-2xx streaming send buffers the reply")
    };
    assert_eq!(failed_answer.status, 500);
    assert_eq!(failed_answer.body, br#"{"error":"boom"}"#.to_vec());
    failed.finish().expect("the fixture request");
}
