use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use google_cloud_auth::build_errors::Error as BuildError;
use google_cloud_auth::credentials::AccessTokenCredentials;
use google_cloud_auth::credentials::external_account;
use google_cloud_auth::credentials::mds;
use google_cloud_auth::credentials::service_account::{
    AccessSpecifier, Builder as ServiceAccountBuilder,
};
use google_cloud_auth::credentials::user_account;
use google_cloud_gax::backoff_policy::BackoffPolicyArg;
use google_cloud_gax::exponential_backoff::ExponentialBackoff;
use google_cloud_gax::retry_policy::{AlwaysRetry, RetryPolicyArg, RetryPolicyExt as _};
use google_cloud_gax::retry_throttler::{AdaptiveThrottler, RetryThrottlerArg};
use http::header::HeaderValue;
use http::{HeaderMap, Method};
use secrecy::SecretString;
use url::Url;

use super::aws::{AwsCredentialsSource, AwsWifTokenSource};
use super::error::CredentialSourceError;
use crate::aws_federation::FederationInput;

const DEFAULT_METADATA_HOST: &str = "metadata.google.internal";
const METADATA_PROJECT_PATH: &str = "/computeMetadata/v1/project/project-id";
const METADATA_PROJECT_DEADLINE: Duration = Duration::from_secs(5);

// the retry, backoff and throttling the SDK applies to each token request
#[derive(Clone, Debug)]
pub struct HttpPolicy {
    pub retry: RetryPolicyArg,
    pub backoff: BackoffPolicyArg,
    pub throttler: RetryThrottlerArg,
}

impl Default for HttpPolicy {
    fn default() -> Self {
        Self {
            retry: RetryPolicyArg::from(AlwaysRetry.with_attempt_limit(3)),
            backoff: BackoffPolicyArg::from(ExponentialBackoff::default()),
            throttler: RetryThrottlerArg::from(AdaptiveThrottler::default()),
        }
    }
}

pub trait TokenSource: Send + Sync {
    fn access_token(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<SecretString, CredentialSourceError>> + Send + '_>>;
}

// the six constructors VertexBase.load_auth dispatches to, behind a seam so a test injects them
pub trait CredentialBuilders: Send + Sync {
    fn service_account(
        &self,
        document: &str,
        scopes: &[String],
    ) -> Result<Arc<dyn TokenSource>, CredentialSourceError>;
    fn authorized_user(
        &self,
        document: &str,
        scopes: &[String],
    ) -> Result<Arc<dyn TokenSource>, CredentialSourceError>;
    fn identity_pool(
        &self,
        document: &str,
        scopes: &[String],
    ) -> Result<Arc<dyn TokenSource>, CredentialSourceError>;
    fn identity_pool_with_aws(
        &self,
        document: &str,
        scopes: &[String],
    ) -> Result<Arc<dyn TokenSource>, CredentialSourceError>;
    fn metadata(
        &self,
        endpoint: Option<&str>,
        scopes: &[String],
    ) -> Result<Arc<dyn TokenSource>, CredentialSourceError>;
    fn aws_federation(
        &self,
        input: FederationInput,
        resolver: Arc<dyn AwsCredentialsSource>,
        clock: Arc<dyn CredentialClock>,
    ) -> Result<Arc<dyn TokenSource>, CredentialSourceError>;
    fn metadata_project(
        &self,
        endpoint: Option<&str>,
    ) -> Pin<Box<dyn Future<Output = Option<String>> + Send + '_>>;
}

pub trait CredentialClock: Send + Sync {
    fn now(&self) -> SystemTime;
}

pub struct SystemClock;

impl CredentialClock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

pub struct GoogleAuth {
    pub client: ProxiedClient,
    pub policy: HttpPolicy,
}

struct GoogleTokenSource(AccessTokenCredentials);

impl TokenSource for GoogleTokenSource {
    fn access_token(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<SecretString, CredentialSourceError>> + Send + '_>>
    {
        Box::pin(async move {
            let token = self
                .0
                .access_token()
                .await
                .map_err(CredentialSourceError::Refresh)?;
            Ok(SecretString::from(token.token))
        })
    }
}

fn build(
    kind: &'static str,
    result: Result<AccessTokenCredentials, BuildError>,
) -> Result<Arc<dyn TokenSource>, CredentialSourceError> {
    let credentials = result.map_err(|source| CredentialSourceError::Build { kind, source })?;
    let source: Arc<dyn TokenSource> = Arc::new(GoogleTokenSource(credentials));
    Ok(source)
}

// every refresh-based builder applies the same scopes and HTTP policy to its SDK builder
macro_rules! with_http_policy {
    ($builder:expr, $scopes:expr, $policy:expr) => {
        $builder
            .with_scopes($scopes.iter().cloned())
            .with_retry_policy($policy.retry.clone())
            .with_backoff_policy($policy.backoff.clone())
            .with_retry_throttler($policy.throttler.clone())
    };
}

impl CredentialBuilders for GoogleAuth {
    fn service_account(
        &self,
        document: &str,
        scopes: &[String],
    ) -> Result<Arc<dyn TokenSource>, CredentialSourceError> {
        let key = serde_json::from_str(document).map_err(CredentialSourceError::Document)?;
        let builder = ServiceAccountBuilder::new(key)
            .with_access_specifier(AccessSpecifier::from_scopes(scopes.iter().cloned()));
        build("service_account", builder.build_access_token_credentials())
    }

    fn authorized_user(
        &self,
        document: &str,
        scopes: &[String],
    ) -> Result<Arc<dyn TokenSource>, CredentialSourceError> {
        let key = serde_json::from_str(document).map_err(CredentialSourceError::Document)?;
        let builder = with_http_policy!(user_account::Builder::new(key), scopes, &self.policy);
        build("authorized_user", builder.build_access_token_credentials())
    }

    fn identity_pool(
        &self,
        document: &str,
        scopes: &[String],
    ) -> Result<Arc<dyn TokenSource>, CredentialSourceError> {
        let key = serde_json::from_str(document).map_err(CredentialSourceError::Document)?;
        let builder = with_http_policy!(external_account::Builder::new(key), scopes, &self.policy);
        build("external_account", builder.build_access_token_credentials())
    }

    // google-cloud-auth picks the AWS subject token from credential_source itself
    fn identity_pool_with_aws(
        &self,
        document: &str,
        scopes: &[String],
    ) -> Result<Arc<dyn TokenSource>, CredentialSourceError> {
        self.identity_pool(document, scopes)
    }

    fn metadata(
        &self,
        endpoint: Option<&str>,
        scopes: &[String],
    ) -> Result<Arc<dyn TokenSource>, CredentialSourceError> {
        let mut builder = with_http_policy!(mds::Builder::default(), scopes, &self.policy);
        if let Some(host) = endpoint {
            builder = builder.with_endpoint(format!("http://{host}"));
        }
        build("metadata", builder.build_access_token_credentials())
    }

    fn aws_federation(
        &self,
        input: FederationInput,
        resolver: Arc<dyn AwsCredentialsSource>,
        clock: Arc<dyn CredentialClock>,
    ) -> Result<Arc<dyn TokenSource>, CredentialSourceError> {
        Ok(Arc::new(AwsWifTokenSource {
            input,
            resolver,
            client: self.client.clone(),
            clock,
        }))
    }

    fn metadata_project(
        &self,
        endpoint: Option<&str>,
    ) -> Pin<Box<dyn Future<Output = Option<String>> + Send + '_>> {
        let owned_endpoint = endpoint.map(str::to_owned);
        Box::pin(async move {
            let host = owned_endpoint.as_deref().unwrap_or(DEFAULT_METADATA_HOST);
            let url = Url::parse(&format!("http://{host}{METADATA_PROJECT_PATH}")).ok()?;
            let mut headers = HeaderMap::new();
            headers.insert("metadata-flavor", HeaderValue::from_static("Google"));
            let request = OutboundRequest {
                method: Method::GET,
                url,
                query: Vec::new(),
                headers,
                body: RequestBody::Empty,
                deadline: METADATA_PROJECT_DEADLINE,
            };
            let response = self.client.send_buffered(request).await.ok()?;
            if response.status != 200 {
                return None;
            }
            let text = String::from_utf8(response.body).ok()?;
            let project = text.trim();
            (!project.is_empty()).then(|| project.to_owned())
        })
    }
}
