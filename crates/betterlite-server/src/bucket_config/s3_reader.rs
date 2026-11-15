use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use aws_credential_types::Credentials as SigningCredentials;
use aws_sigv4::http_request::{
    PayloadChecksumKind, PercentEncodingMode, SessionTokenMode, SignableBody, SignableRequest,
    SignatureLocation, SigningError, SigningParams, SigningSettings, UriPathNormalizationMode,
    sign,
};
use aws_sigv4::sign::v4;
use aws_sigv4::sign::v4::signing_params::BuildError;
use aws_smithy_async::rt::sleep::{SharedAsyncSleep, TokioSleep};
use aws_smithy_async::time::SharedTimeSource;
use aws_smithy_http_client::tls::{Provider, rustls_provider::CryptoMode};
use aws_smithy_runtime_api::client::identity::Identity;
use aws_types::os_shim_internal::Fs;
use betterlite_bedrock::aws_settings::sources::{Credentials, NativeSources, SourceError};
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use secrecy::ExposeSecret as _;
use url::Url;

use super::object::BucketObjectReader;

pub const DEFAULT_REGION: &str = "us-east-1";
pub const REGION_VARIABLE: &str = "AWS_DEFAULT_REGION";

const SERVICE: &str = "s3";
const METHOD: &str = "GET";
const CONTENT_SHA256_HEADER: &str = "x-amz-content-sha256";
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const CREDENTIALS_TIMEOUT: Duration = Duration::from_secs(60);

// Python's quote leaves the unreserved characters alone, so only these and "/" stay as themselves
const OBJECT_KEY_SAFE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~')
    .remove(b'/');

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

pub trait Clock: Send + Sync + Debug {
    fn now(&self) -> SystemTime;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum S3SigningError {
    #[error("the SigV4 signing parameters for S3 are not valid")]
    Parameters(#[source] BuildError),
    #[error("the S3 request could not be signed")]
    Signing(#[source] SigningError),
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

    #[must_use]
    pub fn object_url(&self, object_key: &str) -> String {
        let key = utf8_percent_encode(object_key, OBJECT_KEY_SAFE);
        if self.bucket_name.contains('.') {
            return format!("https://s3.{}/{}/{}", self.region, self.bucket_name, key);
        }
        format!("https://{}.s3.{}/{}", self.bucket_name, self.region, key)
    }

    fn signed_headers(
        &self,
        url: &str,
        credentials: &Credentials,
    ) -> Result<Vec<(String, String)>, S3SigningError> {
        let identity = Identity::from(SigningCredentials::new(
            credentials.access_key_id.expose_secret().to_owned(),
            credentials.secret_access_key.expose_secret().to_owned(),
            credentials
                .session_token
                .as_ref()
                .map(|token| token.expose_secret().to_owned()),
            None,
            "betterlite-server",
        ));
        let mut settings = SigningSettings::default();
        // botocore's S3 signer leaves the path as sent, so the canonical form must not normalize it
        settings.uri_path_normalization_mode = UriPathNormalizationMode::Disabled;
        settings.percent_encoding_mode = PercentEncodingMode::Single;
        settings.payload_checksum_kind = PayloadChecksumKind::NoHeader;
        settings.signature_location = SignatureLocation::Headers;
        settings.session_token_mode = SessionTokenMode::Include;
        let parameters = v4::SigningParams::builder()
            .identity(&identity)
            .region(&self.region)
            .name(SERVICE)
            .time(self.clock.now())
            .settings(settings)
            .build()
            .map_err(S3SigningError::Parameters)?;
        let prepared = [(CONTENT_SHA256_HEADER, EMPTY_SHA256)];
        let signable = SignableRequest::new(
            METHOD,
            url,
            prepared.iter().copied(),
            SignableBody::Precomputed(EMPTY_SHA256.to_owned()),
        )
        .map_err(S3SigningError::Signing)?;
        let output =
            sign(signable, &SigningParams::from(parameters)).map_err(S3SigningError::Signing)?;
        let (instructions, _signature) = output.into_parts();
        let (signed, _query) = instructions.into_parts();
        let mut headers = Vec::with_capacity(signed.len().saturating_add(prepared.len()));
        headers.extend(
            prepared
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
        );
        headers.extend(
            signed
                .into_iter()
                .map(|header| (header.name().to_owned(), header.value().to_owned())),
        );
        Ok(headers)
    }

    async fn fetch(&self, object_key: &str) -> Option<Vec<u8>> {
        let Some(credentials) = self.credentials.as_ref() else {
            return None;
        };
        let url = self.object_url(object_key);
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

mod tests;
