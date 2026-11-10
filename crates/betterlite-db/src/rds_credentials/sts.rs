use std::borrow::Cow;
use std::collections::HashMap;
use std::time::Duration;

use aws_config::profile::load as load_profiles;
use aws_config::timeout::TimeoutConfig;
use aws_config::{BehaviorVersion, ConfigLoader};
use aws_credential_types::Credentials;
use aws_credential_types::provider::ProvideCredentials as _;
use aws_credential_types::provider::error::CredentialsError;
use aws_runtime::env_config::file::EnvConfigFiles;
use aws_sdk_sts::Client;
use aws_sdk_sts::types::Credentials as StsCredentials;
use aws_types::os_shim_internal::Env;
use aws_types::region::Region;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::RdsCredentialsError;
use super::{CREDENTIAL_PROVIDER, RdsSources, WEB_IDENTITY_DURATION_SECONDS};

// botocore's default connect and read timeout is 60 s, and the configured RDS timeout never reaches STS
const STS_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) enum StsRequest<'request> {
    Role {
        role: &'request str,
        session: &'request str,
    },
    WebIdentity {
        role: &'request str,
        session: &'request str,
        token: &'request SecretString,
    },
}

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

pub(super) async fn assume_role(
    request: StsRequest<'_>,
    explicit: Option<Credentials>,
    region: &str,
    sources: &RdsSources<'_>,
) -> Result<Credentials, RdsCredentialsError> {
    let config = sources
        .loader(environment_map(sources), region, explicit)
        .load()
        .await;
    let returned = send_sts_request(&Client::new(&config), request).await?;
    temporary_credentials(returned.as_ref())
}

fn temporary_credentials(
    returned: Option<&StsCredentials>,
) -> Result<Credentials, RdsCredentialsError> {
    let temporary = returned.ok_or(RdsCredentialsError::StsWithoutCredentials)?;
    if temporary.access_key_id().is_empty() {
        return Err(RdsCredentialsError::StsMissingField {
            field: "AccessKeyId",
        });
    }
    if temporary.secret_access_key().is_empty() {
        return Err(RdsCredentialsError::StsMissingField {
            field: "SecretAccessKey",
        });
    }
    if temporary.session_token().is_empty() {
        return Err(RdsCredentialsError::StsMissingField {
            field: "SessionToken",
        });
    }
    Ok(Credentials::new(
        temporary.access_key_id(),
        temporary.secret_access_key(),
        Some(temporary.session_token().to_owned()),
        None,
        CREDENTIAL_PROVIDER,
    ))
}

async fn send_sts_request(
    client: &Client,
    request: StsRequest<'_>,
) -> Result<Option<StsCredentials>, RdsCredentialsError> {
    match request {
        StsRequest::Role { role, session } => Ok(client
            .assume_role()
            .role_arn(role)
            .role_session_name(session)
            .send()
            .await
            .map_err(|source| RdsCredentialsError::AssumeRole {
                source: Box::new(source),
            })?
            .credentials()
            .cloned()),
        StsRequest::WebIdentity {
            role,
            session,
            token,
        } => Ok(client
            .assume_role_with_web_identity()
            .role_arn(role)
            .role_session_name(session)
            .web_identity_token(token.expose_secret())
            .duration_seconds(WEB_IDENTITY_DURATION_SECONDS)
            .send()
            .await
            .map_err(|source| RdsCredentialsError::AssumeRoleWithWebIdentity {
                source: Box::new(source),
            })?
            .credentials()
            .cloned()),
    }
}

pub(super) async fn provider_credentials(
    profile: Option<&str>,
    region: &str,
    sources: &RdsSources<'_>,
) -> Result<Credentials, RdsCredentialsError> {
    let mut environment = environment_map(sources);
    if let Some(name) = profile {
        // boto3 raises ProfileNotFound and never lets the ambient chain sign for a typo
        verify_profile(name, &environment, sources).await?;
        for variable in [
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_SESSION_TOKEN",
        ] {
            environment.remove(variable);
        }
    }
    let mut loader = sources.loader(environment, region, None);
    if let Some(name) = profile {
        loader = loader.profile_name(name.to_owned());
    }
    let config = loader.load().await;
    let provider = config
        .credentials_provider()
        .ok_or(RdsCredentialsError::NoCredentials)?;
    let resolved = provider.provide_credentials().await;
    if let Some(name) = profile {
        return resolved.map_err(|error| RdsCredentialsError::Profile {
            profile: name.to_owned(),
            source: error,
        });
    }
    resolved.map_err(|error| {
        if matches!(error, CredentialsError::CredentialsNotLoaded(_)) {
            RdsCredentialsError::NoCredentials
        } else {
            RdsCredentialsError::Provider { source: error }
        }
    })
}

async fn verify_profile(
    name: &str,
    environment: &HashMap<String, String>,
    sources: &RdsSources<'_>,
) -> Result<(), RdsCredentialsError> {
    let profiles = load_profiles(
        &sources.filesystem,
        &Env::from(environment.clone()),
        &EnvConfigFiles::default(),
        Some(Cow::Owned(name.to_owned())),
    )
    .await
    .map_err(|source| RdsCredentialsError::ProfileFile {
        profile: name.to_owned(),
        source,
    })?;
    if profiles.get_profile(name).is_none() {
        return Err(RdsCredentialsError::MissingProfile {
            profile: name.to_owned(),
        });
    }
    Ok(())
}
