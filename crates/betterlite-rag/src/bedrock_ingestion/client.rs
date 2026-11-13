use std::error::Error as StdError;

use async_trait::async_trait;
use betterlite_bedrock::signing::headers::{merge_computed, prepare, without_computed};
use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_bedrock::signing::sigv4::{Signable, sign_request};
use serde::de::DeserializeOwned;

use super::error::BedrockIngestionError;
use super::settings::BedrockRagSettings;
use super::wire::{
    CreateDataSourceRequest, CreateDataSourceResponse, CreateKnowledgeBaseRequest,
    CreateKnowledgeBaseResponse, GetDataSourceResponse, GetKnowledgeBaseResponse,
    ListDataSourcesResponse,
};
use clock::Clock;
use credentials::RagCredentialResolver;
use endpoints::{data_source_url, data_sources_url, knowledge_base_url, knowledge_bases_url};
use request::{aws_settings, json_headers, parse_response};

pub mod clock;
pub mod credentials;
pub mod endpoints;
mod request;
mod tests;

pub const BEDROCK_AGENT_SERVICE: &str = "bedrock";
pub const S3_SERVICE: &str = "s3";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwsRagResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl AwsRagResponse {
    pub fn require_success<TransportFailure>(
        self,
        operation: &'static str,
    ) -> Result<Self, BedrockIngestionError<TransportFailure>>
    where
        TransportFailure: StdError + 'static,
    {
        if (200..300).contains(&self.status) {
            return Ok(self);
        }
        Err(BedrockIngestionError::Aws {
            operation,
            status: self.status,
            body: super::error::aws_error_body(&self.body),
        })
    }
}

#[async_trait]
pub trait AwsRagTransport: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    async fn send(&self, request: AwsSignedRequest) -> Result<AwsRagResponse, Self::Error>;
}

#[derive(Debug, Clone, Copy)]
pub struct SignedRequest<'request> {
    pub service: &'request str,
    pub signing_region: &'request str,
    pub method: &'request str,
    pub url: &'request str,
    pub headers: &'request [(String, String)],
    pub body: &'request [u8],
}

enum Payload<'request> {
    Empty,
    KnowledgeBase(&'request CreateKnowledgeBaseRequest),
    DataSource(&'request CreateDataSourceRequest),
}

pub struct BedrockRagClients<'deps, Transport> {
    pub settings: BedrockRagSettings,
    pub resolver: Box<dyn RagCredentialResolver + 'deps>,
    pub clock: Box<dyn Clock + 'deps>,
    pub transport: Transport,
}

macro_rules! json_operations {
    ($clients:ident; $($name:ident($($argument:ident: $kind:ty),*) -> $response:ty => $operation:literal, $method:literal, $url:expr, $payload:expr;)*) => {
        $(
            pub async fn $name(
                &self,
                $($argument: $kind),*
            ) -> Result<$response, BedrockIngestionError<Transport::Error>> {
                let $clients = self;
                let url = $url;
                $clients.send_json($operation, $method, url, $payload).await
            }
        )*
    };
}

impl<Transport> BedrockRagClients<'_, Transport>
where
    Transport: AwsRagTransport,
{
    pub async fn signed_request(
        &self,
        service: &str,
        method: &str,
        url: &str,
        body: &[u8],
    ) -> Result<AwsRagResponse, BedrockIngestionError<Transport::Error>> {
        let signing_region = self.settings.aws_region_name.as_str();
        let request = SignedRequest {
            service,
            signing_region,
            method,
            url,
            headers: &[],
            body,
        };
        self.signed_request_with(request).await
    }

    pub async fn signed_request_with(
        &self,
        request: SignedRequest<'_>,
    ) -> Result<AwsRagResponse, BedrockIngestionError<Transport::Error>> {
        let credentials = self
            .resolver
            .credentials(
                &aws_settings(&self.settings),
                &self.settings.aws_region_name,
            )
            .await
            .map_err(|source| BedrockIngestionError::Credentials { source })?;
        let supplied = without_computed(
            request.headers.to_vec(),
            credentials.session_token.is_some(),
        );
        let computed = sign_request(&Signable {
            method: request.method,
            url: request.url,
            headers: &supplied,
            body: request.body,
            service: request.service,
            region: request.signing_region,
            credentials: &credentials,
            now: self.clock.now(),
        })?;
        let signed = AwsSignedRequest {
            method: request.method.to_owned(),
            url: request.url.to_owned(),
            headers: prepare(
                merge_computed(supplied, computed),
                request.body.len(),
                request.method,
            ),
            body: request.body.to_vec(),
        };
        self.transport
            .send(signed)
            .await
            .map_err(|source| BedrockIngestionError::Transport { source })
    }

    json_operations! {
        clients;
        list_data_sources(knowledge_base_id: &str) -> ListDataSourcesResponse =>
            "ListDataSources", "POST",
            data_sources_url(&clients.settings.aws_region_name, knowledge_base_id),
            Payload::Empty;
        get_data_source(knowledge_base_id: &str, data_source_id: &str) -> GetDataSourceResponse =>
            "GetDataSource", "GET",
            data_source_url(&clients.settings.aws_region_name, knowledge_base_id, data_source_id),
            Payload::Empty;
        create_knowledge_base(request: &CreateKnowledgeBaseRequest) -> CreateKnowledgeBaseResponse =>
            "CreateKnowledgeBase", "PUT",
            knowledge_bases_url(&clients.settings.aws_region_name),
            Payload::KnowledgeBase(request);
        get_knowledge_base(knowledge_base_id: &str) -> GetKnowledgeBaseResponse =>
            "GetKnowledgeBase", "GET",
            knowledge_base_url(&clients.settings.aws_region_name, knowledge_base_id),
            Payload::Empty;
        create_data_source(knowledge_base_id: &str, request: &CreateDataSourceRequest) -> CreateDataSourceResponse =>
            "CreateDataSource", "PUT",
            data_sources_url(&clients.settings.aws_region_name, knowledge_base_id),
            Payload::DataSource(request);
    }

    async fn send_json<Response>(
        &self,
        operation: &'static str,
        method: &'static str,
        url: String,
        payload: Payload<'_>,
    ) -> Result<Response, BedrockIngestionError<Transport::Error>>
    where
        Response: DeserializeOwned,
    {
        let body = match payload {
            Payload::Empty => Vec::new(),
            Payload::KnowledgeBase(request) => serde_json::to_vec(request)
                .map_err(|source| BedrockIngestionError::Serialize { operation, source })?,
            Payload::DataSource(request) => serde_json::to_vec(request)
                .map_err(|source| BedrockIngestionError::Serialize { operation, source })?,
        };
        let headers = json_headers(!body.is_empty());
        let signing_region = self.settings.aws_region_name.as_str();
        let outbound = SignedRequest {
            service: BEDROCK_AGENT_SERVICE,
            signing_region,
            method,
            url: &url,
            headers: &headers,
            body: &body,
        };
        let response = self.signed_request_with(outbound).await?;
        parse_response(operation, &response.require_success(operation)?.body)
    }
}
