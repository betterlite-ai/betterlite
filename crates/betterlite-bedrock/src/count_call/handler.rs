use std::collections::BTreeMap;
use std::future::Future;
use std::time::{Duration, SystemTime};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use http::Method;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::value::RawValue;
use url::Url;

use crate::aws_endpoints::region::{LlmRegion, llm_region_name};
use crate::aws_settings::sources::Credentials;
use crate::bedrock_settings::{BedrockError, HeaderError};
use crate::count_tokens::endpoint::{CountTokensEndpoint, count_tokens_endpoint};
use crate::count_tokens::validate::validate_count_tokens_request;
use crate::credentials::AwsCredentialResolver;
use crate::credentials::error::CredentialError;
use crate::signing::service::{ServiceName, ServiceRequest, sign_service_request};
use crate::signing::target::RegionInputs;

use super::error::CountCallError;
use super::record::{
    HandlerFailure, header_map, processing_record, read_field, reply_fields, transformed_body,
    upstream_record,
};
use super::reply::CountResponse;

pub type ParamMap = BTreeMap<String, Box<RawValue>>;

// LiteLLM's CountTokens handler hard-codes this timeout, so no deployment setting reaches it
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONTENT_TYPE: &str = "Content-Type";
const JSON_CONTENT_TYPE: &str = "application/json";

#[derive(Debug, Clone, Default)]
pub struct CountTokensEnvironment {
    pub region_name: Option<String>,
    pub region: Option<String>,
    pub session_region: Option<String>,
    pub runtime_endpoint: Option<String>,
    pub bearer_token: Option<SecretString>,
}

#[derive(Debug, Clone, Copy)]
pub struct CountTokensHandlerRequest<'call> {
    pub request_data: &'call IndexMap<String, Json>,
    pub litellm_params: &'call ParamMap,
    pub resolved_model: &'call str,
}

pub trait CountTokensTransport: Send + Sync {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

impl CountTokensTransport for ProxiedClient {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        // LiteLLM's AsyncHTTPHandler follows redirects, so each hop is checked the same way
        self.send_following(request)
            .await?
            .buffered(self.limits.body_bytes)
            .await
    }
}

pub trait CountTokensCredentials: Send + Sync {
    fn resolve(
        &self,
        settings: AwsSettings,
        region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send;
}

impl CountTokensCredentials for AwsCredentialResolver<'_> {
    async fn resolve(
        &self,
        settings: AwsSettings,
        region: String,
    ) -> Result<Credentials, CredentialError> {
        self.resolve_credentials(&settings, Some(&region)).await
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CountTokensHandlerError {
    #[error("the CountTokens endpoint answered {status}")]
    Record {
        status: u16,
        error: Box<BedrockError>,
    },
    #[error("the error record cannot carry header {name:?}")]
    Header { name: String, source: HeaderError },
}

pub trait CountTokensHandler: Send + Sync {
    fn handle_count_tokens_request(
        &self,
        request: &CountTokensHandlerRequest<'_>,
        now: SystemTime,
    ) -> impl Future<Output = Result<Option<CountResponse>, CountTokensHandlerError>> + Send;
}

pub struct BedrockCountTokensHandler<T, C> {
    pub transport: T,
    pub credentials: C,
    pub environment: CountTokensEnvironment,
}

impl<T, C> CountTokensHandler for BedrockCountTokensHandler<T, C>
where
    T: CountTokensTransport,
    C: CountTokensCredentials,
{
    async fn handle_count_tokens_request(
        &self,
        request: &CountTokensHandlerRequest<'_>,
        now: SystemTime,
    ) -> Result<Option<CountResponse>, CountTokensHandlerError> {
        match self.count(request, now).await {
            Ok(reply) => Ok(reply),
            // LiteLLM's `except BedrockError: raise` passes an already built record through unchanged
            Err(HandlerFailure::Bedrock(error)) => Err(CountTokensHandlerError::Record {
                status: error.error.status_code,
                error,
            }),
            Err(HandlerFailure::Processing(source)) => match processing_record(&source) {
                Ok(error) => Err(CountTokensHandlerError::Record {
                    status: error.error.status_code,
                    error: Box::new(error),
                }),
                Err(header) => Err(CountTokensHandlerError::Header {
                    name: match &header {
                        HeaderError::NotAscii { name } => name.clone(),
                    },
                    source: header,
                }),
            },
        }
    }
}

impl<T, C> BedrockCountTokensHandler<T, C>
where
    T: CountTokensTransport,
    C: CountTokensCredentials,
{
    async fn count(
        &self,
        request: &CountTokensHandlerRequest<'_>,
        now: SystemTime,
    ) -> Result<Option<CountResponse>, HandlerFailure> {
        validate_count_tokens_request(request.request_data).map_err(CountCallError::Request)?;
        tracing::debug!(
            subject = "processing",
            value = %dumps(&Json::Text(request.resolved_model.to_owned())),
            "Processing CountTokens request for resolved model: {}",
            request.resolved_model
        );
        let explicit = read_field::<String>(request.litellm_params, "aws_region_name")?;
        let region = llm_region_name(LlmRegion {
            explicit: explicit.as_deref(),
            model: Some(request.resolved_model),
            model_id: None,
            environment_region_name: self.environment.region_name.as_deref(),
            environment_region: self.environment.region.as_deref(),
            session_region: self.environment.session_region.as_deref(),
        })
        .map_err(CountCallError::Region)?;
        tracing::debug!(
            subject = "region",
            value = %dumps(&Json::Text(region.clone())),
            "Retrieved AWS region: {region}"
        );
        let body = transformed_body(request)?;
        let endpoint = self.endpoint(request, &region)?;
        let response = self.signed_response(request, &body, &endpoint, now).await?;
        reply_fields(&response)
    }

    fn endpoint(
        &self,
        request: &CountTokensHandlerRequest<'_>,
        region: &str,
    ) -> Result<String, HandlerFailure> {
        let api_base = read_field::<String>(request.litellm_params, "api_base")?;
        let configured =
            read_field::<String>(request.litellm_params, "aws_bedrock_runtime_endpoint")?;
        let endpoint = count_tokens_endpoint(CountTokensEndpoint {
            model: request.resolved_model,
            aws_region_name: region,
            api_base: api_base.as_deref(),
            aws_bedrock_runtime_endpoint: configured.as_deref(),
            environment_endpoint: self.environment.runtime_endpoint.as_deref(),
        });
        tracing::debug!(
            subject = "endpoint",
            value = %dumps(&Json::Text(endpoint.clone())),
            "Making request to: {endpoint}"
        );
        Ok(endpoint)
    }

    async fn signed_response(
        &self,
        request: &CountTokensHandlerRequest<'_>,
        body: &IndexMap<String, Json>,
        endpoint: &str,
        now: SystemTime,
    ) -> Result<UpstreamResponse, HandlerFailure> {
        let api_key =
            read_field::<String>(request.litellm_params, "api_key")?.map(SecretString::from);
        let signed = sign_service_request(
            ServiceRequest {
                service: ServiceName::Bedrock,
                headers: &[(CONTENT_TYPE.to_owned(), Some(JSON_CONTENT_TYPE.to_owned()))],
                optional_params: request.litellm_params,
                request_data: body,
                api_base: endpoint,
                model: Some(request.resolved_model),
                api_key: api_key.as_ref(),
                configured_bearer: self.environment.bearer_token.as_ref(),
                region_inputs: RegionInputs {
                    environment_region_name: self.environment.region_name.as_deref(),
                    environment_region: self.environment.region.as_deref(),
                    session_region: self.environment.session_region.as_deref(),
                },
                now,
            },
            // the signer hands borrowed inputs; the trait takes them owned so one future type covers every call
            |settings, region| {
                self.credentials
                    .resolve(settings.clone(), region.to_owned())
            },
        )
        .await
        .map_err(CountCallError::Signing)?;
        self.post(endpoint, &signed.headers, signed.body).await
    }

    async fn post(
        &self,
        endpoint: &str,
        headers: &[(String, String)],
        body: Vec<u8>,
    ) -> Result<UpstreamResponse, HandlerFailure> {
        let outbound = OutboundRequest {
            method: Method::POST,
            url: Url::parse(endpoint).map_err(|source| CountCallError::Endpoint {
                endpoint: endpoint.to_owned(),
                source,
            })?,
            query: Vec::new(),
            headers: header_map(headers)?,
            body: RequestBody::Json(body),
            deadline: REQUEST_TIMEOUT,
        };
        let response = self
            .transport
            .send(outbound)
            .await
            .map_err(CountCallError::Transport)?;
        tracing::debug!(
            subject = "status",
            value = %dumps(&Json::Number(serde_json::Number::from(response.status))),
            "Response status: {}",
            response.status
        );
        if response.status != 200 {
            return Err(HandlerFailure::Bedrock(Box::new(upstream_record(
                &response, endpoint,
            )?)));
        }
        Ok(response)
    }
}
