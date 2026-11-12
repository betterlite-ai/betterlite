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
    header_map, processing_record, read_field, reply_fields, transformed_body, upstream_record,
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
    #[error(transparent)]
    Header(#[from] HeaderError),
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
            Err(CountCallError::Upstream(error)) => Err(CountTokensHandlerError::Record {
                status: error.error.status_code,
                error,
            }),
            Err(source) => {
                let error = processing_record(&source)?;
                Err(CountTokensHandlerError::Record {
                    status: error.error.status_code,
                    error: Box::new(error),
                })
            }
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
    ) -> Result<Option<CountResponse>, CountCallError> {
        validate_count_tokens_request(request.request_data)?;
        let model = request.resolved_model;
        let named = dumps(&Json::Text(model.to_owned()));
        tracing::debug!(subject = "processing", value = %named, "Processing CountTokens request for resolved model: {model}");
        let explicit = read_field::<String>(request.litellm_params, "aws_region_name")?;
        let region = llm_region_name(LlmRegion {
            explicit: explicit.as_deref(),
            model: Some(request.resolved_model),
            model_id: None,
            environment_region_name: self.environment.region_name.as_deref(),
            environment_region: self.environment.region.as_deref(),
            session_region: self.environment.session_region.as_deref(),
        })?;
        let located = dumps(&Json::Text(region.clone()));
        tracing::debug!(subject = "region", value = %located, "Retrieved AWS region: {region}");
        let body = transformed_body(request)?;
        let endpoint = self.endpoint(request, &region)?;
        let response = self.signed_response(request, &body, &endpoint, now).await?;
        reply_fields(&response)
    }

    fn endpoint(
        &self,
        request: &CountTokensHandlerRequest<'_>,
        region: &str,
    ) -> Result<String, CountCallError> {
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
        let target = dumps(&Json::Text(endpoint.clone()));
        tracing::debug!(subject = "endpoint", value = %target, "Making request to: {endpoint}");
        Ok(endpoint)
    }

    async fn signed_response(
        &self,
        request: &CountTokensHandlerRequest<'_>,
        body: &IndexMap<String, Json>,
        endpoint: &str,
        now: SystemTime,
    ) -> Result<UpstreamResponse, CountCallError> {
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
        .await?;
        self.post(endpoint, &signed.headers, signed.body).await
    }

    async fn post(
        &self,
        endpoint: &str,
        headers: &[(String, String)],
        body: Vec<u8>,
    ) -> Result<UpstreamResponse, CountCallError> {
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
        let response = self.transport.send(outbound).await?;
        // AsyncHTTPHandler raises for every non-2xx reply before LiteLLM logs the status
        if (200..300).contains(&response.status) {
            let status = dumps(&Json::Number(serde_json::Number::from(response.status)));
            tracing::debug!(subject = "status", value = %status, "Response status: {status}");
        }
        if response.status != 200 {
            return Err(CountCallError::Upstream(Box::new(upstream_record(
                &response, endpoint,
            )?)));
        }
        Ok(response)
    }
}
