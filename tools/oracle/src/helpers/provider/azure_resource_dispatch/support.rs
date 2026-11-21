use std::collections::VecDeque;
use std::fmt::Display;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, merge_query};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::files::transport::{FileTransport, StreamReply};
use betterlite_openai::passthrough::transformation::types::{PassthroughLogging, RelayCallType};
use betterlite_openai::thread_runs::error::ThreadRunsError;
use betterlite_openai::thread_runs::stream::{RunChunks, RunStreamTransport};
use betterlite_oracle_helper_api::error::HelperError;
use futures_util::stream::iter;
use http::{HeaderMap, HeaderName, HeaderValue};
use secrecy::SecretString;
use tokio::runtime::{Builder, Runtime};

// Reached only if an operation and the result variant it reads back ever disagree
pub(super) const OTHER: &str = "the dispatch returned another outcome";

pub(super) fn failure<M>(message: M) -> Box<HelperError>
where
    M: Display,
{
    Box::new(HelperError {
        class: String::from("LeafFailure"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

pub(super) fn with_status(mut error: Box<HelperError>, status: u16) -> Box<HelperError> {
    error.status = Some(status);
    error
}

pub(super) fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(failure)
}

#[derive(Debug, Clone)]
pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) body: String,
    pub(super) headers: HeaderMap,
}

impl Reply {
    pub(super) fn json(status: u16, body: &str) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/json"),
        );
        Self {
            status,
            body: body.to_owned(),
            headers,
        }
    }

    pub(super) fn polled(body: &str) -> Self {
        let mut reply = Self::json(200, body);
        reply.headers.insert(
            HeaderName::from_static("openai-poll-after-ms"),
            HeaderValue::from_static("0"),
        );
        reply
    }

    pub(super) fn upstream(&self) -> UpstreamResponse {
        UpstreamResponse {
            status: self.status,
            headers: self.headers.clone(),
            body: self.body.as_bytes().to_vec(),
        }
    }
}

// The request a fake transport received, narrowed to what the compared answer reads
#[derive(Debug, Clone)]
pub(super) struct Seen {
    pub(super) method: String,
    pub(super) url: String,
}

impl Seen {
    fn of(request: &OutboundRequest) -> Self {
        let mut url = request.url.clone();
        merge_query(&mut url, &request.query);
        Self {
            method: request.method.as_str().to_owned(),
            url: url.as_str().to_owned(),
        }
    }
}

pub(super) struct FileFake {
    pub(super) reply: Reply,
    pub(super) seen: Mutex<Vec<Seen>>,
}

impl FileFake {
    pub(super) fn new(reply: Reply) -> Self {
        Self {
            reply,
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn first(&self) -> Option<Seen> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned()
    }

    fn record(&self, request: OutboundRequest) {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Seen::of(&request));
    }
}

impl FileTransport for FileFake {
    fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.record(request);
        ready(Ok(self.reply.upstream()))
    }

    fn send_stream(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<StreamReply, TransportError>> + Send {
        self.record(request);
        ready(Err(TransportError::Closed))
    }
}

// One queued reply per request: a polled run sends its create and each poll, a streamed one a single body
pub(super) struct AssistantsFake {
    replies: Mutex<VecDeque<Reply>>,
    pub(super) seen: Mutex<Vec<Seen>>,
}

impl AssistantsFake {
    pub(super) fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn requests(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, request: &OutboundRequest) {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Seen::of(request));
    }

    fn next(&self) -> Reply {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| Reply::json(500, "{}"))
    }
}

impl AssistantsTransport for AssistantsFake {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.record(&request);
        ready(Ok(self.next().upstream()))
    }
}

impl RunStreamTransport for AssistantsFake {
    fn open(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<RunChunks, ThreadRunsError>> + Send {
        self.record(&request);
        let reply = self.next();
        ready(if reply.status >= 400 {
            Err(ThreadRunsError::Status(
                reply.status,
                reply.body.into_bytes(),
            ))
        } else {
            let chunks: RunChunks = Box::pin(iter(vec![Ok(reply.body.into_bytes())]));
            Ok(chunks)
        })
    }
}

#[derive(Default)]
pub(super) struct RecordingPassthrough {
    pub(super) call_types: Vec<String>,
}

impl PassthroughLogging for RecordingPassthrough {
    fn set_call_type(&mut self, call_type: RelayCallType) {
        self.call_types.push(call_type.as_str().to_owned());
    }
}

pub(super) struct NoSources;

fn unavailable() -> SourceFailure {
    SourceFailure {
        class: String::from("ValueError"),
        status: None,
        message: String::from("the fixture resolves no token source"),
    }
}

impl CredentialSources for NoSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable())
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable())
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable())
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(unavailable())
    }
}

pub(super) fn credentials() -> DeploymentSettings {
    DeploymentSettings {
        azure_ad_token: None,
        tenant_id: None,
        client_id: None,
        client_secret: None,
        azure_scope: String::from("https://cognitiveservices.azure.com/.default"),
        azure_username: None,
        azure_password: None,
    }
}
