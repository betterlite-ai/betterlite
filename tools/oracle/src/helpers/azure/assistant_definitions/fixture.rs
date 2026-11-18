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
use http::HeaderMap;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::Reply;

pub(super) const PAGE: &str =
    r#"{"object":"list","data":[],"first_id":null,"last_id":null,"has_more":false}"#;
pub(super) const ASSISTANT: &str = r#"{"id":"asst_1","object":"assistant","created_at":1,"model":"gpt-4o","tools":[],"tool_resources":null}"#;
pub(super) const DELETED: &str = r#"{"id":"asst_1","deleted":true,"object":"assistant.deleted"}"#;

// The transport the SDK calls in place of the network: records what reached it and replies with the case body.
pub(super) struct Wire {
    seen: Mutex<Vec<Value>>,
    status: u16,
    body: String,
}

impl Wire {
    pub(super) fn replying(reply: Option<&Reply>, canned: &str) -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
            status: reply.map_or(200, |reply| reply.status),
            body: reply.map_or_else(|| canned.to_owned(), |reply| reply.body.clone()),
        })
    }

    pub(super) fn take(&self) -> Vec<Value> {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        mem::take(&mut seen)
    }
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    let value = headers.get(name)?;
    value.to_str().ok().map(str::to_owned)
}

impl AssistantsTransport for Wire {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let mut url = request.url.clone();
        merge_query(&mut url, &request.query);
        let body = match &request.body {
            RequestBody::Json(bytes) => serde_json::from_slice(bytes).unwrap_or(Value::Null),
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
                Value::Null
            }
        };
        let content_type = header(&request.headers, "content-type")
            .or_else(|| (!body.is_null()).then_some(String::from("application/json")));
        let recorded = json!({
            "method": request.method.as_str(),
            "url": url.as_str(),
            "api-key": header(&request.headers, "api-key"),
            "authorization": header(&request.headers, "authorization"),
            "openai-beta": header(&request.headers, "openai-beta"),
            "accept": header(&request.headers, "accept"),
            "content-type": content_type,
            "body": body,
        });
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
        ready(Ok(UpstreamResponse {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.body.clone().into_bytes(),
        }))
    }
}

// A caller that holds no Azure AD token source: every token request refuses.
pub(super) struct NoSources;

fn refuse<T>(kind: &str) -> Result<T, SourceFailure> {
    Err(SourceFailure {
        class: String::from("CredentialUnavailableError"),
        status: None,
        message: format!("no {kind} source is configured"),
    })
}

impl CredentialSources for NoSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        refuse("client secret")
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        refuse("username and password")
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        refuse("SDK refresh")
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        refuse("OIDC")
    }
}
