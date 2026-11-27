use crate::login_return::cache::{LoginCaches, LoginCode, LoginCodeCache, LoginCodeCacheError};
use crate::login_return::redirect::{LoginRedirect, LoginReturnError, ResumeRequest, resume_login};
use crate::sso_pkce::VerifierEntropy;
use async_trait::async_trait;
use betterlite_auth::auth_cache::payload::PayloadError;
use betterlite_auth::auth_cache::{AuthCache, AuthCacheError, gateway_cache};
use betterlite_auth::trusted_network::request::NetworkRequest;
use betterlite_auth::trusted_network::settings::TrustedNetworkSettings;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};
use betterlite_config::sections::login_return::Settings as LoginSettings;
use secrecy::{ExposeSecret as _, SecretString};
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
pub(super) const SESSION_TOKEN: &str = "jwt.header.payload.signature";
pub(super) fn session_token() -> SecretString {
    SecretString::from(SESSION_TOKEN)
}
pub(super) fn settings(control_plane_url: Option<&str>) -> LoginSettings {
    LoginSettings {
        control_plane_url: control_plane_url.map(str::to_owned),
    }
}
pub(super) const fn user_only<Cache>(user: &Cache) -> LoginCaches<'_, RecordingCache, Cache> {
    LoginCaches { redis: None, user }
}
pub(super) async fn resume<Cache>(
    request: ResumeRequest<'_>,
    settings: &LoginSettings,
    trusted: &TrustedNetworkSettings,
    user: &Cache,
) -> Result<Option<LoginRedirect>, LoginReturnError>
where
    Cache: LoginCodeCache,
{
    resume_login(request, settings, trusted, user_only(user), &FixedEntropy).await
}
pub(super) const fn plain_network(scheme: &str) -> NetworkRequest<'_> {
    NetworkRequest {
        peer: None,
        x_forwarded_for: None,
        x_forwarded_proto: None,
        host: None,
        scheme,
    }
}
pub(super) const fn forwarded_network<'request>(
    peer: &'request str,
    x_forwarded_proto: &'request str,
    scheme: &'request str,
) -> NetworkRequest<'request> {
    NetworkRequest {
        peer: Some(peer),
        x_forwarded_for: None,
        x_forwarded_proto: Some(x_forwarded_proto),
        host: None,
        scheme,
    }
}
pub(super) fn trusted_proxy_settings() -> TrustedNetworkSettings {
    TrustedNetworkSettings {
        proxy_base_url: String::new(),
        use_x_forwarded_for: true,
        trusted_proxy_ranges: None,
        mcp_trusted_proxy_ranges: Some(vec!["10.0.0.0/8".to_owned()]),
        mcp_xff_num_trusted_hops: None,
    }
}
pub(super) fn public_base_url(base_url: &str) -> TrustedNetworkSettings {
    TrustedNetworkSettings {
        proxy_base_url: base_url.to_owned(),
        use_x_forwarded_for: false,
        trusted_proxy_ranges: None,
        mcp_trusted_proxy_ranges: None,
        mcp_xff_num_trusted_hops: None,
    }
}
#[derive(Default)]
pub(super) struct RecordingCache {
    writes: Mutex<Vec<RecordedWrite>>,
    fails: bool,
}
#[derive(Clone, Debug)]
pub(super) struct RecordedWrite {
    pub(super) key: String,
    pub(super) token: String,
    pub(super) redirect_url: String,
    pub(super) ttl_seconds: f64,
}
impl RecordingCache {
    pub(super) const fn failing() -> Self {
        Self {
            writes: Mutex::new(Vec::new()),
            fails: true,
        }
    }
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
        if self.fails {
            let source = serde_json::Error::io(io::Error::other("cache unavailable"));
            return Err(LoginCodeCacheError::Local(AuthCacheError::Payload(
                Box::new(PayloadError::Json(source)),
            )));
        }
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
