use std::iter::empty;
use std::time::{Duration, SystemTime};

use aws_credential_types::Credentials;
use aws_sigv4::http_request::{
    SignableBody, SignableRequest, SignatureLocation, SigningError, SigningParams, SigningSettings,
    sign,
};
use aws_sigv4::sign::v4;
use aws_smithy_runtime_api::client::identity::Identity;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::loader::{ConfigError, EnvironmentLookup};

use super::{
    RawSettings, config_or_environment_flag, non_empty, resolve_secret_opt, resolve_string_opt,
};

const ELASTICACHE_TOKEN_TTL_SECONDS: u64 = 900;
const ELASTICACHE_SERVICE_NAME: &str = "elasticache";

#[derive(Debug, Clone)]
pub struct AwsSettings {
    pub user_name: String,
    pub cache_name: String,
    pub region: String,
    pub serverless: bool,
    pub token_ttl_seconds: u64,
}

impl AwsSettings {
    #[must_use]
    pub fn connect_query(&self) -> String {
        let mut pairs = vec![("Action", "connect"), ("User", self.user_name.as_str())];
        if self.serverless {
            pairs.push(("ResourceType", "ServerlessCache"));
        }
        url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct AwsCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
}

pub trait AwsCredentialsResolver {
    fn resolve(&self) -> Result<AwsCredentials, ElastiCacheSigningError>;
}

#[derive(Debug, Error)]
pub enum ElastiCacheSigningError {
    #[error("failed to resolve AWS credentials for ElastiCache IAM Redis authentication: {reason}")]
    Credentials { reason: String },
    #[error("failed to build the AWS SigV4 signing parameters: {source}")]
    SigningParams {
        #[source]
        source: v4::signing_params::BuildError,
    },
    #[error("failed to build a signable ElastiCache connect request: {source}")]
    SignableRequest {
        #[source]
        source: SigningError,
    },
    #[error("failed to sign the ElastiCache connect request: {source}")]
    Sign {
        #[source]
        source: SigningError,
    },
}

pub struct ElastiCacheIamProvider {
    pub settings: AwsSettings,
}

impl ElastiCacheIamProvider {
    pub fn get_credentials(
        &self,
        resolver: &dyn AwsCredentialsResolver,
        now: SystemTime,
    ) -> Result<(String, String), ElastiCacheSigningError> {
        let credentials = resolver.resolve()?;
        let signed = presign_connect_url(&self.settings, &credentials, now)?;
        Ok((self.settings.user_name.clone(), signed))
    }
}

fn presign_connect_url(
    settings: &AwsSettings,
    credentials: &AwsCredentials,
    now: SystemTime,
) -> Result<String, ElastiCacheSigningError> {
    let identity = Identity::from(Credentials::new(
        credentials.access_key_id.clone(),
        credentials.secret_access_key.clone(),
        credentials.session_token.clone(),
        None,
        "redis-credentials",
    ));

    let mut signing_settings = SigningSettings::default();
    signing_settings.signature_location = SignatureLocation::QueryParams;
    signing_settings.expires_in = Some(Duration::from_secs(settings.token_ttl_seconds));

    let signing_params = SigningParams::from(
        v4::SigningParams::builder()
            .identity(&identity)
            .region(&settings.region)
            .name(ELASTICACHE_SERVICE_NAME)
            .time(now)
            .settings(signing_settings)
            .build()
            .map_err(|source| ElastiCacheSigningError::SigningParams { source })?,
    );

    let uri = format!(
        "https://{}/?{}",
        settings.cache_name,
        settings.connect_query()
    );
    let signable = SignableRequest::new("GET", uri.as_str(), empty(), SignableBody::Bytes(&[]))
        .map_err(|source| ElastiCacheSigningError::SignableRequest { source })?;

    let output = sign(signable, &signing_params)
        .map_err(|source| ElastiCacheSigningError::Sign { source })?;
    let (instructions, _signature) = output.into_parts();
    let (_headers, query_params) = instructions.into_parts();

    let mut query = url::form_urlencoded::Serializer::new(settings.connect_query());
    for (name, value) in query_params {
        query.append_pair(name, value.as_ref());
    }
    Ok(format!("{}/?{}", settings.cache_name, query.finish()))
}

fn required_setting<L>(
    configured: Option<String>,
    path: &str,
    lookup: &L,
    env_fallbacks: &[&str],
    variable: &str,
) -> Result<String, ConfigError>
where
    L: EnvironmentLookup,
{
    let mut resolved = non_empty(resolve_string_opt(configured, path, lookup)?);
    for name in env_fallbacks {
        if resolved.is_some() {
            break;
        }
        resolved = non_empty(lookup.lookup(name));
    }
    resolved.ok_or_else(|| ConfigError::MissingEnvironment {
        variable: variable.to_owned(),
        path: path.to_owned(),
    })
}

pub(super) fn build_aws<L>(settings: &RawSettings, lookup: &L) -> Result<AwsSettings, ConfigError>
where
    L: EnvironmentLookup,
{
    let region = required_setting(
        settings.aws_iam_region.clone(),
        "litellm_settings.cache_params.aws_iam_region",
        lookup,
        &["REDIS_AWS_IAM_REGION", "AWS_REGION", "AWS_DEFAULT_REGION"],
        "aws_iam_region",
    )?;
    let user_name = required_setting(
        settings.aws_iam_user_name.clone(),
        "litellm_settings.cache_params.aws_iam_user_name",
        lookup,
        &["REDIS_AWS_IAM_USER_NAME"],
        "aws_iam_user_name",
    )?;
    let cache_name = required_setting(
        settings.aws_iam_cache_name.clone(),
        "litellm_settings.cache_params.aws_iam_cache_name",
        lookup,
        &["REDIS_AWS_IAM_CACHE_NAME"],
        "aws_iam_cache_name",
    )?;
    ensure_tls(settings, lookup)?;
    let serverless = config_or_environment_flag(
        settings.aws_iam_serverless.as_ref(),
        lookup,
        "REDIS_AWS_IAM_SERVERLESS",
    );
    Ok(AwsSettings {
        user_name,
        cache_name: cache_name.to_ascii_lowercase(),
        region,
        serverless,
        token_ttl_seconds: ELASTICACHE_TOKEN_TTL_SECONDS,
    })
}

fn ensure_tls<L>(settings: &RawSettings, lookup: &L) -> Result<(), ConfigError>
where
    L: EnvironmentLookup,
{
    let url = resolve_secret_opt(
        settings
            .url
            .as_ref()
            .map(|secret| secret.expose_secret().to_owned()),
        "litellm_settings.cache_params.url",
        lookup,
        "REDIS_URL",
    )?;
    let ssl_enabled = config_or_environment_flag(settings.ssl.as_ref(), lookup, "REDIS_SSL");
    if uses_tls(url.as_ref(), ssl_enabled)? {
        Ok(())
    } else {
        Err(ConfigError::InvalidScalar {
            path: "litellm_settings.cache_params.ssl".to_owned(),
            value: false,
        })
    }
}

fn uses_tls(configured_url: Option<&SecretString>, ssl_enabled: bool) -> Result<bool, ConfigError> {
    let Some(present_url) = configured_url else {
        return Ok(ssl_enabled);
    };
    let parsed = url::Url::parse(present_url.expose_secret()).map_err(|_parse_error| {
        ConfigError::InvalidScalar {
            path: "litellm_settings.cache_params.url".to_owned(),
            value: false,
        }
    })?;
    Ok(parsed.scheme().eq_ignore_ascii_case("rediss"))
}
