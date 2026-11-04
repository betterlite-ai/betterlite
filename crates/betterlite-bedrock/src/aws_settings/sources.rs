use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::time::Duration;

use aws_config::{BehaviorVersion, ConfigLoader, retry::RetryConfig, timeout::TimeoutConfig};
use aws_credential_types::provider::{ProvideCredentials as _, error::CredentialsError};
use aws_smithy_async::{rt::sleep::SharedAsyncSleep, time::SharedTimeSource};
use aws_smithy_runtime_api::client::http::SharedHttpClient;
use aws_types::{
    os_shim_internal::{Env, Fs},
    region::Region,
};
use betterlite_config::sections::bedrock_settings::AwsSettings;
use secrecy::SecretString;

#[derive(thiserror::Error)]
pub enum SourceError {
    #[error("AWS credential source failed")]
    Provider(#[source] CredentialsError),
    #[error("AWS credential provider is unavailable")]
    MissingProvider,
}

impl fmt::Debug for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[derive(Debug, Clone)]
pub struct Credentials {
    pub access_key_id: SecretString,
    pub secret_access_key: SecretString,
    pub session_token: Option<SecretString>,
}

#[derive(Debug, Clone)]
pub struct ResolvedCredentials {
    pub credentials: Option<Credentials>,
    pub cache_ttl: Option<Duration>,
}

pub struct NativeSources {
    pub environment: BTreeMap<String, String>,
    pub filesystem: Fs,
    pub client: SharedHttpClient,
    pub clock: SharedTimeSource,
    pub sleep: SharedAsyncSleep,
    pub timeout: Duration,
}

impl NativeSources {
    fn loader(&self, settings: &AwsSettings) -> ConfigLoader {
        let mut environment = HashMap::from_iter(self.environment.clone());
        if settings.profile_name.is_some() {
            for name in [
                "AWS_ACCESS_KEY_ID",
                "AWS_SECRET_ACCESS_KEY",
                "AWS_SESSION_TOKEN",
            ] {
                environment.remove(name);
            }
        }
        let loader = aws_config::defaults(BehaviorVersion::latest())
            .env(Env::from(environment))
            .fs(self.filesystem.clone())
            .http_client(self.client.clone())
            .time_source(self.clock.clone())
            .sleep_impl(self.sleep.clone())
            .retry_config(RetryConfig::disabled())
            .timeout_config(
                TimeoutConfig::builder()
                    .operation_timeout(self.timeout)
                    .build(),
            )
            .region(settings.region_name.clone().map(Region::new));
        match &settings.profile_name {
            Some(profile) => loader.profile_name(profile),
            None => loader,
        }
    }

    pub async fn load(&self, settings: &AwsSettings) -> Result<Credentials, SourceError> {
        let configured = self.loader(settings).load().await;
        let provider = configured
            .credentials_provider()
            .ok_or(SourceError::MissingProvider)?;
        let supplied = provider
            .provide_credentials()
            .await
            .map_err(SourceError::Provider)?;
        Ok(Credentials {
            access_key_id: SecretString::from(supplied.access_key_id().to_owned()),
            secret_access_key: SecretString::from(supplied.secret_access_key().to_owned()),
            session_token: supplied
                .session_token()
                .map(|token| SecretString::from(token.to_owned())),
        })
    }
}

pub async fn resolve_local_credentials(
    settings: &AwsSettings,
    native: &NativeSources,
) -> Result<ResolvedCredentials, SourceError> {
    if settings.profile_name.is_some() {
        return Ok(ResolvedCredentials {
            credentials: Some(native.load(settings).await?),
            cache_ttl: None,
        });
    }
    if let (Some(access), Some(secret)) = (&settings.access_key_id, &settings.secret_access_key)
        && (settings.session_token.is_some() || settings.region_name.is_some())
    {
        return Ok(ResolvedCredentials {
            credentials: Some(Credentials {
                access_key_id: access.clone(),
                secret_access_key: secret.clone(),
                session_token: settings.session_token.clone(),
            }),
            cache_ttl: settings
                .session_token
                .is_none()
                .then_some(Duration::from_mins(59)),
        });
    }
    Ok(ResolvedCredentials {
        credentials: Some(native.load(settings).await?),
        cache_ttl: None,
    })
}

#[must_use]
pub const fn auth_selector_presence(inputs: &AwsSettings) -> [bool; 5] {
    [
        inputs.web_identity_token.is_some()
            && inputs.role_name.is_some()
            && inputs.session_name.is_some(),
        inputs.role_name.is_some(),
        inputs.profile_name.is_some(),
        inputs.access_key_id.is_some()
            && inputs.secret_access_key.is_some()
            && inputs.session_token.is_some(),
        inputs.access_key_id.is_some()
            && inputs.secret_access_key.is_some()
            && inputs.region_name.is_some(),
    ]
}
