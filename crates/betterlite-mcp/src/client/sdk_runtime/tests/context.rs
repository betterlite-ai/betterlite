use std::collections::BTreeMap;
use std::future::{Future, pending, ready};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use std::{env, fs, process};

use rmcp::RoleClient;
use rmcp::service::{RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::Transport;
use tokio::time::sleep;

use crate::client::sdk_runtime::handler::ServerRequestHandlers;
use crate::client::sdk_runtime::session::SdkSessionContext;
use crate::client::sdk_transport::error::SdkError;
use crate::client::sessions::negotiate::Callbacks;
use crate::client::sessions::runtime::{McpRuntime as _, SessionContext as _, SessionOptions};
use crate::client::sessions::stream::MessageHandler;
use crate::client::sessions::transport::{StdioServerParameters, TransportContext as _};

use super::support;

struct RecordingTransport {
    closed: Arc<AtomicBool>,
}

impl Transport<RoleClient> for RecordingTransport {
    type Error = SdkError;

    fn send(
        &mut self,
        _item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), SdkError>> + Send + 'static {
        ready(Ok(()))
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        pending().await
    }

    fn close(&mut self) -> impl Future<Output = Result<(), SdkError>> {
        self.closed.store(true, Ordering::SeqCst);
        ready(Ok(()))
    }
}

fn options() -> SessionOptions<SdkError> {
    SessionOptions {
        read_timeout: Duration::from_secs(5),
        callbacks: Callbacks::default(),
        handlers: ServerRequestHandlers::default(),
        message_handler: MessageHandler::default(),
    }
}

fn recording(closed: &Arc<AtomicBool>) -> SdkSessionContext<RecordingTransport> {
    SdkSessionContext::new(
        RecordingTransport {
            closed: Arc::clone(closed),
        },
        options(),
        MessageHandler::default(),
        None,
    )
}

#[tokio::test]
async fn a_session_exit_closes_the_service_before_it_returns() {
    let closed = Arc::new(AtomicBool::new(false));
    let mut context = recording(&closed);
    let session = context.enter().await.expect("the service starts");
    drop(session);
    context.exit().await.expect("the service stops");
    assert!(
        closed.load(Ordering::SeqCst),
        "the service closed its transport before exit returned"
    );
}

#[tokio::test]
async fn a_session_context_opens_only_one_service() {
    let closed = Arc::new(AtomicBool::new(false));
    let mut context = recording(&closed);
    let session = context.enter().await.expect("the service starts");
    let second = context.enter().await;
    assert!(matches!(second, Err(SdkError::AlreadyOpen)));
    drop(session);
    drop(context);
}

fn parameters(script: String) -> StdioServerParameters {
    StdioServerParameters {
        command: "/bin/sh".to_owned(),
        args: vec!["-c".to_owned(), script],
        env: BTreeMap::new(),
        defaults: BTreeMap::new(),
    }
}

async fn wait_for(path: &Path) {
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(10))
        .expect("the wait deadline fits");
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "the server never wrote its marker"
        );
        sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn a_transport_exit_stops_a_stdio_server_that_traps_term() {
    let marker = env::temp_dir().join(format!("betterlite-b9653-exit-{}.marker", process::id()));
    drop(fs::remove_file(&marker));
    let marker_path = marker.display().to_string();
    let script = format!(
        "trap 'printf done > {marker_path}; exit 0' TERM; while :; do /bin/sleep 0.1; done"
    );
    let mut context = support::runtime().stdio_client(parameters(script));
    let streams = context.enter().await.expect("the server starts");
    drop(streams);
    context.exit().await.expect("the transport exits");
    assert!(marker.exists(), "the server did not finish on SIGTERM");
    drop(fs::remove_file(&marker));
}

#[tokio::test]
async fn closing_the_session_streams_ends_the_stdio_server_input() {
    let marker = env::temp_dir().join(format!(
        "betterlite-b9653-stream-close-{}.marker",
        process::id()
    ));
    drop(fs::remove_file(&marker));
    let marker_path = marker.display().to_string();
    let script = format!("/bin/cat > /dev/null; printf done > {marker_path}");
    let mut context = support::runtime().stdio_client(parameters(script));
    let mut streams = context.enter().await.expect("the server starts");
    streams.close().await.expect("the streams close");
    wait_for(&marker).await;
    drop(streams);
    context.exit().await.expect("the transport exits");
    drop(fs::remove_file(&marker));
}
