use std::future::{Future, ready};
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
use http::HeaderMap;
use secrecy::SecretString;
use serde::Serialize;
use serde_json::Value;

use super::Reply;

pub(super) const THREAD: &str =
    r#"{"id":"thread_1","object":"thread","created_at":7,"metadata":{"team":"a"}}"#;

// One recorded request, keyed as the Python helper keys its dict of the same request.
#[derive(Serialize)]
struct Recorded {
    method: String,
    url: String,
    #[serde(rename = "api-key")]
    api_key: Option<String>,
    authorization: Option<String>,
    #[serde(rename = "openai-beta")]
    openai_beta: Option<String>,
    accept: Option<String>,
    #[serde(rename = "content-type")]
    content_type: Option<String>,
    body: Value,
}

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

// The transport the SDK calls in place of the network: records what reached it and replies with the case body.
pub(super) struct Recorder {
    pub(super) seen: Mutex<Vec<Value>>,
    status: u16,
    body: String,
}

impl Recorder {
    pub(super) fn replying(reply: Option<&Reply>, canned: &str) -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
            status: reply.map_or(200, |given| given.status),
            body: reply.map_or_else(|| canned.to_owned(), |given| given.body.clone()),
        })
    }

    fn record(&self, request: &OutboundRequest) {
        let mut url = request.url.clone();
        merge_query(&mut url, &request.query);
        let body = body_of(&request.body);
        let content_type = header(request, "content-type")
            .or_else(|| (!body.is_null()).then_some(String::from("application/json")));
        let recorded = Recorded {
            method: request.method.as_str().to_owned(),
            url: url.to_string(),
            api_key: header(request, "api-key"),
            authorization: header(request, "authorization"),
            openai_beta: header(request, "openai-beta"),
            accept: header(request, "accept"),
            content_type,
            body,
        };
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(serde_json::to_value(recorded).unwrap_or(Value::Null));
    }
}

impl AssistantsTransport for Recorder {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.record(&request);
        ready(Ok(UpstreamResponse {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.body.clone().into_bytes(),
        }))
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
