use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use aws_smithy_async::rt::sleep::{SharedAsyncSleep, TokioSleep};
use aws_smithy_async::time::SharedTimeSource;
use aws_smithy_http_client::tls::{Provider, rustls_provider::CryptoMode};
use aws_types::os_shim_internal::Fs;
use betterlite_bedrock::aws_settings::sources::{Credentials, NativeSources, SourceError};
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::s3_transport::download::EMPTY_SHA256;
use betterlite_logging::s3_transport::error::S3TransportError;
use betterlite_logging::s3_transport::signing::{Clock, S3_SERVICE, SystemClock};
use betterlite_sigv4::sign::{Signable, SigningMode, sign_request};
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue};
use url::Url;

use super::object::BucketObjectReader;

pub const DEFAULT_REGION: &str = "us-east-1";
pub const REGION_VARIABLE: &str = "AWS_DEFAULT_REGION";

const METHOD: &str = "GET";
const CONTENT_SHA256_HEADER: &str = "x-amz-content-sha256";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const CREDENTIALS_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3GetRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3GetResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[async_trait]
pub trait S3Get: Send + Sync + Debug {
    async fn get(&self, request: S3GetRequest) -> Result<S3GetResponse, S3TransportFailure>;
}

#[derive(Debug, thiserror::Error)]
pub enum S3TransportFailure {
    #[error("the S3 object URL is not valid")]
    Url(#[source] url::ParseError),
    #[error("the S3 request header {name} is not valid")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the S3 request header value for {name} is not valid")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error(transparent)]
    Transport(#[from] TransportError),
}

#[derive(Debug, Clone)]
pub struct ProxiedS3Get {
    pub client: ProxiedClient,
}

impl ProxiedS3Get {
    pub fn new(policy: &Arc<ProxyPolicy>) -> Result<Self, HttpError> {
        Ok(Self {
            client: ProxiedClient::outbound(policy)?,
        })
    }
}

fn header_map(headers: &[(String, String)]) -> Result<HeaderMap, S3TransportFailure> {
    let mut map = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
            S3TransportFailure::HeaderName {
                name: name.clone(),
                source,
            }
        })?;
        let header_value =
            HeaderValue::from_str(value).map_err(|source| S3TransportFailure::HeaderValue {
                name: name.clone(),
                source,
            })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

#[async_trait]
impl S3Get for ProxiedS3Get {
    async fn get(&self, request: S3GetRequest) -> Result<S3GetResponse, S3TransportFailure> {
        let url = Url::parse(&request.url).map_err(S3TransportFailure::Url)?;
        let outbound = OutboundRequest {
            method: Method::GET,
            url,
            query: Vec::new(),
            headers: header_map(&request.headers)?,
            body: RequestBody::Empty,
            deadline: REQUEST_TIMEOUT,
        };
        let response = self.client.send_buffered(outbound).await?;
        Ok(S3GetResponse {
            status: response.status,
            body: response.body,
        })
    }
}

pub struct S3Reader {
    pub bucket_name: String,
    pub region: String,
    pub credentials: Option<Credentials>,
    pub transport: Arc<dyn S3Get>,
    pub clock: Arc<dyn Clock>,
}

impl S3Reader {
    #[must_use]
    pub async fn initialize(
        bucket_name: &str,
        environment: &BTreeMap<String, String>,
        transport: Arc<dyn S3Get>,
    ) -> Self {
        let region = region_from_environment(environment);
        let credentials = match resolve_credentials(&region, environment).await {
            Ok(credentials) => Some(credentials),
            Err(error) => {
                tracing::error!(
                    bucket_name,
                    cause = %error,
                    "Error creating the S3 client for bucket"
                );
                None
            }
        };
        Self {
            bucket_name: bucket_name.to_owned(),
            region,
            credentials,
            transport,
            clock: Arc::new(SystemClock),
        }
    }

    fn signed_headers(
        &self,
        url: &str,
        credentials: &Credentials,
    ) -> Result<Vec<(String, String)>, S3TransportError> {
        let prepared = [(CONTENT_SHA256_HEADER.to_owned(), EMPTY_SHA256.to_owned())];
        let signed = sign_request(&Signable {
            method: METHOD,
            url,
            headers: &prepared,
            body: &[],
            service: S3_SERVICE,
            region: &self.region,
            credentials,
            mode: SigningMode::S3SigV4Auth,
            now: self.clock.now(),
        })?;
        let mut headers = Vec::with_capacity(prepared.len().saturating_add(signed.len()));
        headers.extend(prepared);
        headers.extend(signed);
        Ok(headers)
    }

    async fn fetch(&self, object_key: &str) -> Option<Vec<u8>> {
        let credentials = self.credentials.as_ref()?;
        let url = addressing::object_url(&self.bucket_name, &self.region, object_key);
        let headers = match self.signed_headers(&url, credentials) {
            Ok(headers) => headers,
            Err(error) => {
                tracing::error!(
                    bucket_name = %self.bucket_name,
                    object_key,
                    cause = %error,
                    "Error retrieving object from S3 bucket"
                );
                return None;
            }
        };
        let response = match self.transport.get(S3GetRequest { url, headers }).await {
            Ok(response) => response,
            Err(error) => {
                tracing::error!(
                    bucket_name = %self.bucket_name,
                    object_key,
                    cause = %error,
                    "Error retrieving object from S3 bucket"
                );
                return None;
            }
        };
        if response.status != 200 {
            tracing::error!(
                bucket_name = %self.bucket_name,
                object_key,
                status = response.status,
                "Error retrieving object from S3 bucket"
            );
            return None;
        }
        Some(response.body)
    }
}

#[async_trait]
impl BucketObjectReader for S3Reader {
    async fn read_object(&self, object_key: &str) -> Option<Vec<u8>> {
        self.fetch(object_key).await
    }
}

fn region_from_environment(environment: &BTreeMap<String, String>) -> String {
    environment
        .get(REGION_VARIABLE)
        .filter(|value| !value.is_empty())
        .cloned()
        .unwrap_or_else(|| DEFAULT_REGION.to_owned())
}

fn credential_settings(region: &str) -> AwsSettings {
    AwsSettings {
        access_key_id: None,
        secret_access_key: None,
        session_token: None,
        region_name: Some(region.to_owned()),
        session_name: None,
        profile_name: None,
        role_name: None,
        web_identity_token: None,
        sts_endpoint: None,
        external_id: None,
        session_tags: None,
        role_arn: None,
        web_identity_token_file: None,
        bearer_token: None,
    }
}

async fn resolve_credentials(
    region: &str,
    environment: &BTreeMap<String, String>,
) -> Result<Credentials, SourceError> {
    let native = NativeSources {
        environment: environment.clone(),
        filesystem: Fs::real(),
        client: aws_smithy_http_client::Builder::new()
            .tls_provider(Provider::Rustls(CryptoMode::AwsLc))
            .build_https(),
        clock: SharedTimeSource::default(),
        sleep: SharedAsyncSleep::new(TokioSleep::new()),
        timeout: CREDENTIALS_TIMEOUT,
    };
    native.load(&credential_settings(region)).await
}

mod addressing;
mod tests;
