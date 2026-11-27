use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_cache::payload::PayloadError;
use betterlite_auth::auth_cache::{AuthCache, AuthCacheError, gateway_cache};
use betterlite_auth::trusted_network::request::NetworkRequest;
use betterlite_auth::trusted_network::settings::TrustedNetworkSettings;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};
use betterlite_config::sections::login_return::Settings as LoginSettings;
use secrecy::{ExposeSecret as _, SecretString};

use crate::login_return::cache::{LoginCaches, LoginCode, LoginCodeCache, LoginCodeCacheError};
use crate::login_return::redirect::{LoginRedirect, LoginReturnError, ResumeRequest, resume_login};
use crate::sso_pkce::VerifierEntropy;

pub(super) const SESSION_TOKEN: &str = "jwt.header.payload.signature";

pub(super) fn session_token() -> SecretString {
    SecretString::from(SESSION_TOKEN)
}

pub(super) fn settings(control_plane_url: Option<&str>) -> LoginSettings {
    LoginSettings {
        control_plane_url: control_plane_url.map(str::to_owned),
    }
}

pub(super) fn user_only<Cache>(user: &Cache) -> LoginCaches<'_, RecordingCache, Cache> {
    LoginCaches { redis: None, user }
}

pub(super) async fn resume_relative<Cache>(
    token: &SecretString,
    destination: &[u8],
    network: NetworkRequest<'_>,
    trusted: &TrustedNetworkSettings,
    user: &Cache,
) -> Result<Option<LoginRedirect>, LoginReturnError>
where
    Cache: LoginCodeCache,
{
    resume_login(
        ResumeRequest {
            destination: Some(destination),
            session_token: token,
            network,
        },
        &settings(None),
        trusted,
        user_only(user),
        &FixedEntropy,
    )
    .await
}

pub(super) async fn resume_absolute<Cache>(
    token: &SecretString,
    destination: &[u8],
    control_plane_url: &str,
    user: &Cache,
) -> Result<Option<LoginRedirect>, LoginReturnError>
where
    Cache: LoginCodeCache,
{
    resume_login(
        ResumeRequest {
            destination: Some(destination),
            session_token: token,
            network: NetworkRequest::default(),
        },
        &settings(Some(control_plane_url)),
        &TrustedNetworkSettings::default(),
        user_only(user),
        &FixedEntropy,
    )
    .await
}

#[derive(Default)]
pub(super) struct RecordingCache {
    writes: Mutex<Vec<RecordedWrite>>,
}

#[derive(Clone, Debug)]
pub(super) struct RecordedWrite {
    pub(super) key: String,
    pub(super) token: String,
    pub(super) redirect_url: String,
    pub(super) ttl_seconds: f64,
}

impl RecordingCache {
    pub(super) fn writes(&self) -> Vec<RecordedWrite> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl LoginCodeCache for RecordingCache {
    async fn store(
        &self,
        key: &str,
        value: &LoginCode,
        ttl_seconds: f64,
    ) -> Result<(), LoginCodeCacheError> {
        let write = RecordedWrite {
            key: key.to_owned(),
            token: value.token.expose_secret().to_owned(),
            redirect_url: value.redirect_url.clone(),
            ttl_seconds,
        };
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(write);
        Ok(())
    }
}

pub(super) struct FailingCache;

#[async_trait]
impl LoginCodeCache for FailingCache {
    async fn store(
        &self,
        _key: &str,
        _value: &LoginCode,
        _ttl_seconds: f64,
    ) -> Result<(), LoginCodeCacheError> {
        let source = serde_json::Error::io(std::io::Error::other("cache unavailable"));
        Err(LoginCodeCacheError::Local(AuthCacheError::Payload(
            Box::new(PayloadError::Json(source)),
        )))
    }
}

pub(super) type LocalLoginCache = AuthCache<DualCache<LocalMemoryOps>>;

pub(super) fn local_cache(clock: Arc<dyn Fn() -> f64 + Send + Sync>) -> LocalLoginCache {
    let memory = Arc::new(LocalMemoryOps::new(1024, clock).unwrap());
    gateway_cache(
        Arc::clone(&memory),
        memory,
        None,
        DualCacheSettings {
            default_in_memory_ttl: None,
            default_redis_ttl: None,
            batch_cache_expiry: None,
            max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
            redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
        },
        45.0_f64,
    )
}

// "name=value; Attribute=value" split into its parts, so a test compares attributes and not clock bytes
pub(super) fn cookie_attributes(header: &str) -> Vec<(String, String)> {
    header
        .split("; ")
        .map(|part| match part.split_once('=') {
            Some((name, value)) => (name.to_owned(), value.to_owned()),
            None => (part.to_owned(), String::new()),
        })
        .collect()
}

pub(super) fn attribute<'attributes>(
    attributes: &'attributes [(String, String)],
    name: &str,
) -> Option<&'attributes str> {
    attributes
        .iter()
        .find(|(key, _value)| key == name)
        .map(|(_key, value)| value.as_str())
}

pub(super) fn code_from_location(location: &str) -> String {
    location
        .split("code=")
        .nth(1)
        .expect("the redirect location carries the login code")
        .to_owned()
}

pub(super) struct FixedEntropy;

impl VerifierEntropy for FixedEntropy {
    fn fill(&self, bytes: &mut [u8]) -> Result<(), getrandom::Error> {
        bytes.fill(0xAB);
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct CountingEntropy(pub(super) AtomicUsize);

impl VerifierEntropy for CountingEntropy {
    fn fill(&self, bytes: &mut [u8]) -> Result<(), getrandom::Error> {
        self.0.fetch_add(1, Ordering::Relaxed);
        bytes.fill(0x11);
        Ok(())
    }
}

pub(super) struct FailingEntropy;

impl VerifierEntropy for FailingEntropy {
    fn fill(&self, _bytes: &mut [u8]) -> Result<(), getrandom::Error> {
        Err(getrandom::Error::UNSUPPORTED)
    }
}
