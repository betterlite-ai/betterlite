use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::cohere_legacy_embedding::execute::LegacyEmbeddingTransport;
use http::{HeaderMap, HeaderName, HeaderValue};

use crate::bedrock_embedding_io::transport::{
    ClientFactory, ClientKind, EmbeddingReply, EmbeddingRequest, EmbeddingTransport,
    SuppliedClient, Timeout, TransferError,
};

// LiteLLM builds the async cohere client with litellm.LlmProviders.COHERE
const COHERE_CLIENT_PROVIDER: &str = "cohere";

// B1418 hands the transport one request, so the async branch builds the client with the caller timeout and posts through it
pub(super) struct CohereSender<'client, Factory: ClientFactory, Transport> {
    pub(super) supplied: Option<SuppliedClient<'client, Factory::Client>>,
    pub(super) factory: &'client Factory,
    pub(super) fallback: &'client Transport,
    pub(super) asynchronous: bool,
    pub(super) timeout: Option<Timeout>,
}

impl<Factory, Transport> LegacyEmbeddingTransport for CohereSender<'_, Factory, Transport>
where
    Factory: ClientFactory + Sync,
    Factory::Client: LegacyEmbeddingTransport,
    Transport: LegacyEmbeddingTransport,
{
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        match (self.asynchronous, self.supplied.as_ref()) {
            (true, Some(client)) if client.kind == ClientKind::Async => {
                post(client.transport, request).await
            }
            (true, _) => {
                let client = self
                    .factory
                    .async_client(COHERE_CLIENT_PROVIDER, self.timeout.clone());
                post(&client, request).await
            }
            (false, Some(client)) if client.kind == ClientKind::Sync => {
                client.transport.send(request).await
            }
            (false, _) => self.fallback.send(request).await,
        }
    }
}

async fn post<Client>(
    client: &Client,
    request: OutboundRequest,
) -> Result<UpstreamResponse, TransportError>
where
    Client: EmbeddingTransport,
{
    let embedding = EmbeddingRequest {
        url: request.url.to_string(),
        headers: request
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect(),
        body: match request.body {
            RequestBody::Json(bytes) | RequestBody::Bytes { data: bytes, .. } => bytes,
            RequestBody::Empty | RequestBody::Multipart { .. } => Vec::new(),
        },
    };
    match client.post(embedding).await {
        Ok(reply) => Ok(upstream(reply)),
        // the async handler wraps a timeout in litellm.Timeout, whose status is 408
        Err(TransferError::Timeout { .. }) => Ok(UpstreamResponse {
            status: 408,
            headers: HeaderMap::new(),
            body: Vec::new(),
        }),
        Err(TransferError::Failed(error)) => Err(error),
    }
}

fn upstream(reply: EmbeddingReply) -> UpstreamResponse {
    let headers = reply
        .headers
        .into_iter()
        .filter_map(|(name, value)| {
            Some((
                HeaderName::try_from(name).ok()?,
                HeaderValue::try_from(value).ok()?,
            ))
        })
        .collect();
    UpstreamResponse {
        status: reply.status,
        headers,
        body: reply.body,
    }
}
