#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use tokio::io::AsyncWriteExt as _;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::sleep;

use crate::thread_runs::error::ThreadRunsError;
use crate::thread_runs::operations::{RunCall, RunDispatch, RunInvocation, RunsApi};
use crate::thread_runs::records::RunRequest;
use crate::thread_runs::stream::RunStream;

use super::support::{KEY, options};

const REQUEST: &str = r#"{"thread_id":"thread_a","assistant_id":"asst_a","additional_instructions":"extra","instructions":"base","metadata":{"tag":"x"},"model":"gpt-4o-mini","tools":[{"type":"code_interpreter"}]}"#;

fn request() -> RunRequest {
    serde_json::from_str(REQUEST).unwrap()
}

fn proxied_call(base: &str) -> RunCall<ProxiedClient> {
    let mut options = options(KEY);
    options.api_base = Some(format!("{base}/v1"));
    options.timeout = Some(Duration::from_millis(200));
    RunCall {
        options,
        client: None,
        dispatch: RunDispatch {
            arun_thread: false,
            stream: true,
        },
        handler: None,
    }
}

async fn serve_stream_script(listener: TcpListener) {
    let (mut complete, _) = listener.accept().await.unwrap();
    complete
        .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-type: application/json\r\ncontent-length: 9\r\nconnection: close\r\n\r\nnot found")
        .await
        .unwrap();
    let (_silent, _) = listener.accept().await.unwrap();
    let (mut error_head, _) = listener.accept().await.unwrap();
    error_head
        .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 64\r\n\r\n")
        .await
        .unwrap();
    let (mut body_head, _) = listener.accept().await.unwrap();
    body_head
        .write_all(
            b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: 64\r\n\r\n",
        )
        .await
        .unwrap();
    let (mut truncated, _) = listener.accept().await.unwrap();
    truncated
        .write_all(
            b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: 64\r\nconnection: close\r\n\r\nevent: x\n",
        )
        .await
        .unwrap();
    truncated.shutdown().await.unwrap();
    sleep(Duration::from_secs(30)).await;
}

async fn open_stream(api: &RunsApi<ProxiedClient>, base: &str) -> Box<RunStream<ProxiedClient>> {
    let outcome = api
        .run_thread(proxied_call(base), &request())
        .await
        .unwrap();
    let RunInvocation::Streaming(stream) = outcome else {
        panic!("expected a stream manager");
    };
    stream
}

async fn stream_status_error(api: &RunsApi<ProxiedClient>, base: &str) {
    let failure = open_stream(api, base).await.consume().await.err().unwrap();
    assert!(
        matches!(&failure, ThreadRunsError::Status(404, body) if body == b"not found"),
        "{failure}"
    );
}

async fn stalled_headers(api: &RunsApi<ProxiedClient>, base: &str) {
    let error = open_stream(api, base).await.events().await.err().unwrap();
    assert!(matches!(error, ThreadRunsError::Timeout), "{error}");
}

async fn stalled_error_body(api: &RunsApi<ProxiedClient>, base: &str) {
    let error = open_stream(api, base).await.events().await.err().unwrap();
    assert!(matches!(error, ThreadRunsError::Timeout), "{error}");
}

async fn stalled_body(api: &RunsApi<ProxiedClient>, base: &str) {
    let mut stream = open_stream(api, base).await;
    stream.request.deadline = Duration::from_millis(100);
    stream.read_timeout = Duration::from_secs(30);
    let error = stream.consume().await.err().unwrap();
    assert!(matches!(error, ThreadRunsError::Timeout), "{error}");
}

// A body cut short before its content length is a transport failure, not the read timeout.
async fn truncated_body(api: &RunsApi<ProxiedClient>, base: &str) {
    let mut call = proxied_call(base);
    call.options.timeout = Some(Duration::from_secs(5));
    let outcome = api.run_thread(call, &request()).await.unwrap();
    let RunInvocation::Streaming(stream) = outcome else {
        panic!("expected a stream manager");
    };
    let error = stream.consume().await.err().unwrap();
    assert!(matches!(error, ThreadRunsError::Transport(_)), "{error}");
}

// The production transport raises a status error for a 404 and the caller timeout for each stalled stage.
#[tokio::test]
async fn proxied_stream_transport() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_stream_script(listener));
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let api = RunsApi {
        transport: Arc::new(ProxiedClient::outbound(&policy).unwrap()),
    };
    stream_status_error(&api, &base).await;
    stalled_headers(&api, &base).await;
    stalled_error_body(&api, &base).await;
    stalled_body(&api, &base).await;
    truncated_body(&api, &base).await;
    tasks.abort_all();
}
