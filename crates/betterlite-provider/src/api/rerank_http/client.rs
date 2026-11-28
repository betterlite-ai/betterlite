use std::future::Future;
use std::time::Duration;

use betterlite_core::api::file_http_content::service::ClientSource;
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_models::decisions::Json;
use http::Method;
use http::header::HeaderMap;
use thiserror::Error;
use url::Url;

use crate::api::rerank_contracts::RerankLogging;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    Sync,
    Asynchronous,
}

// `if _is_async is True`: only the boolean true reaches arerank, and a 1 or a "true" stays on the sync path
#[must_use]
pub const fn client_kind(is_async: &Json) -> ClientKind {
    if matches!(is_async, Json::Bool(true)) {
        ClientKind::Asynchronous
    } else {
        ClientKind::Sync
    }
}

pub struct SuppliedClient<'client, Client> {
    pub kind: ClientKind,
    pub transport: &'client Client,
}

pub struct RerankHttpRequest {
    pub url: Url,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub timeout: Option<Duration>,
}

#[derive(Debug, Error)]
pub enum RerankSendError {
    #[error("the request timed out")]
    Timeout,
    #[error(transparent)]
    Failed(#[from] HttpError),
}

pub trait RerankTransport: Send + Sync {
    fn send(
        &self,
        request: RerankHttpRequest,
        logging: Option<&RerankLogging>,
    ) -> impl Future<Output = Result<UpstreamResponse, RerankSendError>> + Send;
}

pub(super) enum Selected<'client, Client> {
    Supplied(&'client Client),
    Created(Client),
}

impl<Client> Selected<'_, Client> {
    pub(super) const fn get(&self) -> &Client {
        match self {
            Self::Supplied(client) => client,
            Self::Created(client) => client,
        }
    }
}

// An httpx client of the other kind is replaced by the handler's own client, and a matching one is reused as is
pub(super) fn select_client<'borrow, S>(
    supplied: Option<&'borrow SuppliedClient<'_, S::Client>>,
    source: &'borrow S,
    kind: ClientKind,
) -> Result<Selected<'borrow, S::Client>, ClientError>
where
    S: ClientSource,
{
    if let Some(client) = supplied
        && client.kind == kind
    {
        return Ok(Selected::Supplied(client.transport));
    }
    Ok(Selected::Created(source.client()?))
}

// The handler hands the bytes to httpx as they are: a body with no content type, and the caller's headers alone
impl RerankTransport for ProxiedClient {
    async fn send(
        &self,
        request: RerankHttpRequest,
        _logging: Option<&RerankLogging>,
    ) -> Result<UpstreamResponse, RerankSendError> {
        let deadline = request.timeout.unwrap_or(self.limits.read_timeout);
        let built = self
            .client
            .request(Method::POST, request.url)
            .timeout(deadline)
            .headers(request.headers)
            .body(request.body)
            .build()
            .map_err(HttpError::Request)?;
        self.execute_buffered(built, self.limits.body_bytes)
            .await
            .map_err(send_failure)
    }
}

fn send_failure(error: HttpError) -> RerankSendError {
    if matches!(error, HttpError::Timeout) {
        return RerankSendError::Timeout;
    }
    RerankSendError::Failed(error)
}
