use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::client::http_transport::capture::{BodyChunks, CapturedBody, ErrorCapture, Exchange};
use crate::client::http_transport::send::HttpBase;
use crate::client::sdk_runtime::handler::ServerRequestHandlers;
use crate::client::sdk_runtime::runtime::SdkRuntime;
use crate::client::sessions::client::{SessionClient, SessionSettings};
use crate::client::sessions::negotiate::{Callbacks, ProtocolVersion};
use crate::client::sessions::runtime::McpRuntime;
use crate::client::sessions::stdio::SDK_SHUTDOWN;
use crate::client::sessions::transport::{StdioConfig, TransportType};
use crate::client::tools::progress::ProgressHost;
use crate::credentials::{McpAuthType, McpClient as CredentialClient};

pub(crate) const CALL_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct NoCapture;

impl ErrorCapture for NoCapture {
    fn capture<'body>(
        &'body self,
        _exchange: Exchange,
        _body: BodyChunks<'body>,
    ) -> Pin<Box<dyn Future<Output = CapturedBody> + Send + 'body>> {
        Box::pin(async {
            CapturedBody {
                retained: Vec::new(),
                preview: String::new(),
            }
        })
    }
}

pub(crate) fn tls() -> TlsPolicy {
    TlsPolicy {
        verification: Verification::Enabled,
        ca_file: None,
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: "betterlite-test".to_owned(),
    }
}

pub(crate) fn base() -> HttpBase {
    let trust = ProxyTrust::default();
    HttpBase {
        proxy: Arc::new(ProxyPolicy::resolve(&trust, &BTreeMap::new()).unwrap()),
        tls: tls(),
    }
}

pub(crate) fn runtime() -> SdkRuntime {
    SdkRuntime::new(
        base(),
        Arc::new(NoCapture),
        TaskTracker::new(),
        SDK_SHUTDOWN,
    )
}

pub(crate) fn settings(transport_type: TransportType) -> SessionSettings {
    SessionSettings {
        transport_type,
        stdio: Some(StdioConfig {
            command: "/bin/cat".to_owned(),
            args: Vec::new(),
            env: None,
        }),
        environment: BTreeMap::new(),
        npm_cache_dir: String::new(),
        tls: tls(),
        protocol_version: ProtocolVersion::Auto,
        client_version: "1.105.1".to_owned(),
        callbacks: Callbacks::default(),
        handlers: ServerRequestHandlers::default(),
    }
}

pub(crate) fn credentials(server_url: String, timeout: Duration) -> CredentialClient {
    CredentialClient {
        server_url,
        auth_type: McpAuthType::None,
        auth_header_name: None,
        timeout,
        extra_headers: BTreeMap::new(),
        resolved_auth: None,
        aws_auth: None,
        boxed_aws_auth: None,
        auth_value: None,
        credential_slot: None,
    }
}

pub(crate) fn client_on<Runtime>(
    runtime: Arc<Runtime>,
    settings: SessionSettings,
    url: &str,
    timeout: Duration,
) -> SessionClient<Runtime>
where
    Runtime: McpRuntime,
{
    let auth = credentials(url.to_owned(), timeout);
    SessionClient {
        runtime,
        auth,
        settings,
        tracker: TaskTracker::new(),
        caller_cancelled: CancellationToken::new(),
        last_initialize_instructions: None,
    }
}

#[derive(Default)]
pub(crate) struct RecordingHost {
    seen: Mutex<Vec<(f64, Option<f64>)>>,
}

impl RecordingHost {
    pub(crate) fn seen(&self) -> Vec<(f64, Option<f64>)> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ProgressHost for RecordingHost {
    type Error = io::Error;

    fn report(
        &self,
        progress: f64,
        total: Option<f64>,
    ) -> impl Future<Output = Result<(), io::Error>> + Send {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((progress, total));
        ready(Ok(()))
    }
}
