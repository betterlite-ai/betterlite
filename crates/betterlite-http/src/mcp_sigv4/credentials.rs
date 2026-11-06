use std::time::UNIX_EPOCH;

use aws_config::{BehaviorVersion, ConfigLoader, timeout::TimeoutConfig};
use aws_credential_types::{
    Credentials,
    provider::{ProvideCredentials as _, SharedCredentialsProvider, error::CredentialsError},
};
use aws_sdk_sts::Client;
use aws_types::{os_shim_internal::Env, region::Region};

use super::error::McpSigV4Error;
use super::{AwsSources, McpSigV4Settings};

pub(super) const PROVIDER_NAME: &str = "mcp-sigv4";

fn loader(sources: &AwsSources) -> ConfigLoader {
    aws_config::defaults(BehaviorVersion::latest())
        .env(Env::from(sources.environment.clone()))
        .fs(sources.filesystem.clone())
        .http_client(sources.client.clone())
        .time_source(sources.clock.clone())
        .sleep_impl(sources.sleep.clone())
        .timeout_config(
            TimeoutConfig::builder()
                .operation_timeout(sources.timeout)
                .build(),
        )
}

pub(super) async fn assume_role(
    role_arn: &str,
    explicit: Option<Credentials>,
    settings: &McpSigV4Settings,
    region_name: &str,
    sources: &AwsSources,
) -> Result<Credentials, McpSigV4Error> {
    let seconds = sources
        .clock
        .now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| McpSigV4Error::ClockBeforeEpoch { source })?
        .as_secs();
    let session_name = settings
        .aws_session_name
        .clone()
        .unwrap_or_else(|| format!("litellm-mcp-{seconds}"));
    let loader = loader(sources).region(Region::new(region_name.to_owned()));
    let config = match explicit {
        Some(credentials) => loader.credentials_provider(credentials).load().await,
        None => loader.load().await,
    };
    let response = Client::new(&config)
        .assume_role()
        .role_arn(role_arn)
        .role_session_name(session_name)
        .send()
        .await
        .map_err(|source| McpSigV4Error::AssumeRole {
            source: Box::new(source),
        })?;
    let temporary = response
        .credentials()
        .ok_or(McpSigV4Error::AssumeRoleWithoutCredentials)?;
    Ok(Credentials::new(
        temporary.access_key_id(),
        temporary.secret_access_key(),
        Some(temporary.session_token().to_owned()),
        None,
        PROVIDER_NAME,
    ))
}

pub(super) async fn get_credentials(
    sources: &AwsSources,
) -> Result<Option<SharedCredentialsProvider>, McpSigV4Error> {
    let config = loader(sources).load().await;
    let Some(provider) = config.credentials_provider() else {
        return Ok(None);
    };
    match provider.provide_credentials().await {
        Ok(_) => Ok(Some(provider)),
        Err(CredentialsError::CredentialsNotLoaded(_)) => Ok(None),
        Err(source) => Err(McpSigV4Error::Provider { source }),
    }
}
