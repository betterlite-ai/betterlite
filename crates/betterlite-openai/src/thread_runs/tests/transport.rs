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
    let (mut first, _) = listener.accept().await.unwrap();
    first
        .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-type: application/json\r\ncontent-length: 9\r\nconnection: close\r\n\r\nnot found")
        .await
        .unwrap();
    let (mut second, _) = listener.accept().await.unwrap();
    second
        .write_all(
            b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: 64\r\n\r\n",
        )
        .await
        .unwrap();
    sleep(Duration::from_secs(30)).await;
}

// The production transport turns a stream 404 into a status error and a stalled body into a timeout.
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
    let outcome = api
        .run_thread(proxied_call(&base), &request())
        .await
        .unwrap();
    let RunInvocation::Streaming(stream) = outcome else {
        panic!("expected a stream manager");
    };
    let failure = stream.consume().await.err().unwrap();
    assert!(
        matches!(&failure, ThreadRunsError::Status(404, body) if body == b"not found"),
        "{failure}"
    );
    let stalled = api
        .run_thread(proxied_call(&base), &request())
        .await
        .unwrap();
    let RunInvocation::Streaming(stalled_stream) = stalled else {
        panic!("expected a stream manager");
    };
    let timeout_error = stalled_stream.consume().await.err().unwrap();
    assert!(
        matches!(timeout_error, ThreadRunsError::Timeout),
        "{timeout_error}"
    );
    tasks.abort_all();
}
