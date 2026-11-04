use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};

use super::credential_cache::CachedCredentials;

const STATIC_CREDENTIAL_TTL: Duration = Duration::from_secs(3540);

#[derive(Default)]
pub struct LocalCredentialInputs {
    pub profile_name: Option<String>,
    pub role_name: Option<String>,
    pub web_identity_token: Option<SecretString>,
    pub session_name: Option<String>,
    pub access_key_id: Option<SecretString>,
    pub secret_access_key: Option<SecretString>,
    pub session_token: Option<SecretString>,
    pub region_name: Option<String>,
}

pub struct ResolvedCredentials {
    pub credentials: CachedCredentials,
    pub cache_ttl: Option<Duration>,
}

pub trait LocalCredentialProvider {
    type Error;

    fn profile(&self, profile_name: &str) -> Result<CachedCredentials, Self::Error>;
    fn static_session(&self, region: &str) -> Result<CachedCredentials, Self::Error>;
    fn ambient_session(&self) -> Result<CachedCredentials, Self::Error>;
}

pub fn resolve_local_credentials<P: LocalCredentialProvider>(
    inputs: &LocalCredentialInputs,
    provider: &P,
) -> Result<ResolvedCredentials, P::Error> {
    if let Some(profile_name) = inputs.profile_name.as_deref() {
        return provider
            .profile(profile_name)
            .map(|credentials| ResolvedCredentials {
                credentials,
                cache_ttl: None,
            });
    }
    if let (Some(access_key_id), Some(secret_access_key), Some(session_token)) = (
        inputs.access_key_id.as_ref(),
        inputs.secret_access_key.as_ref(),
        inputs.session_token.as_ref(),
    ) {
        return Ok(ResolvedCredentials {
            credentials: CachedCredentials {
                access_key_id: SecretString::from(access_key_id.expose_secret().to_owned()),
                secret_access_key: SecretString::from(secret_access_key.expose_secret().to_owned()),
                session_token: Some(SecretString::from(session_token.expose_secret().to_owned())),
            },
            cache_ttl: None,
        });
    }
    if let (Some(_), Some(_), Some(region)) = (
        inputs.access_key_id.as_ref(),
        inputs.secret_access_key.as_ref(),
        inputs.region_name.as_deref(),
    ) {
        return provider
            .static_session(region)
            .map(|credentials| ResolvedCredentials {
                credentials,
                cache_ttl: Some(STATIC_CREDENTIAL_TTL),
            });
    }
    provider
        .ambient_session()
        .map(|credentials| ResolvedCredentials {
            credentials,
            cache_ttl: None,
        })
}

pub fn auth_selector_presence(inputs: &LocalCredentialInputs) -> (bool, bool, bool, bool, bool) {
    (
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
    )
}
