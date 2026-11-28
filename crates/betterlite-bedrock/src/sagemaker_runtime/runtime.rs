use std::future::Future;
use std::pin::Pin;
use std::str::from_utf8;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aws_smithy_async::time::SharedTimeSource;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::sagemaker_completion::error::SagemakerError;
use betterlite_openai::sagemaker_completion::response::TextTokenCounter;
use futures_util::{Stream, StreamExt as _};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use indexmap::IndexMap;
use url::Url;
use uuid::Uuid;

use crate::aws_settings::pop_aws_auth_params;
use crate::aws_settings::sources::Credentials;
use crate::credentials::CredentialSource;
use crate::signing::error::SigningError;
use crate::signing::target::{RegionInputs, auth_settings};

use super::endpoint::ParamMap;
use super::error::RuntimeError;

pub type ChunkStream = Pin<Box<dyn Stream<Item = Result<Vec<u8>, HttpError>> + Send>>;

const SECRET_HEADERS: [&str; 2] = ["authorization", "x-amz-security-token"];
// an error body is read only to report it, so a runaway reply cannot hold the call open past this
const ERROR_BODY_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Default)]
pub struct SagemakerEnvironment {
    pub region_name: Option<String>,
    pub region: Option<String>,
    pub session_region: Option<String>,
}

impl SagemakerEnvironment {
    pub(crate) fn inputs(&self) -> RegionInputs<'_> {
        RegionInputs {
            environment_region_name: self.region_name.as_deref(),
            environment_region: self.region.as_deref(),
            session_region: self.session_region.as_deref(),
        }
    }
}

pub trait SagemakerTransport: Send + Sync {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;

    fn open_stream(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<(u16, ChunkStream), TransportError>> + Send;
}

impl SagemakerTransport for ProxiedClient {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        // LiteLLM's httpx clients follow redirects, so each hop is checked the same way
        self.send_following(request)
            .await?
            .buffered(self.limits.body_bytes)
            .await
    }

    async fn open_stream(
        &self,
        request: OutboundRequest,
    ) -> Result<(u16, ChunkStream), TransportError> {
        let response = self.send_following(request).await?;
        let status = response.response.status().as_u16();
        Ok((status, response.body_stream()))
    }
}

pub struct SagemakerRuntime<'deps, T, C> {
    pub transport: T,
    pub credentials: C,
    pub environment: SagemakerEnvironment,
    pub clock: SharedTimeSource,
    pub counter: &'deps (dyn TextTokenCounter + Sync),
    pub timeout: Duration,
    pub identifier: fn() -> Uuid,
}

pub struct PreparedCall {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub body_json: IndexMap<String, Json>,
    pub region: String,
}

impl PreparedCall {
    #[must_use]
    pub fn additional_args(&self) -> AdditionalArgs {
        let headers = self
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
            .collect();
        AdditionalArgs {
            api_base: Some(self.url.clone()),
            headers: Some(headers),
            complete_input_dict: Some(RawBody::Object(self.body_json.clone())),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        }
    }
}

impl<T, C> SagemakerRuntime<'_, T, C> {
    pub(crate) fn outbound(
        &self,
        prepared: &PreparedCall,
    ) -> Result<OutboundRequest, RuntimeError> {
        Ok(OutboundRequest {
            method: Method::POST,
            url: Url::parse(&prepared.url).map_err(|_parse| RuntimeError::Endpoint {
                endpoint: prepared.url.clone(),
            })?,
            query: Vec::new(),
            headers: header_map(&prepared.headers)?,
            body: RequestBody::Json(prepared.body.clone()),
            deadline: self.timeout,
        })
    }
}

impl<T, C> SagemakerRuntime<'_, T, C>
where
    T: Sync,
    C: CredentialSource,
{
    pub(crate) async fn credentials_for(
        &self,
        params: &ParamMap,
        region: &str,
    ) -> Result<Credentials, RuntimeError> {
        let mut copy = params.clone();
        let auth = pop_aws_auth_params(&mut copy).map_err(SigningError::from)?;
        let settings = auth_settings(auth, region);
        Ok(self
            .credentials
            .resolve(settings, region.to_owned())
            .await?)
    }
}

fn header_map(headers: &[(String, String)]) -> Result<HeaderMap, RuntimeError> {
    let mut map = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        let (Ok(header_name), Ok(mut header_value)) = (
            HeaderName::try_from(name.as_str()),
            HeaderValue::try_from(value.as_str()),
        ) else {
            return Err(RuntimeError::Header { name: name.clone() });
        };
        header_value.set_sensitive(SECRET_HEADERS.contains(&header_name.as_str()));
        map.append(header_name, header_value);
    }
    Ok(map)
}

#[must_use]
pub fn wire_error(status: u16, body: &[u8]) -> SagemakerError {
    SagemakerError {
        status_code: status,
        message: body_text(body),
        headers: None,
    }
}

// a failed stream still carries its body, which LiteLLM cannot read on an unread response
#[must_use]
pub async fn stream_error(status: u16, mut chunks: ChunkStream) -> SagemakerError {
    let mut body = Vec::new();
    while body.len() < ERROR_BODY_BYTES {
        match chunks.next().await {
            Some(Ok(chunk)) => {
                let remaining = ERROR_BODY_BYTES.saturating_sub(body.len());
                body.extend(chunk.iter().copied().take(remaining));
            }
            Some(Err(_)) | None => break,
        }
    }
    wire_error(status, &body)
}

#[must_use]
pub fn reply_headers(headers: &HeaderMap) -> Headers {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|text| (name.as_str().to_owned(), text.to_owned()))
        })
        .collect()
}

#[must_use]
pub fn body_text(body: &[u8]) -> String {
    from_utf8(body).map_or_else(
        |_invalid| format!("{} bytes that are not UTF-8", body.len()),
        str::to_owned,
    )
}

#[must_use]
pub fn epoch_seconds(now: SystemTime) -> u64 {
    now.duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

mod tests;
