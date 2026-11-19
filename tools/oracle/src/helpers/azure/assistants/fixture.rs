use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::mem;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody, merge_query};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::thread_runs::error::ThreadRunsError;
use betterlite_openai::thread_runs::records::RunEvent;
use betterlite_openai::thread_runs::stream::{RunChunks, RunEventHandler, RunStreamTransport};
use futures_util::stream::iter;
use http::{HeaderMap, HeaderName, HeaderValue};
use secrecy::SecretString;
use serde_json::{Value, json};

use super::Reply;

const NO_REPLY: &str = r#"{"error":{"message":"the fixture queues no reply"}}"#;

// The headers the Python helper records, under the names it keys them by.
fn header(request: &OutboundRequest, name: &str) -> Option<String> {
    request
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn body_of(body: &RequestBody) -> Value {
    match body {
        RequestBody::Json(bytes) => serde_json::from_slice(bytes).unwrap_or(Value::Null),
        RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
            Value::Null
        }
    }
}

fn headers_of(reply: &Reply) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in &reply.headers {
        if let (Ok(name), Ok(value)) = (
            HeaderName::try_from(name.as_str()),
            HeaderValue::from_str(value),
        ) {
            headers.insert(name, value);
        }
    }
    headers
}

// The transport the SDK calls in place of the network: records what reached it and serves the queued replies.
pub(super) struct Recorder {
    pub(super) seen: Mutex<Vec<Value>>,
    replies: Mutex<VecDeque<Reply>>,
}

impl Recorder {
    pub(super) fn start(replies: &[Reply]) -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
            replies: Mutex::new(replies.iter().cloned().collect()),
        })
    }

    pub(super) fn take_seen(&self) -> Vec<Value> {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        mem::take(&mut *seen)
    }

    fn record(&self, request: &OutboundRequest) {
        let mut url = request.url.clone();
        merge_query(&mut url, &request.query);
        let body = body_of(&request.body);
        let content_type = header(request, "content-type")
            .or_else(|| (!body.is_null()).then_some(String::from("application/json")));
        let recorded = json!({
            "method": request.method.as_str(),
            "url": url.to_string(),
            "api-key": header(request, "api-key"),
            "authorization": header(request, "authorization"),
            "openai-beta": header(request, "openai-beta"),
            "accept": header(request, "accept"),
            "content-type": content_type,
            "body": body,
        });
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
    }

    fn next(&self) -> Reply {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| Reply {
                status: 500,
                body: NO_REPLY.to_owned(),
                headers: BTreeMap::new(),
            })
    }
}

impl AssistantsTransport for Recorder {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.record(&request);
        let reply = self.next();
        ready(Ok(UpstreamResponse {
            status: reply.status,
            headers: headers_of(&reply),
            body: reply.body.into_bytes(),
        }))
    }
}

impl RunStreamTransport for Recorder {
    fn open(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<RunChunks, ThreadRunsError>> + Send {
        self.record(&request);
        let reply = self.next();
        if reply.status >= 400 {
            return ready(Err(ThreadRunsError::Status(
                reply.status,
                reply.body.into_bytes(),
            )));
        }
        let chunks: RunChunks = Box::pin(iter(vec![Ok(reply.body.into_bytes())]));
        ready(Ok(chunks))
    }
}

// The handler the case attaches, logging what the SDK calls as the Python adapter logs it.
pub(super) struct RecordingHandler {
    calls: Mutex<Vec<String>>,
}

impl RecordingHandler {
    pub(super) fn start() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn log(&self, entry: String) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }
}

impl RunEventHandler for RecordingHandler {
    fn on_event(&self, event: &RunEvent) {
        self.log(event.name().to_owned());
    }

    fn on_exception(&self, error: &ThreadRunsError) {
        let label = if matches!(error, ThreadRunsError::ProviderError { .. }) {
            "stream_error"
        } else {
            "other"
        };
        self.log(format!("exception:{label}"));
    }

    fn on_timeout(&self) {
        self.log(String::from("timeout"));
    }

    fn on_end(&self) {
        self.log(String::from("end"));
    }
}

// A caller that holds no Azure AD token source: every token request refuses.
pub(super) struct NoSources;

fn unavailable(kind: &str) -> SourceFailure {
    SourceFailure {
        class: String::from("CredentialUnavailableError"),
        status: None,
        message: format!("no {kind} source is configured"),
    }
}

impl CredentialSources for NoSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("client secret"))
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("username and password"))
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("SDK refresh"))
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(unavailable("OIDC"))
    }
}
