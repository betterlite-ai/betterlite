use std::time::Duration;

use async_trait::async_trait;
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::CredentialSource;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::sagemaker_runtime::runtime::{ChunkStream, SagemakerTransport};
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_openai::watsonx_auth::transport::WatsonxTransport;
use http::{HeaderMap, Method};
use url::Url;

use super::error::EnterpriseError;

// the client B173 pools, as the three providers' calls need it: a buffered send and a streamed one
#[async_trait]
pub trait EnterpriseTransport: Send + Sync {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError>;
    async fn open_stream(
        &self,
        request: OutboundRequest,
    ) -> Result<(u16, ChunkStream), TransportError>;
}

pub struct Pooled<'client> {
    pub client: &'client ProxiedClient,
}

#[async_trait]
impl EnterpriseTransport for Pooled<'_> {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        let response = self.client.send_following(request).await?;
        response.buffered(self.client.limits.body_bytes).await
    }

    async fn open_stream(
        &self,
        request: OutboundRequest,
    ) -> Result<(u16, ChunkStream), TransportError> {
        let response = self.client.send_following(request).await?;
        let status = response.response.status().as_u16();
        Ok((status, response.body_stream()))
    }
}

// the RPITIT traits the provider crates expose take no trait object, so the dispatch passes this view
pub struct SharedTransport<'deps> {
    pub inner: &'deps dyn EnterpriseTransport,
}

impl SagemakerTransport for SharedTransport<'_> {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        self.inner.send(request).await
    }

    async fn open_stream(
        &self,
        request: OutboundRequest,
    ) -> Result<(u16, ChunkStream), TransportError> {
        self.inner.open_stream(request).await
    }
}

impl WatsonxTransport for SharedTransport<'_> {
    async fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> Result<UpstreamResponse, TransportError> {
        self.inner.send(request).await
    }
}

// CredentialSource resolves through a seam, so a resolver owned by the caller can sit behind it
#[async_trait]
pub trait AwsCredentials: Send + Sync {
    async fn resolve(
        &self,
        settings: AwsSettings,
        region: String,
    ) -> Result<Credentials, CredentialError>;
}

pub struct SharedCredentials<'deps> {
    pub inner: &'deps dyn AwsCredentials,
}

impl CredentialSource for SharedCredentials<'_> {
    async fn resolve(
        &self,
        settings: AwsSettings,
        region: String,
    ) -> Result<Credentials, CredentialError> {
        self.inner.resolve(settings, region).await
    }
}

pub fn request(
    method: Method,
    url: &str,
    headers: &HeaderMap,
    body: Vec<u8>,
    deadline: Duration,
) -> Result<OutboundRequest, EnterpriseError> {
    Ok(OutboundRequest {
        method,
        url: Url::parse(url).map_err(|source| EnterpriseError::Url {
            url: url.to_owned(),
            source,
        })?,
        query: Vec::new(),
        headers: headers.clone(),
        body: RequestBody::Json(body),
        deadline,
    })
}
