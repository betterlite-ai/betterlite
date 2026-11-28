use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::Json as PromptJson;
use http::Method;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::time::timeout;

use super::super::transport::{HubGetAsync as _, Pooled, WireRequest};
use super::super::{DispatchDeps, HubCompletion, HubDispatchError, predibase};
use super::support::{
    Arm as _, Services, chat_context, empty_globals, environment, object, user_message,
};

const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);

type Exchange = (
    String,
    Result<Option<HubCompletion>, HubDispatchError>,
    LoggingState,
);

fn client() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

// answers one request on the listener with the given status line and body, and returns the request text
async fn serve_once(listener: &TcpListener, status: &str, body: &str) -> String {
    let (mut stream, _peer) = listener.accept().await.unwrap();
    let mut request = Vec::new();
    loop {
        let mut chunk = [0_u8; 4096];
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        request.extend_from_slice(&chunk[..count]);
        let text = String::from_utf8(request.clone()).unwrap();
        if let Some((head, sent)) = text.split_once("\r\n\r\n") {
            let length = head
                .to_ascii_lowercase()
                .lines()
                .find_map(|line| line.strip_prefix("content-length: ")?.parse::<usize>().ok())
                .unwrap_or(0);
            if sent.len() >= length {
                break;
            }
        }
    }
    let reply = format!(
        "HTTP/1.1 {status}\r\nx-hub: 1\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(reply.as_bytes()).await.unwrap();
    String::from_utf8(request).unwrap()
}

// opens a Predibase stream against a local server that answers with the given status
async fn predibase_stream(status: &str, acompletion: bool) -> Exchange {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let client = client();
    let pooled = Pooled { client: &client };
    let services = Services::new(Vec::new(), Vec::new(), Vec::new());
    let globals = empty_globals();
    let env = environment(&[("PREDIBASE_API_KEY", "pb-env")]);
    let deps = DispatchDeps {
        post: &pooled,
        ..services.deps(&globals, &env)
    };
    let (mut state, log_env) = services.logging();
    let mut log = LogContext {
        state: &mut state,
        env: &log_env,
    };
    let mut messages = vec![user_message()];
    let stream = PromptJson::Bool(true);
    let tenant = PromptJson::Text(String::from("tenant-1"));
    let mut options = object(&[("stream", stream), ("tenant_id", tenant)]);
    let mut call = chat_context("m", &mut messages, &mut options, Some(base.as_str()));
    call.acompletion = acompletion;
    let opened = (ChatConfigTag::Predibase, &mut call).run(&deps, &mut log);
    let exchange = async { tokio::join!(serve_once(&listener, status, "boom"), opened) };
    let (request, outcome) = timeout(EXCHANGE_TIMEOUT, exchange).await.unwrap();
    (request, outcome, state)
}

#[tokio::test]
async fn the_pooled_hub_returns_the_status_headers_and_body() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/api/models/m", listener.local_addr().unwrap());
    let client = client();
    let pooled = Pooled { client: &client };
    let headers = Headers::from([(String::from("Accept"), String::from("application/json"))]);
    let request = WireRequest {
        method: Method::GET,
        url: &url,
        query: &[("expand", "inferenceProviderMapping")],
        headers: &headers,
        body: None,
        deadline: EXCHANGE_TIMEOUT,
    };
    let exchange = async {
        tokio::join!(
            serve_once(&listener, "404 Not Found", r#"{"error":"missing"}"#),
            pooled.get(&request)
        )
    };
    let (sent, answer) = timeout(EXCHANGE_TIMEOUT, exchange).await.unwrap();
    let response = answer.unwrap();
    assert_eq!(response.status_code, 404);
    assert_eq!(response.headers.get("x-hub").map(String::as_str), Some("1"));
    assert_eq!(response.body, br#"{"error":"missing"}"#);
    let line = "GET /api/models/m?expand=inferenceProviderMapping HTTP/1.1\r\n";
    assert!(sent.starts_with(line));
    assert!(
        sent.to_ascii_lowercase()
            .contains("accept: application/json")
    );
}

#[tokio::test]
async fn the_async_predibase_stream_fails_on_a_status_other_than_200() {
    let (request, outcome, state) = predibase_stream("500 Internal Server Error", true).await;
    let Err(HubDispatchError::Predibase(error)) = outcome else {
        panic!("a failed stream answer is a Predibase error");
    };
    assert_eq!(error.status_code, 500);
    assert_eq!(error.message, "boom");
    let line = "POST /tenant-1/deployments/v2/llms/m/generate_stream HTTP/1.1\r\n";
    assert!(request.starts_with(line));
    assert!(request.contains(r#""stream":true"#));
    let event = state.model_call_details.get("log_event_type");
    assert_eq!(event, Some(&Json::Text(String::from("pre_api_call"))));
}

#[tokio::test]
async fn the_async_predibase_stream_logs_the_post_call_on_200() {
    let (_request, outcome, state) = predibase_stream("200 OK", true).await;
    let Some(HubCompletion::Predibase(completion)) = outcome.unwrap() else {
        panic!("the predibase tag returns a predibase completion");
    };
    assert!(matches!(*completion, predibase::Completion::Stream(_)));
    let event = state.model_call_details.get("log_event_type");
    assert_eq!(event, Some(&Json::Text(String::from("post_api_call"))));
    let original = state.model_call_details.get("original_response");
    assert_eq!(original, Some(&Json::Null));
}

#[tokio::test]
async fn the_sync_predibase_stream_fails_on_a_status_other_than_200() {
    let (request, outcome, _logged) = predibase_stream("500 Internal Server Error", false).await;
    let Err(HubDispatchError::Predibase(error)) = outcome else {
        panic!("a failed sync stream answer is a Predibase error");
    };
    assert_eq!(error.status_code, 500);
    assert_eq!(error.message, "boom");
    let line = "POST /tenant-1/deployments/v2/llms/m/generate_stream HTTP/1.1\r\n";
    assert!(request.starts_with(line));
    assert!(!request.contains(r#""stream""#));
}

#[tokio::test]
async fn the_sync_predibase_stream_returns_the_stream_on_200_without_a_post_call() {
    let (_request, outcome, state) = predibase_stream("200 OK", false).await;
    let Some(HubCompletion::Predibase(completion)) = outcome.unwrap() else {
        panic!("the predibase tag returns a predibase completion");
    };
    assert!(matches!(*completion, predibase::Completion::Stream(_)));
    let event = state.model_call_details.get("log_event_type");
    assert_eq!(event, Some(&Json::Text(String::from("pre_api_call"))));
}

#[test]
fn the_wire_request_carries_its_parts_to_the_outbound_request() {
    let headers = Headers::from([(String::from("x-call"), String::from("1"))]);
    let request = WireRequest {
        method: Method::POST,
        url: "http://mock/path",
        query: &[("a", "b")],
        headers: &headers,
        body: Some(b"{}".as_slice()),
        deadline: EXCHANGE_TIMEOUT,
    };
    let outbound = request.outbound().unwrap();
    assert_eq!(outbound.method, Method::POST);
    assert_eq!(outbound.url.as_str(), "http://mock/path");
    assert_eq!(outbound.query, vec![(String::from("a"), String::from("b"))]);
    assert_eq!(outbound.headers.get("x-call").unwrap(), "1");
    assert_eq!(outbound.deadline, EXCHANGE_TIMEOUT);
}
