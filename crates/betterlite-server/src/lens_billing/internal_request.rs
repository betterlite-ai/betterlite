use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use betterlite_auth::reservation_settlement::handle::ReservationHandle;
use betterlite_logging::logging_privacy::PrivacyContext;
use betterlite_redis::request_batch::lock;

use crate::lens_inference::request_body::AnalysisBody;

mod tests;

pub const METHOD: &str = "POST";
pub const CHAT_PATH: &str = "/v1/chat/completions";
pub const SERVER_HOST: &str = "litellm.internal";
pub const SERVER_PORT: u16 = 80;
pub const CONTENT_TYPE: &str = "content-type";
const JSON_CONTENT_TYPE: &str = "application/json";
const DEFAULT_SCHEME: &str = "http";

// the parts of the worker's own request that decide the in-process one it stands in for
#[derive(Clone, Debug, Default)]
pub struct Incoming {
    pub scheme: Option<String>,
    pub client: Option<SocketAddr>,
    pub x_forwarded_for: Option<String>,
    pub x_forwarded_proto: Option<String>,
    pub host: Option<String>,
    pub privacy: MessagePrivacy,
}

impl Incoming {
    // incoming.url.scheme or "http": a scheme the worker request does not carry reads as plain HTTP
    #[must_use]
    pub fn scheme(&self) -> &str {
        self.scheme
            .as_deref()
            .filter(|scheme| !scheme.is_empty())
            .unwrap_or(DEFAULT_SCHEME)
    }
}

// IPAddressUtils.get_mcp_client_ip, resolved against the worker's own request by B743
pub trait ClientIp: Send + Sync {
    fn get_mcp_client_ip(&self, incoming: &Incoming) -> Option<String>;
}

// request.state.message_logging_privacy: inherit_message_logging_privacy(True) sets it for the chat call
#[derive(Clone, Debug, Default)]
pub struct MessagePrivacy(pub(crate) Arc<Mutex<PrivacyContext>>);

// the token inherit_message_logging_privacy resets on exit, restoring what a nested scope found
#[must_use]
pub struct MessagePrivacyScope {
    privacy: MessagePrivacy,
    previous: PrivacyContext,
}

impl MessagePrivacyScope {
    pub fn enter(privacy: &MessagePrivacy, context: PrivacyContext) -> Self {
        let previous = *lock(&privacy.0);
        *lock(&privacy.0) = context;
        Self {
            privacy: privacy.clone(),
            previous,
        }
    }
}

impl Drop for MessagePrivacyScope {
    fn drop(&mut self) {
        *lock(&self.privacy.0) = self.previous;
    }
}

// LiteLLM's request.state.budget_reservation: authorize stamps what it reserved, complete releases it afterwards
#[derive(Clone, Debug, Default)]
pub struct ReservationCell(pub(crate) Arc<Mutex<Option<ReservationHandle>>>);

impl ReservationCell {
    pub fn set(&self, reservation: ReservationHandle) {
        let replaced = lock(&self.0).replace(reservation).is_some();
        if replaced {
            tracing::warn!(
                "a second budget reservation replaced the one already stamped for the lens analysis request"
            );
        }
    }
}

// the Request complete hands the chat pipeline, with the ASGI parts the pipeline reads
#[derive(Clone, Debug)]
pub struct InternalRequest {
    pub method: &'static str,
    pub path: &'static str,
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub scheme: String,
    pub server: (String, u16),
    pub client: Option<(String, u16)>,
    pub body: Vec<u8>,
    pub reservation: ReservationCell,
    pub privacy: MessagePrivacy,
}

#[derive(Debug, thiserror::Error)]
pub enum RequestBuildError {
    #[error("the in-process request body could not be serialized: {0}")]
    Body(#[source] serde_json::Error),
}

pub fn build_request(
    data: &AnalysisBody<'_>,
    incoming: &Incoming,
    client_ip: &dyn ClientIp,
) -> Result<InternalRequest, RequestBuildError> {
    let client = client_ip
        .get_mcp_client_ip(incoming)
        .filter(|address| !address.is_empty())
        .map(|address| {
            let port = incoming.client.map_or(0, |peer| peer.port());
            (address, port)
        });
    Ok(InternalRequest {
        method: METHOD,
        path: CHAT_PATH,
        query: String::new(),
        headers: vec![(CONTENT_TYPE.to_owned(), JSON_CONTENT_TYPE.to_owned())],
        scheme: incoming.scheme().to_owned(),
        server: (SERVER_HOST.to_owned(), SERVER_PORT),
        client,
        body: serde_json::to_vec(data).map_err(RequestBuildError::Body)?,
        reservation: ReservationCell::default(),
        privacy: incoming.privacy.clone(),
    })
}
