use std::error::Error as StdError;

use async_trait::async_trait;
use betterlite_bedrock::signing::headers::{merge_computed, prepare, without_computed};
use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_bedrock::signing::sigv4::{Signable, sign_request};
use betterlite_config::sections::bedrock_settings::AwsSettings;
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
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
use endpoints::bedrock_agent_endpoint;

pub mod clock;
pub mod credentials;
pub mod endpoints;
mod tests;

pub const BEDROCK_AGENT_SERVICE: &str = "bedrock";
pub const S3_SERVICE: &str = "s3";

const JSON_CONTENT_TYPE: &str = "application/json";
const PATH_SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b'/')
    .add(b'\\');

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

#[derive(Debug, Clone, Copy)]
struct JsonRequest<'request> {
    method: &'request str,
    url: &'request str,
    body: Option<&'request [u8]>,
}

pub struct BedrockRagClients<'deps, Transport> {
    pub settings: BedrockRagSettings,
    pub resolver: Box<dyn RagCredentialResolver + 'deps>,
    pub clock: Box<dyn Clock + 'deps>,
    pub transport: Transport,
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
        let request = SignedRequest {
            service,
            signing_region: &self.settings.aws_region_name,
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

    pub async fn list_data_sources(
        &self,
        knowledge_base_id: &str,
    ) -> Result<ListDataSourcesResponse, BedrockIngestionError<Transport::Error>> {
        let url = format!(
            "{}/knowledgebases/{}/datasources/",
            bedrock_agent_endpoint(&self.settings.aws_region_name),
            utf8_percent_encode(knowledge_base_id, PATH_SEGMENT)
        );
        let request = JsonRequest {
            method: "POST",
            url: &url,
            body: Some(b"{}"),
        };
        let response = self.json_request("ListDataSources", request).await?;
        parse_response("ListDataSources", &response.body)
    }

    pub async fn get_data_source(
        &self,
        knowledge_base_id: &str,
        data_source_id: &str,
    ) -> Result<GetDataSourceResponse, BedrockIngestionError<Transport::Error>> {
        let url = format!(
            "{}/knowledgebases/{}/datasources/{}",
            bedrock_agent_endpoint(&self.settings.aws_region_name),
            utf8_percent_encode(knowledge_base_id, PATH_SEGMENT),
            utf8_percent_encode(data_source_id, PATH_SEGMENT)
        );
        let request = JsonRequest {
            method: "GET",
            url: &url,
            body: None,
        };
        let response = self.json_request("GetDataSource", request).await?;
        parse_response("GetDataSource", &response.body)
    }

    pub async fn create_knowledge_base(
        &self,
        request: &CreateKnowledgeBaseRequest,
    ) -> Result<CreateKnowledgeBaseResponse, BedrockIngestionError<Transport::Error>> {
        let body =
            serde_json::to_vec(request).map_err(|source| BedrockIngestionError::Serialize {
                operation: "CreateKnowledgeBase",
                source,
            })?;
        let url = format!(
            "{}/knowledgebases/",
            bedrock_agent_endpoint(&self.settings.aws_region_name)
        );
        let outgoing = JsonRequest {
            method: "PUT",
            url: &url,
            body: Some(&body),
        };
        let response = self.json_request("CreateKnowledgeBase", outgoing).await?;
        parse_response("CreateKnowledgeBase", &response.body)
    }

    pub async fn get_knowledge_base(
        &self,
        knowledge_base_id: &str,
    ) -> Result<GetKnowledgeBaseResponse, BedrockIngestionError<Transport::Error>> {
        let url = format!(
            "{}/knowledgebases/{}",
            bedrock_agent_endpoint(&self.settings.aws_region_name),
            utf8_percent_encode(knowledge_base_id, PATH_SEGMENT)
        );
        let request = JsonRequest {
            method: "GET",
            url: &url,
            body: None,
        };
        let response = self.json_request("GetKnowledgeBase", request).await?;
        parse_response("GetKnowledgeBase", &response.body)
    }

    pub async fn create_data_source(
        &self,
        request: &CreateDataSourceRequest,
    ) -> Result<CreateDataSourceResponse, BedrockIngestionError<Transport::Error>> {
        let body =
            serde_json::to_vec(request).map_err(|source| BedrockIngestionError::Serialize {
                operation: "CreateDataSource",
                source,
            })?;
        let url = format!(
            "{}/knowledgebases/{}/datasources/",
            bedrock_agent_endpoint(&self.settings.aws_region_name),
            utf8_percent_encode(&request.knowledge_base_id, PATH_SEGMENT)
        );
        let outgoing = JsonRequest {
            method: "PUT",
            url: &url,
            body: Some(&body),
        };
        let response = self.json_request("CreateDataSource", outgoing).await?;
        parse_response("CreateDataSource", &response.body)
    }

    async fn json_request(
        &self,
        operation: &'static str,
        request: JsonRequest<'_>,
    ) -> Result<AwsRagResponse, BedrockIngestionError<Transport::Error>> {
        let headers = if request.body.is_some() {
            vec![("Content-Type".to_owned(), JSON_CONTENT_TYPE.to_owned())]
        } else {
            Vec::new()
        };
        let region = self.settings.aws_region_name.as_str();
        let response = self
            .signed_request_with(SignedRequest {
                service: BEDROCK_AGENT_SERVICE,
                signing_region: region,
                method: request.method,
                url: request.url,
                headers: &headers,
                body: request.body.unwrap_or_default(),
            })
            .await?;
        response.require_success(operation)
    }
}

fn parse_response<Response, TransportFailure>(
    operation: &'static str,
    body: &[u8],
) -> Result<Response, BedrockIngestionError<TransportFailure>>
where
    Response: DeserializeOwned,
    TransportFailure: StdError + 'static,
{
    serde_json::from_slice(body)
        .map_err(|source| BedrockIngestionError::Response { operation, source })
}

// the ten BaseAWSLLM.get_credentials keys, plus the region every call resolves them with
pub(crate) fn aws_settings(settings: &BedrockRagSettings) -> AwsSettings {
    AwsSettings {
        access_key_id: settings.credentials.access_key_id.clone(),
        secret_access_key: settings.credentials.secret_access_key.clone(),
        session_token: settings.credentials.session_token.clone(),
        region_name: Some(settings.aws_region_name.clone()),
        session_name: settings.credentials.session_name.clone(),
        profile_name: settings.credentials.profile_name.clone(),
        role_name: settings.credentials.role_name.clone(),
        web_identity_token: settings.credentials.web_identity_token.clone(),
        sts_endpoint: settings.credentials.sts_endpoint.clone(),
        external_id: settings.credentials.external_id.clone(),
        session_tags: None,
        role_arn: None,
        web_identity_token_file: None,
        bearer_token: None,
    }
}
