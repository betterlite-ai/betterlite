use std::collections::BTreeMap;
use std::future::Future;
use std::str::FromStr as _;
use std::time::{Duration, SystemTime};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_logging::python_json::{dumps, dumps_object};
use betterlite_models::decisions::Json;
use betterlite_openai::error::{ErrorRequest, ErrorResponse};
use http::Method;
use http::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::value::RawValue;
use url::Url;

use crate::aws_endpoints::region::{LlmRegion, llm_region_name};
use crate::aws_settings::sources::Credentials;
use crate::bedrock_settings::{
    BedrockError, ErrorHeaders, ErrorInput, ErrorReply, HeaderError, ResponseBody,
};
use crate::count_tokens::endpoint::{CountTokensEndpoint, count_tokens_endpoint};
use crate::count_tokens::response::transform_response;
use crate::count_tokens::transform_request;
use crate::count_tokens::validate::validate_count_tokens_request;
use crate::credentials::AwsCredentialResolver;
use crate::credentials::error::CredentialError;
use crate::signing::request::AwsSignedRequest;
use crate::signing::service::{ServiceName, ServiceRequest, sign_service_request};
use crate::signing::target::RegionInputs;

use super::error::CountCallError;
use super::reply::{CountResponse, decode_bedrock_count_reply};

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
            Err(HandlerFailure::Bedrock(error)) => Err(recorded(error)),
            Err(HandlerFailure::Processing(source)) => match processing_record(&source) {
                Ok(error) => Err(recorded(Box::new(error))),
                Err(header) => Err(CountTokensHandlerError::Header {
                    name: source_name(&header),
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
        let region = self.resolved_region(request)?;
        let body = transformed_body(request)?;
        let endpoint = self.endpoint(request, &region)?;
        let response = self.signed_response(request, &body, &endpoint, now).await?;
        reply_fields(&response)
    }

    fn resolved_region(
        &self,
        request: &CountTokensHandlerRequest<'_>,
    ) -> Result<String, HandlerFailure> {
        tracing::debug!(
            subject = "processing",
            value = %dumps(&Json::Text(request.resolved_model.to_owned())),
            "Processing CountTokens request for resolved model: {}",
            request.resolved_model
        );
        let explicit = read_string(request.litellm_params, "aws_region_name")?;
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
        Ok(region)
    }

    fn endpoint(
        &self,
        request: &CountTokensHandlerRequest<'_>,
        region: &str,
    ) -> Result<String, HandlerFailure> {
        let api_base = read_string(request.litellm_params, "api_base")?;
        let configured = read_string(request.litellm_params, "aws_bedrock_runtime_endpoint")?;
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
        let signed = self.sign(request, body, endpoint, now).await?;
        let outbound = OutboundRequest {
            method: Method::POST,
            url: Url::parse(endpoint).map_err(|source| CountCallError::Endpoint {
                endpoint: endpoint.to_owned(),
                source,
            })?,
            query: Vec::new(),
            headers: header_map(&signed.headers)?,
            body: RequestBody::Json(signed.body),
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

    async fn sign(
        &self,
        request: &CountTokensHandlerRequest<'_>,
        body: &IndexMap<String, Json>,
        endpoint: &str,
        now: SystemTime,
    ) -> Result<AwsSignedRequest, CountCallError> {
        let headers = [(CONTENT_TYPE.to_owned(), Some(JSON_CONTENT_TYPE.to_owned()))];
        let api_key = read_secret(request.litellm_params, "api_key")?;
        sign_service_request(
            ServiceRequest {
                service: ServiceName::Bedrock,
                headers: &headers,
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
        .map_err(CountCallError::Signing)
    }
}

fn transformed_body(
    request: &CountTokensHandlerRequest<'_>,
) -> Result<IndexMap<String, Json>, HandlerFailure> {
    let text = serde_json::to_string(&transform_request(request.request_data))
        .map_err(CountCallError::Encode)?;
    let body: IndexMap<String, Json> =
        serde_json::from_str(&text).map_err(CountCallError::Encode)?;
    tracing::debug!(
        subject = "transformed",
        value = %dumps_object(&body),
        "Transformed request: {}",
        dumps_object(&body)
    );
    Ok(body)
}

fn reply_fields(response: &UpstreamResponse) -> Result<Option<CountResponse>, HandlerFailure> {
    let reply = decode_bedrock_count_reply(&response.body)?;
    tracing::debug!(
        subject = "reply",
        value = %dumps_object(&reply),
        "Bedrock response: {}",
        dumps_object(&reply)
    );
    let transformed = transform_response(&reply);
    let fields = IndexMap::from([("input_tokens".to_owned(), transformed.input_tokens)]);
    tracing::debug!(
        subject = "final",
        value = %dumps_object(&fields),
        "Final response: {}",
        dumps_object(&fields)
    );
    Ok(Some(fields))
}

enum HandlerFailure {
    Bedrock(Box<BedrockError>),
    Processing(Box<CountCallError>),
}

impl From<CountCallError> for HandlerFailure {
    fn from(source: CountCallError) -> Self {
        Self::Processing(Box::new(source))
    }
}

impl From<HeaderError> for HandlerFailure {
    fn from(source: HeaderError) -> Self {
        Self::Processing(Box::new(CountCallError::ErrorHeader { source }))
    }
}

fn recorded(error: Box<BedrockError>) -> CountTokensHandlerError {
    CountTokensHandlerError::Record {
        status: error.error.status_code,
        error,
    }
}

fn source_name(source: &HeaderError) -> String {
    match source {
        HeaderError::NotAscii { name } => name.clone(),
    }
}

fn header_map(headers: &[(String, String)]) -> Result<HeaderMap, CountCallError> {
    let mut map = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        let header_name =
            HeaderName::from_str(name).map_err(|source| CountCallError::HeaderName {
                name: name.clone(),
                source,
            })?;
        let mut header_value =
            HeaderValue::from_str(value).map_err(|source| CountCallError::HeaderValue {
                name: name.clone(),
                source,
            })?;
        if header_name == AUTHORIZATION {
            header_value.set_sensitive(true);
        }
        map.insert(header_name, header_value);
    }
    Ok(map)
}

fn header_pairs(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned()))
        })
        .collect()
}

// LiteLLM reads response.text, which decodes any bytes; betterlite refuses bytes that are not UTF-8
fn response_text(body: Vec<u8>) -> String {
    String::from_utf8(body)
        .unwrap_or_else(|error| format!("{} bytes that are not UTF-8", error.as_bytes().len()))
}

fn upstream_record(
    response: &UpstreamResponse,
    endpoint: &str,
) -> Result<BedrockError, HeaderError> {
    let text = response_text(response.body.clone());
    let headers = header_pairs(&response.headers);
    log_upstream_failure(response.status, &text);
    let reply = ErrorReply {
        response: ErrorResponse {
            status_code: response.status,
            request: ErrorRequest {
                method: "POST".to_owned(),
                url: endpoint.to_owned(),
            },
        },
        headers: headers.clone(),
        body: ResponseBody::Read(text.clone()),
    };
    BedrockError::try_from(ErrorInput {
        status_code: response.status,
        message: text,
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: Some(ErrorHeaders::Http(headers)),
        response: Some(reply),
    })
}

fn log_upstream_failure(status: u16, text: &str) {
    if (400..=599).contains(&status) {
        // AsyncHTTPHandler raises for these statuses, so LiteLLM's HTTPStatusError branch catches them
        tracing::error!(
            subject = "HTTP error in CountTokens handler",
            detail = %text,
            "HTTP error in CountTokens handler: {text}"
        );
    } else {
        tracing::error!(
            subject = "AWS Bedrock error",
            detail = %text,
            "AWS Bedrock error: {text}"
        );
    }
}

fn processing_record(source: &CountCallError) -> Result<BedrockError, HeaderError> {
    let message = format!("CountTokens processing error: {source}");
    tracing::error!(
        subject = "Error in CountTokens handler",
        detail = %source,
        "Error in CountTokens handler: {source}"
    );
    BedrockError::try_from(ErrorInput {
        status_code: 500,
        message,
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: None,
        response: None,
    })
}

fn read_string(
    parameters: &ParamMap,
    field: &'static str,
) -> Result<Option<String>, CountCallError> {
    parameters.get(field).map_or(Ok(None), |raw| {
        serde_json::from_str::<Option<String>>(raw.get())
            .map_err(|source| CountCallError::Field { field, source })
    })
}

fn read_secret(
    parameters: &ParamMap,
    field: &'static str,
) -> Result<Option<SecretString>, CountCallError> {
    parameters.get(field).map_or(Ok(None), |raw| {
        serde_json::from_str::<Option<String>>(raw.get())
            .map(|value| value.map(SecretString::from))
            .map_err(|source| CountCallError::Field { field, source })
    })
}
