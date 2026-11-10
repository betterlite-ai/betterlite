use std::collections::HashMap;
use std::time::Duration;

use aws_config::timeout::TimeoutConfig;
use aws_config::{BehaviorVersion, ConfigLoader};
use aws_credential_types::Credentials;
use aws_credential_types::provider::ProvideCredentials as _;
use aws_credential_types::provider::error::CredentialsError;
use aws_sdk_sts::Client;
use aws_sdk_sts::types::Credentials as StsCredentials;
use aws_types::os_shim_internal::Env;
use aws_types::region::Region;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::RdsCredentialsError;
use super::{CREDENTIAL_PROVIDER, RdsSources, WEB_IDENTITY_DURATION_SECONDS};

// botocore's default connect and read timeout is 60 s, and the configured RDS timeout never reaches STS
const STS_TIMEOUT: Duration = Duration::from_secs(60);

impl RdsSources<'_> {
    pub(super) fn loader(
        &self,
        environment: HashMap<String, String>,
        region: &str,
        explicit: Option<Credentials>,
    ) -> ConfigLoader {
        let loader = aws_config::defaults(BehaviorVersion::latest())
            .env(Env::from(environment))
            .fs(self.filesystem.clone())
            .http_client(self.client.clone())
            .time_source(self.clock.clone())
            .sleep_impl(self.sleep.clone())
            .region(Region::new(region.to_owned()))
            .timeout_config(
                TimeoutConfig::builder()
                    .connect_timeout(STS_TIMEOUT)
                    .read_timeout(STS_TIMEOUT)
                    .build(),
            );
        match explicit {
            Some(credentials) => loader.credentials_provider(credentials),
            None => loader,
        }
    }
}

pub(super) fn environment_map(sources: &RdsSources<'_>) -> HashMap<String, String> {
    let mut environment = HashMap::with_capacity(sources.environment.len());
    for (name, value) in sources.environment {
        environment.insert(name.clone(), value.clone());
    }
    environment
}

pub(super) async fn assume_role_with_web_identity(
    role: &str,
    session: &str,
    oidc_token: &SecretString,
    region: &str,
    sources: &RdsSources<'_>,
) -> Result<Credentials, RdsCredentialsError> {
    let config = sources
        .loader(environment_map(sources), region, None)
        .load()
        .await;
    let response = Client::new(&config)
        .assume_role_with_web_identity()
        .role_arn(role)
        .role_session_name(session)
        .web_identity_token(oidc_token.expose_secret())
        .duration_seconds(WEB_IDENTITY_DURATION_SECONDS)
        .send()
        .await
        .map_err(|source| RdsCredentialsError::AssumeRoleWithWebIdentity {
            source: Box::new(source),
        })?;
    let temporary = response
        .credentials()
        .ok_or(RdsCredentialsError::StsWithoutCredentials)?;
    Ok(temporary_credentials(temporary))
}

pub(super) async fn assume_role(
    role: &str,
    session: &str,
    credentials: Option<Credentials>,
    region: &str,
    sources: &RdsSources<'_>,
) -> Result<Credentials, RdsCredentialsError> {
    let config = sources
        .loader(environment_map(sources), region, credentials)
        .load()
        .await;
    let response = Client::new(&config)
        .assume_role()
        .role_arn(role)
        .role_session_name(session)
        .send()
        .await
        .map_err(|source| RdsCredentialsError::AssumeRole {
            source: Box::new(source),
        })?;
    let temporary = response
        .credentials()
        .ok_or(RdsCredentialsError::StsWithoutCredentials)?;
    Ok(temporary_credentials(temporary))
}

pub(super) async fn ambient_credentials(
    region: &str,
    sources: &RdsSources<'_>,
) -> Result<Credentials, RdsCredentialsError> {
    let config = sources
        .loader(environment_map(sources), region, None)
        .load()
        .await;
    let provider = config
        .credentials_provider()
        .ok_or(RdsCredentialsError::NoCredentials)?;
    provider.provide_credentials().await.map_err(|source| {
        if matches!(source, CredentialsError::CredentialsNotLoaded(_)) {
            RdsCredentialsError::NoCredentials
        } else {
            RdsCredentialsError::Provider { source }
        }
    })
}

pub(super) async fn profile_credentials(
    profile: &str,
    region: &str,
    sources: &RdsSources<'_>,
) -> Result<Credentials, RdsCredentialsError> {
    let mut environment = environment_map(sources);
    // boto3 reads a named profile only, so the env pair never outranks the profile file
    for name in [
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "AWS_SESSION_TOKEN",
    ] {
        environment.remove(name);
    }
    let config = sources
        .loader(environment, region, None)
        .profile_name(profile.to_owned())
        .load()
        .await;
    let provider = config
        .credentials_provider()
        .ok_or(RdsCredentialsError::NoCredentials)?;
    provider
        .provide_credentials()
        .await
        .map_err(|source| RdsCredentialsError::Profile {
            profile: profile.to_owned(),
            source,
        })
}

fn temporary_credentials(temporary: &StsCredentials) -> Credentials {
    Credentials::new(
        temporary.access_key_id(),
        temporary.secret_access_key(),
        Some(temporary.session_token().to_owned()),
        None,
        CREDENTIAL_PROVIDER,
    )
}
